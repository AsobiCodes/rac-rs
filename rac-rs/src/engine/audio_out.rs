//! [PC] The last stage of the sound output, between the mixer and the device (`audio.rs`): a DC blocker, and a short fade-in after the
//! device ran dry.
//!
//! Why: a paused menu freezes the SPU voices (retail does the same), and the mix then sits at a constant level (about 0.0135 in the pause
//! menu). A real PS2's analog output is AC-coupled, so that level never reaches the speakers; here the device would get it. Whenever the
//! device then plays a gap (the audio engine calls the callback 15-40 ms late about every 0.6 s on some PCs) the level steps to zero and
//! back: a click, loud on headphones in a quiet menu. The one-pole high-pass below (5 Hz) turns the constant into zero, so a gap in a quiet
//! menu is silence next to silence. It does not change music or effects (-0.26 dB at 20 Hz, -0.02 dB at 100 Hz, 0 above).
//! A buffer that is the device's whole ring (the first one, or one after the ring ran dry: the device has played silence meanwhile) is faded
//! in over 3 ms instead of starting at full level.
//! RAC_AUDIO_RAW=1 skips this stage (A/B runs).

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// Device buffers the stage found to be the whole ring (stream start, or the device had played everything we gave it). Read by the
/// RAC_AUDIO_STATS probe.
pub static ST_OUT_DRAIN: AtomicU64 = AtomicU64::new(0);

/// High-pass corner of the DC blocker.
pub const DC_CUTOFF_HZ: f64 = 5.0;
/// Fade-in after a drained ring.
pub const FADE_IN_MS: f64 = 3.0;

/// y[n] = x[n] - x[n-1] + r * y[n-1], r = 1 - 2 pi fc / rate.
#[derive(Clone, Copy, Debug)]
pub struct DcBlocker {
	r: f32,
	x1: f32,
	y1: f32,
}

impl DcBlocker {
	pub fn new(rate: f64, cutoff_hz: f64) -> DcBlocker {
		let r = (1.0 - 2.0 * std::f64::consts::PI * cutoff_hz / rate).clamp(0.0, 1.0) as f32;
		DcBlocker { r, x1: 0.0, y1: 0.0 }
	}

	#[inline]
	pub fn process(&mut self, x: f32) -> f32 {
		// A NaN / inf sample would stay in x1 / y1 and silence the rest of the session: it reads as silence instead.
		let x = if x.is_finite() { x } else { 0.0 };
		let mut y = x - self.x1 + self.r * self.y1;
		// the tail of a decaying level would end in denormals (slow on some CPUs); an overflow restarts from silence
		if !y.is_finite() || y.abs() < 1e-20 {
			y = 0.0;
		}
		self.x1 = x;
		self.y1 = y;
		y
	}
}

/// The output stage of one device stream (interleaved f32 frames; the mixer fills the first two channels).
pub struct OutputStage {
	channels: usize,
	dc: [DcBlocker; 2],
	fade_len: u32,
	/// Frames of the fade-in already done; `>= fade_len` = none running.
	fade_pos: u32,
	/// The largest buffer (frames) the device has asked for: its whole ring, which the first callback gets.
	ring: usize,
	/// The smallest buffer seen: a device whose callbacks are always the whole ring (period == buffer) cannot show a drain.
	min_frames: usize,
	calls: u64,
}

impl OutputStage {
	pub fn new(rate: f64, channels: usize) -> OutputStage {
		let dc = DcBlocker::new(rate, DC_CUTOFF_HZ);
		OutputStage { channels: channels.max(1), dc: [dc, dc], fade_len: ((rate * FADE_IN_MS / 1000.0) as u32).max(1), fade_pos: u32::MAX, ring: 0, min_frames: usize::MAX, calls: 0 }
	}

