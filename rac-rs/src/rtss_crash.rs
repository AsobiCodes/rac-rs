//! RivaTuner (RTSS) + Vulkan crash box. With RivaTuner running (its `RTSSHooks64.dll` injected into the game) and NVIDIA driver 621.01,
//! the Vulkan renderer crashes a few seconds after the start (access violation in `nvoglv64.dll`, user test 2026-10-22), also with the
//! RTSS Vulkan layer switched off (`rtss_layer.rs`); DirectX 12 works. Not seen since dev s97 (one shared Vulkan device): 2026-10-23,
//! 10 hidden Vulkan starts with RTSS running and the user's play session on dev s100 (counter on screen) were fine; the box stays as a
//! safety net. A rare case, so no automatic switch: the crash filter
//! (`runlog.rs` `win::crashed`) and, once, the start-up watchdog (60 s without an answer) call in here, and the player gets a Yes/No
//! box offering `renderer=dx12` in `pc_options.txt` (every other key stays). The box runs on a fresh thread, never on the crashed one.
//! Tests (hidden window, `RAC_TEST_WINDOW` / `RAC_RETAIL_SMOKE` / `RAC_TEST_CRASH` set): no box, only the run-log lines.
//! `RAC_TEST_CRASH=rtss` pretends RTSS is loaded (and crashes like `=1`); `RAC_TEST_RTSS_ANSWER=yes|no` answers the box (tests only).
use std::sync::atomic::{AtomicBool, Ordering};

/// The window's graphics API is Vulkan (set when the GPU is opened; the player's choice until the adapter is known).
static VULKAN: AtomicBool = AtomicBool::new(true);
/// `RAC_TEST_CRASH=rtss`: act as if RTSSHooks64.dll were loaded.
static PRETEND: AtomicBool = AtomicBool::new(false);
/// The freeze box was offered already.
static FROZEN_ASKED: AtomicBool = AtomicBool::new(false);

/// `viewer/gpu/renderer.rs`: the API of the window (before the instance is made, then the adapter's).
pub fn note_backend(vulkan: bool) {
	VULKAN.store(vulkan, Ordering::Relaxed);
}

/// `runlog.rs`, `RAC_TEST_CRASH=rtss`.
pub fn pretend_loaded() {
	PRETEND.store(true, Ordering::Relaxed);
}

/// What to do for a crash or a freeze.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Action {
	Nothing,
	/// A test run: the log lines only.
	LogOnly,
	/// A test run with `RAC_TEST_RTSS_ANSWER`: the answer (true = Yes) without a box.
	Auto(bool),
	Ask,
}

fn decide(rtss: bool, vulkan: bool, test_mode: bool, answer: Option<&str>) -> Action {
	match (rtss && vulkan, answer, test_mode) {
		(false, ..) => Action::Nothing,
		(true, Some("yes"), _) => Action::Auto(true),
		(true, Some("no"), _) => Action::Auto(false),
		(true, _, true) => Action::LogOnly,
		(true, _, false) => Action::Ask,
	}
}

pub(crate) fn test_mode() -> bool {
	["RAC_TEST_WINDOW", "RAC_RETAIL_SMOKE", "RAC_TEST_CRASH"].iter().any(|k| std::env::var_os(k).is_some())
}

#[cfg(windows)]
fn rtss_loaded() -> bool {
	#[link(name = "kernel32")]
	unsafe extern "system" {
		fn GetModuleHandleA(name: *const u8) -> *mut std::ffi::c_void;
	}
	// SAFETY: a NUL-terminated name; the call only looks the module up.
	PRETEND.load(Ordering::Relaxed) || !unsafe { GetModuleHandleA(b"RTSSHooks64.dll\0".as_ptr()) }.is_null()
}

#[cfg(not(windows))]
fn rtss_loaded() -> bool {
	PRETEND.load(Ordering::Relaxed)
}

/// The box's text; `crash` false = the window froze at the start.
fn text(crash: bool) -> String {
	format!(
		"The game {} because RivaTuner Statistics Server (RTSS) is running. With current NVIDIA drivers RivaTuner only works with the DirectX 12 renderer.\n\nSwitch the renderer to DirectX 12 now? (You can change it later in Options > Graphics > Screen > Renderer, or in the launcher.)",
		if crash { "crashed" } else { "is not responding, probably" }
	)
}

