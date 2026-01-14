use gud_gadget::PixelFormat;
use minifb::{Window, WindowOptions};
use std::time::Instant;

pub struct Display {
    width: usize,
    height: usize,
    pix_format: PixelFormat,
    window: Window,
}

impl Display {
    pub fn new(width: usize, height: usize, framerate: f32, pix_format: PixelFormat) -> Self {
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

        Self {
            width,
            height,
            pix_format,
            window,
        }
    }

    fn convert_buffer(pix_format: gud_gadget::PixelFormat, input: &[u8]) -> Vec<u32> {
        let num_pixels = (input.len() as f32 / pix_format.bps()).round() as usize;
        let mut output = Vec::with_capacity(num_pixels);

        for i in 0..num_pixels {
            let mut a = 0xFFu8;
            let r: u8;
            let g: u8;
            let b: u8;

            match pix_format {
                PixelFormat::R1 => {
                    let byte = input[i / 8];
                    let val = (byte >> (i % 8) & 0b1) * 0xFF;
                    r = val;
                    g = val;
                    b = val;
                }
                PixelFormat::R8 => {
                    let val = input[i];
                    r = val;
                    g = val;
                    b = val;
                }
                PixelFormat::XRGB1111 => {
                    let byte = input[i / 2];
                    let val = (byte >> ((i % 2) * 4)) & 0x0F;
                    r = (val & 0b0100) << 5;
                    g = (val & 0b0010) << 6;
                    b = (val & 0b0001) << 7;
                }
                PixelFormat::RGB332 => {
                    let byte = input[i];
                    r = byte & 0b11100000;
                    g = (byte << 3) & 0b11100000;
                    b = (byte << 6) & 0b11000000;
                }
                PixelFormat::RGB565 => {
                    let lo = input[2 * i] as u16;
                    let hi = input[2 * i + 1] as u16;
                    let raw = (hi << 8) | lo;

                    r = ((raw >> 8) & 0b11111000) as u8;
                    g = ((raw >> 3) & 0b11111100) as u8;
                    b = ((raw << 3) & 0b11111000) as u8;
                }
                PixelFormat::RGB888 => {
                    // lsb
                    b = input[3 * i];
                    g = input[3 * i + 1];
                    r = input[3 * i + 2];
                }
                PixelFormat::XRGB8888 => {
                    // lsb
                    b = input[4 * i + 1];
                    g = input[4 * i + 2];
                    r = input[4 * i + 3];
                }
                PixelFormat::ARGB8888 => {
                    // lsb
                    a = input[4 * i];
                    b = input[4 * i + 1];
                    g = input[4 * i + 2];
                    r = input[4 * i + 3];
                }
            }

            // Pack as 0xAARRGGBB (opaque alpha = 0xFF)
            let pixel = ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
            output.push(pixel);
        }

        output
    }

    pub fn display_buf(&mut self, buffer: &[u8]) {
        let rgba_buffer: &[u32] = match self.pix_format {
            PixelFormat::XRGB8888 | PixelFormat::ARGB8888 => bytemuck::cast_slice(buffer),
            _ => &{
                let start = Instant::now();
                let res = Self::convert_buffer(self.pix_format, buffer);
                println!(
                    "Conversion took: {}ms",
                    Instant::now().duration_since(start).as_millis()
                );

                res
            },
        };

        self.window
            .update_with_buffer(&rgba_buffer, self.width, self.height)
            .unwrap();
    }

    pub fn check_close(&mut self) -> bool {
        return !self.window.is_open();
    }
}
