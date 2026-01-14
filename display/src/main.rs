use gud_gadget::{DisplayMode, Event};
use minifb::{Window, WindowOptions};
use std::env::args;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use usb_gadget::function::custom::{Custom, Interface};
use usb_gadget::{default_udc, Class, Config, Gadget, Strings};

fn rgb565_to_rgba8888(input: &[u8]) -> Vec<u32> {
    assert!(input.len() % 2 == 0, "RGB565 buffer length must be even");

    let num_pixels = input.len() / 2;
    let mut output = Vec::with_capacity(num_pixels);

    for i in 0..num_pixels {
        let lo = input[2 * i] as u16;
        let hi = input[2 * i + 1] as u16;
        let raw = (hi << 8) | lo;

        // Extract 5/6/5 bits
        let r5 = ((raw >> 11) & 0x1F) as u8;
        let g6 = ((raw >> 5) & 0x3F) as u8;
        let b5 = (raw & 0x1F) as u8;

        // Expand to 8 bits per channel:
        // replicate high bits into low bits for smoother mapping
        let r8 = (r5 << 3) | (r5 >> 2);
        let g8 = (g6 << 2) | (g6 >> 4);
        let b8 = (b5 << 3) | (b5 >> 2);

        // Pack as 0xAARRGGBB (opaque alpha = 0xFF)
        let pixel = (0xFFu32 << 24) | ((r8 as u32) << 16) | ((g8 as u32) << 8) | (b8 as u32);
        output.push(pixel);
    }

    output
}

fn display_buf(window: &mut Window, buffer: &[u8], width: u32, height: u32) -> () {
    let rgba_buffer = rgb565_to_rgba8888(buffer);
    // let rgba_buffer = vec![0xFFFFFFFFu32; buffer.len() / 2];

    // for i in (0..buffer.len()).step_by(100) {
    //     let sum = buffer[i..i + 100].iter().sum::<u8>();
    //     if sum == 0 {
    //         continue;
    //     }
    //     println!("{}: Sum {}", i, sum);
    // }

    window
        .update_with_buffer(&rgba_buffer, width as usize, height as usize)
        .unwrap();
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    let res = args()
        .skip(1)
        .next()
        .expect("specify resolution in the format WIDTHxHEIGHT@FRAMERATE");

    let x_idx = res.find("x");
    let at_idx = res.find("@");

    let (width, height, framerate) = match (x_idx, at_idx) {
        (Some(x), Some(at)) => {
            let width = res[0..x].parse::<u32>().expect("invalid width");
            let height = res[x + 1..at].parse::<u32>().expect("invalid height");
            let framerate = res[at + 1..].parse::<f32>().expect("invalid framerate");
            (width, height, framerate)
        }
        (_, _) => {
            panic!("Not a valid resolution: {}", res);
        }
    };

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

    let mut window = Window::new(
        "GUD Display",
        width as usize,
        height as usize,
        WindowOptions::default(),
    )
    .unwrap_or_else(|e| {
        panic!("{}", e);
    });

    window.set_target_fps(framerate.round() as usize);

    // This buffer receives the frames
    let mut buffer = vec![0u8; (width * height * 2) as usize];

    let mut last_window_update = Instant::now();

    let mut frames = 0;
    let mut last_print = Instant::now();

    while running.load(Ordering::Relaxed) {
        if !window.is_open() {
            // Window closed
            break;
        }

        if Instant::now()
            .duration_since(last_window_update)
            .as_millis()
            > 1000
        {
            // Update the display at least once a second (gud does not refresh if there are no changes)
            display_buf(&mut window, &buffer, width, height);
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
                        width, height
                    );
                    req.send_descriptor(width, height, width, height)
                        .expect("failed to send descriptor");
                }
                Event::GetPixelFormats(req) => {
                    println!("Sending pixel format (RGB565)");
                    req.send_pixel_formats(&[gud_gadget::PixelFormat::RGB565])
                        .unwrap()
                }
                Event::GetDisplayModes(req) => {
                    let mode = DisplayMode::from_res(width, height, framerate, true);
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
                    display_buf(&mut window, &buffer, width, height);
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
