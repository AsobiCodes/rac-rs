//! [PC] wgpu's errors that no error scope catches (validation errors, out of memory) and the lost-device report. wgpu's default
//! handler panics; on an enc-finish thread (presenter.rs: `CommandEncoder::finish` validates the whole frame there) that panic
//! aborts the process (player crash 2026-11-03, 0xC0000409 right after an option change in the pause menu). For players the error
//! is one log line and the game goes on: wgpu marks the command buffer invalid and the frame's whole submit fails, but the
//! frame's window image is still presented (presenter.rs). No submit drew that image: on Vulkan it is in no defined layout and
//! nothing waited on its acquire, so the window shows garbage or the last picture for that frame (validation layers flag it). A
//! discard is no better: each one keeps an acquired image until the next configure, and a few in a row can block the next
//! acquire. The next frame (gpu/frame_sizes.rs plans the sizes again) draws normally.
//! [PC] A LASTING error must not become a silent frozen window (review R14 F1: a pipeline a driver refuses outside pipe_guard,
//! an out-of-memory target at a big Render scale or with an upscaler on an 8 GB Steam Deck, a target the frame_sizes rules miss).
//! `frame_check` (first thing in `Gpu::draw`, main thread) feeds `Escalate` with the frames that saw new errors. Errors in most
//! frames (no 4 clean frames in a row) for 2 s put the game's size options back to their defaults (Pack `reset_sizes`: applied
//! and saved at once, on any screen; asked again each frame for up to 10 s while the game has not read its options yet), with a
//! log line; another such 2 s panic on the main thread (crash.log and the crash box in player builds, as before the lenient
//! handler), and so does the first 2 s in a game without size options. A clean minute re-arms the reset. Frames slower than 1 s
//! each never escalate (each starts the count again, as a minimised window does). Out of memory goes the same way: strict, it
//! would panic on whichever thread met it (the present worker included), and a size reset is its likely cure. A panicking
//! enc-finish job (presenter.rs `finish_pool`) counts too.
//! [PC] A lost device (driver reset, GPU timeout; wgpu also loses it on an out-of-memory from the driver) reports no errors, so
//! the window would stay black: `frame_check` panics at the next frame instead (review R15 m2), after the size reset when the
//! device ran out of memory (the next start then begins with the defaults).
//! Strict (the panic, as before) in the agent test runs (RAC_TEST_WINDOW=hidden, RAC_RETAIL_SMOKE) so a validation error still
//! fails a test, and inside `texpack::guard::guarded` (the overlay helper, ovwrite.rs, and the texture packs catch that panic and
//! fall back). RAC_GPU_STRICT=1 / 0 forces strict / lenient, RAC_GPU_FAKE_ERRORS=1 (tests) one made-up error per frame (ENV.md).
//! No on-screen notice of the reset (the engine has no general one): the log line says it, and the crash text names it.
//! On DX12 the GS capture has a device of its own (gpu/setup.rs), which keeps wgpu's panicking handler.

use crate::pack::SizeReset;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

/// Errors the lenient handler has let through in this run (STATE.md).
static COUNT: AtomicU32 = AtomicU32::new(0);
/// The last of them, as wgpu wrote it (made one line only when shown: `last_line`).
static LAST: Mutex<String> = Mutex::new(String::new());
/// The device was lost (not destroyed): `frame_check` panics at the next frame.
static LOST: AtomicBool = AtomicBool::new(false);
/// Why: wgpu's reason and message.
static LOST_WHY: Mutex<String> = Mutex::new(String::new());

/// Whether a wgpu error panics (agent test runs, RAC_GPU_STRICT=1) instead of being logged.
pub fn strict() -> bool {
	static STRICT: OnceLock<bool> = OnceLock::new();
	*STRICT.get_or_init(|| {
		let test_run = std::env::var_os("RAC_TEST_WINDOW").is_some_and(|v| v == "hidden") || std::env::var_os("RAC_RETAIL_SMOKE").is_some();
		strict_of(std::env::var("RAC_GPU_STRICT").ok().as_deref(), test_run)
	})
}

