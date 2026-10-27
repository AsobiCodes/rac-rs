//! The install backend of the Linux launcher.
//!
//! The Windows launcher ran the PowerShell worker (`setup.ps1`, archived in
//! `windows/engine_windows.rs`). The Linux engine (`setup.sh --iso <iso>`) comes
//! in a later version; until then the launcher manages installs that are
//! already set up with `setup.sh`.
use crate::engine::{Engine, EngineOpts};

pub trait InstallBackend: Send + Sync {
    fn name(&self) -> &'static str;
    /// Can this system set the game up from a disc at all?
    fn supported(&self) -> bool;
    fn start(&self, opts: &EngineOpts) -> std::io::Result<Engine>;
}

pub const UNSUPPORTED_TEXT: &str = crate::engine::UNSUPPORTED_TEXT;

pub struct LinuxBackend;

impl InstallBackend for LinuxBackend {
    fn name(&self) -> &'static str {
        "setup.sh (later version)"
    }
    fn supported(&self) -> bool {
        false
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
    fn linux_backend_says_install_comes_later() {
        let o = EngineOpts::default();
        let e = backend().start(&o).err().unwrap();
        assert_eq!(e.kind(), std::io::ErrorKind::Unsupported);
        assert!(e.to_string().contains("setup.sh"));
        assert!(!supported());
    }
}
