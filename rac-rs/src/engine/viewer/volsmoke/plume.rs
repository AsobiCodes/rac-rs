//! A level fire's smoke as one rising plume (user 2026-11-04: one volume per retail quad "just stays in one place and kinda looks
//! blobby": the quads stand still, so did their volumes, and a crowd of them merged into round balls). The taken quads of one
//! emitter (its columns and volume sheets stand round one place: `group`) give the plume's foot, height, widths, thickness and colour
//! (`Plume::of`); the plume is a stream of puffs born at its foot that rise, widen, lean with the wind and thin out at the top
//! (`Plume::puffs`). A puff's place is a function of the clock (its phase in a fixed cycle, `Knobs::period`) and of the plume's seed
//! and shape, and each puff is born and dies at thickness 0 (no pop). The quads change with the camera, so the plumes are followed
//! from frame to frame (plume_track.rs: the seed kept, the shape eased). The billow noise rises at the
//! puffs' speed (their `rise`, flow.rs' mean): the lumps ride up with the puffs instead of sliding over a still shape.

use super::volumes::Raw;

/// One taken smoke quad: its index among the frame's effect quads, its corners, its thickness (the row's density x its fade) and
/// albedo, its billow.
#[derive(Clone, Copy, Debug)]
pub(super) struct Taken {
	pub index: u32,
	pub pos: [[f32; 3]; 4],
	pub thick: f32,
	pub albedo: [f32; 3],
	pub billow: f32,
}

/// The look numbers (RAC_TUNE `smoke_plume_*`, ENV.md).
#[derive(Clone, Copy, Debug)]
pub(super) struct Knobs {
	/// Puffs per plume (the quality's: Low 4, Medium 6, High 8).
	pub n: usize,
	/// Seconds a puff takes from the foot to the top.
	pub period: f32,
	/// The top's sideways lean with the wind, times the plume's height.
	pub lean: f32,
	/// Gain on the puffs' width and on their thickness.
	pub width: f32,
	pub thick: f32,
	/// Gain on the quads' billow (the shader's edge erosion grows with it: a plume's edge breaks up more than a lone puff's).
	pub billow: f32,
}

impl Knobs {
	/// The quality's puff count for Smoke level `level` (1 Low, 2 Medium, 3 High).
	pub(super) fn tuned(level: u8) -> Knobs {
		let t = crate::devtune::f32;
		let n = match t("smoke_plume_n", 0.0) as usize {
			0 => [0, 4, 6, 8][level.min(3) as usize],
			m => m.min(16),
		};
		Knobs { n, period: t("smoke_plume_t", 7.0).max(0.5), lean: t("smoke_plume_lean", 0.3), width: t("smoke_plume_w", 1.0), thick: t("smoke_plume_k", 1.6), billow: t("smoke_plume_billow", 4.0) }
	}
}

/// The wind's direction (flow.rs `WIND`, normalised): the plumes lean the way the billow noise drifts.
const WIND: [f32; 2] = [0.916, 0.401];

/// One emitter's smoke.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Plume {
	/// The foot: the quads' horizontal centre and the lowest corner's height.
	pub foot: [f32; 3],
	/// The quads' height (metres; camera-facing quads lean toward the eye, so their height is the vertical extent over the cosine
	/// of the eye's elevation).
	pub height: f32,
	/// Horizontal half-width at the foot (the corners in the lowest quarter) and the widest.
	pub w0: f32,
	pub w1: f32,
	pub thick: f32,
	pub albedo: [f32; 3],
	pub billow: f32,
	/// The quads it stands for.
	pub src: Vec<u32>,
}

