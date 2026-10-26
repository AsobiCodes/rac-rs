//! [PC] The effect clock's frame stamp. Every effect that animates with time (water waves and ripples, vegetation wind, fog and
//! cloud drift, the underwater look, the post shimmer, the star) reads the wall clock when its pass is encoded, late in the frame.
//! That moment lies after the game ticks of the frame: a frame that ran a retail field (5 of every 7 frames at 70 fps) read its
//! time 3-5 ms later than a frame that ran none, so the effect time advanced by 11.7 ms in one frame and 15.7 ms in the next
//! while the frames were shown at an even 14.3 ms: waves and ripples moved in uneven steps that beat with the field rate, a
//! jiggle that grows at lower frame rates (user clip 2026-10-26, Pokitaru sea by the bridge). The display interpolation (`smooth.rs`,
//! `interp.rs`) already works from the frame's start (`handler/frame_start.rs`); `test_clock::secs` now takes the effect time as
//! it was at that frame start too: the wall reading minus the time since the frame began (`lag`).

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;

static EPOCH: OnceLock<Instant> = OnceLock::new();
/// Nanoseconds from `EPOCH` to the start of the frame being drawn, plus 1 (0 = no frame has started).
static STARTED: AtomicU64 = AtomicU64::new(0);

/// Top of a frame (`frame_begin`): remember the moment.
pub(in crate::viewer) fn frame_started() {
	let e = *EPOCH.get_or_init(Instant::now);
	STARTED.store(e.elapsed().as_nanos() as u64 + 1, Relaxed);
}

/// Seconds since this frame began; 0 before the first frame or when the frame is a long one (over `MAX_LAG`: a stall or a pass
/// drawn outside any frame keeps the plain wall clock).
pub(in crate::viewer) fn lag() -> f32 {
	let (Some(e), s) = (EPOCH.get(), STARTED.load(Relaxed)) else { return 0.0 };
	if s == 0 {
		return 0.0;
	}
	let l = (e.elapsed().as_nanos() as u64).saturating_sub(s - 1) as f32 * 1e-9;
	if l > MAX_LAG { 0.0 } else { l }
}

const MAX_LAG: f32 = 0.1;

#[cfg(test)]
mod tests {
	use super::*;

	/// No frame yet: no lag; after one starts, the lag grows from 0.
	#[test]
	fn lag_counts_from_the_frame_start() {
		frame_started();
		let a = lag();
		std::thread::sleep(std::time::Duration::from_millis(5));
		let b = lag();
		assert!((0.0..0.05).contains(&a) && b >= a + 0.004 && b < MAX_LAG, "{a} {b}");
	}
}
