//! [PC] The movie player's queue and clock rules as pure functions (unit tests below): what the worker may read next, which decoded frames
//! the clock has already passed, and how the output device's latency moves the clock. `player.rs` calls these; they hold no state.
//!
//! The audio is the master clock and must never go quiet while the stream still has audio (GitHub #7). The stream interleaves audio and video
//! packets, so the audio can only be reached by reading the video packets in front of it: the video side therefore never gets to stop the
//! reading (a full decoded queue spills raw packets, frames the clock has passed are dropped), and only the audio's own lead stops it.

use std::collections::VecDeque;
use std::time::Duration;

/// Decoded frames held ahead of the clock (0.5 s at 25 fps) while the audio has enough queued.
pub const MAX_FRAMES: usize = 12;
/// Decoded frames at which further video packets are spilled undecoded (`spill`) instead of decoded.
pub const HARD_FRAMES: usize = 90;
/// Seconds of audio the worker keeps queued.
pub const MAX_AUDIO: f64 = 2.0;
/// Seconds of audio queued before the clock starts, and below which the audio counts as starved (the worker reads on whatever the video holds).
pub const PRIME: f64 = 0.25;
/// Bytes of undecoded video the spill holds (about 30 s of this disc's movies); past it the worker waits, the only wait the video can impose.
pub const SPILL_MAX: usize = 24 << 20;
/// Longest output latency the clock allows for; a bigger report is not believed (a Bluetooth headset is 200 to 300 ms).
pub const MAX_LATENCY_MS: f64 = 400.0;
/// Weight of a new latency report in the smoothed value: the device reports a slightly different figure at every callback.
const LATENCY_SMOOTH: f64 = 0.05;

/// What the worker looks at to decide its next step.
pub struct Fill {
	/// Decoded frames waiting.
	pub frames: usize,
	/// Undecoded video bytes spilled.
	pub spilled: usize,
	/// Seconds of audio queued.
	pub lead: f64,
	/// The audio device is the clock (else the wall clock paces the movie, video only).
	pub audio_on: bool,
}

/// True when the worker should sleep a moment instead of reading the next packet. The audio's own lead is the one thing that stops it for
/// good: enough audio queued. A full decoded video queue stops it only while the audio is not starved.
pub fn must_wait(f: &Fill) -> bool {
	let starved = f.audio_on && f.lead < PRIME;
	if f.audio_on && f.lead > MAX_AUDIO {
		return true;
	}
	if f.spilled >= SPILL_MAX {
		return true;
	}
	f.frames >= MAX_FRAMES && !starved
}

/// True when the next video packet goes to the spill: the decoded queue is full, or older packets are spilled already (the decoder takes
/// the packets in stream order).
pub fn spill(frames: usize, spilled_packets: usize) -> bool {
	frames >= HARD_FRAMES || spilled_packets > 0
}

/// Drops the frames in front of the clock except the newest of them: that one is the picture due now, and the owner of the queue would skip
/// the others anyway (`Player::poll` shows the last due frame). Returns how many were dropped.
pub fn drop_late<T>(q: &mut VecDeque<T>, pts: impl Fn(&T) -> u64, clock: u64) -> usize {
	let mut n = 0;
	while q.len() >= 2 && pts(&q[1]) <= clock {
		q.pop_front();
		n += 1;
	}
	n
}

/// The smoothed output latency in ms after one more report; reports are clamped to `0..=MAX_LATENCY_MS`. Before the first report the clock ran
/// uncompensated (0), so the first report moves the value by one smoothing step like every other: a bogus first timestamp (seen on some ALSA /
/// PipeWire setups) shifts the clock by 5 % of it instead of all of it and is outvoted by the reports after it; a real device converges on its
/// latency within about a second (5 % per poll).
pub fn smooth_latency(prev: Option<f64>, reported: Duration) -> f64 {
	let r = (reported.as_secs_f64() * 1000.0).clamp(0.0, MAX_LATENCY_MS);
	let p = prev.unwrap_or(0.0);
	p + (r - p) * LATENCY_SMOOTH
}