	/// Runs one device buffer (after the mixer). Returns true when the buffer was the device's whole ring and so started with a fade-in.
	pub fn process(&mut self, data: &mut [f32]) -> bool {
		let frames = data.len() / self.channels;
		if frames > 0 {
			self.ring = self.ring.max(frames);
			self.min_frames = self.min_frames.min(frames);
		}
		// A callback that is handed the whole ring (larger than the steady callbacks) found it empty: the stream just started, or the
		// device played everything and then silence while the callback was late. Its first samples come out of silence, so they start
		// from silence.
		let first = self.calls == 0;
		self.calls += 1;
		let drained = frames > 0 && (first || (frames >= self.ring && self.min_frames < self.ring));
		if drained {
			self.fade_pos = 0;
			ST_OUT_DRAIN.fetch_add(1, Relaxed);
		}
		for frame in data.chunks_exact_mut(self.channels) {
			for (c, x) in frame.iter_mut().enumerate().take(2) {
				*x = self.dc[c].process(*x);
			}
			if self.fade_pos < self.fade_len {
				self.fade_pos += 1;
				let g = self.fade_pos as f32 / self.fade_len as f32;
				for x in frame.iter_mut() {
					*x *= g;
				}
			}
			for x in frame.iter_mut() {
				*x = x.clamp(-1.0, 1.0);
			}
		}
		drained
	}
}

/// RAC_AUDIO_RAW=1: no output stage.
pub fn raw() -> bool {
	std::env::var_os("RAC_AUDIO_RAW").is_some()
}

#[cfg(test)]
mod tests {
	use super::*;

	fn run(stage: &mut OutputStage, input: &[f32], buf: usize) -> Vec<f32> {
		let mut out = Vec::new();
		for chunk in input.chunks(buf * 2) {
			let mut v = chunk.to_vec();
			stage.process(&mut v);
			out.extend(v);
		}
		out
	}

	fn rms(x: &[f32]) -> f64 {
		(x.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / x.len() as f64).sqrt()
	}

	/// Stereo sine of `hz` and amplitude `a`, `secs` long, plus a constant `dc`.
	fn sine(rate: f64, hz: f64, a: f32, dc: f32, secs: f64) -> Vec<f32> {
		let n = (rate * secs) as usize;
		(0..n * 2).map(|i| dc + a * (2.0 * std::f64::consts::PI * hz * (i / 2) as f64 / rate).sin() as f32).collect()
	}

	#[test]
	fn the_blocker_turns_a_constant_into_zero() {
		let mut s = OutputStage::new(48000.0, 2);
		let out = run(&mut s, &vec![0.0135; 48000 * 2], 480);
		// after half a second the level is gone: below 1e-6 (-120 dB)
		assert!(out[48000..].iter().all(|v| v.abs() < 1e-6), "left {}", out[48000..].iter().fold(0.0f32, |m, v| m.max(v.abs())));
	}

