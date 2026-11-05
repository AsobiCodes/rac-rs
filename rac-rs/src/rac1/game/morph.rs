//! Morph-o-Ray (item 21): the held class 185 updater (WPN-6.7a), the conversion into a class 270 sheep and the 20-slot
//! sheep ring (WPN-6.7b), the sheep updater (WPN-6.7c1 prologue / death, 6.7c2 states 1-4 and tail, 6.7c3 state 5 and
//! the Suck Cannon wrappers over the BRIDGE-54 view), the beam tick and draw (WPN-6.7e1 / 6.7e2) and the sheep debris
//! class 428 (WPN-6.7d) (decomp/notes/weapons_framework.md "lane 7: Morph-o-Ray"). Class 804 (WPN-6.7e3) is never
//! spawned on Veldin / Novalis and has no port.
//!
//! The conversion flash FUN_0027e068 (WPN-6.5e3 `Effect::Flash5`) is `morph_flash`; the HUD meter 0x7533 open /
//! close (WPN-6.7e4) is `morph_meter_open` / `morph_meter_close` on the slot-4 owner `Game.breath_hud` (the meter value
//! is `Game.morph_meter`).
//!
//! Port carriers [I]: the beam globals (0x1dc2e0.., 0x16176c reset flag) live in `Game.morph_beam`; the actor +0x58
//! anim rate is `anim.clock.m`; the sheep state byte is a u8 with 253 / 254 standing for -3 / -2 (the BRIDGE-54 view
//! takes `&mut u8`); the target-eligible byte +0xbc has no carrier; the held loop voice follows the hero.

use super::*;
use crate::particles::WorldQuad;
use crate::physics::cross;

/// The held class.
pub const HELD: u32 = 185;
/// The sheep class.
pub const SHEEP: u32 = 270;
/// The sheep debris class.
pub const DEBRIS: u32 = 428;
/// The item row (gold byte 0x13e535 = 0x13e520 + 21).
const ITEM: u8 = 21;
/// The loop-voice owner of the held cue 0 (the viewer's `loop_at` places it at the hero).
pub const MORPH_LOOP_OWNER: usize = HERO_LOOP_OWNER - 2;
/// Gravity 0x13f5e0.
const G: V3 = [0.0, 0.0, -1.0];
/// The draw corner table 0x1dc2a0 (x unused).
const CORNERS: [[f32; 3]; 4] = [[0.0, -1.0, 1.0], [0.0, -1.0, -1.0], [0.0, 1.0, 1.0], [0.0, 1.0, -1.0]];
const UV4: [[f32; 2]; 4] = [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]];

/// FUN_00220f40 set_length(l, v): `v * l / |v|`, zero for a zero vector.
fn set_len(v: V3, l: f32) -> V3 {
	super::bolt::set_len(v, l)
}





/// A colour word as vertex colour: bytes R (low), G, B, A each / 128.
fn rgba(w: u32) -> [f32; 4] {
	let b = w.to_le_bytes();
	[b[0] as f32 / 128.0, b[1] as f32 / 128.0, b[2] as f32 / 128.0, b[3] as f32 / 128.0]
}

/// A strip quad (v0, v1, v2, v3) as a fan WorldQuad [v0, v1, v3, v2], additive.
fn strip_quad(v: [V3; 4], uv: [[f32; 2]; 4], c: [u32; 4], tex: usize) -> WorldQuad {
	WorldQuad { pos: [v[0], v[1], v[3], v[2]], uv: [uv[0], uv[1], uv[3], uv[2]], colour: [rgba(c[0]), rgba(c[1]), rgba(c[3]), rgba(c[2])], tex, additive: true }
}





/// The beam globals 0x1dc2e0 (points), 0x1dc3a0 / 0x1dc460 (twisted paths), 0x1dc520 (rings), the phases, the three
/// travelling glows and the draw registration (WPN-6.7e1). Zero at level load (bss).
#[derive(Clone, Debug, Default)]
pub struct MorphBeam {
	points: [V3; 12],
	path1: [V3; 12],
	path2: [V3; 12],
	rings: [[V3; 20]; 12],
	ring_u: f32,
	s21: f32,
	s20: f32,
	path_u: f32,
	sub_on: [bool; 3],
	sub_alpha: [f32; 3],
	sub_size: [f32; 3],
	sub_pos: [V3; 3],
	sub_dir: [V3; 3],
	row1: V3,
	/// FUN_0021b098 registered the draw this tick.
	pub(super) drawn: bool,
}







impl Game {
























