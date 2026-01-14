use gud_gadget::{DisplayMode, Event};
use std::env::args;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use usb_gadget::function::custom::{Custom, Interface};
use usb_gadget::{default_udc, Class, Config, Gadget, Strings};

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

    // This buffer receives the frames. We don't currently do anything with them.
    let mut buffer = vec![0u8; (width * height * 2) as usize];

    let mut frames = 0;
    let mut last_print = Instant::now();

    while running.load(Ordering::Relaxed) {
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
