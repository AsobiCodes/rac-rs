//! [PC] Linux: the keep-awake. Windows asks `SetThreadExecutionState` to keep the screen on while the game window has the focus
//! (`awake.rs`: the rule, the guard, the pad); this is the same hold for Linux, driven by the same calls: `awake::set_state` hands its
//! "continuous" requests to `set_hold` here, so the on/off rule (focused and not minimised) and the test-window switch
//! (RAC_TEST_WINDOW: no hold) are the ones of `awake.rs`, and the one-shot pad reset (no ES_CONTINUOUS) does nothing here.
//!
//! The hold is a spawned program, no new library, and it depends on the desktop (`XDG_CURRENT_DESKTOP`):
//! - GNOME (Ubuntu, Fedora and most beginner desktops) blanks and locks the screen by its own idle monitor and ignores the logind idle
//!   lock, so there the hold is `gnome-session-inhibit --inhibit idle ... tail --pid=<this pid> -f /dev/null` (the session manager's own
//!   idle inhibitor, held while `tail` runs).
//! - KDE, the Steam Deck's Desktop Mode and the others: `systemd-inhibit --what=idle ... tail --pid=<this pid> -f /dev/null` takes a
//!   logind "idle" lock that lasts as long as `tail` runs (KDE's PowerDevil honours it).
//! In both, `tail` ends by itself when the game ends, also when the game crashes (no lock is left behind). The first way that stays up
//! counts; none: nothing is held. Each change goes into the run log (stderr). Windows never calls this.
#![cfg_attr(windows, allow(dead_code))]

use std::sync::Mutex;

/// The flags `awake.rs` passes (the `SetThreadExecutionState` ones).
const CONTINUOUS: u32 = 0x8000_0000;
const DISPLAY_OR_SYSTEM: u32 = 0x1 | 0x2;

/// How long a freshly spawned inhibitor has to stay alive to count as holding (it fails within a few ms when logind or the session
/// manager is missing).
const SETTLE_MS: u64 = 150;

/// How the hold is made, best first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Method {
	/// `systemd-inhibit ... tail --pid` (a logind idle lock: KDE and most others honour it, GNOME does not).
	Inhibit,
	/// `gnome-session-inhibit ... tail --pid` (the GNOME session manager's idle inhibitor).
	Gnome,
}

/// The programs `Inhibit` needs.
pub(crate) const INHIBIT_TOOLS: [&str; 2] = ["systemd-inhibit", "tail"];
/// The programs `Gnome` needs.
pub(crate) const GNOME_TOOLS: [&str; 2] = ["gnome-session-inhibit", "tail"];

/// Whether `XDG_CURRENT_DESKTOP` (a colon list such as `ubuntu:GNOME`) names a GNOME session.
pub(crate) fn is_gnome(desktop: &str) -> bool {
	desktop.split(':').any(|d| d.to_ascii_lowercase().contains("gnome"))
}

/// The methods that can be tried on this system, best first. On GNOME the session manager's inhibitor goes first (GNOME ignores the
/// logind lock for its own screen blanking, so `systemd-inhibit` would "succeed" and hold nothing); elsewhere the logind lock goes
/// first and the GNOME one is the last resort. `has` says whether a program exists (a parameter so the tests choose).
pub(crate) fn methods(has: &dyn Fn(&str) -> bool, desktop: &str) -> Vec<Method> {
	let gnome = GNOME_TOOLS.iter().all(|t| has(t)).then_some(Method::Gnome);
	let systemd = INHIBIT_TOOLS.iter().all(|t| has(t)).then_some(Method::Inhibit);
	let order = if is_gnome(desktop) { [gnome, systemd] } else { [systemd, gnome] };
	order.into_iter().flatten().collect()
}

/// The `tail` that ends when process `pid` (the game) is gone: the inhibitors hold the lock exactly while it runs.
fn tail_args(pid: u32) -> [String; 4] {
	["tail".to_string(), format!("--pid={pid}"), "-f".to_string(), "/dev/null".to_string()]
}

/// The logind idle lock: held while the `tail` behind it runs, which ends when process `pid` (the game) is gone.
pub(crate) fn inhibit_command(pid: u32) -> Vec<String> {
	["systemd-inhibit", "--what=idle", "--who=rac-rs", "--why=playing", "--mode=block"]
		.iter()
		.map(|s| s.to_string())
		.chain(tail_args(pid))
		.collect()
}

