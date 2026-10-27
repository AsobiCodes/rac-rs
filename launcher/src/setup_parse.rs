//! Reads the output of `setup.ps1 -Console` exactly as the old setup window did (Feed-Line, Update-Progress, Get-ErrorText, Get-DetailsText):
//! step markers `== (a)` ... `== (e)` and `== done`, `level N done`, cargo's `Compiling` lines, `expect-compiles: N`, `ERROR:` lines.
use std::time::{Duration, Instant};

pub const STEP_NAMES_INSTALL: [&str; 5] = ["Download tools", "Check disc", "Prepare game code", "Build game", "Finish"];
pub const STEP_NAMES_UPDATE: [&str; 5] = ["Download update", "Check tools", "Prepare game code", "Build game", "Finish"];
/// "Compiling" lines of a complete first build (measured); the bar never goes above 98% for the build.
pub const EXPECTED_COMPILES: u32 = 563;
const MAX_LOG_LINES: usize = 60_000;

#[derive(Clone, Debug)]
pub struct Progress {
    pub update: bool,
    pub step: usize,
    pub levels: u32,
    pub compiled: u32,
    pub expect: u32,
    pub tools_sub: f64,
    pub vu: bool,
    pub finished: bool,
    pub progress: f64,
    pub last_error: String,
    pub status: String,
    /// `desktop shortcut created: <path>` of the worker.
    pub shortcut: Option<String>,
    pub build_t0: Option<Instant>,
    pub log: Vec<String>,
    /// Exit code of the worker once it ended (3 = the update could not be downloaded).
    pub exit_code: Option<i32>,
}

impl Progress {
    pub fn new(update: bool) -> Progress {
        Progress {
            update,
            step: 0,
            levels: 0,
            compiled: 0,
            expect: 0,
            tools_sub: 0.0,
            vu: false,
            finished: false,
            progress: 0.0,
            last_error: String::new(),
            status: String::new(),
            shortcut: None,
            build_t0: None,
            log: Vec::new(),
            exit_code: None,
        }
    }