/// `strict` for a RAC_GPU_STRICT value and whether this is an agent test run.
pub fn strict_of(v: Option<&str>, test_run: bool) -> bool {
	match v.map(str::trim) {
		Some("1") => true,
		Some("0") => false,
		_ => test_run,
	}
}

/// The log's rate limit for the `n`-th event (1-based): the first 8, then every power of two.
pub fn logs(n: u32) -> bool {
	n <= 8 || n.is_power_of_two()
}

/// The errors let through so far.
#[cfg(test)]
pub fn count() -> u32 {
	COUNT.load(Ordering::Relaxed)
}

/// wgpu's multi-line error text as one log line (at most 400 characters).
fn one_line(e: &str) -> String {
	let s = e.split_whitespace().collect::<Vec<_>>().join(" ");
	match s.char_indices().nth(400) {
		Some((i, _)) => format!("{}...", &s[..i]),
		None => s,
	}
}

/// The last error as one line.
fn last_line() -> String {
	LAST.lock().map(|l| one_line(&l)).unwrap_or_default()
}

/// The uncaptured-error handler: panics when `strict` (or inside `texpack::guard::guarded`), else counts and logs.
fn handler(strict: bool) -> impl Fn(wgpu::Error) + Send + Sync + 'static {
	move |e| {
		if strict || crate::texpack::guard::panic_expected() {
			panic!("wgpu error: {e}\n");
		}
		note("wgpu error", &e.to_string());
	}
}

/// Any thread: one lenient failure (`what`: "wgpu error", or presenter.rs's panicking enc-finish job) counted for `frame_check`
/// and logged (rate-limited, `logs`). The text is kept as it came (no allocation once the buffer is warm); only a logged one is
/// made one line here.
pub(super) fn note(what: &str, text: &str) {
	let n = COUNT.fetch_add(1, Ordering::Relaxed) + 1;
	if logs(n) {
		eprintln!("gpu: {what} #{n} (not fatal: the frame's submit fails, the game goes on): {}", one_line(text));
	}
	if let Ok(mut l) = LAST.lock() {
		l.clear();
		l.push_str(text);
	}
}

/// What `Escalate::step` asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
	Go,
	/// Errors in most frames for `SPAN_MS`: the size options back to their defaults; `frame_check` reports the outcome
	/// (`Escalate::reset_done`). Asked again every frame while the game cannot apply it yet, `HOLD_MS` at most.
	ResetSizes,
	/// Errors in most frames for another `SPAN_MS` after the reset, or the reset was not applied within `HOLD_MS`: panic
	/// (crash.log, the crash box).
	Crash,
}

/// How long the errors must come in (in most frames) before each step.
const SPAN_MS: u64 = 2000;
/// Clean frames in a row that end a run of bad ones. Fewer are let through: a frame's errors come back from the present worker's
/// submit one or two frames late, so a bad run reads as bad frames with the odd clean one in between. So errors in 1 frame of 4
/// still make a run.
const CLEAN_FRAMES: u32 = 4;
/// A pause between two drawn frames longer than this starts the count again (a minimised window draws nothing; so do frames
/// that each take longer than this).
const GAP_MS: u64 = 1000;
/// Bad frames a run needs besides the time (a few very slow frames are not "most frames for 2 s").
const MIN_BAD: u32 = 20;
/// How long the game may take to apply the reset (it has not read its options file yet) before the crash.
const HOLD_MS: u64 = 10_000;
/// A clean stretch this long after the last bad frame re-arms the reset (the player may have chosen big sizes again).
const REARM_MS: u64 = 60_000;

