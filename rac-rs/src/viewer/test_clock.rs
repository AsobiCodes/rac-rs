//! [PC] TEST ONLY: the regression gate's two switches (gate/gate.ps1), never set by players. With both unset every function here
//! hands back the wall-clock value it was given (`secs`: as at the frame's start, frame_clock.rs) and the program runs exactly as before.
//! - RAC_TEST_CLOCK=1: every effect time that feeds pixels (water, vegetation wind, underwater, sun-shadow easing, the post
//!   shimmer, the HUD clock, the display interpolation fraction) comes from the retail fields shown so far, `secs` = fields *
//!   one field's time, not from the wall clock. Two runs of one exe from one savestate and pad then draw the same pictures.
//!   The counter is the retail snapshot's field number (`Retail::shown_field`), set once per game tick (handler/frame_ticks.rs)
//!   and once per front-end field (front_mirror.rs `front_mirror_tick`: cold boot, title, New Game intro).
//! - RAC_TEST_UNPACED=1: no real-time pacing: every frame runs exactly one field (handler/frame_start.rs `frame_input`, and
//!   front_mirror.rs `front_mirror_tick` in the front end) and the end-of-frame cap (`Pace::wait`, handler/frame_render.rs) is
//!   skipped. RAC_TEST_CLOCK turns it on as well. In-level movies still play in real time (fmv/player.rs keeps its own wall
//!   clock); under RAC_TEST_CLOCK the front-end movies (logo, attract, New Game intro) end at once (front_mirror.rs).

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// Retail fields shown so far (the snapshot's field number after the last tick).
static FIELDS: AtomicU64 = AtomicU64::new(0);

/// The `dt` consumers: each keeps the field count of its previous call, so it gets the fields shown since then.
#[derive(Clone, Copy)]
pub(super) enum Site {
	Frame,
	Sun,
	Under,
}

const UNSET: u64 = u64::MAX;
static LAST: [AtomicU64; 3] = [AtomicU64::new(UNSET), AtomicU64::new(UNSET), AtomicU64::new(UNSET)];

fn env_on(cell: &'static OnceLock<bool>, name: &str) -> bool {
	*cell.get_or_init(|| std::env::var(name).is_ok_and(|v| v == "1"))
}

/// RAC_TEST_CLOCK=1: effect times come from the field count.
pub(super) fn clock_on() -> bool {
	static ON: OnceLock<bool> = OnceLock::new();
	env_on(&ON, "RAC_TEST_CLOCK")
}

/// RAC_TEST_UNPACED=1: one field per frame, no frame cap. RAC_TEST_CLOCK implies it: the frame time then comes from the
/// field count, so a paced loop would wait for fields that never come.
pub(super) fn unpaced() -> bool {
	static ON: OnceLock<bool> = OnceLock::new();
	env_on(&ON, "RAC_TEST_UNPACED") || clock_on()
}

/// After a game tick: the retail field now shown.
pub(super) fn set_fields(n: u32) {
	FIELDS.store(n as u64, Relaxed);
}

/// Effect time in 1/N fields (RAC_TEST_SUB; whole fields without it): the fields shown plus the frame's place inside its field.
fn now() -> u64 {
	FIELDS.load(Relaxed) * sub() as u64 + SUB_K.load(Relaxed)
}

/// Seconds of effect time: `wall()` as at the frame's start normally (frame_clock.rs), else the fields shown * one field's time.
pub(super) fn secs(wall: impl FnOnce() -> f32) -> f32 {
	if clock_on() {
		now() as f32 / sub() as f32 * crate::retail::disc::field_dt()
	} else {
		// [PC] The effect time as at the frame's start (frame_clock.rs): not at the late moment the pass is encoded.
		let w = wall();
		w - super::handler::frame_clock::lag()
	}
}

/// Seconds since the previous call of this site: `wall()` normally, else the fields shown since then * one field's time
/// (0 at the first call).
pub(super) fn dt(site: Site, wall: impl FnOnce() -> f32) -> f32 {
	if !clock_on() {
		return wall();
	}
	let n = now();
	let last = LAST[site as usize].swap(n, Relaxed);
	if last == UNSET { 0.0 } else { n.saturating_sub(last) as f32 / sub() as f32 * crate::retail::disc::field_dt() }
}

/// The display interpolation fraction (0..1): `wall` normally, else 0 (the last field's state), or with RAC_TEST_SUB=N the
/// frame's place inside its field (k / N).
pub(super) fn alpha(wall: f32) -> f32 {
	if clock_on() { SUB_K.load(Relaxed) as f32 / sub() as f32 } else { wall }
}

/// RAC_TEST_SUB=N (with RAC_TEST_CLOCK / RAC_TEST_UNPACED): N display frames per retail field, drawn at alpha 0, 1/N, ..
/// (frame smoothing between fields as a fast display shows it, deterministic). Default 1 = one field per frame.
pub(crate) fn sub() -> u32 {
	static N: OnceLock<u32> = OnceLock::new();
	*N.get_or_init(|| std::env::var("RAC_TEST_SUB").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(1).max(1))
}

/// The next frame's place inside its field (0 = the frame that runs the field), RAC_TEST_SUB.
static SUB_K: AtomicU64 = AtomicU64::new(0);

/// Unpaced: whether this frame runs a field (always without RAC_TEST_SUB; with it, the field's first frame, and in a shot run
/// only while the shot still waits for its field: `wait`, so no extra tick makes the previous camera equal the newest).
pub(super) fn sub_ticks(wait: impl FnOnce() -> bool) -> bool {
	sub() == 1 || SUB_K.load(Relaxed) == 0 && wait()
}

/// End of a drawn frame: the next frame's place inside its field. In a shot run (`shot`) only a captured frame moves on, so
/// the captures run alpha 0, 1/N, .. and the shot step (one field every N captures) lines up with the place.
pub(crate) fn sub_next(shot: bool, captured: bool) {
	if sub() > 1 && (!shot || captured) {
		SUB_K.store((SUB_K.load(Relaxed) + 1) % sub() as u64, Relaxed);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn switches_off_return_the_wall_value() {
		assert!(!clock_on() && !unpaced());
		// (The wall time as at the frame's start: less by the time since the frame began, frame_clock.rs.)
		assert!((1.15..=1.25).contains(&secs(|| 1.25)));
		assert_eq!(dt(Site::Frame, || 0.5), 0.5);
		assert_eq!(alpha(0.75), 0.75);
	}
}
