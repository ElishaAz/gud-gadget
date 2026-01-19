use minifb::{Window, WindowOptions};

pub struct MinifbBackend {
    width: usize,
    height: usize,
    window: Window,
}

impl MinifbBackend {
    pub fn new(width: usize, height: usize, framerate: f32) -> Self {
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
            window,
        }
    }
}

impl super::Backend for MinifbBackend {
    fn display(&mut self, buffer: &[u32]) {
        self.window
            .update_with_buffer(&buffer, self.width, self.height)
            .unwrap();
    }

    fn check_close(&mut self) -> bool {
        return !self.window.is_open();
    }
}
