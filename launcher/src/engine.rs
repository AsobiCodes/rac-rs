//! The install / repair / update engine: setup.ps1 as a hidden child, its output parsed by `setup_parse::Progress`.
//!
//!   powershell.exe -NoProfile -ExecutionPolicy Bypass -File setup.ps1 -Console -Iso <iso> -SkipHash [-ApplyUpdate -NewVersion X [-FinishOnly]
//!   -SkipUnblock [-TestUpdateBase D]] -NoShortcut [-Regenerate]
//!
//! The launcher itself makes the desktop shortcut (to the launcher), so the worker always gets -NoShortcut.
use crate::platform;
use crate::setup_parse::Progress;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::Instant;

#[derive(Clone, Debug, Default)]
pub struct EngineOpts {
    pub root: PathBuf,
    pub iso: PathBuf,
    /// Some((version, finish_only)) = `-ApplyUpdate`.
    pub update: Option<(String, bool)>,
    /// Repair / rebuild: `-Regenerate` (makes the game code again).
    pub regenerate: bool,
    pub test_update_base: Option<String>,
    /// More switches for flow tests (`-CheckOnly`, `-NoBuild`); also from the environment variable RAC_LAUNCHER_SETUP_ARGS.
    pub extra: Vec<String>,
}

/// The arguments after `powershell.exe`.
pub fn build_args(o: &EngineOpts) -> Vec<String> {
    let mut a: Vec<String> = ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"].iter().map(|s| s.to_string()).collect();
    a.push(o.root.join("setup.ps1").to_string_lossy().to_string());
    a.extend(["-Console", "-Iso"].map(String::from));
    a.push(o.iso.to_string_lossy().to_string());
    a.push("-SkipHash".into());
    if let Some((ver, finish)) = &o.update {
        a.extend(["-ApplyUpdate", "-NewVersion"].map(String::from));
        a.push(ver.clone());
        if *finish {
            a.push("-FinishOnly".into());
        }
        a.push("-SkipUnblock".into());
        if let Some(b) = &o.test_update_base {
            a.push("-TestUpdateBase".into());
            a.push(b.trim_end_matches('\\').to_string());
        }
    }
    a.push("-NoShortcut".into());
    if o.regenerate {
        a.push("-Regenerate".into());
    }
    a.extend(o.extra.iter().cloned());
    a
}

pub fn env_extra_args() -> Vec<String> {
    std::env::var("RAC_LAUNCHER_SETUP_ARGS").map(|s| s.split_whitespace().map(String::from).collect()).unwrap_or_default()
}

/// A running (or finished) worker.
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

impl Engine {
    pub fn start(opts: &EngineOpts) -> std::io::Result<Engine> {
        let script = opts.root.join("setup.ps1");
        if !script.is_file() {
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("setup.ps1 was not found in {}", opts.root.display())));
        }
        let mut cmd = Command::new(platform::powershell_exe());
        cmd.args(build_args(opts)).current_dir(&opts.root).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        platform::hidden(&mut cmd);
        let mut child = cmd.spawn()?;
        let (tx, rx) = channel();
        if let Some(o) = child.stdout.take() {
            pump(o, tx.clone());
        }
        if let Some(e) = child.stderr.take() {
            pump(e, tx);
        }
        let pid = child.id();
        Ok(Engine { child, pid, rx, progress: Progress::new(opts.update.is_some()), started: Instant::now(), done: None, cancelled: false })
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

/// Where the setup worker's script lives (for messages).
pub fn script_path(root: &Path) -> PathBuf {
    root.join("setup.ps1")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> EngineOpts {
        EngineOpts { root: PathBuf::from("C:\\g\\rac-rs"), iso: PathBuf::from("D:\\iso\\Ratchet & Clank.iso"), ..Default::default() }
    }

    #[test]
    fn install_args_match_the_old_window() {
        let a = build_args(&opts());
        assert_eq!(
            a,
            vec!["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "C:\\g\\rac-rs\\setup.ps1", "-Console", "-Iso", "D:\\iso\\Ratchet & Clank.iso", "-SkipHash", "-NoShortcut"]
        );
    }

    #[test]
    fn update_and_repair_args() {
        let mut o = opts();
        o.update = Some(("0.4.0".into(), true));
        o.test_update_base = Some("E:\\base\\".into());
        o.regenerate = true;
        let a = build_args(&o).join(" ");
        assert!(a.contains("-SkipHash -ApplyUpdate -NewVersion 0.4.0 -FinishOnly -SkipUnblock -TestUpdateBase E:\\base -NoShortcut -Regenerate"), "{a}");
    }

    #[test]
    fn missing_script_is_an_error_not_a_panic() {
        let mut o = opts();
        o.root = std::env::temp_dir().join("racl_no_such_root");
        assert!(Engine::start(&o).is_err());
    }
}
