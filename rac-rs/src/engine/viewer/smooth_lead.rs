//! [PC] Frame smoothing's sample time at high frame rates (GitHub #28 / #57: micro-stutter while the camera pans at 120-240 Hz,
//! no retail counterpart). A display frame takes its interpolation fraction from the clock at the start of its redraw, but a frame
//! that runs a game tick (50 or 60 a second) does that field's work in the same redraw (new props, skinning, uploads, culling) and
//! reaches the screen ~2-3 ms later than a frame without one (old exe, 240 fps cap, Eudora walk: present minus sample 3.7 ms
//! without a tick, ~6 ms with one). At 240 Hz that is most of a refresh interval, 50 times a second: the picture shows a moment
//! that is that much too old exactly on the tick frames, which reads as an uneven pan.
//! Fix: a frame with a tick draws its blend that much later (`lead`), so every frame shows the moment it reaches the screen at;
//! the amount is learnt from the redraw lengths (one average for frames with a tick, one for frames without). Only above ~90 fps
//! (below that the extra time is a small part of the frame and nothing changes). Nothing here reaches the game: only `alpha`.
//! RAC_SMOOTH_LEAD=0 switches it off (A/B checks); RAC_TEST_CLOCK runs ignore it (`test_clock::alpha`).

use std::cell::RefCell;

/// Weight of one redraw in the averages.
const K: f32 = 1.0 / 32.0;
/// A stall longer than this counts as this much (a 40 ms hitch must not move the averages for seconds).
const SAMPLE_MAX_MS: f32 = 12.0;
/// The lead never exceeds this.
const LEAD_MAX_MS: f32 = 6.0;
/// Only when the average frame interval is below this (above ~90 fps).
const INTERVAL_MAX_MS: f32 = 11.1;

#[derive(Default)]
struct Lead {
	/// Average redraw length of frames with / without a game tick, ms (None until the first of its kind).
	tick: Option<f32>,
	plain: Option<f32>,
	/// Average frame interval, ms.
	interval: Option<f32>,
	/// The lead of the last `lead` call, ms (for the frame CSV).
	last_ms: f32,
}

thread_local! {
	static LEAD: RefCell<Lead> = RefCell::new(Lead::default());
}

fn off() -> bool {
	static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*OFF.get_or_init(|| std::env::var("RAC_SMOOTH_LEAD").is_ok_and(|v| v == "0"))
}

fn ema(avg: &mut Option<f32>, v: f32) {
	*avg = Some(match *avg {
		Some(a) => a + (v - a) * K,
		None => v,
	});
}

impl Lead {
	fn note(&mut self, redraw_ms: f32, interval_ms: f32, ticks: u32) {
		let r = redraw_ms.clamp(0.0, SAMPLE_MAX_MS);
		ema(if ticks > 0 { &mut self.tick } else { &mut self.plain }, r);
		ema(&mut self.interval, interval_ms.clamp(0.0, 100.0));
	}

	fn lead_ms(&self, ticks: u32) -> f32 {
		match (ticks > 0, self.tick, self.plain, self.interval) {
			(true, Some(t), Some(p), Some(i)) if i < INTERVAL_MAX_MS => (t - p).clamp(0.0, LEAD_MAX_MS),
			_ => 0.0,
		}
	}
}

/// End of a redraw (before the frame cap): its length, the time since the previous redraw began, and the game ticks it ran.
pub fn note(redraw_ms: f32, interval_ms: f32, ticks: u32) {
	LEAD.with(|l| l.borrow_mut().note(redraw_ms, interval_ms, ticks));
}

/// Seconds to add to the interpolation time of a frame that ran `ticks` game ticks (0 without a tick, below ~90 fps, or off).
pub fn lead(ticks: u32) -> f32 {
	LEAD.with(|l| {
		let mut l = l.borrow_mut();
		l.last_ms = if off() { 0.0 } else { l.lead_ms(ticks) };
		l.last_ms / 1000.0
	})
}

/// The lead the last frame used, ms (frame CSV column).
pub(super) fn last_ms() -> f32 {
	LEAD.with(|l| l.borrow().last_ms)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn fed(fps: f32, tick_ms: f32, plain_ms: f32) -> Lead {
		let mut l = Lead::default();
		for i in 0..400 {
			let t = i % 5 == 0;
			l.note(if t { tick_ms } else { plain_ms }, 1000.0 / fps, t as u32);
		}
		l
	}

	#[test]
	fn tick_frames_lead_by_their_extra_redraw_time() {
		let l = fed(240.0, 5.0, 2.0);
		assert!((l.lead_ms(1) - 3.0).abs() < 0.05, "{}", l.lead_ms(1));
		assert_eq!(l.lead_ms(0), 0.0, "frames without a tick keep their sample time");
	}

	#[test]
	fn no_lead_below_90_fps_or_before_both_kinds_were_seen() {
		assert_eq!(fed(60.0, 5.0, 2.0).lead_ms(1), 0.0);
		let mut l = Lead::default();
		l.note(5.0, 4.0, 1);
		assert_eq!(l.lead_ms(1), 0.0, "no frame without a tick yet");
	}

	#[test]
	fn lead_is_bounded_and_never_negative() {
		assert_eq!(fed(240.0, 30.0, 1.0).lead_ms(1), LEAD_MAX_MS);
		assert_eq!(fed(240.0, 1.0, 3.0).lead_ms(1), 0.0);
	}
}
