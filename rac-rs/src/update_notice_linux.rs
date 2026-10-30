//! [PC] Linux: the update notice. The Windows game asks `setup.ps1 -Launch` before it starts whether a newer version is waiting
//! (`main.rs: update_check`: the VERSION file of the public repository against the one in the install folder) and shows an
//! "Update available" window. This is the notice half of that for Linux: the same check, the same skip rules (RAC_NO_UPDATE_CHECK, which the
//! launcher sets because it does the check itself; the hidden test runs unless RAC_TEST_UPDATE_BASE is set), and when a newer version
//! exists one window (`dialog_linux.rs`) that tells the player to open the RAC-RS Launcher to update. The game never updates itself here.
//!
//! It runs on a thread of its own and never holds the game back: a slow network, a missing downloader or a window nobody sees
//! (Steam's Gaming Mode has no place for one; there only a log line is written) cannot stop the start. No new library: the
//! download is a spawned `curl`, `python3` or `wget` (the first that answers; the next one is tried only when the earlier failed, and all
//! of them together get 5 s, like the 4 s of the Windows check). Every outcome goes into
//! the run log (stderr). The logic is plain functions that compile and are tested on every system.
#![cfg_attr(windows, allow(dead_code))]
#![cfg_attr(not(all(target_os = "linux", feature = "windows_gui")), allow(dead_code))]

use std::path::{Path, PathBuf};

/// Where the public VERSION file lives (the `$RepoRaw` of setup.ps1).
pub(crate) const REPO_RAW: &str = "https://raw.githubusercontent.com/AsobiCodes/rac-rs/main";
/// The longest a download may take (setup.ps1 `Read-Remote`: 4000 ms).
const FETCH_SECS: u64 = 4;
/// The version a missing or unreadable VERSION file counts as (setup.ps1 `Get-LocalVersion`).
const FIRST_RELEASE: [u64; 3] = [0, 1, 0];

/// "X.Y.Z" (digits only, three parts) from VERSION text, around a byte order mark and white space.
pub(crate) fn parse_version(s: &str) -> Option<[u64; 3]> {
	let s = s.trim_start_matches('\u{feff}').trim();
	let mut parts = s.split('.');
	let mut v = [0u64; 3];
	for slot in v.iter_mut() {
		let p = parts.next()?;
		if p.is_empty() || p.len() > 9 || !p.bytes().all(|b| b.is_ascii_digit()) {
			return None;
		}
		*slot = p.parse().ok()?;
	}
	parts.next().is_none().then_some(v)
}

fn show_version(v: [u64; 3]) -> String {
	format!("{}.{}.{}", v[0], v[1], v[2])
}

/// The local version as the window shows it: the file's, else 0.1.0.
pub(crate) fn local_version(file_text: Option<&str>) -> String {
	show_version(file_text.and_then(parse_version).unwrap_or(FIRST_RELEASE))
}

/// The remote version when it is newer than the local one (`remote_text` / `local_text` are the VERSION files' texts).
pub(crate) fn newer(remote_text: &str, local_text: Option<&str>) -> Option<String> {
	let r = parse_version(remote_text)?;
	let l = local_text.and_then(parse_version).unwrap_or(FIRST_RELEASE);
	(r > l).then(|| show_version(r))
}

/// The window text.
pub(crate) fn notice_text(local: &str, remote: &str) -> String {
	format!(
		"Update available: v{local} to v{remote}\n\nTo update, open the RAC-RS Launcher and press the update button. \
		 The game is rebuilt on your PC, so it takes a while; your saves and settings are kept.\n\n\
		 The launcher is at github.com/AsobiCodes/rac-rs-launcher if you do not have it yet."
	)
}

/// The same skip rules as the Windows check: RAC_NO_UPDATE_CHECK (Play old version, the launcher), and the automated test runs
/// (RAC_TEST_WINDOW / RAC_RETAIL_SMOKE) unless RAC_TEST_UPDATE_BASE says where a test version file is. `has` = is this variable set.
pub(crate) fn skip(has: &dyn Fn(&str) -> bool) -> bool {
	has("RAC_NO_UPDATE_CHECK") || ((has("RAC_TEST_WINDOW") || has("RAC_RETAIL_SMOKE")) && !has("RAC_TEST_UPDATE_BASE"))
}

