//! The hero marker's inputs and its per-vertex CPU build (`Gpu::set_marker`). The legacy full build, the selective clipper
//! (`marker_clip.rs`, only the faces that may cross the guard band) and the check modes all build vertices with `MarkerCtx::vertex`,
//! so a vertex is the same bytes whichever of them asks for it. `MarkerIn::hero_params` is the GPU build's parameter block
//! (`gpuhero.rs`) of the same inputs.

use super::*;

/// A loose wrench's vertices `[a, b)` with the light of its own moby: the vertex `light` and the third-light id the build writes there
/// instead of the hero's (`loose_range.rs`; retail lights a thrown wrench where it is, not at Ratchet's feet).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(in crate::viewer) struct Own {
	pub a: usize,
	pub b: usize,
	pub light: [f32; 4],
	pub id: f32,
}

/// What the CPU build and the GPU build both read besides the model and the poses (one `set_marker` call).
#[derive(Clone, Copy)]
pub(in crate::viewer) struct MarkerIn {
	pub feet: [f32; 3],
	pub c: f32,
	pub sn: f32,
	/// BRIDGE-75: a tilted hero uses the full euler basis (rows of euler_m); untilted keeps the yaw-only turn.
	pub rows: Option<[[f32; 3]; 3]>,
	pub tint: Option<([f32; 3], usize)>,
	pub hero_len: usize,
	pub clank: Option<(usize, usize, [f32; 3])>,
	pub ghosts: [Option<(usize, usize, u8)>; 5],
	pub bomb: Option<(usize, usize, [f32; 3])>,
	pub shift: Option<(usize, usize, f32)>,
	pub id: f32,
	pub light: [f32; 4],
	/// The loose wrench ranges, each lit by its own moby (`Own`); the hero's `id` / `light` everywhere else. Empty = all `None`.
	pub own: [Option<Own>; super::super::loose_range::MAX],
}

/// The poses a vertex is built from: the model's own, one skinned pose, or the blend of two (per vertex, as `blend_posed`).
#[derive(Clone, Copy)]
pub(in crate::viewer) enum Pose<'a> {
	Model,
	Plain(&'a Posed),
	Blend { before: &'a Posed, now: &'a Posed, t: f32 },
}

pub(in crate::viewer) struct MarkerCtx<'a> {
	pub inp: MarkerIn,
	pub model: &'a [Vertex],
	pub pose: Pose<'a>,
}

impl MarkerIn {
	/// The light and the third-light id of vertex `k`: its own loose wrench's, else the hero's.
	pub(in crate::viewer) fn lit(&self, k: usize) -> ([f32; 4], f32) {
		match self.own.iter().flatten().find(|o| (o.a..o.b).contains(&k)) {
			Some(o) => (o.light, o.id),
			None => (self.light, self.id),
		}
	}

	/// Model space to world orientation (without the feet offset).
	pub(in crate::viewer) fn turn(&self, v: [f32; 3]) -> [f32; 3] {
		match &self.rows {
			Some(r) => [
				v[0] * r[0][0] + v[1] * r[1][0] + v[2] * r[2][0],
				v[0] * r[0][1] + v[1] * r[1][1] + v[2] * r[2][1],
				v[0] * r[0][2] + v[1] * r[1][2] + v[2] * r[2][2],
			],
			None => [v[0] * self.c - v[1] * self.sn, v[0] * self.sn + v[1] * self.c, v[2]],
		}
	}