/// The escalation state (pure: the time comes in, `frame_check` feeds it).
#[derive(Debug, Default)]
pub struct Escalate {
	/// When the current run of bad frames began (None: no run).
	start: Option<u64>,
	/// The last drawn frame's time.
	last: Option<u64>,
	/// The last bad frame's time.
	last_bad: Option<u64>,
	/// Bad frames in the run, clean frames in a row.
	bad: u32,
	clean: u32,
	/// Steps taken: 0, 1 = sizes reset, 2 = crashed.
	stage: u8,
	/// The reset was asked for at this time and the game could not apply it yet.
	pending: Option<u64>,
	/// When the last reset was applied (the crash text says how long before).
	reset_at: Option<u64>,
}

impl Escalate {
	pub const fn new() -> Escalate {
		Escalate { start: None, last: None, last_bad: None, bad: 0, clean: 0, stage: 0, pending: None, reset_at: None }
	}

	/// One drawn frame at `now_ms`; `errors`: new lenient errors came in since the previous frame.
	pub fn step(&mut self, now_ms: u64, errors: bool) -> Act {
		if let Some(t) = self.pending {
			// The game could not apply the reset yet: asked again every frame, the crash after HOLD_MS.
			self.last = Some(now_ms);
			if now_ms.saturating_sub(t) < HOLD_MS {
				return Act::ResetSizes;
			}
			(self.pending, self.stage) = (None, 2);
			return Act::Crash;
		}
		if self.last.is_some_and(|t| now_ms.saturating_sub(t) > GAP_MS) {
			self.start = None;
		}
		self.last = Some(now_ms);
		if !errors {
			self.clean += 1;
			if self.clean >= CLEAN_FRAMES {
				self.start = None;
			}
			return Act::Go;
		}
		if self.stage == 1 && self.last_bad.is_some_and(|t| now_ms.saturating_sub(t) > REARM_MS) {
			self.stage = 0;
		}
		self.last_bad = Some(now_ms);
		self.clean = 0;
		let start = match self.start {
			Some(s) => s,
			None => {
				(self.start, self.bad) = (Some(now_ms), 0);
				now_ms
			}
		};
		self.bad += 1;
		if now_ms.saturating_sub(start) < SPAN_MS || self.bad < MIN_BAD {
			return Act::Go;
		}
		// The next step needs a run of its own (the reset applies at the next frame's re-plan).
		self.start = None;
		self.stage = self.stage.saturating_add(1);
		if self.stage == 1 { Act::ResetSizes } else { Act::Crash }
	}

	/// After `ResetSizes`: `applied` = the game put its size options back and saved them; false = not yet (asked again).
	pub fn reset_done(&mut self, now_ms: u64, applied: bool) {
		if applied {
			(self.pending, self.reset_at) = (None, Some(now_ms));
		} else {
			self.pending.get_or_insert(now_ms);
			self.reset_at = None;
		}
	}
}

/// RAC_GPU_FAKE_ERRORS=1 (tests only, ENV.md): every frame counts one made-up error, the proof of the escalation in a game run.
fn fake() -> bool {
	static ON: OnceLock<bool> = OnceLock::new();
	*ON.get_or_init(|| std::env::var("RAC_GPU_FAKE_ERRORS").is_ok_and(|v| v.trim() == "1"))
}

/// `Gpu::draw`, first thing (main thread): a lost device panics; new errors since the last frame go into the escalation, which
/// resets the game's size options or panics here, on the main thread, so the player gets the crash report instead of a frozen
/// window. Nothing to do (no lock, no clock) while no error came in: two Relaxed loads.
pub fn frame_check() {
	static SEEN: AtomicU32 = AtomicU32::new(0);
	static ESC: Mutex<Escalate> = Mutex::new(Escalate::new());
	static T0: OnceLock<Instant> = OnceLock::new();
	if LOST.load(Ordering::Relaxed) {
		let why = LOST_WHY.lock().map(|w| w.clone()).unwrap_or_default();
		panic!("{}", lost_text(&why, crate::pack::get().reset_sizes.as_ref()));
	}
	if fake() {
		note("made-up error (RAC_GPU_FAKE_ERRORS)", "RAC_GPU_FAKE_ERRORS=1");
	}
	let n = COUNT.load(Ordering::Relaxed);
	if n == 0 {
		return;
	}
	let errors = SEEN.swap(n, Ordering::Relaxed) != n;
	let now = T0.get_or_init(Instant::now).elapsed().as_millis() as u64;
	let Ok(mut e) = ESC.lock() else { return };
	if let Err(text) = check(&mut e, now, errors, n, crate::pack::get().reset_sizes.as_ref()) {
		drop(e);
		panic!("{text}");
	}
}

