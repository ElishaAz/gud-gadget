use clap::Parser;
use display::Display;
use gud_gadget::{DisplayMode, Event, PixelFormat};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{
    str::FromStr,
    sync::atomic::{AtomicBool, Ordering},
};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use usb_gadget::function::custom::{Custom, Interface};
use usb_gadget::{default_udc, Class, Config, Gadget, Strings};

mod backends;
mod colors;
mod display;

#[derive(Debug, Clone, Copy)]
struct VideoMode {
    width: u32,
    height: u32,
    framerate: f32,
}

impl FromStr for VideoMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Expect WIDTHxHEIGHT@FRAMERATE
        let parts: Vec<&str> = s.split('@').collect();
        if parts.len() != 2 {
            return Err(format!("invalid format, expected WIDTHxHEIGHT@FRAMERATE"));
        }
        let framerate: f32 = parts[1]
            .parse()
            .map_err(|_| "invalid framerate".to_string())?;
        let dims: Vec<&str> = parts[0].split('x').collect();

        if dims.len() != 2 {
            return Err(format!("invalid WIDTHxHEIGHT format"));
        }

        let width: u32 = dims[0].parse().map_err(|_| "invalid WIDTH".to_string())?;
        let height: u32 = dims[1].parse().map_err(|_| "invalid height".to_string())?;

        Ok(Self {
            width,
            height,
            framerate,
        })
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    mode: VideoMode,
    #[arg(short, long, default_value_t=PixelFormat::RGB565)]
    format: PixelFormat,
    #[arg(
        short,
        long,
        default_value = "default",
        help = "The rendering backend",
        hide_default_value = true
    )]
    backend: backends::BackendNames,
    #[arg(
        short,
        long,
        help = "Disable lookup table for pixel format conversions"
    )]
    no_lut: bool,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    let udc = default_udc().expect("no UDC found");

    usb_gadget::remove_all().expect("UDC init failed");

    let (mut gud_data, gud_data_ep) = gud_gadget::PixelDataEndpoint::new();
    let (mut gud, gud_handle) = Custom::builder()
        .with_interface(
            Interface::new(Class::vendor_specific(Class::VENDOR_SPECIFIC, 0), "GUD")
                .with_endpoint(gud_data_ep),
        )
        .build();

    let _reg = Gadget::new(
        Class::interface_specific(),
        gud_gadget::OPENMOKO_GUD_ID,
        Strings::new("The Internet", "Generic USB Display", ""),
    )
    .with_config(Config::new("gud").with_function(gud_handle))
    .bind(&udc)
    .expect("UDC binding failed");

    let running = Arc::new(AtomicBool::new(true));

    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .expect("cleanup handler registration failed");

    let backend = backends::create_backend(
        args.backend,
        args.mode.width as usize,
        args.mode.height as usize,
        args.mode.framerate,
    );

    let mut display = Display::new(
        args.mode.width as usize,
        args.mode.height as usize,
        args.mode.framerate,
        args.format,
        backend,
        !args.no_lut,
    );

    // This buffer receives the frames
    let mut buffer = vec![
        0u8;
        (args.mode.width as f32 * args.mode.height as f32 * args.format.bpp() as f32 / 8.0).ceil()
            as usize
    ];
    println!(
        "Buffer size: {} (width: {}, height: {}, bpp: {})",
        buffer.len(),
        args.mode.width,
        args.mode.height,
        args.format.bpp()
    );

    let mut last_window_update = Instant::now();

    let mut frames = 0;
    let mut last_print = Instant::now();

    while running.load(Ordering::Relaxed) {
        if display.check_close() {
            // Window closed
            running.store(false, Ordering::SeqCst);
        }

        if Instant::now()
            .duration_since(last_window_update)
            .as_millis()
            > 1000
        {
            // Update the display at least once a second (gud does not refresh if there are no changes)
            display.display_buf(&buffer);
            last_window_update = Instant::now();
        }

        let event = gud
            .event_timeout(Duration::from_millis(100))
            .expect("read GUD event");
        if event.is_none() {
            continue;
        }
        let event = event.unwrap();

        if let Ok(Some(gud_event)) = gud_gadget::event(event) {
            match gud_event {
                Event::GetDescriptor(req) => {
                    println!(
                        "Sending display descriptor. Width: {}, Height: {}",
                        args.mode.width, args.mode.height
                    );
                    req.send_descriptor(
                        args.mode.width,
                        args.mode.height,
                        args.mode.width,
                        args.mode.height,
                    )
                    .expect("failed to send descriptor");
                }
                Event::GetPixelFormats(req) => {
                    println!("Sending pixel format ({})", args.format);
                    req.send_pixel_formats(&[args.format]).unwrap()
                }
                Event::GetDisplayModes(req) => {
                    let mode = DisplayMode::from_res(
                        args.mode.width,
                        args.mode.height,
                        args.mode.framerate,
                        true,
                    );
                    let modes = [mode];

                    println!("Sending display modes: {:?}", modes);

                    req.send_modes(&modes).expect("failed to send modes");
                }
                Event::Buffer(info) => {
                    frames += 1;
                    // println!("Got a frame: {:?}", info);
                    gud_data
                        .recv_buffer(info, &mut buffer)
                        .expect("recv_buffer failed");
                    display.display_buf(&buffer);
                    last_window_update = Instant::now();
                }
            }
        }

        if last_print.elapsed() > Duration::from_secs(1) {
            last_print = Instant::now();
            println!("FPS: {}", frames);
            frames = 0;
        }
    }

    Ok(())
}