/// The GNOME session manager's idle inhibitor around the same `tail`.
pub(crate) fn gnome_command(pid: u32) -> Vec<String> {
	["gnome-session-inhibit", "--app-id", "rac-rs", "--inhibit", "idle", "--reason", "playing", "--"]
		.iter()
		.map(|s| s.to_string())
		.chain(tail_args(pid))
		.collect()
}

/// What `awake::set_state(flags)` means here: Some(true) = start holding, Some(false) = stop, None = a one-shot reset (nothing to do).
pub(crate) fn request(flags: u32) -> Option<bool> {
	(flags & CONTINUOUS != 0).then_some(flags & DISPLAY_OR_SYSTEM != 0)
}

/// What is held now.
enum Held {
	/// The process group of the inhibitor (`systemd-inhibit` or `gnome-session-inhibit`) and its `tail` (the group id is the child's pid).
	Inhibit(std::process::Child, &'static str),
}

static HELD: Mutex<Option<Held>> = Mutex::new(None);

/// `awake::set_state` on Linux: starts or ends the hold for a continuous request. Returns what was held before, as the flags the
/// Windows call returns (0 = nothing).
pub(crate) fn set_state(flags: u32) -> u32 {
	let Some(on) = request(flags) else { return 0 };
	set_hold(on)
}

/// Starts or ends the hold; returns the flags of what was held before (`awake::WANT` or 0).
pub(crate) fn set_hold(on: bool) -> u32 {
	let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
	let before = if held.is_some() { crate::awake::WANT } else { 0 };
	if on && held.is_none() {
		*held = acquire();
	} else if !on {
		if let Some(h) = held.take() {
			release(h);
		}
	}
	before
}

/// Tries the methods in order; the first that holds wins. None = nothing could be held (said in the run log).
#[cfg(target_os = "linux")]
fn acquire() -> Option<Held> {
	let path = std::env::var("PATH").unwrap_or_default();
	let has = |t: &str| crate::dialog_linux::find_in_path(t, &path).is_some();
	let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
	for m in methods(&has, &desktop) {
		let (argv, name): (Vec<String>, &'static str) = match m {
			Method::Inhibit => (inhibit_command(std::process::id()), "systemd-inhibit"),
			Method::Gnome => (gnome_command(std::process::id()), "gnome-session-inhibit"),
		};
		if let Some(child) = spawn_inhibit(&argv, name) {
			eprintln!("keep-awake: held with {name} (idle lock until the game ends)");
			return Some(Held::Inhibit(child, name));
		}
	}
	eprintln!("keep-awake: not available (no systemd-inhibit or gnome-session-inhibit with tail)");
	None
}

#[cfg(not(target_os = "linux"))]
fn acquire() -> Option<Held> {
	None
}

/// `argv` (an inhibitor around a `tail`) in a process group of its own (so one kill reaches both), checked to still be alive after
/// `SETTLE_MS`; `name` is for the run log.
#[cfg(target_os = "linux")]
fn spawn_inhibit(argv: &[String], name: &str) -> Option<std::process::Child> {
	use std::os::unix::process::CommandExt;
	use std::process::{Command, Stdio};
	let mut child = Command::new(&argv[0])
		.args(&argv[1..])
		.process_group(0)
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.spawn()
		.ok()?;
	let t = std::time::Instant::now();
	while t.elapsed().as_millis() < SETTLE_MS as u128 {
		match child.try_wait() {
			Ok(None) => std::thread::sleep(std::time::Duration::from_millis(10)),
			_ => {
				eprintln!("keep-awake: {name} did not stay up (no logind / session manager, or no GNU tail?), trying the next way");
				return None;
			}
		}
	}
	Some(child)
}

#[cfg(target_os = "linux")]
fn release(h: Held) {
	match h {
		Held::Inhibit(mut child, _) => {
			// SAFETY: a plain kill(2) on the process group this program created (its id is the child's pid).
			unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
			let _ = child.wait();
			eprintln!("keep-awake: released");
		}
	}
}

#[cfg(not(target_os = "linux"))]
fn release(_h: Held) {}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_lock_command_is_systemd_inhibit_around_a_tail_that_follows_the_game() {
		let c = inhibit_command(4242);
		assert_eq!(c[0], "systemd-inhibit");
		for a in ["--what=idle", "--who=rac-rs", "--why=playing", "--mode=block"] {
			assert!(c.iter().any(|x| x == a), "{a}");
		}
		let t = c.iter().position(|x| x == "tail").unwrap();
		assert_eq!(&c[t..], ["tail", "--pid=4242", "-f", "/dev/null"]);
		assert!(c[..t].iter().all(|x| x != "--pid=4242"), "the pid belongs to tail, not to systemd-inhibit");
	}