/// Yes/No box (true = Yes), topmost.
#[cfg(windows)]
fn ask(text: &str) -> bool {
	#[link(name = "user32")]
	unsafe extern "system" {
		fn MessageBoxW(hwnd: *mut std::ffi::c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
	}
	let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
	// MB_YESNO | MB_ICONWARNING | MB_SETFOREGROUND | MB_TOPMOST; IDYES = 6.
	// SAFETY: NUL-terminated UTF-16 strings that outlive the call.
	unsafe { MessageBoxW(std::ptr::null_mut(), wide(text).as_ptr(), wide("Ratchet & Clank (rac-rs)").as_ptr(), 0x4 | 0x30 | 0x1_0000 | 0x4_0000) == 6 }
}

#[cfg(not(windows))]
fn ask(_text: &str) -> bool {
	false
}

/// `renderer=dx12` into the player's options file, every other key as it was.
fn switch_to_dx12(path: &std::path::Path) -> std::io::Result<()> {
	use crate::retail::pcopts::PcOptions;
	let mut o = PcOptions::load(path);
	o.renderer = Some(true);
	o.store(path)
}

/// Yes: the file gets `renderer=dx12`; the log says so.
fn answered_yes(log: fn(&str)) {
	let r = switch_to_dx12(&crate::retail::pcopts::file());
	log(&line(&format!("RivaTuner box: Yes, renderer=dx12 {}", if r.is_ok() { "saved" } else { "could not be saved" })));
}

fn line(s: &str) -> String {
	format!("runlog: +{:.3} {s}\n", crate::loadlog::secs())
}

fn handle(crash: bool, log: fn(&str)) {
	const WHY: &str = "RivaTuner (RTSSHooks64.dll) is loaded and the renderer is Vulkan:";
	let answer = std::env::var("RAC_TEST_RTSS_ANSWER").ok();
	match decide(rtss_loaded(), VULKAN.load(Ordering::Relaxed), test_mode(), answer.as_deref()) {
		Action::Nothing => {}
		Action::LogOnly => log(&line(&format!("{WHY} would show the RivaTuner box (test run, no box)"))),
		Action::Auto(yes) => {
			log(&line(&format!("{WHY} would show the RivaTuner box (test answer {})", if yes { "Yes" } else { "No" })));
			if yes {
				answered_yes(log);
			}
		}
		Action::Ask => {
			log(&line(&format!("{WHY} showing the RivaTuner box")));
			let Ok(h) = std::thread::Builder::new().name("rtss box".into()).spawn(move || ask(&text(crash))) else { return };
			if crash {
				// The crashed thread waits for the answer; the filter hands on afterwards.
				if h.join().unwrap_or(false) {
					answered_yes(log);
				}
			} else {
				// A freeze keeps the watchdog free: the answer is handled on its own thread.
				let _ = std::thread::Builder::new().name("rtss answer".into()).spawn(move || {
					if h.join().unwrap_or(false) {
						answered_yes(log);
					}
				});
			}
		}
	}
}

/// The crash filter, after the crash lines are written (waits for the box; the filter hands on afterwards).
pub fn on_crash(log: fn(&str)) {
	handle(true, log);
}

/// The start-up watchdog, after a long stall: once.
pub fn on_freeze(log: fn(&str)) {
	if !FROZEN_ASKED.swap(true, Ordering::Relaxed) {
		handle(false, log);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_box_needs_rtss_and_vulkan() {
		assert_eq!(decide(false, true, false, None), Action::Nothing);
		assert_eq!(decide(true, false, false, None), Action::Nothing, "DirectX 12 is the fix, not the case");
		assert_eq!(decide(true, true, false, None), Action::Ask);
	}

	#[test]
	fn tests_get_no_box() {
		assert_eq!(decide(true, true, true, None), Action::LogOnly);
		assert_eq!(decide(true, true, true, Some("yes")), Action::Auto(true));
		assert_eq!(decide(true, true, true, Some("no")), Action::Auto(false));
		assert_eq!(decide(true, true, true, Some("maybe")), Action::LogOnly);
		assert_eq!(decide(false, true, true, Some("yes")), Action::Nothing);
	}

	#[test]
	fn the_text_names_both_cases() {
		assert!(text(true).starts_with("The game crashed because RivaTuner"));
		assert!(text(false).contains("not responding"));
		assert!(text(true).contains("DirectX 12"));
	}

	#[test]
	fn yes_writes_dx12_and_keeps_the_other_keys() {
		let d = std::env::temp_dir().join(format!("rtss_crash_test_{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&d);
		std::fs::create_dir_all(&d).unwrap();
		let f = d.join("pc_options.txt");
		std::fs::write(&f, "camera=modern\nrenderer=vulkan\nsome_future_key=7\n").unwrap();
		switch_to_dx12(&f).unwrap();
		let t = std::fs::read_to_string(&f).unwrap();
		assert!(t.contains("renderer=dx12") && !t.contains("renderer=vulkan"), "{t}");
		assert!(t.contains("camera=modern") && t.contains("some_future_key=7"), "{t}");
		// no file yet: created with the one key
		let g = d.join("new").join("pc_options.txt");
		switch_to_dx12(&g).unwrap();
		assert!(std::fs::read_to_string(&g).unwrap().contains("renderer=dx12"));
		let _ = std::fs::remove_dir_all(&d);
	}
}
