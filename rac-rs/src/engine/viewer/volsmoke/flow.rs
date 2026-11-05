//! The billow noise's flow (review R18 B2). The shared noise coordinate of `smoke_at` (shader.rs) moves by ONE offset per frame:
//! the wind drift sideways and the frame's mean rise speed upward (the volumes' rows weighted by their drawn mass, eased),
//! integrated here from frame to frame. A per-sample blend of rise speeds times the absolute clock sheared the noise wherever
//! volumes of different rise overlap, worse every minute (after 10 minutes the billows in such an overlap were squashed ~90x into
//! grain, and a shifting weight moved the coordinate by metres: boiling). One offset per frame cannot shear, and a change of the mean
//! changes the speed, not the place. The offset is kept wrapped by whole noise tiles (`WRAP` for the big and medium lumps, the
//! detail tile for the wisps): it stays small and exact after hours of play, so the picture after an hour is the one after 5 s.

use super::volumes::{drawn, Raw};

/// Wrap of the big and medium lumps' offset (metres): a whole number of tiles of every level `smoke_at` uses. Its tile level `lv` is
/// clamped below `LV_MAX` (10): slot a's tile 2^l (l <= 9), slot b's 2^(l+1) <= 1024, the medium tiles a third and two thirds of
/// slot a's, so 1024 m is 2, 1, 6 and 3 (or more) whole tiles of each.
pub(super) const WRAP: f64 = 1024.0;
/// The bound of `lv` in the shader (the literal there; a test ties them).
pub(super) const LV_MAX: &str = "9.99";

/// The wind direction of the drift (the fog's direction, volfog.rs; the smoke's only copy: its `f.wind` stays 0), metres per
/// second per unit of `smoke_drift`.
const WIND: [f64; 2] = [0.8, 0.35];
/// Time constant (s) of the eased mean rise.
const EASE_S: f64 = 2.0;
/// The longest step integrated at once (a frame after a stretch without smoke volumes, a hitch).
const MAX_DT: f64 = 0.25;

/// The flow's state: last frame's clock, the eased rise speed, the offset wrapped two ways.
#[derive(Default)]
pub(super) struct Flow {
	last: Option<f64>,
	rise: f64,
	big: [f64; 3],
	det: [f64; 3],
}

/// The look numbers the flow uses (RAC_TUNE smoke_drift, smoke_rise, smoke_detail, smoke_t0).
#[derive(Clone, Copy)]
pub(super) struct Knobs {
	pub drift: f32,
	pub gain: f32,
	pub detail: f32,
	/// [test] Seconds of play the flow starts at (RAC_TUNE `smoke_t0`, default 0): the first frame's offset is the one a flow at
	/// this speed reaches after that long, so a test sees what an hour of play gives.
	pub t0: f32,
}

impl Knobs {
	pub(super) fn tuned() -> Knobs {
		let t = crate::devtune::f32;
		Knobs { drift: t("smoke_drift", 0.35), gain: t("smoke_rise", 1.0), detail: t("smoke_detail", 1.1).max(0.05), t0: t("smoke_t0", 0.0).max(0.0) }
	}
}

/// The volumes' rise speed weighted by drawn mass (thickness x share x area); None without any.
fn mean_rise(vols: &[Raw]) -> Option<f64> {
	let (mut s, mut w) = (0.0f64, 0.0f64);
	for r in vols {
		let k = (drawn(r) * r.radius * r.radius) as f64;
		(s, w) = (s + k * r.rise as f64, w + k);
	}
	(w > 0.0).then(|| s / w)
}