/// Steam's Gaming Mode (gamescope): a window of ours has no place there and could take the controller's focus, so only the log line.
pub(crate) fn gaming_mode(has: &dyn Fn(&str) -> bool, desktop: &str) -> bool {
	has("SteamGamepadUI") || has("GAMESCOPE_WAYLAND_DISPLAY") || desktop.to_ascii_lowercase().contains("gamescope")
}

/// Where the remote VERSION is read from.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Source {
	File(PathBuf),
	Url(String),
}

/// The source for RAC_TEST_UPDATE_BASE (`base`: a folder, a file:// URL or an http(s) URL; None = the public repository).
pub(crate) fn source(base: Option<&str>) -> Source {
	match base {
		None => Source::Url(format!("{REPO_RAW}/VERSION")),
		Some(b) if b.starts_with("http://") || b.starts_with("https://") => Source::Url(format!("{}/VERSION", b.trim_end_matches('/'))),
		Some(b) => Source::File(PathBuf::from(b.strip_prefix("file://").unwrap_or(b)).join("VERSION")),
	}
}

/// The download programs, in the order they are tried.
pub(crate) const FETCH_TOOLS: [&str; 3] = ["curl", "python3", "wget"];

const PY_FETCH: &str = "import sys, urllib.request as u\n\
	r = u.Request(sys.argv[1], headers={'User-Agent': 'rac-rs-updater'})\n\
	sys.stdout.write(u.urlopen(r, timeout=4).read(4096).decode('ascii', 'replace'))";

/// The program and its arguments that print the text at `url` on stdout; None for an unknown program. No user configuration is read
/// (curl -q, python -I, wget --no-hsts keeps ~/.wget-hsts away).
pub(crate) fn fetch_command(tool: &str, url: &str) -> Option<Vec<String>> {
	let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<String>>();
	let mut a = match tool {
		"curl" => s(&["curl", "-q", "-fsSL", "--connect-timeout", "3", "--max-time", &FETCH_SECS.to_string(), "--max-filesize", "4096", "-A", "rac-rs-updater"]),
		"python3" => s(&["python3", "-I", "-c", PY_FETCH]),
		"wget" => s(&["wget", "-q", "--no-hsts", "-T", &FETCH_SECS.to_string(), "-t", "1", "-O", "-"]),
		_ => return None,
	};
	a.push(url.to_string());
	Some(a)
}

