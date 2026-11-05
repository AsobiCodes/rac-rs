//! The plumes followed from frame to frame (user 2026-11-04, test build 12: the smoke "flickers and changes positions as u move").
//! A plume is made fresh every frame from the quads the game drew (plume.rs `Plume::of`), and those change with the camera: the
//! game draws more or fewer sheets of one fire as the eye moves (Novalis fg1/1_0016 with the camera turning: 14 or 42 quads per
//! fire, the foot moving 1.1 m and the widths 1.3 m from one frame to the next), and camera-facing quads move their corners with
//! the eye. Worse, the seed (the puffs' phases and jitter) came from the foot rounded to 0.5 m, so a foot a millimetre over a
//! rounding edge re-seeded the whole plume, often for one frame and back (traced: 22 frames with a jump in 200 fields).
//!
//! Here each fresh plume takes over last frame's nearest one: it keeps that one's seed for good, and its shape (foot, height,
//! widths, thickness, colour, billow) eases to the fresh one with the time constant `TAU`. A plume that appears while the smoke
//! is running fades in over `FADE` seconds; one the game stopped drawing fades out from its last shape over the same time (its
//! volumes stand for no quad: there is none to give back). After a gap in the frames (`GAP`: the smoke off, a movie, a level
//! load) the list starts over and the plumes show at once, as there is no picture before them to pop from.
use super::plume::Plume;

/// Seconds over which a plume's shape covers 63 % of the way to a new one (1 - 1/e).
const TAU: f32 = 1.0;
/// Seconds a new plume takes to fade in and a vanished one to fade out.
const FADE: f32 = 0.6;
/// A longer gap between two frames (seconds of the smoke clock) starts the list over.
const GAP: f64 = 0.5;

/// No quad stands for this volume (a plume fading out: the game drew none of its quads this frame).
pub(super) const NO_QUAD: u32 = u32::MAX;

#[derive(Clone, Debug)]
struct Tracked {
	p: Plume,
	seed: u32,
	/// 0..1: the thickness share while it fades in or out.
	presence: f32,
}

/// What `step` remembers between frames.
#[derive(Default)]
pub(super) struct PlumeTrack {
	list: Vec<Tracked>,
	last: Option<f64>,
}

/// A plume to draw: its shape, its seed and its presence (0..1, times its thickness).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Shown {
	pub p: Plume,
	pub seed: u32,
	pub presence: f32,
}