    pub fn step_names(&self) -> &'static [&'static str; 5] {
        if self.update {
            &STEP_NAMES_UPDATE
        } else {
            &STEP_NAMES_INSTALL
        }
    }

    pub fn feed_line(&mut self, line: &str) {
        if self.log.len() < MAX_LOG_LINES {
            self.log.push(line.to_string());
        }
        let t = line.trim();
        if !t.is_empty() && !t.starts_with("==") {
            self.status = t.chars().take(100).collect();
        }
        if let Some(k) = step_marker(line) {
            let (idx, status) = match k {
                "u" => (0, "Downloading and applying the update ..."),
                "a" => (if self.update { 1 } else { 0 }, "Getting the tools ready ..."),
                "b" => (1, "Checking the disc image ..."),
                "c" => (2, "Making the game code from your disc ..."),
                "c2" => (2, "Making the graphics code from your disc ..."),
                "d" => (3, "Building the game (the long part) ..."),
                _ => (4, "Finishing ..."),
            };
            if k == "c2" {
                self.vu = true;
            }
            self.status = status.to_string();
            self.step = idx;
            if k == "d" && self.build_t0.is_none() {
                self.build_t0 = Some(Instant::now());
            }
        } else if let Some(n) = line.strip_prefix("expect-compiles: ") {
            if let Some(n) = leading_number(n) {
                self.expect = n;
            }
        } else if line.starts_with("== done") {
            self.finished = true;
            self.step = 4;
        } else if is_level_done(line) {
            self.levels += 1;
        } else if line.starts_with(|c: char| c.is_whitespace()) && line.trim_start().starts_with("Compiling ") {
            self.compiled += 1;
        } else if line.contains("downloading Rust") {
            self.tools_sub = 0.15;
        } else if line.contains("downloading Python") {
            self.tools_sub = 0.6;
        } else if line.contains("downloading the GNU assembler") {
            self.tools_sub = 0.8;
        } else if let Some(p) = line.split("desktop shortcut created: ").nth(1) {
            self.shortcut = Some(p.trim().to_string());
        }
        if let Some(e) = line.strip_prefix("ERROR: ") {
            self.last_error = e.to_string();
        } else if self.last_error.is_empty() {
            if let Some(e) = cargo_error(line) {
                self.last_error = e;
            }
        }
    }

    /// The bar, 0..=100: the stages are weighted as in the old window; during the build it follows cargo's "Compiling" count
    /// (an update has no count to trust at first: it also creeps by time); never above 98 before the end.
    pub fn update_progress(&mut self, now: Instant) -> u32 {
        let v = match self.step {
            0 => 1.0 + 7.0 * self.tools_sub,
            1 => 9.0,
            2 => {
                if self.vu {
                    29.0
                } else {
                    10.0 + 18.0 * (self.levels as f64 / 19.0).min(1.0)
                }
            }
            3 => {
                let exp = if self.expect > 0 { self.expect } else { EXPECTED_COMPILES } as f64;
                // cargo prints "Compiling" when a crate STARTS
                let done = if self.update { self.compiled.saturating_sub(1) } else { self.compiled } as f64;
                let mut v = 30.0 + 68.0 * (done / exp).min(1.0);
                if self.update {
                    if let Some(t0) = self.build_t0 {
                        let s = now.saturating_duration_since(t0).as_secs_f64();
                        v = v.max(30.0 + 60.0 * (1.0 - (-s / 150.0).exp()));
                    }
                }
                v
            }
            _ => 99.0,
        };
        let v = if self.finished { 100.0 } else { v };
        if v > self.progress {
            self.progress = v;
        }
        self.progress.floor().min(100.0) as u32
    }

    fn step_name(&self) -> &'static str {
        self.step_names()[self.step.min(4)]
    }

    /// Get-ErrorText: what the player reads when the worker failed.
    pub fn error_text(&self) -> String {
        if self.update && self.exit_code == Some(3) {
            return "The update could not be downloaded.\n\nPlease check your internet connection and click Try again. Nothing on your PC was changed.".to_string();
        }
        let all = self.log.join("\n");
        let no_compile: String = self.log.iter().filter(|l| !l.trim_start().starts_with("Compiling ")).cloned().collect::<Vec<_>>().join("\n").to_lowercase();
        let net = self.step == 0
            || [
                "could not download", "failed to download", "couldn't resolve", "could not resolve", "resolve host", "timed out", "unable to connect",
                "spurious network", "network failure", "failed to get ",
            ]
            .iter()
            .any(|p| no_compile.contains(p))
            || (no_compile.contains("download of ") && no_compile.contains(" failed"));
        let lall = all.to_lowercase();
        let disk = ["no space left", "not enough space", "os error 112", "there is not enough space"].iter().any(|p| lall.contains(p));
        let mut m = format!("The step '{}' did not finish.", self.step_name());
        if self.step == 1 {
            m = "The disc image was not accepted.\n\nIt must be Ratchet & Clank PAL / Europe v1.00 or USA v1.00 as one .iso file. Click Try again and pick the right file.".to_string();
        } else if disk {
            m += "\n\nThere may not be enough free disk space (about 5 GB are needed). Free some space and click Try again.";
        } else if net {
            m += "\n\nPlease check your internet connection, then click Try again. Setup continues where it stopped.";
        } else {
            if !self.last_error.is_empty() {
                m += &format!("\n\n{}", self.last_error);
            }
            m += "\n\nClick Try again. If it keeps failing, click Copy details and share them when you ask for help.";
        }
        if self.update {
            m += "\n\nYour old version is still installed. Click Play old version to keep playing it.";
        }
        m
    }

    /// Get-DetailsText: for "Copy details".
    pub fn details_text(&self, windows: &str) -> String {
        let tail: Vec<&String> = self.log.iter().rev().take(200).collect::<Vec<_>>().into_iter().rev().collect();
        let log = tail.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n");
        format!("Ratchet & Clank (rac-rs) setup - problem report\nStep: {}\nError: {}\nWindows: {}\n\nLog (last 200 lines):\n{}", self.step_name(), self.last_error, windows, log)
    }

    pub fn elapsed_text(since: Instant, now: Instant) -> String {
        format_elapsed(now.saturating_duration_since(since))
    }
}

pub fn format_elapsed(d: Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}", s / 60, s % 60)
}

fn leading_number(s: &str) -> Option<u32> {
    let d: String = s.trim().chars().take_while(|c| c.is_ascii_digit()).collect();
    d.parse().ok()
}

/// `== (u)`, `== (a)` ... `== (e)`, `== (c2)`.
fn step_marker(line: &str) -> Option<&str> {
    let r = line.strip_prefix("== (")?;
    let end = r.find(')')?;
    let k = &r[..end];
    matches!(k, "u" | "a" | "b" | "c" | "c2" | "d" | "e").then_some(k)
}

/// `^\s*level \d+ done`
fn is_level_done(line: &str) -> bool {
    let Some(r) = line.trim_start().strip_prefix("level ") else { return false };
    let digits: String = r.chars().take_while(|c| c.is_ascii_digit()).collect();
    !digits.is_empty() && r[digits.len()..].starts_with(" done")
}

