use gud_gadget::PixelFormat;
use std::time::Instant;

use crate::{backends::Backend, colors::Colors};

pub struct Display {
    colors: Colors,
    use_lut: bool,
    buffer: Vec<u32>,
    backend: Box<dyn Backend>,
}

impl Display {
    pub fn new(
        width: usize,
        height: usize,
        framerate: f32,
        pix_format: PixelFormat,
        mut backend: Box<dyn Backend>,
        use_lut: bool,
    ) -> Self {
        backend.init(width, height, framerate);

        let colors = Colors::new(pix_format);

        let buffer = Vec::with_capacity(width * height * 8 / pix_format.bpp());

        Self {
            colors,
            use_lut,
            buffer,
            backend,
        }
    }

    pub fn display_buf(&mut self, input: &[u8]) {
        let start = Instant::now();
        let buffer = self.colors.convert(input, &mut self.buffer, self.use_lut);
        tracing::debug!("Color conversion took {} us", start.elapsed().as_micros());

        self.backend.display(buffer);
    }

    pub fn check_close(&mut self) -> bool {
        return self.backend.check_close();
    }
}