fn hdist(a: [f32; 3], b: [f32; 3]) -> f32 {
	((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

fn centre(q: &Taken) -> [f32; 3] {
	std::array::from_fn(|a| q.pos.iter().map(|p| p[a]).sum::<f32>() * 0.25)
}

/// A quad's horizontal half-width: its farthest corner from its centre, sideways.
fn half_width(q: &Taken) -> f32 {
	let c = centre(q);
	q.pos.iter().map(|p| hdist(*p, c)).fold(0.0, f32::max)
}

/// The quads in groups, one per emitter: two quads join when their centres are closer sideways than the larger of 3 m and their
/// half-widths' sum (an emitter's column slots stand inside its cuboid, its sheets round its centre). Groups in order of their first quad.
pub(super) fn group(qs: &[Taken]) -> Vec<Vec<usize>> {
	let mut root: Vec<usize> = (0..qs.len()).collect();
	fn find(root: &mut [usize], mut i: usize) -> usize {
		while root[i] != i {
			root[i] = root[root[i]];
			i = root[i];
		}
		i
	}
	let info: Vec<([f32; 3], f32)> = qs.iter().map(|q| (centre(q), half_width(q))).collect();
	for i in 0..qs.len() {
		for j in i + 1..qs.len() {
			if hdist(info[i].0, info[j].0) <= (info[i].1 + info[j].1).max(3.0) {
				let (a, b) = (find(&mut root, i), find(&mut root, j));
				root[a.max(b)] = a.min(b);
			}
		}
	}
	let mut out: Vec<Vec<usize>> = Vec::new();
	let mut slot = vec![usize::MAX; qs.len()];
	for i in 0..qs.len() {
		let r = find(&mut root, i);
		if slot[r] == usize::MAX {
			slot[r] = out.len();
			out.push(Vec::new());
		}
		out[slot[r]].push(i);
	}
	out
}

impl Plume {
	/// The plume of the quads `ix` of `qs` seen from `eye`; None when they are all faded out.
	pub(super) fn of(qs: &[Taken], ix: &[usize], eye: [f32; 3]) -> Option<Plume> {
		let live: Vec<&Taken> = ix.iter().map(|&i| &qs[i]).filter(|q| q.thick > 0.0).collect();
		if live.is_empty() {
			return None;
		}
		let n = live.len() as f32;
		let cs: Vec<[f32; 3]> = live.iter().map(|q| centre(q)).collect();
		let (cx, cy) = (cs.iter().map(|c| c[0]).sum::<f32>() / n, cs.iter().map(|c| c[1]).sum::<f32>() / n);
		let corners = || live.iter().flat_map(|q| q.pos.iter().copied());
		let (lo, hi) = corners().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p[2]), hi.max(p[2])));
		let mid = [cx, cy, 0.5 * (lo + hi)];
		let to_eye = [eye[0] - mid[0], eye[1] - mid[1], eye[2] - mid[2]];
		let len = (to_eye[0] * to_eye[0] + to_eye[1] * to_eye[1] + to_eye[2] * to_eye[2]).sqrt().max(1e-3);
		let cos_el = ((to_eye[0] * to_eye[0] + to_eye[1] * to_eye[1]).sqrt() / len).max(0.4);
		let height = (hi - lo) / cos_el;
		let w1 = corners().map(|p| hdist(p, mid)).fold(0.0, f32::max);
		let low = lo + 0.25 * (hi - lo);
		let w0 = corners().filter(|p| p[2] <= low).map(|p| hdist(p, mid)).fold(0.0, f32::max).max(0.35 * w1);
		let thick = live.iter().map(|q| q.thick).sum::<f32>() / n;
		let albedo = std::array::from_fn(|a| live.iter().map(|q| q.albedo[a]).sum::<f32>() / n);
		let billow = live.iter().map(|q| q.billow).sum::<f32>() / n;
		(height > 0.05 && w1 > 0.05).then(|| Plume { foot: [cx, cy, lo], height, w0, w1, thick, albedo, billow, src: ix.iter().map(|&i| qs[i].index).collect() })
	}

	/// A new plume's seed: its foot on a 0.5 m grid, hashed (plumes side by side do not breathe in step). A followed plume keeps the
	/// seed it was born with (plume_track.rs): a foot a millimetre over a rounding edge would re-seed it.
	pub(super) fn seed(&self) -> u32 {
		let g = |v: f32| (v * 2.0).round() as i32 as u32;
		hash(g(self.foot[0]).wrapping_mul(73856093) ^ g(self.foot[1]).wrapping_mul(19349663) ^ g(self.foot[2]).wrapping_mul(83492791))
	}

	/// The plume's puffs at clock `now` (s) with seed `seed` (`seed`, kept by plume_track.rs): `k.n` puffs evenly spread over the cycle. Puff i is at phase f = frac(now / period +
	/// i / n + seed) of its rise: from 0.2 to 1.25 heights above the foot, its radius from 0.8 x the foot's half-width to 0.8 x the
	/// widest, leaning with the wind by `lean` x height x f^2, thinning as it widens, faded in over its first 15 % and out over its
	/// last half. Each emission (the cycle count) jitters the puff's place and size by its own hash (no regular stack).
	pub(super) fn puffs(&self, now: f64, k: &Knobs, seed: u32) -> Vec<Raw> {
		let rise = self.height * 1.05 / k.period;
		(0..k.n)
			.filter_map(|i| {
				let s = now / k.period as f64 + i as f64 / k.n as f64 + (seed >> 8) as f64 / 16777216.0;
				let cycle = s.floor();
				let f = (s - cycle) as f32;
				let h = hash(seed ^ (cycle as i64 as u32).wrapping_mul(0x9e37_79b9) ^ (i as u32).wrapping_mul(0x85eb_ca6b));
				let rnd = |b: u32| ((h >> b) & 0xff) as f32 / 127.5 - 1.0;
				let fade = smooth(0.0, 0.15, f) * (1.0 - smooth(0.5, 1.0, f));
				let r0 = 0.8 * self.w0 * k.width;
				let r = (r0 + 0.8 * (self.w1 - self.w0) * k.width * f.powf(0.6)) * (1.0 + 0.15 * rnd(16));
				let thick = self.thick * k.thick * fade * (r0 / r.max(1e-3)).min(1.0).powf(1.2);
				if thick <= 1e-3 || r <= 0.05 {
					return None;
				}
				let lean = k.lean * self.height * f * f;
				let jit = 0.3 * r;
				let pos = [
					self.foot[0] + WIND[0] * lean + jit * rnd(0),
					self.foot[1] + WIND[1] * lean + jit * rnd(8),
					self.foot[2] + self.height * (0.2 + 1.05 * f),
				];
				Some(Raw { pos, radius: r, thick, albedo: self.albedo, share: 1.0, age: f, rise, billow: self.billow * k.billow, tall: 1.35 - 0.35 * f })
			})
			.collect()
	}
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
	let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
	t * t * (3.0 - 2.0 * t)
}