/// `frame_check` after the counting (the unit tests' door): one frame at `now` into `e` with `n` errors so far; the reset
/// through the game's `reset`. Err: the panic text.
fn check(e: &mut Escalate, now: u64, errors: bool, n: u32, reset: Option<&SizeReset>) -> Result<(), String> {
	let tail = || format!("({n} errors; last: {})", last_line());
	match e.step(now, errors) {
		Act::Go => Ok(()),
		Act::ResetSizes => {
			let Some(r) = reset else {
				return Err(format!("wgpu errors in most frames for 2 s; this game has no size options to put back to their defaults {}", tail()));
			};
			let first = e.pending.is_none();
			let applied = (r.reset)();
			e.reset_done(now, applied);
			if applied {
				eprintln!("gpu: wgpu errors in most frames for 2 s {}: the game's {} went back to their defaults (saved)", tail(), r.names);
			} else if first {
				eprintln!("gpu: wgpu errors in most frames for 2 s {}: the game's {} go back to their defaults once it has read its options", tail(), r.names);
			}
			Ok(())
		}
		Act::Crash => {
			let names = reset.map_or("size options", |r| r.names);
			Err(match e.reset_at {
				Some(t) => format!("wgpu errors in most frames for 2 s, again {:.1} s after the game's {names} went back to their defaults (saved) {}", now.saturating_sub(t) as f64 / 1000.0, tail()),
				None => format!("wgpu errors in most frames for 2 s; the game's {names} could not go back to their defaults (its options were not read within 10 s) {}", tail()),
			})
		}
	}
}

/// The panic text of a lost device (`why`: wgpu's reason and message). Out of memory: the game's size options go back to their
/// defaults first (saved), so the next start begins with them.
fn lost_text(why: &str, reset: Option<&SizeReset>) -> String {
	let oom = why.contains("Out of memory");
	let what = if oom { "the graphics card ran out of memory" } else { "a graphics driver reset or a GPU timeout" };
	let after = match reset.filter(|_| oom) {
		Some(r) if (r.reset)() => format!("; the game's {} went back to their defaults (saved) for the next start", r.names),
		Some(r) => format!("; the game's {} could not go back to their defaults (its options were not read yet)", r.names),
		None => String::new(),
	};
	format!("the graphics device was lost ({what}: {why}): the game cannot draw any more{after}")
}