/// Runs `argv`, waits `limit` at most (then kills it) and returns its stdout when it ended well.
fn run_limited(argv: &[String], limit: std::time::Duration) -> Option<String> {
	use std::io::Read;
	use std::process::{Command, Stdio};
	let mut child = Command::new(&argv[0]).args(&argv[1..]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
	let t = std::time::Instant::now();
	let status = loop {
		match child.try_wait() {
			Ok(Some(st)) => break st,
			Ok(None) if t.elapsed() < limit => std::thread::sleep(std::time::Duration::from_millis(40)),
			_ => {
				let _ = child.kill();
				let _ = child.wait();
				return None;
			}
		}
	};
	if !status.success() {
		return None;
	}
	let mut text = String::new();
	child.stdout.take()?.take(4096).read_to_string(&mut text).ok()?;
	Some(text)
}

/// What is left of the budget the download programs share at `now`; None when too little is left to start another one.
fn time_left(deadline: std::time::Instant, now: std::time::Instant) -> Option<std::time::Duration> {
	deadline.checked_duration_since(now).filter(|d| *d >= std::time::Duration::from_millis(300))
}

/// The text of the remote VERSION file, or None on any failure (offline, no downloader, a slow link). The programs share ONE budget of
/// `FETCH_SECS + 1` seconds (found by the 5c real-game proof: a budget per program made an unreachable server cost 3 x 4 s = 11 to 12 s).
fn read_remote(src: &Source) -> Option<String> {
	match src {
		Source::File(p) => std::fs::read_to_string(p).ok(),
		Source::Url(url) => {
			let path = std::env::var("PATH").unwrap_or_default();
			let deadline = std::time::Instant::now() + std::time::Duration::from_secs(FETCH_SECS + 1);
			FETCH_TOOLS.iter().find_map(|t| {
				crate::dialog_linux::find_in_path(t, &path)?;
				let argv = fetch_command(t, url)?;
				run_limited(&argv, time_left(deadline, std::time::Instant::now())?)
			})
		}
	}
}

/// The check and the window, on the calling thread. `base` is RAC_TEST_UPDATE_BASE.
fn run(root: &Path, base: Option<String>, gaming: bool) {
	let local_text = std::fs::read_to_string(root.join("VERSION")).ok();
	let Some(remote_text) = read_remote(&source(base.as_deref())) else {
		eprintln!("update notice: no answer (offline, or no curl / python3 / wget)");
		return;
	};
	let Some(remote) = newer(&remote_text, local_text.as_deref()) else {
		eprintln!("update notice: up to date");
		return;
	};
	let local = local_version(local_text.as_deref());
	eprintln!("update notice: v{remote} is available (this is v{local})");
	if gaming {
		eprintln!("update notice: no window in Gaming Mode; open the RAC-RS Launcher in Desktop Mode to update");
		return;
	}
	use crate::dialog_linux::{Kind, show};
	if !show("Ratchet & Clank (rac-rs)", &notice_text(&local, &remote), Kind::Info) {
		eprintln!("update notice: no window could be shown (no zenity, kdialog or xmessage?)");
	}
}

/// Called by `main` for a player start (after the run log started): starts the check on a thread and returns at once.
pub(crate) fn start(root: PathBuf) {
	let has = |k: &str| std::env::var_os(k).is_some();
	if skip(&has) {
		return;
	}
	let base = std::env::var("RAC_TEST_UPDATE_BASE").ok().filter(|b| !b.is_empty());
	let gaming = gaming_mode(&has, &std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default());
	let _ = std::thread::Builder::new().name("update-notice".into()).spawn(move || run(&root, base, gaming));
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn versions_are_three_numbers() {
		assert_eq!(parse_version("0.4.2\n"), Some([0, 4, 2]));
		assert_eq!(parse_version("\u{feff}0.4.2\r\n"), Some([0, 4, 2]));
		assert_eq!(parse_version(" 12.0.101 "), Some([12, 0, 101]));
		for bad in ["", "0.4", "0.4.2.1", "v0.4.2", "0.4.x", "0..2", "<html>", "0.4.2 beta", "1234567890.0.0"] {
			assert_eq!(parse_version(bad), None, "{bad:?}");
		}
	}

	#[test]
	fn only_a_newer_remote_version_counts() {
		assert_eq!(newer("0.4.2\n", Some("0.4.1\n")), Some("0.4.2".to_string()));
		assert_eq!(newer("0.10.0", Some("0.9.9")), Some("0.10.0".to_string()), "numbers, not text");
		assert_eq!(newer("1.0.0", Some("0.99.99")), Some("1.0.0".to_string()));
		assert_eq!(newer("0.4.1", Some("0.4.1")), None);
		assert_eq!(newer("0.4.0", Some("0.4.1")), None, "a local build ahead of the release says nothing");
		assert_eq!(newer("0.4.2", None), Some("0.4.2".to_string()), "no VERSION file counts as 0.1.0");
		assert_eq!(newer("0.4.2", Some("garbage")), Some("0.4.2".to_string()));
		assert_eq!(newer("0.1.0", None), None);
		assert_eq!(newer("<html>captive portal</html>", Some("0.4.1")), None, "an answer that is not a version is no answer");
		assert_eq!(local_version(Some("0.4.1\n")), "0.4.1");
		assert_eq!(local_version(None), "0.1.0");
	}

	#[test]
	fn the_window_tells_the_player_to_open_the_launcher() {
		let t = notice_text("0.4.1", "0.4.2");
		assert!(t.starts_with("Update available: v0.4.1 to v0.4.2"), "{t}");
		assert!(t.contains("open the RAC-RS Launcher"), "{t}");
		assert!(t.contains("saves and settings are kept"), "{t}");
	}

	#[test]
	fn the_skip_rules_are_the_windows_ones() {
		let with = |names: &'static [&'static str]| move |k: &str| names.contains(&k);
		assert!(!skip(&with(&[])), "a normal start checks");
		assert!(skip(&with(&["RAC_NO_UPDATE_CHECK"])), "Play old version, the launcher");
		assert!(skip(&with(&["RAC_TEST_WINDOW"])));
		assert!(skip(&with(&["RAC_RETAIL_SMOKE"])));
		assert!(!skip(&with(&["RAC_TEST_WINDOW", "RAC_TEST_UPDATE_BASE"])), "the update test runs in the hidden window");
		assert!(skip(&with(&["RAC_NO_UPDATE_CHECK", "RAC_TEST_UPDATE_BASE"])));
	}

	#[test]
	fn gaming_mode_is_told_by_steam_and_gamescope() {
		let none = |_: &str| false;
		assert!(!gaming_mode(&none, "KDE"));
		assert!(!gaming_mode(&none, "ubuntu:GNOME"));
		assert!(gaming_mode(&none, "gamescope"));
		assert!(gaming_mode(&|k: &str| k == "SteamGamepadUI", "KDE"));
		assert!(gaming_mode(&|k: &str| k == "GAMESCOPE_WAYLAND_DISPLAY", ""));
	}

	#[test]
	fn the_source_is_the_public_repository_or_the_test_base() {
		assert_eq!(source(None), Source::Url("https://raw.githubusercontent.com/AsobiCodes/rac-rs/main/VERSION".to_string()));
		assert_eq!(source(Some("http://127.0.0.1:8000/")), Source::Url("http://127.0.0.1:8000/VERSION".to_string()));
		assert_eq!(source(Some("/tmp/up")), Source::File(PathBuf::from("/tmp/up").join("VERSION")));
		assert_eq!(source(Some("file:///tmp/up")), Source::File(PathBuf::from("/tmp/up").join("VERSION")));
	}

	#[test]
	fn the_download_commands_end_with_the_url_and_read_no_user_config() {
		let u = "https://example.org/VERSION";
		for t in FETCH_TOOLS {
			let c = fetch_command(t, u).unwrap();
			assert_eq!(c[0], t);
			assert_eq!(c.last().unwrap(), u);
		}
		assert_eq!(fetch_command("curl", u).unwrap()[1], "-q", "curl reads ~/.curlrc unless -q is first");
		assert!(fetch_command("python3", u).unwrap().contains(&"-I".to_string()));
		assert!(fetch_command("wget", u).unwrap().contains(&"--no-hsts".to_string()));
		assert!(fetch_command("fetch", u).is_none());
		assert!(fetch_command("curl", u).unwrap().contains(&"4".to_string()), "4 s at most");
	}

	#[test]
	fn a_version_file_in_a_folder_is_read_and_a_missing_one_is_no_answer() {
		let dir = std::env::temp_dir().join(format!("rac_update_notice_{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		std::fs::write(dir.join("VERSION"), "0.9.0\n").unwrap();
		let src = source(Some(dir.to_str().unwrap()));
		assert_eq!(read_remote(&src).as_deref(), Some("0.9.0\n"));
		std::fs::remove_file(dir.join("VERSION")).unwrap();
		assert_eq!(read_remote(&src), None);
		let _ = std::fs::remove_dir(&dir);
	}

	/// A program that does not end in time is stopped; one that fails gives no text (Unix test programs: `sleep`, `false`, `echo`).
	#[cfg(unix)]
	#[test]
	fn a_slow_or_failing_download_is_no_answer() {
		let argv = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<String>>();
		let t = std::time::Instant::now();
		assert_eq!(run_limited(&argv(&["sleep", "30"]), std::time::Duration::from_millis(300)), None);
		assert!(t.elapsed().as_secs() < 5, "killed at the limit");
		assert_eq!(run_limited(&argv(&["false"]), std::time::Duration::from_secs(2)), None);
		assert_eq!(run_limited(&argv(&["echo", "0.4.2"]), std::time::Duration::from_secs(2)).as_deref(), Some("0.4.2\n"));
		assert_eq!(run_limited(&argv(&["/nonexistent/tool"]), std::time::Duration::from_secs(2)), None);
	}

	/// The downloaders share one budget: an unreachable server costs the budget once, not once per program.
	#[test]
	fn the_downloaders_share_one_budget() {
		use std::time::{Duration, Instant};
		let t0 = Instant::now();
		let deadline = t0 + Duration::from_secs(5);
		assert_eq!(time_left(deadline, t0), Some(Duration::from_secs(5)));
		assert_eq!(time_left(deadline, t0 + Duration::from_secs(4)), Some(Duration::from_secs(1)));
		assert_eq!(time_left(deadline, t0 + Duration::from_millis(4800)), None, "too little left to start another program");
		assert_eq!(time_left(deadline, t0 + Duration::from_secs(6)), None, "past the deadline");
	}
}