impl Flow {
	/// One frame at clock `now` (s, f64: `frame::clock64`; an f32 clock's step jitters by its ulp after hours, review R29 m1):
	/// eases the rise speed toward the volumes' mean, moves the offset by the frame's step, returns it wrapped by `WRAP` (big and
	/// medium lumps) and by the detail tile (wisps), as the shader's `g.flow` / `g.flowd`.
	pub(super) fn advance(&mut self, now: f64, vols: &[Raw], k: Knobs) -> ([f32; 4], [f32; 4]) {
		let target = mean_rise(vols);
		let dt = match self.last {
			None => {
				self.rise = target.unwrap_or(1.0);
				now.max(0.0) + k.t0 as f64
			}
			Some(l) => {
				let dt = (now - l).clamp(0.0, MAX_DT);
				if let Some(r) = target {
					self.rise += (r - self.rise) * (1.0 - (-dt / EASE_S).exp());
				}
				dt
			}
		};
		self.last = Some(now);
		let v = [WIND[0] * k.drift as f64, WIND[1] * k.drift as f64, self.rise * k.gain as f64];
		let d = k.detail.max(0.05) as f64;
		for a in 0..3 {
			self.big[a] = (self.big[a] + v[a] * dt).rem_euclid(WRAP);
			self.det[a] = (self.det[a] + v[a] * dt).rem_euclid(d);
		}
		let f = |o: [f64; 3]| [o[0] as f32, o[1] as f32, o[2] as f32, 0.0];
		(f(self.big), f(self.det))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn vol(rise: f32, thick: f32) -> Raw {
		Raw { pos: [0.0; 3], radius: 2.0, thick, albedo: [0.3; 3], share: 1.0, age: 0.0, rise, billow: 0.4, tall: 1.0 }
	}

	const K: Knobs = Knobs { drift: 0.35, gain: 1.0, detail: 1.1, t0: 0.0 };

	/// The step between two frames, per axis, undoing the wraps.
	fn step(a: [f32; 4], b: [f32; 4], wrap: f32) -> [f32; 3] {
		[0, 1, 2].map(|k| {
			let d = b[k] - a[k];
			d - (d / wrap).round() * wrap
		})
	}

	/// A volume of another rise speed joining changes the flow's speed smoothly, never its place: no frame moves the offset by more
	/// than the fastest speed x the frame time.
	#[test]
	fn a_change_of_rise_eases_the_speed_and_never_jumps_the_offset() {
		let mut f = Flow::default();
		let dt = 1.0 / 60.0;
		let slow = [vol(1.0, 1.0)];
		let mixed = [vol(1.0, 1.0), vol(1.6, 3.0)];
		let mut last = f.advance(5.0, &slow, K).0;
		let mut steps: Vec<f32> = Vec::new();
		for i in 1..600 {
			let now = 5.0 + i as f64 * dt;
			let cur = f.advance(now, if i < 60 { &slow[..] } else { &mixed[..] }, K).0;
			steps.push(step(last, cur, WRAP as f32)[2] / dt as f32);
			last = cur;
		}
		assert!((steps[10] - 1.0).abs() < 1e-3, "rises at the lone row's speed: {}", steps[10]);
		assert!(steps.windows(2).all(|w| (w[1] - w[0]).abs() < 0.02), "the speed eases, it does not jump");
		assert!(steps.iter().all(|&s| (0.99..=1.6).contains(&s)), "never faster than the fastest volume");
		assert!((steps[598] - 1.45).abs() < 0.02, "settles at the drawn-mass mean (1 x 1 + 1.6 x 3) / 4 = 1.45: {}", steps[598]);
	}

	/// The offset after an hour of play is as small and moves as exactly per frame as after 5 s, and its wraps are whole noise tiles
	/// of every octave (the texture repeats: the picture cannot tell a wrapped offset from the true one). The clock itself after 10 h
	/// (36005 s) gives the same step too (review R29 m1: an f32 clock is 3.9 ms coarse there, a 1/60 s step off by up to ~2.6 mm).
	#[test]
	fn an_hour_of_play_looks_like_five_seconds() {
		let run = |now: f64, t0: f32| {
			let mut f = Flow::default();
			let k = Knobs { t0, ..K };
			let a = f.advance(now, &[vol(1.3, 1.0)], k);
			let b = f.advance(now + 1.0 / 60.0, &[vol(1.3, 1.0)], k);
			(a, b)
		};
		let (short, long, clock10h) = (run(5.0, 0.0), run(5.0, 3600.0), run(36005.0, 0.0));
		let f32_step = ((36005.0f32 + 1.0 / 60.0) - 36005.0f32) as f64;
		assert!((f32_step - 1.0 / 60.0).abs() * 1.3 > 1e-4, "the test bites: an f32 clock's step at 10 h is {f32_step}");
		for (a, b) in [short, long, clock10h] {
			assert!(a.0[..3].iter().all(|&v| (0.0..WRAP as f32).contains(&v)) && a.1[..3].iter().all(|&v| (0.0..1.1).contains(&v)));
			let (sb, sd) = (step(a.0, b.0, WRAP as f32), step(a.1, b.1, 1.1));
			for k in 0..3 {
				assert!((sb[k] - (short.1 .0[k] - short.0 .0[k])).abs() < 1e-4, "same step per frame after an hour");
				assert!((sd[k] - sb[k]).abs() < 1e-4, "the detail offset moves with the big one");
			}
		}
		// The hour's true offset vs the wrapped one: a whole number of tiles at every level the shader uses (the smallest: a 0.25 m
		// mean radius at smoke_scale 3.2, level -1).
		let truth = [0.8 * 0.35 * 3605.0, 0.35 * 0.35 * 3605.0, 1.3 * 3605.0];
		for l in -1..=9 {
			let ta = (2.0f64).powi(l);
			for (k, &t) in truth.iter().enumerate() {
				let wrapped = long.0 .0[k] as f64;
				for tile in [ta, 2.0 * ta, ta / 3.0, ta / 1.5] {
					let n = (t - wrapped) / tile;
					assert!((n - n.round()).abs() < 2e-3, "level {l} tile {tile}: {n} tiles");
				}
				let n = (t - long.0 .1[k] as f64) / 1.1;
				assert!((n - n.round()).abs() < 2e-3, "detail: {n} tiles");
			}
		}
	}
}
