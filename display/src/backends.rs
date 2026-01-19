use clap::ValueEnum;

#[cfg(feature = "minifb")]
mod minifb;

#[allow(dead_code, unused_variables)]
pub trait Backend {
    fn init(&mut self, width: usize, height: usize, framerate: f32) {}
    fn display(&mut self, buffer: &[u32]);
    fn check_close(&mut self) -> bool {
        return false;
    }
    fn update(&mut self) {}
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum BackendNames {
    Default,
    #[cfg(feature = "minifb")]
    MiniFB,
}

pub fn create_backend(
    backend: BackendNames,
    width: usize,
    height: usize,
    framerate: f32,
) -> Box<dyn Backend> {
    match backend {
        #[cfg(feature = "minifb")]
        BackendNames::Default | BackendNames::MiniFB => {
            Box::new(minifb::MinifbBackend::new(width, height, framerate))
        }
    }
}
