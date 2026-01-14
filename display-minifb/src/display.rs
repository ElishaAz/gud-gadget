use gud_gadget::PixelFormat;
use minifb::{Window, WindowOptions};
use std::time::Instant;

pub struct Display {
    width: usize,
    height: usize,
    pix_format: PixelFormat,
    lut: Vec<u32>,
    use_lut: bool,
    buffer: Vec<u32>,
    window: Window,
}

impl Display {
    pub fn new(
        width: usize,
        height: usize,
        framerate: f32,
        pix_format: PixelFormat,
        use_lut: bool,
    ) -> Self {
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

        let lut = if use_lut {
            Self::create_lut(pix_format)
        } else {
            vec![]
        };

        let buffer = Vec::with_capacity(width * height * 8 / pix_format.bpp());

        Self {
            width,
            height,
            pix_format,
            lut,
            use_lut,
            buffer,
            window,
        }
    }

    fn create_lut(pix_format: gud_gadget::PixelFormat) -> Vec<u32> {
        match pix_format {
            PixelFormat::R1 => return vec![],
            PixelFormat::R8 => {
                let mut output = Vec::with_capacity(256);
                for i in 0..256u32 {
                    output.push((i << 16) | (i << 8) | i)
                }
                return output;
            }
            PixelFormat::XRGB1111 => {
                let mut output = Vec::with_capacity(256);
                for i in 0..256u32 {
                    let val = (i >> 4) & 0x0F;
                    let r = (val & 0b0100) << 5;
                    let g = (val & 0b0010) << 6;
                    let b = (val & 0b0001) << 7;

                    output.push((r << 16) | (g << 8) | b);

                    let val = i & 0x0F;
                    let r = (val & 0b0100) << 5;
                    let g = (val & 0b0010) << 6;
                    let b = (val & 0b0001) << 7;

                    output.push((r << 16) | (g << 8) | b);
                }
                return output;
            }
            PixelFormat::RGB332 => {
                let mut output = Vec::with_capacity(256);
                for i in 0..256u32 {
                    let r = i & 0b11100000;
                    let g = (i << 3) & 0b11100000;
                    let b = (i << 6) & 0b11000000;
                    output.push((r << 16) | (g << 8) | b)
                }
                return output;
            }
            PixelFormat::RGB565 => {
                let mut output = Vec::with_capacity(u16::MAX as usize + 1usize);

                for i in 0..(u16::MAX as u32 + 1) {
                    let r = (i >> 8) & 0b11111000;
                    let g = (i >> 3) & 0b11111100;
                    let b = (i << 3) & 0b11111000;
                    output.push((r << 16) | (g << 8) | b);
                }

                return output;
            }
            PixelFormat::RGB888 => todo!(),
            PixelFormat::XRGB8888 => todo!(),
            PixelFormat::ARGB8888 => todo!(),
        }
    }

    fn convert_buffer(pix_format: PixelFormat, input: &[u8], dest: &mut Vec<u32>) {
        let num_pixels = input.len() * 8 / pix_format.bpp();

        dest.clear();

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
            dest.push(pixel);
        }
    }

    pub fn convert_with_lut(
        pix_format: PixelFormat,
        input: &[u8],
        lut: &[u32],
        dest: &mut Vec<u32>,
    ) {
        dest.clear();

        match pix_format.bpp() {
            1 => {
                // Can't use a lookup table
                for i in input {
                    for j in 0..8 {
                        dest.push(0x00FFFFFF * ((i >> j) & 0x01) as u32);
                    }
                }
            }
            4 => {
                for i in input {
                    dest.push(lut[(i & 0xFF) as usize]);
                    dest.push(lut[((i >> 4) & 0xFF) as usize]);
                }
            }
            8 => {
                for i in input {
                    dest.push(lut[*i as usize]);
                }
            }
            16 => {
                let input: &[u16] = bytemuck::cast_slice(input);
                for i in input {
                    dest.push(lut[*i as usize]);
                }
            }
            // 24 => {}
            // 32 => {}
            _ => panic!("Unsupported pixel format!"),
        };
    }

    pub fn display_buf(&mut self, input: &[u8]) {
        let rgba_buffer: &[u32] = match self.pix_format {
            PixelFormat::XRGB8888 | PixelFormat::ARGB8888 => bytemuck::cast_slice(input),
            _ => &{
                let start = Instant::now();
                if self.use_lut {
                    Self::convert_with_lut(self.pix_format, input, &self.lut, &mut self.buffer);
                } else {
                    Self::convert_buffer(self.pix_format, input, &mut self.buffer);
                }
                println!(
                    "Conversion took: {}ms",
                    Instant::now().duration_since(start).as_millis()
                );

                &self.buffer
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
