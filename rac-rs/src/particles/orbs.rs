//! Nanotech orb (class 806): the FUN_002d8398 sparkle sim and pickup (PAR-2b), its post-arrival glows and puffs
//! (PAR-2c) and the FUN_002d91b8 body/rim/glass draw (PAR-2a). decomp/notes/particles.md PAR-2a..2c.

use super::orb_tables::*;
use super::*;

/// Bob amplitude, sparkle vector length / base radius, radius wobble amplitude.
const ORB_BOB: f32 = 0.06;
/// Body / glass fx texture 21, rim fx 11.
const ORB_BODY_TEX: usize = 21;
const ORB_RIM_TEX: usize = 11;






fn fb(w: u32) -> f32 {
	f32::from_bits(w)
}


/// PAR-2a glass UV of a vertex at `d` = vertex - eye: the (x, z) pair set to length 2, `* 0.5 + 0.5`, each reduced
/// mod 8 by truncation (FUN_001ffe10).
pub(super) fn glass_uv(d: [f32; 3]) -> [f32; 2] {
	let l = (d[0] * d[0] + d[2] * d[2]).sqrt();
	let (x, z) = if l == 0.0 { (0.0, 0.0) } else { (d[0] * (2.0 / l), d[2] * (2.0 / l)) };
	let m8 = |v: f32| (v / 8.0 - (v / 8.0).trunc()) * 8.0;
	[m8(x * 0.5 + 0.5), m8(z * 0.5 + 0.5)]
}

impl Particles {
	/// The orbs `orb_tick` registered for drawing this tick.
	pub fn orb_draws(&self) -> &[OrbDraw] {
		&self.orb_draws
	}










	/// PAR-2a nanotech orb draw (FUN_002d91b8) for a camera at `eye`: the body (three strips over the 290-vertex
	/// table, fx 21, alpha-over), the additive rim (32 quads, fx 11) and the crate glass while held. The sparkles are
	/// pool sprites (type 0x3e). Retail draws all three with depth test GEQUAL and no depth write (TEST 0x53001).
	pub fn nanotech_orb(&self, o: &OrbDraw, eye: [f32; 3], quads: &mut Vec<WorldQuad>) {
		let sin = o.bob / ORB_BOB;
		let c = [o.pos[0], o.pos[1], o.pos[2] + 0.5 + o.bob];
		let x = nz(sub(eye, c));
		let y = nz(cross(o.parent[2], x));
		let z = cross(x, y);
		let at = |v: [f32; 3]| add(add(add(scale(x, v[0]), scale(y, v[1])), scale(z, v[2])), c);
		// Body: colour 0x50804040 on every vertex.
		let body = [0.5, 0.5, 1.0, 0.625];
		let verts: Vec<[f32; 3]> = ORB_POS.iter().map(|w| at([fb(w[0]), fb(w[1]), fb(w[2])])).collect();
		let st = |i: usize| [fb(ORB_ST[i][0]), fb(ORB_ST[i][1])];
		for (start, n) in [(0, 148), (146, 109), (254, 36)] {
			for k in 0..n - 2 {
				let (a, b, d) = (start + k, start + k + 1, start + k + 2);
				quads.push(WorldQuad { pos: [verts[a], verts[b], verts[d], verts[d]], uv: [st(a), st(b), st(d), st(d)], colour: [body; 4], tex: ORB_BODY_TEX, additive: false });
			}
		}
		// Rim: inner alpha trunc(sin * 64 + 128) over rgb 0x802020, outer 0; corners 0, 1 at 0.3, 2, 3 at 0.3 * 1.125.
		let alpha = (sin * 64.0 + 128.0) as i32;
		let inner = [0.25, 0.25, 1.0, alpha as f32 / FULL];
		let outer = [0.0; 4];
		let outer_scale = 0.3f32 * 1.125;
		for idx in ORB_RIM_IDX {
			let corner = |i: usize| {
				let v = ORB_RIM_V[idx[i]];
				let s = if i < 2 { 0.3 } else { outer_scale };
				at([fb(v[0]) * s, fb(v[1]) * s, fb(v[2]) * s])
			};
			// Strip (0, 1, 2), (1, 2, 3) as a fan quad 0, 1, 3, 2.
			quads.push(WorldQuad {
				pos: [corner(0), corner(1), corner(3), corner(2)],
				uv: [[0.5, 0.5], [0.5, 0.5], [0.5, 1.0], [0.5, 1.0]],
				colour: [inner, inner, outer, outer],
				tex: ORB_RIM_TEX,
				additive: true,
			});
		}
		// Glass (state 1): the crate matrix with the orb position as translation, colour 0x107f7f7f.
		if o.glass {
			let m = o.parent;
			let glass = [127.0 / FULL, 127.0 / FULL, 127.0 / FULL, 16.0 / FULL];
			for rec in ORB_GLASS_IDX {
				let corner = |i: usize| {
					let v = ORB_GLASS_V[rec[i]];
					add(add(add(scale(m[0], fb(v[0])), scale(m[1], fb(v[1]))), scale(m[2], fb(v[2]))), scale(o.pos, fb(v[3])))
				};
				let p = [corner(0), corner(1), corner(3), corner(2)];
				quads.push(WorldQuad { pos: p, uv: p.map(|q| glass_uv(sub(q, eye))), colour: [glass; 4], tex: ORB_BODY_TEX, additive: false });
			}
		}
	}
}
