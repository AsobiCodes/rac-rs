//! The setup process: `setup.sh --iso <iso> --no-pause --skip-hash`, with its output read by
//! `setup_parse::Progress`. The launcher already checked the disc (size and SHA-1), so the setup
//! only checks the size again (`--skip-hash`). Repair passes `--regenerate`.
use crate::platform;
use crate::setup_parse::Progress;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::Instant;

#[derive(Clone, Debug, Default)]
pub struct EngineOpts {
    pub root: PathBuf,
    pub iso: PathBuf,
    /// Repair / rebuild: make the game code again (`--regenerate`).
    pub regenerate: bool,
    /// More arguments for `setup.sh` (tests).
    pub extra: Vec<String>,
}

pub fn env_extra_args() -> Vec<String> {
    std::env::var("RAC_LAUNCHER_SETUP_ARGS").map(|s| s.split_whitespace().map(String::from).collect()).unwrap_or_default()
}

/// The arguments after `sh`.
pub fn build_args(o: &EngineOpts, script: &std::path::Path) -> Vec<String> {
    let mut a = vec![
        script.to_string_lossy().to_string(),
        "--iso".to_string(),
        o.iso.to_string_lossy().to_string(),
        "--no-pause".to_string(),
        "--skip-hash".to_string(),
    ];
    if o.regenerate {
        a.push("--regenerate".to_string());
    }
    a.extend(o.extra.iter().cloned());
    a
}

/// A running (or finished) setup worker.
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
        let script = opts.root.join("setup.sh");
        if !script.is_file() {
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("setup.sh was not found in {}", opts.root.display())));
        }
        let mut cmd = Command::new("sh");
        cmd.args(build_args(opts, &script))
            .current_dir(&opts.root)
            .env("RAC_NO_PAUSE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Own process group: Cancel kills the generators and cargo with setup.sh.
            unsafe {
                cmd.pre_exec(|| {
                    libc::setpgid(0, 0);
                    Ok(())
                });
            }
        }
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
        Ok(Engine { child, pid, rx, progress: Progress::new(false), started: Instant::now(), done: None, cancelled: false })
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

    /// Cancel: the child and everything it started (its process group).
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
    use std::time::Duration;

    #[test]
    fn missing_script_is_an_error_not_a_panic() {
        let o = EngineOpts { root: std::env::temp_dir().join("racl_no_such_root"), iso: PathBuf::new(), ..Default::default() };
        assert!(Engine::start(&o).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn the_progress_markers_are_read() {
        let d = std::env::temp_dir().join(format!("racl_engine_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("setup.sh"), "#!/bin/sh\nprintf '== (a) tools\\n== (b) disc\\n  level 0 done: x\\nexpect-compiles: 10\\n== (d) build\\n   Compiling foo\\nERROR: nope\\n== done\\n'\n").unwrap();
        let o = EngineOpts { root: d.clone(), iso: PathBuf::from("/x.iso"), ..Default::default() };
        let mut e = Engine::start(&o).unwrap();
        for _ in 0..300 {
            e.poll();
            if e.done.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(e.progress.levels, 1);
        assert_eq!(e.progress.expect, 10);
        assert_eq!(e.progress.compiled, 1);
        assert_eq!(e.progress.last_error, "nope");
        assert_eq!(e.done, Some(0), "the last word is the exit code");
        assert!(e.progress.finished);
        let _ = std::fs::remove_dir_all(&d);
    }
}