fn hash(mut h: u32) -> u32 {
	h = (h ^ (h >> 16)).wrapping_mul(0x7feb_352d);
	h = (h ^ (h >> 15)).wrapping_mul(0x846c_a68b);
	h ^ (h >> 16)
}

#[cfg(test)]
mod tests {
	use super::*;

	const K: Knobs = Knobs { n: 8, period: 7.0, lean: 0.3, width: 1.0, thick: 1.0, billow: 4.0 };

	/// A camera-facing smoke quad (facing -y), `w` wide and `h` tall, its bottom middle at `p`, widening upward by `top` x.
	fn quad(index: u32, p: [f32; 3], w: f32, h: f32, top: f32) -> Taken {
		let (b, t) = (0.5 * w, 0.5 * w * top);
		Taken { index, pos: [[p[0] - b, p[1], p[2]], [p[0] + b, p[1], p[2]], [p[0] + t, p[1], p[2] + h], [p[0] - t, p[1], p[2] + h]], thick: 0.9, albedo: [0.4; 3], billow: 0.45 }
	}

	fn fire(at: [f32; 3], first: u32) -> Vec<Taken> {
		(0..4).map(|k| quad(first + k, [at[0] + k as f32 * 0.7 - 1.0, at[1], at[2]], 3.0, 12.0, 2.0)).collect()
	}