	/// R6-6: one NaN or inf sample never sticks in the filter state; the sound goes on with the next good sample.
	#[test]
	fn a_nan_or_inf_sample_does_not_latch() {
		for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
			let mut b = DcBlocker::new(48000.0, DC_CUTOFF_HZ);
			let mut clean = DcBlocker::new(48000.0, DC_CUTOFF_HZ);
			for i in 0..100 {
				let x = 0.2 * (i as f32 * 0.1).sin();
				assert_eq!(b.process(x).to_bits(), clean.process(x).to_bits(), "finite input: unchanged");
			}
			assert!(b.process(bad).is_finite(), "{bad}: the bad sample itself is finite");
			let out: Vec<f32> = (0..48000).map(|i| b.process(0.2 * (i as f32 * 0.1).sin())).collect();
			assert!(out.iter().all(|v| v.is_finite()) && rms(&out[24000..]) > 0.05, "{bad}: the sound goes on");
		}
		let mut b = DcBlocker::new(48000.0, DC_CUTOFF_HZ);
		assert!([f32::MAX, -f32::MAX, f32::MAX].iter().all(|&x| b.process(x).is_finite()), "an overflow restarts from silence");
	}

	#[test]
	fn the_blocker_leaves_music_and_effects_alone() {
		for (hz, tol) in [(30.0, 0.02), (60.0, 0.006), (100.0, 0.002), (1000.0, 0.0005), (8000.0, 0.0005)] {
			let mut s = OutputStage::new(48000.0, 2);
			let input = sine(48000.0, hz, 0.3, 0.0, 2.0);
			let out = run(&mut s, &input, 480);
			let (a, b) = (rms(&input[48000..]), rms(&out[48000..]));
			assert!(((b - a) / a).abs() < tol, "{hz} Hz: rms {a} -> {b}");
		}
	}

	#[test]
	fn a_gap_against_a_dc_level_is_not_a_step() {
		let mut s = OutputStage::new(48000.0, 2);
		// 1.5 s of a frozen mix at 0.0135 (the paused menu), through the stage
		let mut out = run(&mut s, &vec![0.0135; 72000 * 2], 480);
		// the device plays a 30 ms gap of zeros, then the next buffer
		out.extend(vec![0.0; 1440 * 2]);
		let mut v = vec![0.0135; 480 * 2];
		s.process(&mut v);
		out.extend(v);
		let late: &[f32] = &out[48000 * 2..];
		let step = late.chunks_exact(2).zip(late.chunks_exact(2).skip(1)).map(|(a, b)| (b[0] - a[0]).abs().max((b[1] - a[1]).abs())).fold(0.0f32, f32::max);
		assert!(step < 1e-5, "largest step {step} (without the stage it is 0.0135)");
	}

	#[test]
	fn a_step_in_the_content_is_still_a_step_that_decays() {
		// a pause: content, then a frozen level; the blocker passes the edge once and then settles
		let mut s = OutputStage::new(48000.0, 2);
		let mut input = sine(48000.0, 200.0, 0.2, 0.0, 1.0);
		input.extend(vec![0.0135; 48000 * 2]);
		let out = run(&mut s, &input, 480);
		let tail = &out[48000 * 2 + 24000 * 2..];
		assert!(tail.iter().all(|v| v.abs() < 0.002), "settles within 0.5 s");
	}

	#[test]
	fn the_first_buffer_and_a_drained_ring_fade_in() {
		let mut s = OutputStage::new(48000.0, 2);
		let tone = |n: usize| -> Vec<f32> { (0..n * 2).map(|i| 0.4 * (2.0 * std::f32::consts::PI * 440.0 * (i / 2) as f32 / 48000.0).sin() + 0.4).collect() };
		// the first callback gets the whole ring (1056 frames): faded in from silence
		let mut v = tone(1056);
		assert!(s.process(&mut v));
		assert!(v[0].abs() < 0.01 && v[1].abs() < 0.01, "first frame {} {}", v[0], v[1]);
		// the fade is 3 ms = 144 frames: after it the gain is 1
		let mut plain = OutputStage::new(48000.0, 2);
		plain.calls = 1;
		plain.ring = 4096;
		let mut w = tone(1056);
		assert!(!plain.process(&mut w));
		let ramp_end = 144 * 2;
		assert!(w[200].abs() > 0.01 && v[200].abs() < w[200].abs(), "frame 100 is still on the ramp");
		assert!(v[ramp_end..].iter().zip(&w[ramp_end..]).all(|(a, b)| (a - b).abs() < 1e-6), "gain is exactly 1 after 3 ms");
		// a device whose callbacks are always the whole buffer cannot show a drain: no fade after the first
		let mut same = OutputStage::new(48000.0, 2);
		assert!(same.process(&mut tone(480)));
		assert!(!same.process(&mut tone(480)) && !same.process(&mut tone(480)));
		// normal 10 ms callbacks do not fade
		let mut v = tone(480);
		assert!(!s.process(&mut v));
		// a late callback is handed the whole ring again: it fades in again, and the counter moves
		let before = ST_OUT_DRAIN.load(Relaxed);
		let mut v = tone(1056);
		assert!(s.process(&mut v));
		assert!(ST_OUT_DRAIN.load(Relaxed) > before);
		assert!(v[0].abs() < 0.02, "starts from silence: {}", v[0]);
		// a partial catch-up buffer (960 of 1056) is not a drain
		let mut v = tone(960);
		assert!(!s.process(&mut v));
	}

	#[test]
	fn a_mono_device_gets_the_blocker_too() {
		let mut s = OutputStage::new(44100.0, 1);
		let mut v = vec![0.02f32; 441 * 100];
		for chunk in v.chunks_mut(441) {
			s.process(chunk);
		}
		assert!(v[441 * 90..].iter().all(|x| x.abs() < 1e-3));
	}
}
