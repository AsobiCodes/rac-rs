//! Keeps the display and the PC awake while the game window is in use (v0.4.1: Windows does not count gamepad input as activity, so
//! with a DualSense the screen went to sleep / the screensaver started after the idle time). `SetThreadExecutionState` with
//! ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED on the thread that runs the window loop, held exactly while the window has
//! the focus and is not minimised (`Awake::focus` / `minimise`, from the window events), so a game left in the background or
//! minimised lets the screen sleep again. The guard clears the request when it drops (a thread's request also ends with the thread).
//! A pad keeps working without the focus, so real pad input (`pad_input`) also resets the idle timers once a second, like a key press.
//! No-op off Windows and in the hidden-window test runs (RAC_TEST_WINDOW), which must not keep a dev PC awake.

const ES_SYSTEM_REQUIRED: u32 = 0x1;
const ES_DISPLAY_REQUIRED: u32 = 0x2;
const ES_CONTINUOUS: u32 = 0x8000_0000;

/// What `keep` asks for.
pub const WANT: u32 = ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED;

/// `SetThreadExecutionState(flags)` for the calling thread: the previous state, or 0 when this platform has none.
#[cfg(windows)]
pub fn set_state(flags: u32) -> u32 {
	#[link(name = "kernel32")]
	unsafe extern "system" {
		fn SetThreadExecutionState(flags: u32) -> u32;
	}
	// SAFETY: a plain kernel32 call with a flag word.
	unsafe { SetThreadExecutionState(flags) }
}

#[cfg(not(windows))]
pub fn set_state(_flags: u32) -> u32 {
	0
}

/// The request follows the game window: held while `wanted`, cleared otherwise and when this drops. It lives on the thread that runs
/// the window loop (the request is per thread): `WINDOW`, driven by `focus` / `minimise` from the window events.
struct Awake {
	enabled: bool,
	focused: bool,
	minimised: bool,
	held: bool,
}

/// The rule: the window has the focus and is not minimised.
fn wanted(focused: bool, minimised: bool) -> bool {
	focused && !minimised
}

impl Awake {
	/// Nothing is held until the window reports its focus.
	fn new() -> Awake {
		Awake::with(std::env::var_os("RAC_TEST_WINDOW").is_none())
	}

	fn with(enabled: bool) -> Awake {
		Awake { enabled, focused: false, minimised: false, held: false }
	}

	/// The window gained or lost the focus.
	fn focus(&mut self, focused: bool) {
		self.focused = focused;
		self.apply();
	}

	/// The window was minimised or restored.
	fn minimise(&mut self, minimised: bool) {
		self.minimised = minimised;
		self.apply();
	}

	fn apply(&mut self) {
		let want = self.enabled && wanted(self.focused, self.minimised);
		if want != self.held {
			set_state(if want { WANT } else { ES_CONTINUOUS });
			self.held = want;
		}
	}
}

thread_local! {
	static WINDOW: std::cell::RefCell<Awake> = std::cell::RefCell::new(Awake::new());
}

/// The game window gained or lost the focus (a window event, on the event-loop thread).
pub fn focus(focused: bool) {
	WINDOW.with(|w| w.borrow_mut().focus(focused));
}

/// The game window was minimised or restored (a window event, on the event-loop thread).
pub fn minimise(minimised: bool) {
	WINDOW.with(|w| w.borrow_mut().minimise(minimised));
}

/// The fewest milliseconds between two idle-timer resets from the pad.
const PAD_GAP_MS: u64 = 1000;
/// `LAST_PAD` before the first reset.
const NEVER: u64 = u64::MAX;
/// When (ms since the first `pad_input`) the idle timers were last reset from the pad.
static LAST_PAD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(NEVER);

/// The throttle: true (and `last` = `now_ms`) when the last reset is at least `PAD_GAP_MS` ago or there was none.
fn pad_due(last: &std::sync::atomic::AtomicU64, now_ms: u64) -> bool {
	use std::sync::atomic::Ordering::Relaxed;
	let l = last.load(Relaxed);
	if l != NEVER && now_ms.saturating_sub(l) < PAD_GAP_MS {
		return false;
	}
	last.store(now_ms, Relaxed);
	true
}

/// Real gamepad input was seen (retail/prompts.rs `take_pad_pulse`, from viewer/handler/cursor_vis.rs `apply`, event-loop thread): a
/// one-shot `SetThreadExecutionState(DISPLAY | SYSTEM)` without ES_CONTINUOUS, at most once a second. It resets the idle timers like a
/// key press and leaves the focus hold alone, so a pad-only player whose window lost the focus (the pad still plays) keeps the screen
/// on. Scripted pads (RAC_RETAIL_PAD, racpad replays) never reach here; nothing in the hidden-window test runs (RAC_TEST_WINDOW).
pub fn pad_input() {
	static ON: std::sync::OnceLock<(bool, std::time::Instant)> = std::sync::OnceLock::new();
	let (on, t0) = ON.get_or_init(|| (std::env::var_os("RAC_TEST_WINDOW").is_none(), std::time::Instant::now()));
	if *on && pad_due(&LAST_PAD, t0.elapsed().as_millis() as u64) {
		reset_idle();
	}
}

/// The one-shot idle-timer reset (no ES_CONTINUOUS: not a hold, and it leaves a hold alone).
fn reset_idle() {
	set_state(ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED);
}

impl Drop for Awake {
	fn drop(&mut self) {
		if self.held {
			set_state(ES_CONTINUOUS);
		}
	}
}