/// The 90 kHz clock of what is HEARD: the stream's first audio PTS plus the audio frames the device has consumed, less the latency of the
/// output path (the callback pulls a sample that long before the speaker plays it), at least 0.
pub fn heard_ticks(pts0: u64, played: u64, rate: u32, latency_ms: f64) -> u64 {
	(pts0 + played * 90000 / rate.max(1) as u64).saturating_sub((latency_ms * 90.0) as u64)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn fill(frames: usize, spilled: usize, lead: f64, audio_on: bool) -> Fill {
		Fill { frames, spilled, lead, audio_on }
	}

	/// The video queue alone never stops the reading while the audio is starved, however full it is; the audio's own lead and the spill's
	/// limit do. Without the audio device the soft video cap paces the worker as before.
	#[test]
	fn only_the_audio_and_the_spill_limit_stop_the_reading() {
		assert!(!must_wait(&fill(0, 0, 0.0, true)), "empty: read");
		assert!(must_wait(&fill(MAX_FRAMES, 0, 1.0, true)), "queue at its soft cap, audio fine: wait");
		assert!(!must_wait(&fill(MAX_FRAMES, 0, PRIME - 0.01, true)), "queue at its soft cap, audio starved: read on");
		assert!(!must_wait(&fill(HARD_FRAMES, 0, 0.0, true)), "queue FULL and audio starved: still read (the old code slept here)");
		assert!(!must_wait(&fill(HARD_FRAMES + 40, 5000, 0.1, true)), "even past the hard cap");
		assert!(must_wait(&fill(3, 0, MAX_AUDIO + 0.1, true)), "enough audio queued: wait");
		assert!(must_wait(&fill(HARD_FRAMES, SPILL_MAX, 0.0, true)), "the spill is full: the one wait the video can impose");
		assert!(must_wait(&fill(MAX_FRAMES, 0, 0.0, false)) && !must_wait(&fill(MAX_FRAMES - 1, 0, 0.0, false)), "no audio device: the soft cap");
	}

	#[test]
	fn video_spills_when_the_queue_is_full_and_stays_in_order() {
		assert!(!spill(HARD_FRAMES - 1, 0));
		assert!(spill(HARD_FRAMES, 0));
		assert!(spill(2, 7), "older packets are spilled: this one follows them");
	}

	/// The frames the clock has passed are dropped but the newest of them, which is the picture due now.
	#[test]
	fn late_frames_are_dropped_but_the_one_due_now() {
		let mut q: VecDeque<u64> = (0..20u64).map(|i| i * 3600).collect();
		assert_eq!(drop_late(&mut q, |&p| p, 10 * 3600 + 5), 10);
		assert_eq!(q.front(), Some(&(10 * 3600)), "the frame due at the clock stays");
		assert_eq!(q.len(), 10);
		assert_eq!(drop_late(&mut q, |&p| p, 10 * 3600 + 5), 0, "nothing more to drop");
		assert_eq!(drop_late(&mut q, |&p| p, 0), 0, "a clock before the queue drops nothing");
		let mut one: VecDeque<u64> = VecDeque::from(vec![5u64]);
		assert_eq!(drop_late(&mut one, |&p| p, 1_000_000), 0, "the last frame is never dropped");
		// What the owner shows is the same with or without the drop.
		let shown = |q: &VecDeque<u64>, c: u64| q.iter().copied().filter(|&p| p <= c).last();
		let full: VecDeque<u64> = (0..20u64).map(|i| i * 3600).collect();
		let mut cut = full.clone();
		drop_late(&mut cut, |&p| p, 12 * 3600 + 100);
		for c in [12 * 3600 + 100, 12 * 3600 + 3000, 15 * 3600] {
			assert_eq!(shown(&full, c), shown(&cut, c), "clock {c}");
		}
	}

	/// The latency: clamped to 0..400 ms, every report (the first from 0) smoothed; the clock of what is heard runs that far behind
	/// the clock of what the device pulled, and never below 0.
	#[test]
	fn the_latency_is_clamped_smoothed_and_taken_off_the_clock() {
		let ms = |v: f64| Duration::from_secs_f64(v / 1000.0);
		assert!((smooth_latency(None, ms(31.5)) - 31.5 * LATENCY_SMOOTH).abs() < 1e-9, "the first report is one step from 0");
		assert!((smooth_latency(None, ms(5000.0)) - MAX_LATENCY_MS * LATENCY_SMOOTH).abs() < 1e-9, "a bogus first report is clamped, then smoothed: at most 20 ms");
		assert_eq!(smooth_latency(None, Duration::ZERO), 0.0);
		// A bogus first report (400 ms) followed by the device's real 30 ms: the clock is within 1 ms of right after 90 polls (1.5 s at 60 Hz).
		let mut v = Some(smooth_latency(None, ms(400.0)));
		for _ in 0..90 {
			v = Some(smooth_latency(v, ms(30.0)));
		}
		assert!((v.unwrap() - 30.0).abs() < 1.0, "the bogus first report is outvoted: {v:?}");
		// A real first report of 31.5 ms is within 5 ms after 40 polls.
		let mut v = None;
		for _ in 0..40 {
			v = Some(smooth_latency(v, ms(31.5)));
		}
		assert!((v.unwrap() - 31.5).abs() < 5.0, "{v:?}");
		let mut v = Some(31.5);
		for _ in 0..400 {
			v = Some(smooth_latency(v, ms(120.0)));
		}
		assert!((v.unwrap() - 120.0).abs() < 0.01, "converges on a steady report: {v:?}");
		let one = smooth_latency(Some(30.0), ms(130.0));
		assert!(one > 30.0 && one < 40.0, "one outlier moves it only a little: {one}");
		// 48000 played frames = 1 s = 90000 ticks; 100 ms latency = 9000 ticks.
		assert_eq!(heard_ticks(11414, 48000, 48000, 0.0), 11414 + 90000);
		assert_eq!(heard_ticks(11414, 48000, 48000, 100.0), 11414 + 90000 - 9000);
		assert_eq!(heard_ticks(0, 48, 48000, 400.0), 0, "never below 0");
		assert_eq!(heard_ticks(11414, 0, 48000, 31.5), 11414 - 2835, "before the first sample: ahead of the first picture");
	}
}