fn hdist(a: [f32; 3], b: [f32; 3]) -> f32 {
	((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Whether fresh plume `f` may be tracked plume `t` moved on: their feet within the larger of 4 m and half the wider top sideways,
/// and within the larger of 4 m and half the taller height up or down.
fn near(t: &Plume, f: &Plume) -> bool {
	hdist(t.foot, f.foot) <= (0.5 * t.w1.max(f.w1)).max(4.0) && (t.foot[2] - f.foot[2]).abs() <= (0.5 * t.height.max(f.height)).max(4.0)
}

fn dist2(t: &Plume, f: &Plume) -> f32 {
	(0..3).map(|a| (t.foot[a] - f.foot[a]).powi(2)).sum()
}

/// `t`'s shape moved the share `a` of the way to `f`'s; the quads are `f`'s.
fn ease(t: &Plume, f: &Plume, a: f32) -> Plume {
	let mix = |x: f32, y: f32| x + (y - x) * a;
	Plume {
		foot: std::array::from_fn(|i| mix(t.foot[i], f.foot[i])),
		height: mix(t.height, f.height),
		w0: mix(t.w0, f.w0),
		w1: mix(t.w1, f.w1),
		thick: mix(t.thick, f.thick),
		albedo: std::array::from_fn(|i| mix(t.albedo[i], f.albedo[i])),
		billow: mix(t.billow, f.billow),
		src: f.src.clone(),
	}
}

impl PlumeTrack {
	/// This frame's plumes at smoke clock `now` (s) from the `fresh` ones `Plume::of` made: the followed ones in the order of
	/// `fresh`, then those fading out.
	pub(super) fn step(&mut self, fresh: Vec<Plume>, now: f64) -> Vec<Shown> {
		let dt = match self.last {
			Some(l) if now >= l && now - l <= GAP => (now - l) as f32,
			_ => {
				self.list.clear();
				-1.0
			}
		};
		self.last = Some(now);
		// (dt < 0: the list started over, everything shows at once.)
		let (a, fade) = if dt < 0.0 { (1.0, 1.0) } else { (1.0 - (-dt / TAU).exp(), dt / FADE) };
		let mut claimed = vec![false; self.list.len()];
		let mut next = Vec::with_capacity(fresh.len());
		for f in fresh {
			let best = (0..self.list.len()).filter(|&k| !claimed[k] && near(&self.list[k].p, &f)).min_by(|&x, &y| dist2(&self.list[x].p, &f).total_cmp(&dist2(&self.list[y].p, &f)));
			next.push(match best {
				Some(k) => {
					claimed[k] = true;
					let t = &self.list[k];
					Tracked { p: ease(&t.p, &f, a), seed: t.seed, presence: (t.presence + fade).min(1.0) }
				}
				None => Tracked { seed: f.seed(), p: f, presence: fade.min(1.0) },
			});
		}
		for (k, t) in self.list.drain(..).enumerate() {
			let presence = t.presence - fade;
			if !claimed[k] && presence > 0.0 {
				next.push(Tracked { p: Plume { src: Vec::new(), ..t.p }, presence, ..t });
			}
		}
		self.list = next;
		self.list.iter().filter(|t| t.presence > 0.0).map(|t| Shown { p: t.p.clone(), seed: t.seed, presence: t.presence }).collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn plume(foot: [f32; 3], height: f32, w1: f32, src: u32) -> Plume {
		Plume { foot, height, w0: 0.5 * w1, w1, thick: 0.9, albedo: [0.4; 3], billow: 0.45, src: vec![src] }
	}

	const DT: f64 = 1.0 / 60.0;

	/// The traced flicker: a foot that moves a millimetre over a 0.5 m rounding edge re-seeded the plume. Followed, it keeps its
	/// seed, also when the game swaps the sheets (foot 1.1 m over, 7 % taller, wider) and back.
	#[test]
	fn a_followed_plume_keeps_its_seed_over_a_rounding_edge_and_a_sheet_swap() {
		let mut t = PlumeTrack::default();
		let a = plume([51.8, 221.7499, 59.4], 40.7, 19.3, 128);
		let b = plume([51.8, 221.7501, 59.4], 40.7, 19.3, 451);
		assert_ne!(a.seed(), b.seed(), "the edge is real: a fresh seed would jump");
		let s0 = t.step(vec![a.clone()], 10.0)[0].seed;
		assert_eq!(t.step(vec![b.clone()], 10.0 + DT)[0].seed, s0);
		let swap = plume([52.7, 222.9, 59.4], 43.5, 18.0, 451);
		assert_eq!(t.step(vec![swap], 10.0 + 2.0 * DT)[0].seed, s0);
		let back = t.step(vec![a], 10.0 + 3.0 * DT);
		assert_eq!((back.len(), back[0].seed, &back[0].p.src), (1, s0, &vec![128]), "one plume, the same seed, this frame's quads");
	}

	/// A change of the drawn sheets moves the shape a little per frame and all the way within a few seconds.
	#[test]
	fn the_shape_eases_to_a_new_one() {
		let mut t = PlumeTrack::default();
		t.step(vec![plume([0.0, 0.0, 0.0], 33.8, 8.5, 0)], 0.0);
		let swap = plume([0.35, 0.0, 0.0], 36.1, 8.8, 0);
		let one = t.step(vec![swap.clone()], DT)[0].p.clone();
		assert!(one.height > 33.8 && one.height < 33.8 + 0.05 * 2.3 && one.foot[0] < 0.35 * 0.05, "{one:?}");
		let mut last = one;
		for f in 2..=240 {
			let p = t.step(vec![swap.clone()], f as f64 * DT)[0].p.clone();
			assert!(p.height >= last.height && (p.height - last.height) < 0.05, "smooth: {} -> {}", last.height, p.height);
			last = p;
		}
		assert!((last.height - 36.1).abs() < 0.05 && (last.foot[0] - 0.35).abs() < 0.01, "there after 4 s: {last:?}");
	}

	/// A plume that comes into the frames fades in; one the game stops drawing fades out from its last shape with no quad, then goes.
	#[test]
	fn new_plumes_fade_in_and_vanished_ones_fade_out() {
		let mut t = PlumeTrack::default();
		let (a, b) = (plume([0.0, 0.0, 0.0], 30.0, 8.0, 1), plume([60.0, 0.0, 0.0], 30.0, 8.0, 2));
		assert_eq!(t.step(vec![a.clone()], 0.0)[0].presence, 1.0, "the first frame shows at once");
		let v = t.step(vec![a.clone(), b.clone()], DT);
		assert_eq!(v.len(), 2);
		assert!((v[1].presence - DT as f32 / FADE).abs() < 1e-5 && v[0].presence == 1.0);
		let mut now = DT;
		for _ in 0..40 {
			now += DT;
			t.step(vec![a.clone(), b.clone()], now);
		}
		now += DT;
		let v = t.step(vec![a.clone()], now);
		assert_eq!(v.len(), 2, "b fades out");
		assert!(v[1].p.src.is_empty() && v[1].p.foot == b.foot && v[1].presence < 1.0 && v[1].seed == b.seed());
		let mut frames = 1;
		while t.step(vec![a.clone()], now + frames as f64 * DT).len() == 2 {
			frames += 1;
		}
		assert!((frames as f64 * DT - FADE as f64).abs() < 2.0 * DT, "gone after FADE: {frames} frames");
	}

	/// Two plumes side by side keep their own seeds; a gap in the frames starts the list over.
	#[test]
	fn neighbours_keep_their_seeds_and_a_gap_starts_over() {
		let mut t = PlumeTrack::default();
		let (a, b) = (plume([0.0, 0.0, 0.0], 30.0, 8.0, 1), plume([12.0, 0.0, 0.0], 30.0, 8.0, 2));
		let v0 = t.step(vec![a.clone(), b.clone()], 0.0);
		let v1 = t.step(vec![b.clone(), a.clone()], DT);
		assert_eq!((v1[0].seed, v1[1].seed), (v0[1].seed, v0[0].seed), "matched by place, not by order");
		let moved = plume([0.0, 0.0, 0.0], 30.0, 8.0, 1);
		let v2 = t.step(vec![Plume { foot: [0.1, 0.0, 0.0], ..moved }], DT + GAP + 0.1);
		assert_eq!((v2.len(), v2[0].presence, v2[0].foot_seed()), (1, 1.0, true), "started over: at once, its own seed");
	}

	impl Shown {
		fn foot_seed(&self) -> bool {
			self.seed == self.p.seed()
		}
	}
}