	/// The GPU build's parameter block (`parity` = `HeroGpu::parity`, `t` the blend factor, `can_blend` whether the shader blends).
	pub(in crate::viewer) fn hero_params(&self, parity: f32, t: f32, can_blend: bool, n_model: usize) -> super::gpuhero::HeroParams {
		let r = self.rows.unwrap_or([[0.0; 3]; 3]);
		let row4 = |i: usize| [r[i][0], r[i][1], r[i][2], 0.0];
		let mut gh = [[0.0f32; 4]; 5];
		let mut gr = [[0u32; 4]; 5];
		for (k, g) in self.ghosts.iter().enumerate() {
			if let Some(&(a, b, al)) = g.as_ref() {
				gh[k][0] = moby_alpha_encoding(self.lit(a).1, al).0;
				gr[k] = [a as u32, b as u32, 1, 0];
			}
		}
		let (mut ol, mut orr) = ([[0.0f32; 4]; 3], [[0u32; 4]; 3]);
		for (k, o) in self.own.iter().enumerate() {
			if let Some(o) = o {
				ol[k] = o.light;
				orr[k] = [o.a as u32, o.b as u32, 1, o.id.to_bits()];
			}
		}
		let (tint, clank, bomb, shift) = (self.tint.map(|(t, s)| (t, s as u32)), self.clank, self.bomb, self.shift);
		super::gpuhero::HeroParams {
			feet: [self.feet[0], self.feet[1], self.feet[2], 1.0],
			cs: [self.c, self.sn, t, self.id],
			r0: { let mut r = row4(0); r[3] = parity; r },
			r1: row4(1),
			r2: row4(2),
			light: self.light,
			tint: tint.map_or([0.0; 4], |(t, _)| [t[0], t[1], t[2], 0.0]),
			clank: clank.map_or([0.0; 4], |(_, _, t)| [t[0], t[1], t[2], 0.0]),
			bomb: [bomb.map_or(0.0, |b| b.2[0]), bomb.map_or(0.0, |b| b.2[1]), bomb.map_or(0.0, |b| b.2[2]), shift.map_or(0.0, |s| s.2)],
			gh,
			gr,
			ol,
			orr,
			u0: [n_model as u32, can_blend as u32, self.rows.is_some() as u32, tint.is_some() as u32],
			u1: [tint.map_or(0, |t| t.1), self.hero_len as u32, clank.map_or(0, |l| l.0 as u32), clank.map_or(0, |l| l.1 as u32)],
			u2: [clank.is_some() as u32, bomb.is_some() as u32, shift.is_some() as u32, 0],
			u3: [bomb.map_or(0, |b| b.0 as u32), bomb.map_or(0, |b| b.1 as u32), shift.map_or(0, |s| s.0 as u32), shift.map_or(0, |s| s.1 as u32)],
		}
	}
}