#[cfg(test)]
mod rule_tests {
	use super::*;

	/// The rule and the guard's bookkeeping (no OS call is checked here; see `tests` for the real request).
	#[test]
	fn held_only_while_focused_and_not_minimised() {
		for (f, m, want) in [(false, false, false), (true, false, true), (true, true, false), (false, true, false)] {
			assert_eq!(wanted(f, m), want, "focused {f} minimised {m}");
		}
		let mut a = Awake::with(true);
		assert!(!a.held, "nothing held before the window reports");
		a.focus(true);
		assert!(a.held);
		a.minimise(true);
		assert!(!a.held, "minimised");
		a.focus(false);
		a.minimise(false);
		assert!(!a.held, "restored but without the focus");
		a.focus(true);
		assert!(a.held, "focus again");
		a.focus(false);
		assert!(!a.held, "focus lost");
		let mut off = Awake::with(false);
		off.focus(true);
		assert!(!off.held, "disabled (test window) never holds");
	}

	/// Only real pad input resets the timers: the one call site sits behind `take_pad_pulse`, which only `poll_pad` (the XInput and
	/// PlayStation-controller readers; nothing under RAC_TEST_WINDOW=hidden / RAC_RETAIL_SMOKE) sets. The scripted pad (RAC_RETAIL_PAD,
	/// racpad replays: retail/tick.rs) and the scripted cursor device (RAC_CURSOR_DEV) never reach it.
	#[test]
	fn only_real_pad_input_calls_pad_input() {
		let vis = include_str!("viewer/handler/cursor_vis.rs");
		let call = "crate::awake::pad_input()";
		assert_eq!(vis.matches(call).count(), 1);
		let at = vis.find(call).unwrap();
		let pulse = vis.find("if crate::retail::prompts::take_pad_pulse() {").unwrap();
		let scripted = vis.find("if let Some(w) = scripted(frame)").unwrap();
		assert!(pulse < at && scripted < pulse && vis[pulse..at].matches('}').count() == 0, "inside the take_pad_pulse branch");
		for (name, src) in [("tick.rs", include_str!("retail/tick.rs")), ("input.rs", include_str!("retail/input.rs")), ("rec.rs", include_str!("retail/rec.rs"))] {
			assert!(!src.contains("pad_input"), "{name} must not call awake::pad_input");
		}
	}

	/// The pad throttle: the first input counts, then one per `PAD_GAP_MS`, measured from the last counted one.
	#[test]
	fn the_pad_resets_the_idle_timers_at_most_once_a_second() {
		use std::sync::atomic::AtomicU64;
		let last = AtomicU64::new(NEVER);
		assert!(pad_due(&last, 0), "the first input counts, also at time 0");
		for t in [1, 10, 500, PAD_GAP_MS - 1] {
			assert!(!pad_due(&last, t), "{t} ms: too soon");
		}
		assert!(pad_due(&last, PAD_GAP_MS), "a second after the last one");
		assert!(!pad_due(&last, PAD_GAP_MS + 999), "the gap runs from the last counted input, not from the first");
		assert!(pad_due(&last, 2 * PAD_GAP_MS), "and again");
		let early = AtomicU64::new(5_000);
		assert!(!pad_due(&early, 4_000), "a clock that went back never counts as a gap");
	}
}

#[cfg(all(test, windows))]
mod tests {
	use super::*;

	/// What this thread's request holds now (calling with no flags returns the previous state and changes nothing).
	fn state() -> u32 {
		set_state(0)
	}

	/// On a thread of its own (the request is per thread), through the shipped guard: the OS request is set when the window is focused,
	/// cleared on minimise, set again on restore, and cleared by the guard's drop.
	#[test]
	fn the_guard_sets_and_clears_the_os_request() {
		std::thread::spawn(|| {
			let bits = ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED;
			let mut a = Awake::with(true);
			assert_eq!(state() & bits, 0, "nothing before the window reports");
			a.focus(true);
			assert_eq!(state() & WANT, WANT, "focused");
			a.minimise(true);
			assert_eq!(state() & bits, 0, "minimised");
			a.minimise(false);
			assert_eq!(state() & WANT, WANT, "restored");
			a.focus(false);
			assert_eq!(state() & bits, 0, "focus lost");
			a.focus(true);
			assert_eq!(state() & WANT, WANT, "focus again");
			drop(a);
			assert_eq!(state() & bits, 0, "after the guard dropped");
			let mut off = Awake::with(false);
			off.focus(true);
			assert_eq!(state() & bits, 0, "a disabled guard asks for nothing");
		})
		.join()
		.unwrap();
	}

	/// The pad's one-shot reset, on a thread of its own, against the real OS: it creates no hold of its own (it is not continuous) and it
	/// does not drop the focus hold that is there.
	#[test]
	fn the_pad_reset_is_one_shot_and_keeps_the_focus_hold() {
		std::thread::spawn(|| {
			let bits = ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED;
			reset_idle();
			assert_eq!(state() & bits, 0, "no hold from the one-shot alone");
			let mut a = Awake::with(true);
			a.focus(true);
			reset_idle();
			assert_eq!(state() & WANT, WANT, "the focus hold survives the reset");
			a.focus(false);
			assert_eq!(state() & bits, 0, "and still ends with the focus");
			reset_idle();
			assert_eq!(state() & bits, 0, "an unfocused window is not held by a reset");
		})
		.join()
		.unwrap();
	}
}
