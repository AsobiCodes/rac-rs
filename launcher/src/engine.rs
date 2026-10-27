//! The setup process behind the install / repair / update screens.
//!
//! The Windows launcher ran `setup.ps1` (archived in `windows/engine_windows.rs`).
//! The Linux engine, which will run `setup.sh --iso <iso>` and read the same
//! progress markers, comes in a later version; until then `start` says so and
//! the screens stay closed. The process reader below is what that engine will
//! use, so it is kept here rather than duplicated later.
use crate::platform;
use crate::setup_parse::Progress;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::Child;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Instant;

#[derive(Clone, Debug, Default)]
pub struct EngineOpts {
    pub root: PathBuf,
    pub iso: PathBuf,
    /// Some((version, finish_only)): an update.
    pub update: Option<(String, bool)>,
    /// Repair / rebuild: regenerate the game code too.
    pub regenerate: bool,
    pub test_update_base: Option<String>,
    /// More switches for flow tests; also from the environment variable RAC_LAUNCHER_SETUP_ARGS.
    pub extra: Vec<String>,
}

pub fn env_extra_args() -> Vec<String> {
    std::env::var("RAC_LAUNCHER_SETUP_ARGS").map(|s| s.split_whitespace().map(String::from).collect()).unwrap_or_default()
}

/// A running (or finished) setup worker.
///
/// Built by the Linux engine in a later version; the reader is kept ready for it.
#[allow(dead_code)]
pub struct Engine {
    child: Child,
    pid: u32,
    rx: Receiver<String>,
    pub progress: Progress,
    pub started: Instant,
    /// Set once the worker ended and its output is read: the exit code (-1 = killed / unknown).
    pub done: Option<i32>,
    pub cancelled: bool,
}

#[allow(dead_code)]
fn pump<R: std::io::Read + Send + 'static>(r: R, tx: std::sync::mpsc::Sender<String>) {
    std::thread::spawn(move || {
        let mut br = BufReader::new(r);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match br.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let s = String::from_utf8_lossy(&buf).to_string();
                    for part in s.split(['\r', '\n']).filter(|p| !p.is_empty()) {
                        if tx.send(part.to_string()).is_err() {
                            return;
                        }
                    }
                }
            }
        }
    });
}

pub const UNSUPPORTED_TEXT: &str = "Installing or repairing from a disc comes in a later version of the Linux launcher. Run setup.sh in the game folder, then use \"Add existing install...\".";

impl Engine {
    pub fn start(_opts: &EngineOpts) -> std::io::Result<Engine> {
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, UNSUPPORTED_TEXT))
    }

    /// Reads what the worker printed since the last call and checks whether it ended. Returns true when something changed.
    pub fn poll(&mut self) -> bool {
        if self.done.is_some() {
            return false;
        }
        let mut changed = false;
        let mut disconnected = false;
        loop {
            match self.rx.try_recv() {
                Ok(l) => {
                    self.progress.feed_line(&l);
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        self.progress.update_progress(Instant::now());
        if let Ok(Some(st)) = self.child.try_wait() {
            // the pipes close with the process; wait a moment for the last lines
            if disconnected || self.started.elapsed().as_millis() > 0 {
                let t = Instant::now();
                while t.elapsed().as_millis() < 500 {
                    match self.rx.try_recv() {
                        Ok(l) => self.progress.feed_line(&l),
                        Err(TryRecvError::Empty) => std::thread::sleep(std::time::Duration::from_millis(20)),
                        Err(TryRecvError::Disconnected) => break,
                    }
                }
            }
            let code = st.code().unwrap_or(-1);
            self.progress.exit_code = Some(code);
            if code == 0 {
                self.progress.finished = true;
                self.progress.step = 4;
            }
            self.progress.update_progress(Instant::now());
            self.done = Some(code);
            changed = true;
        }
        changed
    }

    /// Cancel: the child and its descendants only.
    pub fn cancel(&mut self) {
        self.cancelled = true;
        if self.done.is_none() {
            platform::kill_tree(self.pid);
        }
    }

    pub fn running(&self) -> bool {
        self.done.is_none()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        if self.done.is_none() {
            platform::kill_tree(self.pid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn starting_says_linux_installs_come_later() {
        let o = EngineOpts { root: PathBuf::from("/tmp/none"), iso: PathBuf::new(), ..Default::default() };
        let e = Engine::start(&o).err().unwrap();
        assert_eq!(e.kind(), std::io::ErrorKind::Unsupported);
        assert!(e.to_string().contains("setup.sh"));
    }
}