impl MarkerCtx<'_> {
	/// Vertex `k` of the marker in world space with its final colour, light and uv (the per-vertex body of the CPU build).
	pub(in crate::viewer) fn vertex(&self, k: usize) -> Vertex {
		let (inp, m) = (&self.inp, &self.model[k]);
		let (light, id) = inp.lit(k);
		let v = match self.pose {
			Pose::Model => *m,
			Pose::Plain((ps, ns)) => Vertex { pos: ps[k], normal: ns[k], ..*m },
			Pose::Blend { before, now, t } => {
				let (pos, normal) = super::smooth::blend_pn(before.0[k], before.1[k], now.0[k], now.1[k], t);
				Vertex { pos, normal, ..*m }
			}
		};
		let p = inp.turn(v.pos);
		let colour = match inp.tint.filter(|&(_, s)| k >= s && k < inp.hero_len) {
			Some((t, _)) => [t[0], t[1], t[2], -2.0],
			None => match inp.clank.filter(|l| (l.0..l.1).contains(&k)) {
				Some((_, _, t)) => [t[0], t[1], t[2], -2.0],
				None => match inp.ghosts.iter().flatten().find(|g| (g.0..g.1).contains(&k)) {
					Some(&(_, _, a)) => [v.colour[0], v.colour[1], v.colour[2], moby_alpha_encoding(id, a).0],
					None => [v.colour[0], v.colour[1], v.colour[2], id],
				},
			},
		};
		// WPN-3.2: the held bomb's colour scale rides the same per-range vertex colour path.
		let colour = match inp.bomb.filter(|b| (b.0..b.1).contains(&k)) {
			Some((_, _, t)) => [colour[0] * t[0], colour[1] * t[1], colour[2] * t[2], colour[3]],
			None => colour,
		};
		let uv = match inp.shift {
			Some((a, b, dv)) if (a..b).contains(&k) => [v.uv[0], v.uv[1] + dv],
			_ => v.uv,
		};
		Vertex { pos: [p[0] + inp.feet[0], p[1] + inp.feet[1], p[2] + inp.feet[2]], uv, normal: inp.turn(v.normal), colour, light, ..v }
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const HERO_LIGHT: [f32; 4] = [0.9, 0.2, 0.9, 7.5];
	const OWN_LIGHT: [f32; 4] = [0.5, 0.5, 0.5, 3.25];

	fn model(n: usize) -> Vec<Vertex> {
		(0..n).map(|i| Vertex { pos: [i as f32, 0.0, 0.0], uv: [0.0, 0.0], normal: [0.0, 0.0, 1.0], colour: [0.5, 0.5, 0.5, 1.0], light: [0.0; 4] }).collect()
	}

	fn input(own: [Option<Own>; 3], ghosts: [Option<(usize, usize, u8)>; 5]) -> MarkerIn {
		MarkerIn { feet: [0.0; 3], c: 1.0, sn: 0.0, rows: None, tint: None, hero_len: 4, clank: None, ghosts, bomb: None, shift: None, id: 1.0, light: HERO_LIGHT, own }
	}

	/// A loose wrench's vertices take its own light and third-light id (afterimage alpha code included); every other vertex keeps the hero's.
	#[test]
	fn an_own_range_is_lit_by_its_own_moby_and_the_rest_by_the_hero() {
		let ghost = (10, 12, 0x40u8);
		let own = [Some(Own { a: 6, b: 8, light: OWN_LIGHT, id: 0.0 }), Some(Own { a: 10, b: 12, light: OWN_LIGHT, id: 2.0 }), None];
		let m = model(14);
		let ctx = MarkerCtx { inp: input(own, [Some(ghost), None, None, None, None]), model: &m, pose: Pose::Model };
		for k in 0..14 {
			let v = ctx.vertex(k);
			let (light, w) = match k {
				6 | 7 => (OWN_LIGHT, 0.0),
				10 | 11 => (OWN_LIGHT, moby_alpha_encoding(2.0, 0x40).0),
				_ => (HERO_LIGHT, 1.0),
			};
			assert_eq!((v.light, v.colour[3]), (light, w), "vertex {k}");
		}
		// Without an own range the vertex is the hero's, the ghost code from the hero's id.
		let ctx = MarkerCtx { inp: input([None; 3], [Some(ghost), None, None, None, None]), model: &m, pose: Pose::Model };
		assert_eq!((ctx.vertex(6).light, ctx.vertex(6).colour[3]), (HERO_LIGHT, 1.0));
		assert_eq!(ctx.vertex(10).colour[3], moby_alpha_encoding(1.0, 0x40).0);
	}

	/// The GPU parameter block carries the same ranges, lights and ids as the CPU build, the afterimage code from the own id.
	#[test]
	fn the_gpu_block_carries_the_own_ranges() {
		let own = [Some(Own { a: 6, b: 8, light: OWN_LIGHT, id: 3.0 }), None, Some(Own { a: 10, b: 12, light: HERO_LIGHT, id: 0.0 })];
		let p = input(own, [None, Some((10, 12, 0x20)), None, None, None]).hero_params(0.0, 1.0, false, 14);
		assert_eq!((p.ol[0], p.orr[0]), (OWN_LIGHT, [6, 8, 1, 3.0f32.to_bits()]));
		assert_eq!((p.ol[1], p.orr[1]), ([0.0; 4], [0; 4]));
		assert_eq!((p.ol[2], p.orr[2]), (HERO_LIGHT, [10, 12, 1, 0.0f32.to_bits()]));
		assert_eq!(p.gh[1][0], moby_alpha_encoding(0.0, 0x20).0);
		assert_eq!(p.light, HERO_LIGHT);
		// No loose wrench: the block is as before (nothing present, the hero's id in the ghost code).
		let q = input([None; 3], [None, Some((10, 12, 0x20)), None, None, None]).hero_params(0.0, 1.0, false, 14);
		assert_eq!((q.ol, q.orr), ([[0.0; 4]; 3], [[0; 4]; 3]));
		assert_eq!(q.gh[1][0], moby_alpha_encoding(1.0, 0x20).0);
	}
}