	#[test]
	fn the_quads_of_one_emitter_make_one_plume_and_two_fires_two() {
		let mut qs = fire([0.0, 0.0, 10.0], 0);
		qs.extend(fire([40.0, 5.0, 10.0], 4));
		let g = group(&qs);
		assert_eq!(g, vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]]);
		let p = Plume::of(&qs, &g[0], [0.0, -50.0, 10.0]).unwrap();
		assert!((p.foot[2] - 10.0).abs() < 1e-4 && (p.height - 12.0).abs() < 0.5, "{p:?}");
		assert!(p.w0 < p.w1 && p.w1 > 3.0, "narrow foot, wide top: {p:?}");
		assert_eq!(p.src, vec![0, 1, 2, 3]);
	}

	/// Quads leaning toward an eye above give the plume about the same height as seen from the side.
	#[test]
	fn the_height_does_not_shrink_when_the_quads_lean_toward_an_eye_above() {
		let side = Plume::of(&fire([0.0, 0.0, 0.0], 0), &[0, 1, 2, 3], [0.0, -50.0, 6.0], ).unwrap().height;
		// The same quads tilted 40 degrees toward an eye 40 degrees up: vertical extent x cos 40.
		let c = 40f32.to_radians().cos();
		let tilted: Vec<Taken> = fire([0.0, 0.0, 0.0], 0).into_iter().map(|mut q| {
			q.pos.iter_mut().for_each(|p| {
				let up = p[2];
				p[2] = up * c;
				p[1] -= up * (1.0 - c * c).sqrt();
			});
			q
		}).collect();
		let above = Plume::of(&tilted, &[0, 1, 2, 3], [0.0, -50.0 * c, 50.0 * (1.0 - c * c).sqrt() + 6.0 * c]).unwrap().height;
		assert!((above - side).abs() < 0.08 * side, "side {side}, above {above}");
	}

	/// Over a cycle every puff rises from the foot to above the quads, widens, and its thickness is 0 at birth and death: frame to
	/// frame each puff moves a little (no jump while it shows).
	#[test]
	fn puffs_rise_widen_and_are_born_and_die_unseen() {
		let qs = fire([0.0, 0.0, 10.0], 0);
		let p = Plume::of(&qs, &[0, 1, 2, 3], [0.0, -50.0, 10.0]).unwrap();
		let dt = 1.0 / 60.0;
		let mut last: Option<Vec<Raw>> = None;
		let (mut lo, mut hi) = (f32::MAX, f32::MIN);
		for f in 0..(60.0 * 14.0) as usize {
			let now = 100.0 + f as f64 * dt;
			let v = p.puffs(now, &K, p.seed());
			assert!(v.len() <= K.n);
			for r in &v {
				(lo, hi) = (lo.min(r.pos[2]), hi.max(r.pos[2]));
				assert!(r.thick <= p.thick + 1e-5 && r.share == 1.0 && (1.0..=1.35).contains(&r.tall));
			}
			if let Some(l) = &last {
				// Each drawn puff of this frame continues one of the last frame, or starts thin.
				for r in &v {
					let near = l.iter().any(|o| ((o.pos[0] - r.pos[0]).powi(2) + (o.pos[1] - r.pos[1]).powi(2) + (o.pos[2] - r.pos[2]).powi(2)).sqrt() < 0.1 && (o.radius - r.radius).abs() < 0.05);
					assert!(near || r.thick < 0.02, "a puff jumped: {r:?}");
				}
			}
			last = Some(v);
		}
		assert!(lo < 10.0 + 0.2 * p.height + 0.6 && hi > 10.0 + p.height, "rises from the foot to above the quads: {lo}..{hi}");
		let v = p.puffs(100.0, &K, p.seed());
		let (low, top) = (v.iter().min_by(|a, b| a.age.total_cmp(&b.age)).unwrap(), v.iter().max_by(|a, b| a.age.total_cmp(&b.age)).unwrap());
		assert!(top.radius > low.radius && top.pos[2] > low.pos[2], "the older puff is higher and wider");
		assert!((v[0].rise - p.height * 1.05 / K.period).abs() < 1e-4, "the noise rises with the puffs");
	}

	#[test]
	fn faded_quads_make_no_plume_and_the_count_follows_the_knob() {
		let mut qs = fire([0.0, 0.0, 0.0], 0);
		qs.iter_mut().for_each(|q| q.thick = 0.0);
		assert!(Plume::of(&qs, &[0, 1, 2, 3], [0.0, -50.0, 0.0]).is_none());
		let p = Plume::of(&fire([0.0, 0.0, 0.0], 0), &[0, 1, 2, 3], [0.0, -50.0, 0.0]).unwrap();
		assert!(p.puffs(3.0, &Knobs { n: 4, ..K }, 0).len() <= 4 && p.puffs(3.0, &Knobs { n: 0, ..K }, 0).is_empty());
	}
}