	/// GNOME ignores the logind lock, so its hold is the session manager's idle inhibitor around the same game-following `tail`.
	#[test]
	fn the_gnome_command_is_the_session_inhibitor_around_the_same_tail() {
		let c = gnome_command(4242);
		assert_eq!(c[0], "gnome-session-inhibit");
		let i = c.iter().position(|x| x == "--inhibit").unwrap();
		assert_eq!(c[i + 1], "idle", "the idle inhibitor, not logout/suspend");
		let sep = c.iter().position(|x| x == "--").unwrap();
		assert!(i < sep, "the options come before the command");
		assert_eq!(&c[sep + 1..], ["tail", "--pid=4242", "-f", "/dev/null"]);
		assert!(c[..sep].iter().all(|x| x != "--pid=4242"), "the pid belongs to tail");
	}

	#[test]
	fn gnome_is_found_in_the_desktop_list() {
		for d in ["GNOME", "ubuntu:GNOME", "gnome", "GNOME-Classic:GNOME", "pop:GNOME"] {
			assert!(is_gnome(d), "{d}");
		}
		for d in ["KDE", "X-Cinnamon", "XFCE", "", "Hyprland", "LXQt"] {
			assert!(!is_gnome(d), "{d}");
		}
	}

	#[test]
	fn the_way_follows_what_is_installed_and_the_desktop() {
		let all = |_: &str| true;
		assert_eq!(methods(&all, "ubuntu:GNOME"), vec![Method::Gnome, Method::Inhibit], "GNOME ignores the logind lock: its own one first");
		assert_eq!(methods(&all, "KDE"), vec![Method::Inhibit, Method::Gnome], "KDE honours the logind lock");
		assert_eq!(methods(&all, ""), vec![Method::Inhibit, Method::Gnome]);
		let only = |names: &'static [&'static str]| move |t: &str| names.contains(&t);
		assert_eq!(methods(&only(&["systemd-inhibit", "tail"]), "GNOME"), vec![Method::Inhibit], "GNOME without its tool: the logind lock is all there is");
		assert_eq!(methods(&only(&["gnome-session-inhibit", "tail"]), "GNOME"), vec![Method::Gnome]);
		assert_eq!(methods(&only(&["systemd-inhibit"]), "KDE"), Vec::<Method>::new(), "no tail: the lock would end at once");
		assert_eq!(methods(&only(&["gnome-session-inhibit"]), "GNOME"), Vec::<Method>::new());
		assert_eq!(methods(&only(&["dbus-send", "gdbus"]), "GNOME"), Vec::<Method>::new(), "no activity-pulse fallback any more");
		assert_eq!(methods(&only(&["tail"]), "KDE"), Vec::<Method>::new());
	}

	/// Windows' rule in flags: continuous + display/system = hold, continuous alone = release, no continuous = the pad's one-shot reset.
	#[test]
	fn the_windows_flags_mean_hold_release_or_nothing() {
		assert_eq!(request(crate::awake::WANT), Some(true));
		assert_eq!(request(CONTINUOUS), Some(false));
		assert_eq!(request(0x1 | 0x2), None, "the pad's one-shot reset");
		assert_eq!(request(0), None);
	}

	/// The real thing on Linux (skipped where there is no `systemd-inhibit` with a running logind): the lock shows up in the list under
	/// the name "rac-rs", and is gone after the release.
	#[cfg(target_os = "linux")]
	#[test]
	fn the_real_lock_appears_and_goes() {
		let list = || {
			let out = std::process::Command::new("systemd-inhibit").arg("--list").output().ok()?;
			out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
		};
		let Some(before) = list() else { return };
		assert!(!before.contains("rac-rs"));
		assert_eq!(set_hold(true), 0);
		// only the logind lock shows up in this list (GNOME's session inhibitor is another service)
		let held = matches!(HELD.lock().unwrap().as_ref(), Some(Held::Inhibit(_, "systemd-inhibit")));
		let during = list().unwrap();
		assert_eq!(set_hold(false), crate::awake::WANT);
		let after = list().unwrap();
		if held {
			assert!(during.contains("rac-rs") && during.contains("playing"), "{during}");
		}
		assert!(!after.contains("rac-rs"), "{after}");
	}
}
