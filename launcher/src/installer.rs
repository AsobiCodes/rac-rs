//! The install engine behind a trait, so a non-Windows build can compile and say plainly what is missing.
//! Windows: the PowerShell worker (`setup.ps1`, see engine.rs). Other systems: a clear "later version" message (no Linux install engine yet).
use crate::engine::{Engine, EngineOpts};
use std::io::{Error, ErrorKind};

pub trait InstallBackend: Send + Sync {
    fn name(&self) -> &'static str;
    /// Can this system set the game up from a disc at all?
    fn supported(&self) -> bool;
    fn start(&self, opts: &EngineOpts) -> std::io::Result<Engine>;
}

/// Windows: `powershell -File setup.ps1 ...` as a hidden child.
pub struct PowerShellBackend;

impl InstallBackend for PowerShellBackend {
    fn name(&self) -> &'static str {
        "PowerShell (setup.ps1)"
    }
    fn supported(&self) -> bool {
        true
    }
    fn start(&self, opts: &EngineOpts) -> std::io::Result<Engine> {
        Engine::start(opts)
    }
}

pub const UNSUPPORTED_TEXT: &str = "Installing on Linux comes in a later version of the launcher.";

/// Everything that is not Windows yet.
pub struct UnsupportedBackend;

impl InstallBackend for UnsupportedBackend {
    fn name(&self) -> &'static str {
        "none"
    }
    fn supported(&self) -> bool {
        false
    }
    fn start(&self, _opts: &EngineOpts) -> std::io::Result<Engine> {
        Err(Error::new(ErrorKind::Unsupported, UNSUPPORTED_TEXT))
    }
}

pub fn backend() -> Box<dyn InstallBackend> {
    if cfg!(windows) {
        Box::new(PowerShellBackend)
    } else {
        Box::new(UnsupportedBackend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_backend_says_so() {
        let o = EngineOpts { root: std::env::temp_dir(), iso: std::path::PathBuf::new(), update: None, regenerate: false, test_update_base: None, extra: vec![] };
        let e = UnsupportedBackend.start(&o).err().unwrap();
        assert_eq!(e.kind(), ErrorKind::Unsupported);
        assert!(e.to_string().contains("later version"));
        assert_eq!(backend().supported(), cfg!(windows));
    }
}
