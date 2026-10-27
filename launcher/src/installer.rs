//! The install backend of the Linux launcher: `setup.sh` in the game's source folder.
use crate::engine::{Engine, EngineOpts};

pub trait InstallBackend: Send + Sync {
    fn name(&self) -> &'static str;
    /// Can this system set the game up from a disc at all?
    fn supported(&self) -> bool;
    fn start(&self, opts: &EngineOpts) -> std::io::Result<Engine>;
}

pub struct LinuxBackend;

impl InstallBackend for LinuxBackend {
    fn name(&self) -> &'static str {
        "setup.sh"
    }
    fn supported(&self) -> bool {
        true
    }
    fn start(&self, opts: &EngineOpts) -> std::io::Result<Engine> {
        Engine::start(opts)
    }
}

pub fn backend() -> Box<dyn InstallBackend> {
    Box::new(LinuxBackend)
}

/// Whether this launcher can install a game from a disc yet.
pub fn supported() -> bool {
    backend().supported()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backend_is_the_linux_setup() {
        assert!(supported());
        assert_eq!(backend().name(), "setup.sh");
        let o = EngineOpts::default();
        assert!(backend().start(&o).is_err(), "no setup.sh in an empty folder");
    }
}