/// `^error(\[E\d+\])?: (.*)$` (a compiler error)
fn cargo_error(line: &str) -> Option<String> {
    let r = line.strip_prefix("error")?;
    let r = if let Some(b) = r.strip_prefix("[E") {
        let end = b.find(']')?;
        if !b[..end].bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        &b[end + 1..]
    } else {
        r
    };
    r.strip_prefix(": ").map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_all(p: &mut Progress, text: &str) {
        for l in text.lines() {
            p.feed_line(l);
        }
    }

    #[test]
    fn markers_levels_and_compiles_of_a_real_setup_run() {
        // the console output of a full setup.ps1 run (RAC_TEST_SETUP_LOG), skipped when not set
        let Some(path) = std::env::var_os("RAC_TEST_SETUP_LOG") else { return };
        let Ok(bytes) = std::fs::read(path) else { return };
        let text = String::from_utf8_lossy(&bytes).to_string();
        let mut p = Progress::new(false);
        let mut steps = vec![];
        for l in text.lines() {
            p.feed_line(l);
            if steps.last() != Some(&p.step) {
                steps.push(p.step);
            }
        }
        assert_eq!(steps, vec![0, 1, 2, 3, 4], "tools, disc, code (+VU), build, finish");
        assert_eq!(p.levels, 19, "19 levels");
        // (setup_full.log predates the (c2) VU step, so p.vu is not asserted)
        assert!(p.finished);
        assert_eq!(p.compiled, text.lines().filter(|l| l.trim_start().starts_with("Compiling ") && l.starts_with(' ')).count() as u32);
        assert!(p.compiled > 400, "{}", p.compiled);
        assert!(p.update_progress(Instant::now()) == 100);
        assert_eq!(p.step_names()[3], "Build game");
    }

    #[test]
    fn bar_follows_the_build_and_stops_at_98() {
        let mut p = Progress::new(false);
        feed_all(&mut p, "== (a) tools\n== (b) disc image\n== (c) generating game code\n== (c2) VU\n== (d) building the game");
        assert_eq!(p.step, 3);
        let a = p.update_progress(Instant::now());
        assert_eq!(a, 30);
        for i in 0..237 {
            p.feed_line(&format!("   Compiling crate-{i} v0.1.0"));
        }
        let b = p.update_progress(Instant::now());
        assert!(b > 30 && b < 98, "{b}");
        for i in 0..2000 {
            p.feed_line(&format!("   Compiling crate-{i} v0.1.0"));
        }
        assert_eq!(p.update_progress(Instant::now()), 98);
        p.feed_line("== done");
        assert_eq!(p.update_progress(Instant::now()), 100);
    }

    #[test]
    fn update_mode_steps_and_expect_compiles() {
        let mut p = Progress::new(true);
        feed_all(&mut p, "== (u) updating to v0.4.0\n  downloading v0.4.0 ...\n  4 files updated, 0 removed\nexpect-compiles: 4\n== (a) tools");
        assert_eq!(p.step, 1);
        assert_eq!(p.expect, 4);
        assert_eq!(p.step_names()[0], "Download update");
        p.feed_line("== (d) building the game");
        p.feed_line("   Compiling rac-rs v0.4.0");
        // the first "Compiling" of an update is not counted as done
        let v = p.update_progress(Instant::now());
        assert!((30..=40).contains(&v), "{v}");
    }

    #[test]
    fn errors_shortcut_and_status() {
        let mut p = Progress::new(false);
        p.feed_line("error[E0425]: cannot find value `x`");
        p.feed_line("error: second one");
        assert_eq!(p.last_error, "cannot find value `x`");
        p.feed_line("ERROR: build failed badly");
        assert_eq!(p.last_error, "build failed badly");
        p.feed_line("  desktop shortcut created: C:\\Users\\x\\Desktop\\Ratchet & Clank (rac-rs).lnk");
        assert_eq!(p.shortcut.as_deref(), Some("C:\\Users\\x\\Desktop\\Ratchet & Clank (rac-rs).lnk"));
        assert!(p.status.starts_with("desktop shortcut created"));
    }

    #[test]
    fn error_texts() {
        let mut p = Progress::new(false);
        p.feed_line("== (b) disc image");
        assert!(p.error_text().starts_with("The disc image was not accepted."));
        let mut p = Progress::new(false);
        p.feed_line("== (d) building");
        p.feed_line("error: failed to get `foo` as a dependency");
        assert!(p.error_text().contains("internet connection"));
        let mut p = Progress::new(false);
        p.feed_line("== (c) code");
        p.feed_line("OSError: [Errno 28] No space left on device");
        assert!(p.error_text().contains("free disk space"));
        let mut p = Progress::new(true);
        p.exit_code = Some(3);
        assert!(p.error_text().starts_with("The update could not be downloaded."));
        let mut p = Progress::new(true);
        p.feed_line("== (d) building");
        p.feed_line("ERROR: weird");
        let t = p.error_text();
        assert!(t.contains("weird") && t.contains("Play old version"), "{t}");
        assert!(p.details_text("Windows 11").contains("Step: Build game"));
    }

    #[test]
    fn elapsed_format() {
        assert_eq!(format_elapsed(Duration::from_secs(7 * 60 + 12)), "07:12");
    }
}