/// `Gpu::new`, right after the device is made: the error handler above and the lost-device report.
pub fn install(device: &wgpu::Device) {
	device.on_uncaptured_error(Arc::new(handler(strict())));
	// wgpu reports no errors of a lost device (driver reset, GPU timeout, a driver's out of memory) and the window just stays
	// blank: the log says so now, `frame_check` panics at the next frame.
	device.set_device_lost_callback(|reason, msg| {
		if reason != wgpu::DeviceLostReason::Destroyed {
			eprintln!("gpu: the graphics device was lost ({reason:?}): {msg}");
			if let Ok(mut w) = LOST_WHY.lock() {
				*w = format!("{reason:?}, {}", one_line(&msg));
			}
			LOST.store(true, Ordering::Relaxed);
		}
	});
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn strict_switch() {
		assert!(strict_of(Some("1"), false));
		assert!(!strict_of(Some("0"), true), "0 wins over a test run");
		assert!(strict_of(None, true), "agent test runs panic as before");
		assert!(!strict_of(None, false), "players: logged, no panic");
		assert!(!strict_of(Some("yes"), false));
	}

	#[test]
	fn log_rate_limit() {
		let logged: Vec<u32> = (1..=300).filter(|&n| logs(n)).collect();
		assert_eq!(logged, [1, 2, 3, 4, 5, 6, 7, 8, 16, 32, 64, 128, 256]);
	}

	#[test]
	fn one_line_joins_and_cuts() {
		assert_eq!(one_line("Validation Error\n\nCaused by:\n  In a"), "Validation Error Caused by: In a");
		let long = one_line(&"y ".repeat(500));
		assert_eq!(long.chars().count(), 403);
	}

	/// Frames every `dt` ms from `from` while `bad(i)` says whether frame i saw errors; the first action that is not `Go`, with
	/// its time.
	fn run(e: &mut Escalate, from: u64, dt: u64, frames: u64, bad: impl Fn(u64) -> bool) -> Option<(u64, Act)> {
		(0..frames).map(|i| (from + i * dt, bad(i))).find_map(|(t, b)| Some((t, e.step(t, b))).filter(|(_, a)| *a != Act::Go))
	}

	#[test]
	fn lasting_errors_reset_the_sizes_then_crash() {
		let mut e = Escalate::new();
		let (t, a) = run(&mut e, 0, 16, 1000, |_| true).expect("escalates");
		assert_eq!(a, Act::ResetSizes);
		assert!((SPAN_MS..SPAN_MS + 20).contains(&t), "after 2 s: {t}");
		let (t2, a2) = run(&mut e, t + 16, 16, 1000, |_| true).expect("escalates again");
		assert_eq!(a2, Act::Crash);
		assert!((t + SPAN_MS..t + SPAN_MS + 40).contains(&t2), "2 s after the reset: {t2}");
	}

	#[test]
	fn the_odd_clean_frame_does_not_end_a_run_but_a_few_do() {
		// The present worker's errors land a frame late now and then: every 3rd frame clean is still one run.
		let mut e = Escalate::new();
		assert_eq!(run(&mut e, 0, 16, 200, |i| i % 3 != 0).map(|r| r.1), Some(Act::ResetSizes));
		// CLEAN_FRAMES clean frames between bursts of 1 s: never 2 s in a row.
		let mut e = Escalate::new();
		let period = 62 + CLEAN_FRAMES as u64;
		assert_eq!(run(&mut e, 0, 16, 2000, |i| i % period < 62), None);
	}

	#[test]
	fn sparse_errors_never_escalate() {
		let mut e = Escalate::new();
		assert_eq!(run(&mut e, 0, 16, 10_000, |i| i % 30 == 0), None, "one bad frame in 30");
	}

	#[test]
	fn a_pause_in_the_drawing_starts_the_count_again() {
		let mut e = Escalate::new();
		assert_eq!(run(&mut e, 0, 16, 100, |_| true), None, "1.6 s of errors");
		// The window was minimised for 5 s: the next bad frames start a new run.
		assert_eq!(run(&mut e, 6600, 16, 100, |_| true), None, "another 1.6 s, not 8 s");
	}

	#[test]
	fn very_slow_frames_need_enough_of_them() {
		let mut e = Escalate::new();
		// 4 frames per second, all bad: 2 s is 8 frames, fewer than MIN_BAD.
		let (t, a) = run(&mut e, 0, 250, 100, |_| true).expect("escalates later");
		assert_eq!((a, t), (Act::ResetSizes, (MIN_BAD as u64 - 1) * 250));
	}

	#[test]
	fn a_cured_run_does_not_crash_until_errors_last_again() {
		let mut e = Escalate::new();
		let (t, _) = run(&mut e, 0, 16, 1000, |_| true).expect("reset");
		e.reset_done(t, true);
		assert_eq!(run(&mut e, t + 16, 16, 1000, |_| false), None, "the reset cured it");
		assert_eq!(run(&mut e, t + 16 + 1000 * 16, 16, 1000, |_| true).map(|r| r.1), Some(Act::Crash), "back within a minute: the crash");
	}

	#[test]
	fn a_clean_minute_arms_the_reset_again() {
		let mut e = Escalate::new();
		let (t, _) = run(&mut e, 0, 16, 1000, |_| true).expect("reset");
		e.reset_done(t, true);
		// 61 s clean (the player chose big sizes again later): the next lasting run resets them again, then crashes.
		let back = t + 16 + 61_000;
		assert_eq!(run(&mut e, t + 16, 16, 61_000 / 16, |_| false), None);
		let (t2, a2) = run(&mut e, back, 16, 1000, |_| true).expect("escalates");
		assert_eq!(a2, Act::ResetSizes);
		e.reset_done(t2, true);
		assert_eq!(run(&mut e, t2 + 16, 16, 1000, |_| true).map(|r| r.1), Some(Act::Crash));
	}

	#[test]
	fn a_reset_the_game_cannot_apply_yet_is_asked_again_then_crashes() {
		let mut e = Escalate::new();
		let (t, _) = run(&mut e, 0, 16, 1000, |_| true).expect("reset");
		e.reset_done(t, false);
		// Asked again every frame, errors or not, until HOLD_MS.
		assert_eq!(e.step(t + 16, false), Act::ResetSizes);
		e.reset_done(t + 16, false);
		assert_eq!(e.step(t + HOLD_MS - 1, true), Act::ResetSizes);
		e.reset_done(t + HOLD_MS - 1, false);
		assert_eq!(e.step(t + HOLD_MS, true), Act::Crash, "never forever");
		// Applied on a later ask: the hold ends, the next lasting run crashes as usual.
		let mut e = Escalate::new();
		let (t, _) = run(&mut e, 0, 16, 1000, |_| true).expect("reset");
		e.reset_done(t, false);
		assert_eq!(e.step(t + 500, true), Act::ResetSizes);
		e.reset_done(t + 500, true);
		assert_eq!(run(&mut e, t + 516, 16, 1000, |_| false), None, "applied: the hold is over");
	}

	static APPLY: AtomicBool = AtomicBool::new(false);
	static CALLS: AtomicU32 = AtomicU32::new(0);

	fn test_reset() -> bool {
		CALLS.fetch_add(1, Ordering::Relaxed);
		APPLY.load(Ordering::Relaxed)
	}

	const SIZES: SizeReset = SizeReset { names: "Upscaler and Render scale", reset: test_reset };

	/// Frames into `check` every 16 ms from `from`, all bad; the first panic text with its time.
	fn check_run(e: &mut Escalate, from: u64, frames: u64, reset: Option<&SizeReset>) -> Option<(u64, String)> {
		(0..frames).map(|i| from + i * 16).find_map(|t| check(e, t, true, 1, reset).err().map(|s| (t, s)))
	}

	/// The frame_check glue (review R15 n2): the game's hook is called once the run lasts, the next run's panic names the reset and
	/// when it was; a game without size options panics at the first step and says so; a reset the game cannot apply yet is asked
	/// for again every frame and the panic says it was not applied.
	#[test]
	fn the_glue_resets_through_the_pack_and_says_so_when_it_crashes() {
		let _turn = CHECK_TURN.lock().unwrap_or_else(|e| e.into_inner());
		APPLY.store(true, Ordering::Relaxed);
		let before = CALLS.load(Ordering::Relaxed);
		let mut e = Escalate::new();
		let (t, text) = check_run(&mut e, 0, 1000, Some(&SIZES)).expect("crashes");
		assert_eq!(CALLS.load(Ordering::Relaxed), before + 1, "one reset");
		assert!((2 * SPAN_MS..2 * SPAN_MS + 60).contains(&t), "{t}");
		assert!(text.contains("again 2.0 s after the game's Upscaler and Render scale went back to their defaults (saved)"), "{text}");
		let mut e = Escalate::new();
		let (t, text) = check_run(&mut e, 0, 1000, None).expect("crashes");
		assert!((SPAN_MS..SPAN_MS + 20).contains(&t), "at the first step: {t}");
		assert!(text.contains("this game has no size options"), "{text}");
		APPLY.store(false, Ordering::Relaxed);
		let before = CALLS.load(Ordering::Relaxed);
		let mut e = Escalate::new();
		let (t, text) = check_run(&mut e, 0, 2000, Some(&SIZES)).expect("crashes");
		assert!((SPAN_MS + HOLD_MS..SPAN_MS + HOLD_MS + 20).contains(&t), "after the hold: {t}");
		assert!(CALLS.load(Ordering::Relaxed) - before > 500, "asked every frame of the hold");
		assert!(text.contains("could not go back to their defaults"), "{text}");
	}

	/// Review R15 m2: a lost device's text; out of memory resets the sizes first (for the next start), a driver reset does not.
	#[test]
	fn a_lost_device_says_why_and_resets_the_sizes_only_when_out_of_memory() {
		let _turn = CHECK_TURN.lock().unwrap_or_else(|e| e.into_inner());
		APPLY.store(true, Ordering::Relaxed);
		let before = CALLS.load(Ordering::Relaxed);
		let t = lost_text("Unknown, Device is lost", Some(&SIZES));
		assert!(t.contains("driver reset or a GPU timeout") && !t.contains("defaults"), "{t}");
		assert_eq!(CALLS.load(Ordering::Relaxed), before, "no reset");
		let t = lost_text("Unknown, Out of memory", Some(&SIZES));
		assert!(t.contains("ran out of memory") && t.contains("Upscaler and Render scale went back to their defaults (saved) for the next start"), "{t}");
		assert_eq!(CALLS.load(Ordering::Relaxed), before + 1);
		assert!(!lost_text("Unknown, Out of memory", None).contains("defaults"), "no size options: nothing claimed");
	}

	/// The tests that use `APPLY` / `CALLS` take turns.
	static CHECK_TURN: Mutex<()> = Mutex::new(());

	#[test]
	fn a_lenient_failure_is_counted() {
		let before = count();
		note("test failure", "line one\n  line two");
		assert!(count() > before);
		assert!(!LAST.lock().unwrap().is_empty());
	}

	fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
		let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
		desc.backends = wgpu::Backends::VULKAN;
		let inst = wgpu::Instance::new(desc);
		let adapter = pollster::block_on(inst.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
		pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
	}

	fn target(device: &wgpu::Device, size: u32, samples: u32) -> wgpu::TextureView {
		device
			.create_texture(&wgpu::TextureDescriptor {
				label: Some("gpu_errors test"),
				size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
				mip_level_count: 1,
				sample_count: samples,
				dimension: wgpu::TextureDimension::D2,
				format: wgpu::TextureFormat::Rgba8Unorm,
				usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
				view_formats: &[],
			})
			.create_view(&Default::default())
	}

	/// The player crash's error on a real device (skipped without Vulkan): an 8x8 MSAA colour resolved into a 16x16 picture,
	/// finished on another thread (as the enc-finish pool does) and submitted. Lenient: no panic, the error is counted.
	#[test]
	fn a_validation_error_off_the_render_thread_is_logged_not_fatal() {
		let Some((device, queue)) = device() else { return };
		device.on_uncaptured_error(Arc::new(handler(false)));
		let (msaa, win) = (target(&device, 8, 4), target(&device, 16, 1));
		let before = count();
		let mut enc = device.create_command_encoder(&Default::default());
		drop(enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("hud"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &msaa, resolve_target: Some(&win), depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Discard } })],
			depth_stencil_attachment: None,
			timestamp_writes: None,
			occlusion_query_set: None,
			multiview_mask: None,
		}));
		let cmd = std::thread::spawn(move || enc.finish()).join().expect("finish must not panic");
		queue.submit([cmd]);
		let _ = device.poll(wgpu::PollType::wait_indefinitely());
		assert!(count() > before, "the error was seen and counted");
	}
}