	/// WPN-6.7e2 FUN_002d31e8: the beam's world quads for this frame (empty unless an active tick registered the draw).
	pub fn morph_beam_quads(&self, cam: V3) -> Vec<WorldQuad> {
		let b = &self.morph_beam;
		let mut out = Vec::new();
		if !b.drawn {
			return out;
		}
		let gold = self.gold(ITEM);
		// (A) Ribbons.
		ribbon(&mut out, &b.path1, 0x7f2020, b.path_u, cam);
		ribbon(&mut out, &b.path2, 0x20207f, b.path_u, cam);
		// (B) Sub glows.
		let rgb = if gold { 0x0000_7f00 } else { 0x007f_207f };
		for k in (0..3).filter(|&k| b.sub_on[k]) {
			let m0 = b.sub_dir[k];
			let m1 = set_len(cross([0.0, 0.0, 1.0], m0), 1.0);
			let m2 = cross(m0, m1);
			let size = b.sub_size[k];
			let v = CORNERS.map(|c| add(add(b.sub_pos[k], scale(m1, c[1] * size)), scale(m2, c[2] * size)));
			let c = rgb | (b.sub_alpha[k] as u32) << 24;
			out.push(strip_quad(v, UV4, [c; 4], 8));
		}
		// (C) Flare.
		let at = add(b.points[0], set_len(b.row1, -0.5));
		let m0 = set_len(sub(cam, at), 1.0);
		let m1 = set_len(cross(G, m0), -1.0);
		let m2 = cross(m0, m1);
		for (s, c) in [(0.4f32, 0x7f207f7fu32), (0.1, 0xffffffff)] {
			let v = CORNERS.map(|k| add(add(at, scale(m1, k[1] * s)), scale(m2, k[2] * s)));
			out.push(strip_quad(v, UV4, [c; 4], 11));
		}
		// (D) Rings.
		let mut col = [if gold { 0x6000_7f00u32 } else { 0x407f_207f }; 4];
		let mut u = b.ring_u;
		let du = 0.066667f32;
		for k in 2..=11 {
			let (mut t21, mut t20) = (b.s21, b.s20);
			for j in 0..20 {
				let j2 = (j + 1) % 20;
				let (a, bb, c, d) = (b.rings[k][j], b.rings[k][j2], b.rings[k - 1][j], b.rings[k - 1][j2]);
				if dot(cross(sub(a, bb), sub(c, bb)), sub(bb, cam)) >= 0.0 {
					t21 += 0.05;
					t20 += 0.05;
					continue;
				}
				if k == 11 {
					col[0] = 0x007f_207f;
					col[1] = 0x007f_207f;
				}
				for t in [&mut t21, &mut t20] {
					out.push(strip_quad([a, bb, c, d], [[u, *t], [u, *t + 0.05], [u - du, *t], [u - du, *t + 0.05]], col, 13));
					*t += 0.05;
				}
			}
			u += du;
		}
		out
	}
}


/// FUN_002d3610: the core (fx 14) and glow (fx 16) passes of one twisted path.
fn ribbon(out: &mut Vec<WorldQuad>, p: &[V3; 12], rgb: u32, path_u: f32, cam: V3) {
	let n = |a: V3, b: V3| {
		let v = cross(sub(a, cam), sub(b, a));
		let l = length(v);
		scale(v, if l == 0.0 { 0.0 } else { 0.05 / l })
	};
	let (c, c0) = (0x7f7f7f | 0x80 << 24, 0x7f7f7f);
	let mut col = [c; 4];
	let mut a0 = add(p[0], n(p[0], p[1]));
	let mut b0 = sub(p[0], n(p[0], p[1]));
	let uv = [[path_u, 0.0], [path_u, 1.0], [path_u + 1.0, 0.0], [path_u + 1.0, 1.0]];
	for i in 1..=10 {
		let a2 = add(p[i], n(p[i], p[i + 1]));
		let b2 = sub(p[i], n(p[i], p[i + 1]));
		match i {
			1 => (col[0], col[1]) = (c0, c0),
			2 => (col[0], col[1]) = (c, c),
			_ => {}
		}
		if i == 10 {
			(col[2], col[3]) = (c0, c0);
		}
		out.push(strip_quad([a0, b0, a2, b2], uv, col, 14));
		(a0, b0) = (a2, b2);
	}
	let c = rgb | 64 << 24;
	let mut col = [c; 4];
	for i in 0..=10 {
		let mut a = sub(p[i], cam);
		let mut b = sub(p[i + 1], cam);
		let (la, lb) = (length(a), length(b));
		if lb < la {
			if lb == 0.0 {
				b[2] = a[2];
			} else {
				b = scale(b, la / lb);
			}
		} else if la == 0.0 {
			a[2] = b[2];
		} else {
			a = scale(a, lb / la);
		}
		let (mut pp, mut q) = (add(cam, a), add(cam, b));
		let d = sub(q, pp);
		pp = sub(pp, scale(d, 0.9));
		q = add(q, scale(d, 0.9));
		let nv = cross(d, sub(cam, pp));
		let l = length(nv);
		let nv = scale(nv, if l == 0.0 { 0.0 } else { 0.4 / l });
		match i {
			0 => (col[0], col[1]) = (rgb, rgb),
			1 => (col[0], col[1]) = (c, c),
			_ => {}
		}
		if i == 10 {
			(col[2], col[3]) = (rgb, rgb);
		}
		out.push(strip_quad([add(pp, nv), sub(pp, nv), add(q, nv), sub(q, nv)], UV4, col, 16));
	}
}

