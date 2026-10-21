//! Retail's second custom draw queue (queue B) and the queue callbacks the mirror ports as world quads.
//!
//! Frame draw Eudora FUN_001eae20 runs queue A (count 0x15f424, FUN_001ebc98) before the effect pool and queue B
//! (count 0x15f428, fn[] 0x16db80, arg[] 0x16dc80; registered by FUN_001ebe00, run by FUN_001ebe50) after it
//! (weapon_fx.md root cause 2) [C]. The fn[] / arg[] arrays sit 0x200 past queue A's on Eudora [C]; the same offset is
//! assumed on the other levels [I].
//!
//! Each ported callback reads its data addresses from its own code in RAM (lui/addiu or gp-relative loads at fixed
//! offsets of the Eudora body; the twins are the same code), so one port serves every level whose twin is listed.
//! A load whose opcode does not match makes the callback draw nothing [C: decode is checked].
//!
//! The mirror snapshot is taken after the field ran its own draw, so state the callback advances while drawing (the
//! tube's texture scroll) is stepped back to its draw-time value here [I].

use crate::particles::WorldQuad;
use ee_oracle::Machine;

/// Queue B element count (max 0x40 like queue A) [C].
pub const COUNT_B: u32 = 0x15f428;
/// fx textures the ported callbacks bind (FUN_001ebb00 index) that the viewer's fx table (viewer/app.rs `fx_keys`) lacked:
/// Taunter rings and Morph end sprites 8 (immediate), Morph ring tube 0xd (gp-0x5514), beam ribbons 0xe / 0x10 (Tesla
/// gp-0x5678 / gp-0x5674, Morph gp-0x550c / gp-0x5508) [C: Eudora RAM, dump/w19_120 and w21_120]. The glows' 0xb and
/// the tube's 0x15 are in the table already. Hero swing trail (`hero_fx`): 0x14 (records 0..2) and 0xc (record 3) [C:
/// immediates of FUN_002992b0]. Swingshot rope (`swing_rope`): 0xf [C: `addiu a0, zero, 0xf` at +0x68 of FUN_002b87b0].
pub const FX_TEXTURES: [usize; 7] = [0x08, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x14];
/// fx textures also bound with GS CLAMP_1 = 5 (`particles::FX_CLAMP`): the hero swing trail's 0x14 / 0xc and the Swingshot
/// rope's 0xf. Every other ported context sends CLAMP_1 = 0 (sd $zero at sp+0x70: suck tube, Taunter, Tesla, Morph,
/// Walloper) [C].
pub const FX_CLAMPED: [usize; 3] = [0x0c, 0x0f, 0x14];
/// Queue A element count.
const COUNT_A: u32 = 0x15f424;

/// Level order (where.py LEVELS): veldin novalis aridia kerwan eudora rilgar nebula umbris batalia gaspar orxon
/// pokitaru hoven oltanis_orbit oltanis quartu kalebo veldin_orbit veldin_finale.
/// Suck Cannon tube draw (Eudora FUN_002df5a0 = Novalis 0x305a88 suck_tube_draw; twins.py --all) [C]. Queue B while
/// held, queue A for the release fade (FUN_002e0c98).
const SUCK_TUBE: [u32; 19] = [
	0x2dd290, 0x305a88, 0x2e7e88, 0x2d8550, 0x2df5a0, 0x312d50, 0x2ede78, 0x308d78, 0x2fef98, 0x2fe6c0, 0x2d1eb0, 0x3064b0, 0x2fd678, 0x300250, 0x2f97f8,
	0x2e21c0, 0x2dbbf8, 0x2dfa58, 0x2e75a8,
];
/// `SUCK_TUBE` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const SUCK_TUBE_USA: [u32; 19] = [
	0x2dd8e8, 0x306158, 0x2e8558, 0x2d8c20, 0x2dfc78, 0x3133c0, 0x2ee550, 0x309450, 0x2ff668, 0x2fed98, 0x2d2588, 0x306b80, 0x2fdd48, 0x300928, 0x2f9e48,
	0x2e2890, 0x2dc2e8, 0x2e0130, 0x2e7c78,
];
/// Taunter ring draw (queue A; Eudora FUN_002a9910 = Novalis 0x2ccb00 taunter_rings_draw) [C].
const TAUNTER: [u32; 19] = [
	0x2b6f10, 0x2ccb00, 0x2b68a8, 0x2a8900, 0x2a9910, 0x2e2b80, 0x2c4098, 0x2dffa0, 0x2bfea0, 0x2d6350, 0x2a8750, 0x2dcc80, 0x2cec30, 0x2d0cb0, 0x2c5b38,
	0x2b0188, 0x2b1788, 0x2b4bd0, 0x2b9700,
];
/// `TAUNTER` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const TAUNTER_USA: [u32; 19] = [
	0x2b7568, 0x2cd1d0, 0x2b6f78, 0x2a8fd0, 0x2a9fe8, 0x2e31f0, 0x2c4770, 0x2e0678, 0x2c0570, 0x2d6a28, 0x2a8e28, 0x2dd350, 0x2cf300, 0x2d1388, 0x2c6188,
	0x2b0858, 0x2b1e78, 0x2b52a8, 0x2b9dd0,
];

fn li(level: u32) -> usize {
	level as usize
}

/// The gp the callbacks run with: 0x166c00 on every level (regs.bin of fg1 / fg2 states of levels 0 5 14 16 all hold gp = 0x166c00;
/// corrected 2026-10-20: was 0x166c40 for levels 0 5 14 16, which read garbage floats / textures there: the Tesla Claw arc,
/// Morph-o-Ray, Walloper and the other gp-slot ports drew nothing on Veldin, Rilgar, Oltanis and Kalebo III; queue_q2::gp
/// had the same fix on 2026-10-12) [C].
fn gp(_level: u32) -> u32 {
	0x166c00
}

/// Queue A / B arrays (fn[], arg[]) of `level`: queue A from retail.rs `draw_queue`, queue B 0x200 past it.
fn arrays(level: u32, b: bool) -> Option<(u32, u32)> {
	let (f, a) = super::draw_queue(level)?;
	Some(if b { (f + 0x200, a + 0x200) } else { (f, a) })
}

/// The live entries (fn, arg) of queue A or B.
pub(super) fn entries(m: &Machine, level: u32, b: bool) -> Vec<(u32, u32)> {
	let Some((fns, args)) = arrays(level, b) else { return Vec::new() };
	(0..m.rd32(if b { COUNT_B } else { COUNT_A }).min(0x40)).map(|i| (m.rd32(fns + 4 * i), m.rd32(args + 4 * i) & 0x01ff_ffff)).collect()
}

/// The Gadgetron vendor glow draw callback (Novalis FUN_002ba2f0, queued on queue B by the class-11 update FUN_002baa58 via
/// FUN_0021b098 in states 1 and 2; binary twins on all 19 levels, twins.py) [C]. Entries are (callback, vendor moby).
pub const VENDOR_FN: [u32; 19] = [0x2a49e8, 0x2ba2f0, 0x2a48a8, 0x294e30, 0x296b70, 0x2ce8d8, 0x2b1958, 0x2cda78, 0x2ad450, 0x2c2b78, 0x295850, 0x2c8858, 0x2bc330, 0x2b0bc8, 0x2aea48, 0x2927b0, 0x29d7b8, 0x29cac0, 0x2a4410];
/// `VENDOR_FN` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
pub const VENDOR_FN_USA: [u32; 19] = [0x2a5040, 0x2ba9c0, 0x2a4f78, 0x295500, 0x297248, 0x2cef48, 0x2b2030, 0x2ce150, 0x2adb20, 0x2c3250, 0x295f28, 0x2c8f28, 0x2bca00, 0x2b12a0, 0x2af098, 0x292e80, 0x29dea8, 0x29d198, 0x2a4ae0];

/// The Veldin robot lightning draw callback (Veldin FUN_002e2498, queued on queue A by the class-1471 helper update
/// FUN_002e1798 while an arc is live; binary twins Oltanis 0x3040a8 and Quartu 0x2e8f58, twins.py) [C]. Other levels have none.
pub fn arc_fn(level: u32) -> Option<u32> {
	match level {
		0 => Some(crate::retail::disc::va(0x2e2498)),
		14 => Some(crate::retail::disc::va(0x3040a8)),
		15 => Some(crate::retail::disc::va(0x2e8f58)),
		_ => None,
	}
}

/// The arc tables a level's callback uses: arc state words (3 x u32), UV scrolls (3 x f32), owner pointers (3 x u32), and the
/// strand polylines (slot i: strand A at +0x280 i, strand B at +0x140 + 0x280 i; 20 points x 16 bytes, strand space).
/// The origin matrices (translation row at -0xf0 + 0x40 i) and end points (-0x60 + 0x10 i) sit below the polylines [C: Veldin
/// 0x1e6cd0 / 0x1e6d60 under 0x1e6dc0; Oltanis FUN_00304940 reads 0x1edcf0 under 0x1edde0].
#[derive(Clone, Copy, Debug)]
pub struct ArcTables {
	pub state: u32,
	pub uv: u32,
	pub owner: u32,
	pub poly: u32,
}

/// Decode the table addresses from the callback's own `lui`/`addiu` pairs (Veldin: 0x161bb8 state, 0x161c38 UV, 0x161c48 owner,
/// 0x1e6dc0 polylines twice; Oltanis +0x550 / +0x7020). The polylines are the address built twice; the other three sorted
/// ascending are state, UV, owner. None when the code does not have that shape [C: decode is checked].
pub fn arc_tables(m: &Machine, f: u32) -> Option<ArcTables> {
	let mut hi = [None::<u32>; 32];
	let mut vals: Vec<u32> = Vec::new();
	for k in 0..0x60 {
		let w = m.rd32(f + 4 * k);
		let (op, rs, rt, imm) = (w >> 26, (w >> 21 & 31) as usize, (w >> 16 & 31) as usize, w & 0xffff);
		match op {
			0x0f => hi[rt] = Some(imm << 16),
			0x09 | 0x0d if rs == rt => {
				if let Some(h) = hi[rs] {
					let v = if op == 9 { h.wrapping_add(imm as u16 as i16 as i32 as u32) } else { h | imm };
					hi[rt] = None;
					if (0x15_ef00..0x30_0000).contains(&v) {
						vals.push(v);
					}
				}
			}
			_ => {}
		}
	}
	let mut distinct = vals.clone();
	distinct.sort_unstable();
	distinct.dedup();
	let poly = *distinct.iter().find(|&&v| vals.iter().filter(|&&x| x == v).count() >= 2)?;
	let rest: Vec<u32> = distinct.into_iter().filter(|&v| v != poly).collect();
	let [state, uv, owner] = rest[..] else { return None };
	(uv == state + 0x80 && owner == uv + 0x10).then_some(ArcTables { state, uv, owner, poly })
}

/// The level's arc callback address when its code is in RAM (first words `addiu sp,-0xc0; lui a1,5`). The slot tables are the
/// live state (owner pointer and state word > 1 as FUN_002e2498 tests them), so the queue entry itself is not required: the
/// helper registers it during the field's update and the mirror may snapshot before that [I].
pub fn arc_live(m: &Machine, level: u32) -> Option<u32> {
	let f = arc_fn(level)?;
	(m.rd32(f) == 0x27bd_ff40 && m.rd32(f + 4) == 0x3c05_0005).then_some(f)
}

/// The vendor moby addresses of the vendor glow entries on queue B this field.
pub fn vendor_mobys(m: &Machine, level: u32) -> Vec<u32> {
	let Some(&f) = (*crate::retail::disc::pick(&VENDOR_FN, &VENDOR_FN_USA)).get(li(level)) else { return Vec::new() };
	entries(m, level, true).into_iter().filter(|e| e.0 == f).map(|e| e.1).collect()
}

/// The two scroll globals the vendor callback advances (+0.01 a call; beam V scroll and ring rise): the first two distinct
/// gp-relative `lwc1` offsets of the callback (Novalis gp-0x58ec / gp-0x58e8, gp = 0x166c00; offsets differ per level) [C].
pub fn vendor_scrolls(m: &Machine, level: u32) -> Option<(f32, f32)> {
	let f = *(*crate::retail::disc::pick(&VENDOR_FN, &VENDOR_FN_USA)).get(li(level))?;
	let mut offs: Vec<i32> = Vec::new();
	for k in 0..0x200 {
		let w = m.rd32(f + 4 * k);
		if w >> 26 == 0x31 && (w >> 21 & 31) == 28 {
			let o = w as i16 as i32;
			if !offs.contains(&o) {
				offs.push(o);
			}
			if offs.len() == 2 {
				break;
			}
		}
	}
	if offs.len() < 2 {
		return None;
	}
	let at = |o: i32| 0x166c00u32.wrapping_add_signed(o);
	let (a, b) = (m.rdf(at(offs[0])), m.rdf(at(offs[1])));
	(a.is_finite() && b.is_finite() && (0.0..=1.02).contains(&a) && (0.0..=1.02).contains(&b)).then_some((a, b))
}

/// The address a `lui rt` at `f + off` forms with the first following addiu / lw / lwc1 based on rt (8 insns).
fn hi_lo(m: &Machine, f: u32, off: u32) -> Option<u32> {
	let w = m.rd32(f + off);
	if w >> 26 != 0x0f {
		return None;
	}
	let rt = w >> 16 & 31;
	(1..=8).map(|k| m.rd32(f + off + 4 * k)).find(|x| matches!(x >> 26, 0x09 | 0x23 | 0x31) && (x >> 21 & 31) == rt).map(|x| ((w & 0xffff) << 16).wrapping_add(x as i16 as i32 as u32))
}

/// The gp-relative address of the lw / lwc1 / swc1 at `f + off`.
fn gp_rel(m: &Machine, level: u32, f: u32, off: u32) -> Option<u32> {
	let x = m.rd32(f + off);
	(matches!(x >> 26, 0x23 | 0x31 | 0x39) && (x >> 21 & 31) == 28).then(|| gp(level).wrapping_add(x as i16 as i32 as u32))
}

fn rgba(w: u32) -> [f32; 4] {
	[(w & 0xff) as f32 / 128.0, (w >> 8 & 0xff) as f32 / 128.0, (w >> 16 & 0xff) as f32 / 128.0, (w >> 24) as f32 / 128.0]
}

/// submit_strip4 FUN_001ef7c0 order (v0 v1 v2 v3, triangles 012 / 123) as a fan quad (0 1 3 2).
fn strip(p: [[f32; 3]; 4], uv: [[f32; 2]; 4], c: [u32; 4], tex: usize, additive: bool) -> WorldQuad {
	WorldQuad { pos: [p[0], p[1], p[3], p[2]], uv: [uv[0], uv[1], uv[3], uv[2]], colour: [rgba(c[0]), rgba(c[1]), rgba(c[3]), rgba(c[2])], tex, additive }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
	a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
/// set_length FUN_001f1a98 (zero vector stays zero).
fn set_len(v: [f32; 3], l: f32) -> [f32; 3] {
	let n = dot(v, v).sqrt();
	if n == 0.0 { [0.0; 3] } else { v.map(|x| x * l / n) }
}

/// Suck Cannon tube (FUN_002df5a0, OV:164987-165233) [C]: texture 0x15 (FUN_001ebb00), ALPHA 0x48 additive, one strip
/// per (ring i in 1..10, vertex j in 0..20) whose face (a - b) x (c - b) points away from the camera 0x166ec0
/// (gp + 0x2c0) [C: Eudora; gp form on the others I]; a = ring i vertex j, b = ring i vertex j+1, c / d = ring i-1;
/// rings at verts + 0x140 i (20 vec4 each), alphas i16 alpha[i]. Colour 0x404040 with alpha clamp(alpha, 0, 0x40):
/// ring i on a/b, ring i-1 on c/d; ring 1 forces c/d to 0x20, ring 9 forces a/b to 0. UV: u (gp-0x4fe8) += 0.001
/// once per draw, each ring u += 0.3, strip u .. u - 0.3; v (gp-0x4fe4) per ring, += scroll (gp-0x4fe0) after each
/// ring, both wrapped by -7.0 above 7.0; v steps 0.05 per vertex.
fn suck_tube(m: &Machine, level: u32, f: u32, out: &mut Vec<WorldQuad>) {
	let (Some(ua), Some(va), Some(sa), Some(alpha), Some(verts)) =
		(gp_rel(m, level, f, 0x4c), gp_rel(m, level, f, 0x100), gp_rel(m, level, f, 0x104), hi_lo(m, f, 0x124), hi_lo(m, f, 0x180))
	else {
		return;
	};
	let cam = m.rdv3(crate::retail::disc::va(gp(level) + 0x2c0));
	let scroll = m.rdf(sa);
	// [I] step the post-draw u / v back to the values the draw started from (wrap -7.0 undone below 0).
	let back = |x: f32, d: f32| if x - d < 0.0 { x - d + 7.0 } else { x - d };
	let mut u = back(m.rdf(ua), 0.001);
	let mut v = m.rdf(va);
	for _ in 1..10 {
		v = back(v, scroll);
	}
	let a = |i: usize| (m.rd16(alpha + 2 * i as u32) as i16).clamp(0, 0x40) as u32;
	let vert = |i: usize, j: usize| m.rdv3(verts + 0x140 * i as u32 + 0x10 * j as u32);
	for i in 1..10 {
		u += 0.3;
		let mut vv = v;
		v += scroll;
		if v > 7.0 {
			v -= 7.0;
		}
		for j in 0..20 {
			let (pa, pb, pc, pd) = (vert(i, j), vert(i, (j + 1) % 20), vert(i - 1, j), vert(i - 1, (j + 1) % 20));
			if dot(cross(sub(pa, pb), sub(pc, pb)), sub(pb, cam)) < 0.0 {
				let mut ca = a(i) << 24 | 0x40_4040;
				let mut cc = a(i - 1) << 24 | 0x40_4040;
				if i == 1 {
					cc = 0x2040_4040;
				} else if i == 9 {
					ca = 0x40_4040;
				}
				out.push(strip([pa, pb, pc, pd], [[u, vv], [u, vv + 0.05], [u - 0.3, vv], [u - 0.3, vv + 0.05]], [ca, ca, cc, cc], 0x15, true));
			}
			vv += 0.05;
		}
	}
}

/// Taunter rings (FUN_002a9910, OV:132201-132357) [C]: texture 8, ALPHA 0x48 additive. dir = -set_len(held +0xd0);
/// up = -gravity 0x13f5e0; r1 = set_len(up x dir, 1), r2 = dir x r1. For each of ten rings (+0 distance f32, +4 timer
/// i16, +8 active i16; stride 0xc) with active != 0: centre = origin + dir * distance, t = min(timer * inv_life, 1),
/// size = start + (end - start) * t (gp-0x57e8 / gp-0x57ec), alpha = trunc(float(gp-0x57f4 i32) * t), colour
/// gp-0x57f0 rgb; corners (0x1cf900, x 0) scaled by size, rows dir / r1 / r2 + centre (vec4_mul_mat4, w 1).
/// UV (0,0) (0,1) (1,0) (1,1).
fn taunter(m: &Machine, level: u32, f: u32, held: u32, out: &mut Vec<WorldQuad>) {
	let (Some(rings), Some(origin), Some(inv), Some(corners), Some(start), Some(end), Some(amax), Some(rgb)) = (
		hi_lo(m, f, 0xc8),
		hi_lo(m, f, 0x100),
		hi_lo(m, f, 0x168),
		hi_lo(m, f, 0x1cc),
		gp_rel(m, level, f, 0x188),
		gp_rel(m, level, f, 0x190),
		gp_rel(m, level, f, 0x194),
		gp_rel(m, level, f, 0x1b8),
	) else {
		return;
	};
	if held == 0 {
		return;
	}
	let dir = set_len(m.rdv3(held + 0xd0), -1.0);
	let up = m.rdv3(0x13f5e0).map(|x| -x);
	let r1 = set_len(cross(up, dir), 1.0);
	let r2 = cross(dir, r1);
	let o = m.rdv3(origin);
	let (s0, s1, amax, rgb, inv) = (m.rdf(start), m.rdf(end), m.rd32(amax) as i32 as f32, m.rd32(rgb) & 0xff_ffff, m.rdf(inv));
	for k in 0..10 {
		let r = rings + 0xc * k;
		if m.rd16(r + 8) == 0 {
			continue;
		}
		let d = m.rdf(r);
		let c: [f32; 3] = std::array::from_fn(|i| o[i] + dir[i] * d);
		let t = ((m.rd16(r + 4) as i16) as f32 * inv).min(1.0);
		let size = s0 + (s1 - s0) * t;
		let w = rgb | ((amax * t) as i32 as u32) << 24;
		let p: [[f32; 3]; 4] = std::array::from_fn(|q| {
			let v = m.rdv3(corners + 0x10 * q as u32).map(|x| x * size);
			std::array::from_fn(|i| v[0] * dir[i] + v[1] * r1[i] + v[2] * r2[i] + c[i])
		});
		out.push(strip(p, [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]], [w; 4], 8, true));
	}
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(v: [f32; 3], s: f32) -> [f32; 3] {
	v.map(|x| x * s)
}
fn len(v: [f32; 3]) -> f32 {
	dot(v, v).sqrt()
}
/// `1 / |v|`, 0 for a zero vector (the beam helpers' `c.eq.s` guard).
fn inv_len(v: [f32; 3]) -> f32 {
	let l = len(v);
	if l == 0.0 { 0.0 } else { 1.0 / l }
}

/// The target of the `jal` at `at`.
fn jal_to(m: &Machine, at: u32) -> Option<u32> {
	let w = m.rd32(at);
	(w >> 26 == 3).then(|| (at & 0xf000_0000) | (w & 0x03ff_ffff) << 2)
}

/// Like `hi_lo`, for any load (lb..lwu, lwc1) or addiu based on the lui's rt (8 insns).
fn hi_ld(m: &Machine, f: u32, off: u32) -> Option<u32> {
	let w = m.rd32(f + off);
	if w >> 26 != 0x0f {
		return None;
	}
	let rt = w >> 16 & 31;
	(1..=8).map(|k| m.rd32(f + off + 4 * k)).find(|x| matches!(x >> 26, 0x09 | 0x20..=0x27 | 0x31) && (x >> 21 & 31) == rt).map(|x| ((w & 0xffff) << 16).wrapping_add(x as i16 as i32 as u32))
}

/// Gravity vector (boot data, every level) [C: Eudora 0x2ad53c lui 0x14 / addiu -0xa20].
const GRAVITY: u32 = 0x13f5e0;
const UV_UNIT: [[f32; 2]; 4] = [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]];

/// A billboard drawn by vec4_mul_mat4: each corner (table vec4, xyz scaled by `size`, w kept: vec_scale FUN_001f1898)
/// times the rows r0 r1 r2 r3 (xyz) [C].
fn sprite(m: &Machine, rows: [[f32; 3]; 4], corners: u32, size: f32, uv: [[f32; 2]; 4], c: u32, tex: usize, out: &mut Vec<WorldQuad>) {
	let p: [[f32; 3]; 4] = std::array::from_fn(|q| {
		let a = corners + 0x10 * q as u32;
		let v = m.rdv3(a).map(|x| x * size);
		let w = m.rdf(a + 0xc);
		std::array::from_fn(|i| v[0] * rows[0][i] + v[1] * rows[1][i] + v[2] * rows[2][i] + w * rows[3][i])
	});
	out.push(strip(p, uv, [c; 4], tex, true));
}

/// Ribbon helper data read from its own code (Tesla FUN_002ace88 / Morph FUN_002b0a60, the same body with other gp
/// slots; the Morph copy takes pass 2's colour in t0) [C].
struct Rib {
	tex1: usize,
	col1: u32,
	tex2: usize,
	col2: u32,
	u: f32,
	w1: f32,
}

/// Beam ribbon (Eudora FUN_002ace88 / FUN_002b0a60, ALPHA 0x48 additive both passes) [C]: `n` points at `pts`
/// (vec4 stride 0x10), camera `cam` (gp + 0x2c0). cross3 FUN_001f1958(out, a, b) is b x a.
/// Pass 1 (texture tex1): strips p[i-1]..p[i] for i in 1..n-1, each end p +- side, side = ((p - cam) x (p_next - p))
/// * (w1 / |..|); UV (u,0) (u,1) (u+1,0) (u+1,1); colour col1 | a1 << 24; strip 1 fades its start pair, strip n-2 its
/// end pair, strip 2 restores the start pair (col1 as stored) [C: 0x2ad11c-0x2ad160].
/// Pass 2 (texture tex2): for i in 0..n-1 the segment p[i]..p[i+1], both ends pushed from the camera to the farther
/// one's distance, stretched by `ext` * (end - start) at each side, width w2 across (end - start) x (cam - start);
/// UV unit; colour col2 | a2 << 24, segment 0 fades its start pair, segment n-2 its end pair, segment 1 restores [C].
fn ribbon(m: &Machine, cam: [f32; 3], pts: u32, n: i32, a1: u32, a2: u32, w2: f32, ext: f32, r: &Rib, out: &mut Vec<WorldQuad>) {
	let p = |i: i32| m.rdv3(pts.wrapping_add(0x10 * i as u32));
	let full1 = r.col1 | a1 << 24;
	let full2 = r.col2 | a2 << 24;
	let side = |a: [f32; 3], b: [f32; 3]| {
		let s = cross(sub(a, cam), sub(b, a));
		scale(s, r.w1 * inv_len(s))
	};
	if n >= 2 {
		let s = side(p(0), p(1));
		let (mut v0, mut v1) = (add(p(0), s), sub(p(0), s));
		let (mut c01, mut c23) = (full1, full1);
		let uv = [[r.u, 0.0], [r.u, 1.0], [r.u + 1.0, 0.0], [r.u + 1.0, 1.0]];
		for i in 1..n - 1 {
			let s = side(p(i), p(i + 1));
			let (v2, v3) = (add(p(i), s), sub(p(i), s));
			if i == 1 {
				c01 = r.col1;
			} else if i == n - 2 {
				c23 = r.col1;
			} else if i == 2 {
				c01 = full1;
			}
			out.push(strip([v0, v1, v2, v3], uv, [c01, c01, c23, c23], r.tex1, true));
			(v0, v1) = (v2, v3);
		}
	}
	let (mut d01, mut d23) = (full2, full2);
	for i in 0..n - 1 {
		let (mut a, mut b) = (sub(p(i), cam), sub(p(i + 1), cam));
		let (la, lb) = (len(a), len(b));
		if lb < la {
			b = if lb == 0.0 { a } else { scale(b, la / lb) };
		} else {
			a = if la == 0.0 { b } else { scale(a, lb / la) };
		}
		let (mut s0, mut s1) = (add(cam, a), add(cam, b));
		let d = sub(s1, s0);
		let e = scale(d, ext);
		s0 = sub(s0, e);
		s1 = add(s1, e);
		let c = cross(d, sub(cam, s0));
		let s = scale(c, w2 * inv_len(c));
		if i == 0 {
			d01 = r.col2;
		} else if i == n - 2 {
			d23 = r.col2;
		} else if i == 1 {
			d01 = full2;
		}
		out.push(strip([add(s0, s), sub(s0, s), add(s1, s), sub(s1, s)], UV_UNIT, [d01, d01, d23, d23], r.tex2, true));
	}
}

/// Whether queue-B callback `f` is the Tesla Claw beam draw (Eudora FUN_002acd18): six ribbon calls at +0x8c.. to the
/// helper right behind it (+0x170) and the spark glow call at +0x14c to +0x740 [C: code shape, any level].
fn is_tesla(m: &Machine, f: u32) -> bool {
	[0x8c, 0xac, 0xd8, 0xfc, 0x120, 0x144].iter().all(|&o| jal_to(m, f + o) == Some(f + 0x170)) && jal_to(m, f + 0x14c) == Some(f + 0x740)
}

/// Whether queue-B callback `f` is the Morph-o-Ray beam draw (Eudora FUN_002b0638): ribbon helper +0x428 at +0x9c /
/// +0xc4, sprite helpers +0xa00 at +0xcc and +0xc40 at +0xd4 [C: code shape, any level].
fn is_morph(m: &Machine, f: u32) -> bool {
	jal_to(m, f + 0x9c) == Some(f + 0x428) && jal_to(m, f + 0xc4) == Some(f + 0x428) && jal_to(m, f + 0xcc) == Some(f + 0xa00) && jal_to(m, f + 0xd4) == Some(f + 0xc40)
}

/// Tesla Claw beam (Eudora FUN_002acd18, Ghidra C has no body; disassembly scratch_wfx/dis) [C], arg = the moby, pvar =
/// moby +0x78. The u scroll (gp-0x56c4) is advanced before the ribbons read it, so the snapshot holds the draw-time value.
/// Ribbons: (1) pvar +0x3e i16 points at 0x1cfac0, alphas gp-0x56dc / gp-0x56d4, w2 gp-0x56b0, ext gp-0x5680;
/// (2) the same count at +0x140, gp-0x56ac / gp-0x56a8, w2 gp-0x56a0, ext gp-0x567c; (3..6) four 5-point arcs at
/// 0x1cff20 + 0x50 k, both alphas i16 0x1615b0[k], w2 gp-0x56a0, ext 0.2. Then FUN_002ad458: two camera-facing glows
/// (texture 0xb) at pvar pos + 0.3 toward the camera: rows r0 = unit(cam - centre), r1 = -unit(gravity x r0),
/// r2 = r0 x r1, r3 = centre; corners 0x1cfa60, UV table 0x1cfaa0 (u, v pairs); colour gp-0x5670 size gp-0x5668,
/// then colour gp-0x56b8 size gp-0x5664. Each size gets rand_float_signed(0, 0.025 / 0.05) in retail; the mirror
/// uses the mean (0) [I: the draw's RNG state is not in the snapshot].
fn tesla(m: &Machine, level: u32, f: u32, arg: u32, out: &mut Vec<WorldQuad>) {
	let h = f + 0x170;
	let g = f + 0x740;
	let rd_gp = |base: u32, off: u32| gp_rel(m, level, base, off);
	let (Some(pts), Some(a1), Some(a2), Some(w2a), Some(exta), Some(b1), Some(b2), Some(w2b), Some(extb), Some(arc_a), Some(arcs), Some(w2c)) = (
		hi_lo(m, f, 0x60),
		rd_gp(f, 0x6c),
		rd_gp(f, 0x70),
		rd_gp(f, 0x7c),
		rd_gp(f, 0x90),
		rd_gp(f, 0x9c),
		rd_gp(f, 0xa0),
		rd_gp(f, 0xa8),
		rd_gp(f, 0xb0),
		hi_ld(m, f, 0xb4),
		hi_lo(m, f, 0xbc),
		rd_gp(f, 0xc8),
	) else {
		return;
	};
	let (Some(tex1), Some(col1), Some(tex2), Some(col2), Some(u), Some(w1)) = (rd_gp(h, 0x28), rd_gp(h, 0x68), rd_gp(h, 0x8c), rd_gp(h, 0xc0), rd_gp(h, 0xc8), rd_gp(h, 0x1a0)) else {
		return;
	};
	let (Some(gc1), Some(gs1), Some(corners), Some(uvt), Some(gc2), Some(gs2)) =
		(rd_gp(g, 0x11c), rd_gp(g, 0x144), hi_lo(m, g, 0x150), hi_lo(m, g, 0x158), rd_gp(g, 0x1d4), rd_gp(g, 0x200))
	else {
		return;
	};
	if arg == 0 {
		return;
	}
	let pvar = m.rd32(arg + 0x78) & 0x01ff_ffff;
	if pvar == 0 {
		return;
	}
	let cam = m.rdv3(crate::retail::disc::va(gp(level) + 0x2c0));
	let r = Rib { tex1: m.rd32(tex1) as usize, col1: m.rd32(col1), tex2: m.rd32(tex2) as usize, col2: m.rd32(col2), u: m.rdf(u), w1: m.rdf(w1) };
	let n = m.rd16(pvar + 0x3e) as i16 as i32;
	ribbon(m, cam, pts, n, m.rd32(a1), m.rd32(a2), m.rdf(w2a), m.rdf(exta), &r, out);
	ribbon(m, cam, pts + 0x140, n, m.rd32(b1), m.rd32(b2), m.rdf(w2b), m.rdf(extb), &r, out);
	for k in 0..4 {
		let a = m.rd16(arc_a + 2 * k) as i16 as i32 as u32;
		ribbon(m, cam, arcs + 0x50 * k, 5, a, a, m.rdf(w2c), 0.2, &r, out);
	}
	// FUN_002ad458
	let pos = m.rdv3(pvar);
	let centre = add(set_len(sub(cam, pos), 0.3), pos);
	let r0 = set_len(sub(cam, centre), 1.0);
	let r1 = set_len(cross(m.rdv3(GRAVITY), r0), -1.0);
	let r2 = cross(r0, r1);
	let uv: [[f32; 2]; 4] = std::array::from_fn(|q| [m.rdf(uvt + 8 * q as u32), m.rdf(uvt + 8 * q as u32 + 4)]);
	sprite(m, [r0, r1, r2, centre], corners, m.rdf(gs1), uv, m.rd32(gc1), 0xb, out);
	sprite(m, [r0, r1, r2, centre], corners, m.rdf(gs2), uv, m.rd32(gc2), 0xb, out);
}

/// Morph-o-Ray beam (Eudora FUN_002b0638, no Ghidra body; disassembly scratch_wfx/dis/morph_*.s) [C], arg = the moby.
/// Scrolls (gp-0x5524, -0x5598, -0x5590, -0x558c) are advanced before use, so the snapshot holds the draw-time values.
/// Order: two 12-point ribbons (FUN_002b0a60) at 0x1d0520 / 0x1d05e0 (alphas gp-0x5544 / gp-0x5540, w2 gp-0x5538,
/// ext gp-0x5534, pass-2 colour gp-0x552c / gp-0x5528); FUN_002b1038: three end sprites (texture 8, flag
/// 0x161768[k] != 0, centre 0x1d15a0[k], r0 = 0x1d15d0[k], r1 = unit(-gravity x r0), r2 = r0 x r1, size 0x161758[k],
/// colour (gold ? gp-0x554c : gp-0x5550) & 0xffffff | trunc(0x161738[k]) << 24); FUN_002b1278: two glows (texture 0xb)
/// at 0x1d0460 + set_len(moby +0xd0, gp-0x54fc), camera-facing (as the Tesla glow), colour gp-0x54f8 size gp-0x5504
/// then colour 0xffffffff size gp-0x5500; then the ring tube: rings 2..=11 of 0x1d06a0 (20 vec4 each, stride 0x140)
/// against ring i-1, texture gp-0x5514, colour gold ? gp-0x5518 : gp-0x551c (gold = byte 0x13e535), drawn when
/// ((v0 - v1) x (v2 - v1)) . (v1 - cam) < 0; ring 11 sets its a/b pair to gp-0x551c & 0xffffff. Each quad is drawn
/// twice with v running from gp-0x5590 and gp-0x558c (+ gp-0x559c per vertex, also over hidden quads); u = gp-0x5598
/// for the ring, u - gp-0x55a0 for ring i-1, u += gp-0x55a0 per ring.
fn morph(m: &Machine, level: u32, f: u32, arg: u32, out: &mut Vec<WorldQuad>) {
	let (h, s3, s2) = (f + 0x428, f + 0xa00, f + 0xc40);
	let rd_gp = |base: u32, off: u32| gp_rel(m, level, base, off);
	let (Some(a1), Some(pts1), Some(a2), Some(w2), Some(ext), Some(c2a), Some(pts2), Some(c2b), Some(gold), Some(col_g), Some(col_n)) = (
		rd_gp(f, 0x7c),
		hi_lo(m, f, 0x80),
		rd_gp(f, 0x84),
		rd_gp(f, 0x94),
		rd_gp(f, 0x98),
		rd_gp(f, 0xa0),
		hi_lo(m, f, 0xa8),
		rd_gp(f, 0xc8),
		hi_ld(m, f, 0xdc),
		rd_gp(f, 0xe8),
		rd_gp(f, 0xec),
	) else {
		return;
	};
	let (Some(ttex), Some(u0), Some(v1a), Some(v2a), Some(rings), Some(fade), Some(vstep), Some(ustep)) =
		(rd_gp(f, 0xf4), rd_gp(f, 0x11c), rd_gp(f, 0x20c), rd_gp(f, 0x210), hi_lo(m, f, 0x220), rd_gp(f, 0x32c), rd_gp(f, 0x310), rd_gp(f, 0x348))
	else {
		return;
	};
	let (Some(tex1), Some(col1), Some(tex2), Some(u), Some(w1)) = (rd_gp(h, 0x28), rd_gp(h, 0x70), rd_gp(h, 0x94), rd_gp(h, 0xc4), rd_gp(h, 0x1a4)) else {
		return;
	};
	let (Some(sg), Some(sgc), Some(snc), Some(sflag), Some(salpha), Some(spos), Some(sdir), Some(ssize), Some(scorn)) = (
		hi_ld(m, s3, 0x34),
		rd_gp(s3, 0x94),
		rd_gp(s3, 0x98),
		hi_lo(m, s3, 0xcc),
		hi_lo(m, s3, 0xe8),
		hi_lo(m, s3, 0x10c),
		hi_lo(m, s3, 0x134),
		hi_lo(m, s3, 0x14c),
		hi_lo(m, s3, 0x1a8),
	) else {
		return;
	};
	let (Some(gcol), Some(glen), Some(gorig), Some(gcorn), Some(gs1), Some(gs2)) =
		(rd_gp(s2, 0x3c), rd_gp(s2, 0x64), hi_lo(m, s2, 0xb0), hi_lo(m, s2, 0x130), rd_gp(s2, 0x140), rd_gp(s2, 0x1b0))
	else {
		return;
	};
	let cam = m.rdv3(crate::retail::disc::va(gp(level) + 0x2c0));
	let mut r = Rib { tex1: m.rd32(tex1) as usize, col1: m.rd32(col1), tex2: m.rd32(tex2) as usize, col2: m.rd32(c2a), u: m.rdf(u), w1: m.rdf(w1) };
	let (a1, a2, w2, ext) = (m.rd32(a1), m.rd32(a2), m.rdf(w2), m.rdf(ext));
	ribbon(m, cam, pts1, 12, a1, a2, w2, ext, &r, out);
	r.col2 = m.rd32(c2b);
	ribbon(m, cam, pts2, 12, a1, a2, w2, ext, &r, out);
	// FUN_002b1038: end sprites.
	let grav = m.rdv3(GRAVITY);
	let base = m.rd32(if m.rd8(sg) != 0 { sgc } else { snc }) & 0xff_ffff;
	for k in 0..3u32 {
		if m.rd32(sflag + 4 * k) == 0 {
			continue;
		}
		let a = m.rdf(salpha + 4 * k) as i32 as u32;
		let r0 = m.rdv3(sdir + 0x10 * k);
		let r1 = set_len(cross(scale(grav, -1.0), r0), 1.0);
		let r2 = cross(r0, r1);
		sprite(m, [r0, r1, r2, m.rdv3(spos + 0x10 * k)], scorn, m.rdf(ssize + 4 * k), UV_UNIT, base | a << 24, 8, out);
	}
	// FUN_002b1278: muzzle glows.
	if arg != 0 {
		let centre = add(m.rdv3(gorig), set_len(m.rdv3(arg + 0xd0), m.rdf(glen)));
		let r0 = set_len(sub(cam, centre), 1.0);
		let r1 = set_len(cross(grav, r0), -1.0);
		let r2 = cross(r0, r1);
		sprite(m, [r0, r1, r2, centre], gcorn, m.rdf(gs1), UV_UNIT, m.rd32(gcol), 0xb, out);
		sprite(m, [r0, r1, r2, centre], gcorn, m.rdf(gs2), UV_UNIT, 0xffff_ffff, 0xb, out);
	}
	// Ring tube.
	let col = m.rd32(if m.rd8(gold) != 0 { col_g } else { col_n });
	let (tex, vstep, ustep) = (m.rd32(ttex) as usize, m.rdf(vstep), m.rdf(ustep));
	let mut uu = m.rdf(u0);
	let mut cab = col;
	let vert = |i: u32, j: u32| m.rdv3(rings + 0x140 * i + 0x10 * j);
	for i in 2..12u32 {
		let (mut va, mut vb) = (m.rdf(v1a), m.rdf(v2a));
		for j in 0..20u32 {
			let (p0, p1, p2, p3) = (vert(i, j), vert(i, (j + 1) % 20), vert(i - 1, j), vert(i - 1, (j + 1) % 20));
			if dot(cross(sub(p0, p1), sub(p2, p1)), sub(p1, cam)) >= 0.0 {
				va += vstep;
				vb += vstep;
				continue;
			}
			if i == 11 {
				cab = m.rd32(fade) & 0xff_ffff;
			}
			let u1 = uu - ustep;
			for v in [&mut va, &mut vb] {
				let v0 = *v;
				*v += vstep;
				out.push(strip([p0, p1, p2, p3], [[uu, v0], [uu, *v], [u1, v0], [u1, *v]], [cab, cab, col, col], tex, true));
			}
		}
		uu += ustep;
	}
}

/// The Swingshot rope as retail's draw callback reads it from RAM (`swing_rope`); the quads need the held Swingshot's
/// attach point 0 (`rope_quads`), which the viewer poses.
#[derive(Clone, Copy, Debug, Default)]
pub struct SwingRope {
	/// The hook moby's position (hook +0x10).
	pub hook: [f32; 3],
	/// The camera position the ribbon faces (Eudora 0x166ec0, Oltanis 0x1674c0, Aridia 0x167340: the lui/addiu of the code).
	pub cam: [f32; 3],
	/// Held pvars +0x18 ripple phase, +0x1c wavelength, +0x20 amplitude.
	pub phase: f32,
	pub wave: f32,
	pub amp: f32,
}

/// Whether queue-A callback `f` is the Swingshot rope draw (Eudora FUN_002b87b0 = Aridia FUN_002c5108 swing_rope_draw,
/// binary twins on all 19 levels, twins.py; queued by the held Swingshot updater, Aridia FUN_002c5498 OV:132943): fx
/// texture 0xf at +0x68, TEX1 0xff90_0000_0260 at +0x74 / +0x7c, ALPHA 0xff_0000_0044 at +0x98, segment 0.2 at +0x1e8,
/// submit_strip4 call at +0x310 [C: the same words in Eudora fg1/4_0159 and Oltanis RAM].
fn is_swing_rope(m: &Machine, f: u32) -> bool {
	[(0x68, 0x2404_000f), (0x74, 0x3405_ff90), (0x7c, 0x34a5_0260), (0x98, 0x34e7_0044), (0x1e8, 0x3c01_3e4c)].iter().all(|&(o, w)| m.rd32(f + o) == w)
		&& jal_to(m, f + 0x310).is_some()
}

/// The Swingshot rope draw's inputs (FUN_002b87b0, Aridia OV:132542-132759) [C]: arg = the held Swingshot moby, pvar
/// p = arg +0x78 (nothing when either is null), hook = (p +0x14) +0x10, ripple phase / wavelength / amplitude = p +0x18 /
/// +0x1c / +0x20, camera = lui at +0x1dc with addiu at +0x284.
fn swing_rope(m: &Machine, f: u32, arg: u32) -> Option<SwingRope> {
	let cam = lui_lo(m, f, 0x1dc, 0x284)?;
	let p = if arg == 0 { 0 } else { m.rd32(arg + 0x78) & 0x01ff_ffff };
	if p == 0 {
		return None;
	}
	let hook = m.rd32(p + 0x14) & 0x01ff_ffff;
	Some(SwingRope { hook: m.rdv3(hook + 0x10), cam: m.rdv3(cam), phase: m.rdf(p + 0x18), wave: m.rdf(p + 0x1c), amp: m.rdf(p + 0x20) })
}

/// FUN_00210d10 / Eudora angle_wrap FUN_001f2710: fold into [-pi, pi) by whole turns.
fn wrap_pi(mut a: f32) -> f32 {
	const P: f32 = 3.1415927;
	while P <= a {
		a = a - P - P;
	}
	while a < -P {
		a = a + P + P;
	}
	a
}

/// The Swingshot rope ribbon (FUN_002b87b0 loop, Aridia OV:132661-132754) [C] from the hook k to `jt`, the held moby's
/// attach point 0 in the world (FUN_002519d0(held, 0)): v = jt - k, L = |v| (nothing when L <= 0); side = v x (v - 0.1 z);
/// c = cos(wrap(phase)); edge pair (k - 0.05 z, k + 0.05 z); then per step seg = min(0.2, L - t), t += seg, v = set_len(v,
/// seg), p += v, e = set_len((cam - p) x v, 0.05), off = set_len(side, sin(wrap(t pi / wave)) * sin(t pi / L) * c * amp),
/// new pair (p + e + off, p - e + off), one strip (prev a, new a, prev b, new b) with UV (0,0) (1,0) (0,1) (1,1), colour
/// 0x80808080, fx texture 0xf with CLAMP_1 5, TEX1 0xff90_0000_0260, ALPHA 0x44 (blend); until t >= L.
pub fn rope_quads(r: &SwingRope, jt: [f32; 3]) -> Vec<WorldQuad> {
	let mut out = Vec::new();
	let k = r.hook;
	let mut v = sub(jt, k);
	let l = len(v);
	if !(0.0 < l) {
		return out;
	}
	let side = cross(v, [v[0], v[1], v[2] - 0.1]);
	let c = crate::camera::fast_cos(wrap_pi(r.phase));
	let mut p = k;
	let (mut a, mut b) = ([k[0], k[1], k[2] - 0.05], [k[0], k[1], k[2] + 0.05]);
	let mut t = 0.0f32;
	let pi = 3.1415927f32;
	let tex = 0x0f | crate::particles::FX_CLAMP;
	loop {
		let seg = if l - t < 0.2 { l - t } else { 0.2 };
		t += seg;
		let s1 = crate::camera::fast_sin(t * pi / l);
		let s2 = crate::camera::fast_sin(wrap_pi(t * pi / r.wave));
		let amt = s2 * s1 * c * r.amp;
		v = set_len(v, seg);
		p = add(p, v);
		let e = set_len(cross(sub(r.cam, p), v), 0.05);
		let off = set_len(side, amt);
		let a2 = add(add(p, e), off);
		let b2 = add(sub(p, e), off);
		out.push(strip([a, a2, b, b2], [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]], [0x8080_8080; 4], tex, false));
		(a, b) = (a2, b2);
		if l <= t {
			break;
		}
	}
	out
}

/// Hook (retail.rs `queue_fx`): the ported queue-A callbacks into `qa` and the queue-B callbacks into `qb`, each in its
/// queue order. The Swingshot rope goes to `rope` with the `qa` length at its queue slot (the viewer adds the posed
/// attach point and draws it there).
pub fn fill(m: &Machine, level: u32, qa: &mut Vec<WorldQuad>, qb: &mut Vec<WorldQuad>, rope: &mut Option<(usize, SwingRope)>) {
	crate::viewer::gs_state::guard_path("queue");
	let Some(i) = Some(li(level)).filter(|&i| i < 19) else { return };
	for (f, arg) in entries(m, level, false) {
		if is_swing_rope(m, f) {
			if let Some(r) = swing_rope(m, f, arg) {
				*rope = Some((qa.len(), r));
			}
		} else if f == (*crate::retail::disc::pick(&TAUNTER, &TAUNTER_USA))[i] {
			taunter(m, level, f, arg, qa);
		} else if f == (*crate::retail::disc::pick(&SUCK_TUBE, &SUCK_TUBE_USA))[i] {
			suck_tube(m, level, f, qa);
		} else if is_barrier(m, level, f) {
			barrier(m, arg, qa);
		} else if is_barrier2(m, level, f) {
			barrier2(m, arg, qa);
		} else {
			super::queue_q2::fill_a(m, level, f, arg, qa);
			super::queue_fxb::fill_a(m, level, f, arg, qa);
		}
	}
	for (f, arg) in entries(m, level, true) {
		if f == (*crate::retail::disc::pick(&SUCK_TUBE, &SUCK_TUBE_USA))[i] {
			suck_tube(m, level, f, qb);
		} else if is_tesla(m, f) {
			tesla(m, level, f, arg, qb);
		} else if is_morph(m, f) {
			morph(m, level, f, arg, qb);
		} else if is_walloper(m, f) {
			walloper(m, level, f, arg, qb);
		} else if is_hero_fx(m, f) {
			hero_fx(m, level, f, arg, qb);
		} else {
			super::queue_fxa::fill_b(m, level, f, arg, qb);
		}
	}
}

/// The address a `lui` at `f + hi` forms with the addiu / load at `f + lo` based on its rt.
fn lui_lo(m: &Machine, f: u32, hi: u32, lo: u32) -> Option<u32> {
	let (w, x) = (m.rd32(f + hi), m.rd32(f + lo));
	(w >> 26 == 0x0f && matches!(x >> 26, 0x09 | 0x23 | 0x31) && (x >> 21 & 31) == (w >> 16 & 31)).then(|| ((w & 0xffff) << 16).wrapping_add(x as i16 as i32 as u32))
}

/// Whether queue-B callback `f` is the Walloper draw (Eudora FUN_002adea8, registered by the held-180 updater
/// FUN_002ad900): ribbon helper +0xc8 called at +0x84, flash helper +0x6a0 at +0xa0 [C: code shape, any level].
fn is_walloper(m: &Machine, f: u32) -> bool {
	jal_to(m, f + 0x84) == Some(f + 0xc8) && jal_to(m, f + 0xa0) == Some(f + 0x6a0)
}

/// The newlib rand() state word (boot FUN_001160d8: `*(*(0x12f76c) + 0x58)`, s = s * 0x41c64e6d + 0x3039) [C].
fn rand_state(m: &Machine) -> u32 {
	m.rd32((m.rd32(0x12f76c) & 0x01ff_ffff) + 0x58)
}

/// Walloper (Eudora FUN_002adea8, disassembly scratch_wfx/dis/wall_*.s) [C], arg = the held moby. Ten channels k: when
/// the i16 count 0x1d03c0[k] != 0, a 5-point ribbon at 0x1d00a0 + 0x50 k through helper FUN_002adf70, which is the
/// Tesla ribbon body (`ribbon`) with u = 0, w1 = gp-0x5620 (f12), w2 = gp-0x561c (f13), ext 0.2, textures gp-0x5600 /
/// gp-0x55fc, colours gp-0x5630 / gp-0x562c, both alphas cvt.w.s(float(i16 0x1d03d8[k]) * intensity 0x161624).
/// Then FUN_002ae548, one camera-facing flash (texture gp-0x55e8, ALPHA 0x48): c = moby +0x10 + moby +0xd0 *
/// gp-0x55f4, r0 = unit(cam - c), r1 = -unit(gravity x r0), r2 = r0 x r1, centre c + 0.2 r0, corners 0x1d0060 x
/// gp-0x55f8, colour (rand_int(4) != 0 ? gp-0x55f0 : gp-0x55ec) with alpha cvt.w.s(float(colour >> 24 signed) *
/// intensity). rand_int(4) = ((rand() >> 16) & 0x7fff) % 4 (FUN_0024a418) [C]; the mirror takes the value of the
/// state word after the field, i.e. assumes this call is the field's last rand() [I].
fn walloper(m: &Machine, level: u32, f: u32, arg: u32, out: &mut Vec<WorldQuad>) {
	let (h, g) = (f + 0xc8, f + 0x6a0);
	let rd_gp = |base: u32, off: u32| gp_rel(m, level, base, off);
	let (Some(pts), Some(alphas), Some(counts), Some(inten), Some(w1), Some(w2)) = (hi_lo(m, f, 0x14), hi_lo(m, f, 0x0c), lui_lo(m, f, 0x04, 0x34), hi_ld(m, f, 0x54), rd_gp(f, 0x68), rd_gp(f, 0x6c)) else {
		return;
	};
	let (Some(tex1), Some(col1), Some(tex2), Some(col2)) = (rd_gp(h, 0x30), rd_gp(h, 0x70), rd_gp(h, 0x94), rd_gp(h, 0xc4)) else {
		return;
	};
	let (Some(push), Some(ftex), Some(ca), Some(cb), Some(inten2), Some(corners), Some(size)) =
		(rd_gp(g, 0x04), rd_gp(g, 0x50), rd_gp(g, 0xb0), rd_gp(g, 0xb4), hi_ld(m, g, 0xc4), hi_lo(m, g, 0x190), rd_gp(g, 0x1a0))
	else {
		return;
	};
	let cam = m.rdv3(crate::retail::disc::va(gp(level) + 0x2c0));
	let k = m.rdf(inten);
	let r = Rib { tex1: m.rd32(tex1) as usize, col1: m.rd32(col1), tex2: m.rd32(tex2) as usize, col2: m.rd32(col2), u: 0.0, w1: m.rdf(w1) };
	let ext = f32::from_bits(0x3e4c_cccd);
	for i in 0..10 {
		if m.rd16(counts + 2 * i) == 0 {
			continue;
		}
		let a = ((m.rd16(alphas + 2 * i) as i16 as f32) * k) as i32 as u32;
		ribbon(m, cam, pts + 0x50 * i, 5, a, a, m.rdf(w2), ext, &r, out);
	}
	// FUN_002ae548
	if arg == 0 {
		return;
	}
	let c0 = add(m.rdv3(arg + 0x10), scale(m.rdv3(arg + 0xd0), m.rdf(push)));
	let pick = ((rand_state(m) & 0x7fff_ffff) >> 16 & 0x7fff) % 4;
	let col = m.rd32(if pick != 0 { ca } else { cb });
	let alpha = (((col as i32 >> 24) as f32) * m.rdf(inten2)) as i32 as u32;
	let col = col & 0x00ff_ffff | alpha << 24;
	let r0 = set_len(sub(cam, c0), 1.0);
	let r1 = set_len(cross(m.rdv3(GRAVITY), r0), -1.0);
	let r2 = cross(r0, r1);
	let centre = add(c0, scale(r0, ext));
	sprite(m, [r0, r1, r2, centre], corners, m.rdf(size), UV_UNIT, col, m.rd32(ftex) as usize, out);
}

/// Whether queue-B callback `f` is the hero swing trail (Eudora FUN_002992b0, Novalis 0x2bcda8, Kerwan 0x297570):
/// the texture immediates 0x14 (+0xb8) / 0xc (+0x164) and the GS context words at +0x29c..+0x2c0 [C: the same words in
/// the three overlays checked].
fn is_hero_fx(m: &Machine, f: u32) -> bool {
	[(0xb8, 0x2406_0014), (0x164, 0x2406_000c), (0x29c, 0x3403_8000), (0x2b4, 0x3404_ff90), (0x2bc, 0x3484_0260), (0x2c0, 0x2407_0005)].iter().all(|&(o, w)| m.rd32(f + o) == w)
		&& jal_to(m, f + 0x290).is_some()
		&& jal_to(m, f + 0x498).is_some()
}

/// Hero swing trail (Eudora FUN_002992b0, Ghidra overlay.elf.c 122690-122954, disassembly scratch_wfx/dis/hero_2992b0.s)
/// [C], arg = the moby, pvar p = moby +0x78. Nothing while t = p +0x70 is 0.0. The draw itself picks the record from the
/// hero animation (hero moby +0x52, 0x17 / 0x18 / 0x19 -> 0 / 1 / 2, else the stored one) and stores it as i16 p +0x78
/// (0x2b for record 3), copies the hero matrix rows +0xc0.. to p +0..+0x2c when t == 1.0, adds max(1, frame delta) *
/// rec +0x48 to t, writes the hero position to p +0x30 and draws while t < 2 * count. All of these are written before the
/// draw reads them, so the snapshot holds the draw-time values; the mirror reads p +0x78, t and the rows [C].
/// Record rec = 0x1cd7e8 + 0x60 k: i16 special quads +0..+6 with colours +0x08 / +0x18 / +0x28 / +0x38 (4 words each),
/// rate +0x48, vec4 vertices +0x4c, (i16 vertex, i16 uv) index quads +0x50, uv pairs +0x54, i16 count +0x58, v speed
/// +0x5c. Per quad q, corner j: colour = the special colour j of the first special index equal to q, else 0x50808080;
/// alpha byte = trunc(float(i8 alpha) * a) (FUN_001f27e8); colour = alpha << 24 | rgb; vertex = table vec4 xyz * size
/// (w kept), world = rows 0..2 * xyz + row 3 * w (FUN_001ef7c0 with matrix p); u = (u - 0.5) * gp-0x58f0 + 0.5,
/// v = (v - 0.5) * gp-0x58f0 + 0.5 + (1 - t * speed). Records 0..2: texture 0x14, size 1.0, a 1.0, rgb 0x808080,
/// ALPHA 0x44 (blend); record 3: texture 0xc, size 1.25, a 0.7, rgb 0x408080, ALPHA 0x48 (additive) [C].
fn hero_fx(m: &Machine, level: u32, f: u32, arg: u32, out: &mut Vec<WorldQuad>) {
	let (Some(tab), Some(uvk)) = (hi_lo(m, f, 0x184), gp_rel(m, level, f, 0x420)) else { return };
	let p = if arg == 0 { 0 } else { m.rd32(arg + 0x78) & 0x01ff_ffff };
	if p == 0 {
		return;
	}
	let t = m.rdf(p + 0x70);
	if t == 0.0 {
		return;
	}
	let (k, size, amul, tex, rgb, additive) = match m.rd16(p + 0x78) {
		0x17 => (0u32, 1.0f32, 1.0f32, 0x14usize, 0x0080_8080u32, false),
		0x18 => (1, 1.0, 1.0, 0x14, 0x0080_8080, false),
		0x19 => (2, 1.0, 1.0, 0x14, 0x0080_8080, false),
		_ => (3, 1.25, f32::from_bits(0x3f33_3333), 0x0c, 0x0040_8080, true),
	};
	// The context's first word (sp+0x70, sent as CLAMP_1 by FUN_001ef7c0's REGLIST tag 8,6,0,...) is 5 here
	// (0x299570): the scrolled v clamps to the texture edge instead of repeating [C].
	let tex = tex | crate::particles::FX_CLAMP;
	let rec = tab + 0x60 * k;
	let count = m.rd16(rec + 0x58) as i16 as i32;
	if t >= (count << 1) as f32 {
		return;
	}
	let (verts, idx, uvs) = (m.rd32(rec + 0x4c) & 0x01ff_ffff, m.rd32(rec + 0x50) & 0x01ff_ffff, m.rd32(rec + 0x54) & 0x01ff_ffff);
	let (uvk, speed) = (m.rdf(uvk), m.rdf(rec + 0x5c));
	let vs = 1.0 - t * speed;
	let rows: [[f32; 3]; 4] = std::array::from_fn(|r| m.rdv3(p + 0x10 * r as u32));
	let special: [i32; 4] = std::array::from_fn(|s| m.rd16(rec + 2 * s as u32) as i16 as i32);
	for q in 0..count {
		let mut pos = [[0.0f32; 3]; 4];
		let mut uv = [[0.0f32; 2]; 4];
		let mut col = [0u32; 4];
		for j in 0..4u32 {
			let e = idx + 0x10 * q as u32 + 4 * j;
			let c = match special.iter().position(|&s| s == q) {
				Some(s) => m.rd32(rec + 8 + 0x10 * s as u32 + 4 * j),
				None => 0x5080_8080,
			};
			let a = (((c >> 24) as u8 as i8 as i32 as f32) * amul) as i32 as u32;
			col[j as usize] = a << 24 | rgb;
			let va = verts.wrapping_add(0x10 * (m.rd16(e) as i16 as i32 as u32));
			let v = m.rdv3(va).map(|x| x * size);
			let w = m.rdf(va + 0xc);
			pos[j as usize] = std::array::from_fn(|i| v[0] * rows[0][i] + v[1] * rows[1][i] + v[2] * rows[2][i] + w * rows[3][i]);
			let ua = uvs.wrapping_add(8 * (m.rd16(e + 2) as i16 as i32 as u32));
			uv[j as usize] = [(m.rdf(ua) - 0.5) * uvk + 0.5, (m.rdf(ua + 4) - 0.5) * uvk + 0.5 + vs];
		}
		out.push(strip(pos, uv, col, tex, additive));
	}
}

/// Whether queue-B callback `f` is the RC range static (Eudora FUN_002db880, Novalis 0x301d68 rc_static_overlay_draw,
/// Aridia 0x2e4168; registered by the range limiter's updater, Eudora FUN_002dba90): screen dims lui/addiu 0x13e500 at
/// +0x10 / +0x18, texture 0x1e at +0x60, colour 0xff7f7f at +0xb0 / +0xb4, scale(0x5a) at +0x128 [C: the same words in
/// the three overlays checked].
fn is_rc_static(m: &Machine, f: u32) -> bool {
	[(0x10, 0x3c11_0014), (0x18, 0x2630_e500), (0x60, 0x2404_001e), (0xb0, 0x3c16_00ff), (0xb4, 0x36d6_7f7f), (0x128, 0x2404_005a)].iter().all(|&(o, w)| m.rd32(f + o) == w)
}

/// The RC range static of one field (FUN_002db880): tiles of 32 x 32 frame pixels over the frame `w` x `h`
/// (0x13e500 / 0x13e504), `cols` = w / 32 + 1 columns (outer loop) by `rows` = h / 32 + 1 (inner loop), tile (c, r) at
/// (32 c - 16, 32 r - 16), texel rect (u, v, u + 32, v + 32) of fx texture 0x1e, colour 0xff7f7f with alpha
/// trunc(float(pvar +0x140) * 127.5 / float(scale(90))) (scale(90) = 75 PAL), each u then v from rand_int(32) [C].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RcStatic {
	pub w: i32,
	pub h: i32,
	pub cols: i32,
	pub rows: i32,
	pub alpha: u32,
	/// (u, v) per tile, column-major (retail's draw order).
	pub tiles: Vec<[u8; 2]>,
}

/// scale_ticks(90) on PAL (FUN_001f14b8: (int)(90 * 0.8333333 + 0.5)) [C: weapon_fx.md, gp 0x162118].
const SCALE_90: i32 = 75;

/// The live RC static callback of queue B, if any. rand_int(n) = ((rand() >> 16) & 0x7fff) % n (FUN_0024a418) with
/// newlib rand s = s * 0x41c64e6d + 0x3039 [C]. The snapshot holds the state after the field, so the 2 * tiles values
/// are rebuilt by stepping the LCG back from it: the static is the field's last rand() user [C: weapon_fx.md stage 7,
/// state e06_13rx, all 63 fields the GS drew had the rebuilt alpha and tile u / v of the gsfull sprite stream].
pub fn rc_static(m: &Machine, level: u32) -> Option<RcStatic> {
	let (f, arg) = entries(m, level, true).into_iter().find(|&(f, _)| is_rc_static(m, f))?;
	let pv = if arg == 0 { 0 } else { m.rd32(arg + 0x78) & 0x01ff_ffff };
	if pv == 0 {
		return None;
	}
	let (w, h) = (m.rd32(0x13e500) as i32, m.rd32(0x13e504) as i32);
	let (cols, rows) = ((if w < 0 { w + 0x1f } else { w } >> 5) + 1, (if h < 0 { h + 0x1f } else { h } >> 5) + 1);
	let c = m.rd32(pv + 0x140) as i32;
	let alpha = ((c as f32 * 127.5) / SCALE_90 as f32) as i32 as u32 & 0xff;
	let n = (cols.max(0) * rows.max(0)) as usize;
	Some(RcStatic { w, h, cols, rows, alpha, tiles: rand_back(rand_state(m), 2 * n).chunks(2).map(|p| [(p[0] % 32) as u8, (p[1] % 32) as u8]).collect() })
}

/// The last `n` rand() results (in call order) that ended at state `s`: rand() steps s then returns (s >> 16) & 0x7fff.
fn rand_back(mut s: u32, n: usize) -> Vec<u32> {
	// Inverse of 0x41c64e6d mod 2^32 (Newton: x = x * (2 - a x), 5 rounds from x = a).
	let a = 0x41c6_4e6du32;
	let mut inv = a;
	for _ in 0..5 {
		inv = inv.wrapping_mul(2u32.wrapping_sub(a.wrapping_mul(inv)));
	}
	let mut out = vec![0u32; n];
	for k in (0..n).rev() {
		out[k] = s >> 16 & 0x7fff;
		s = s.wrapping_sub(0x3039).wrapping_mul(inv);
	}
	out
}

/// The RC static's widescreen extension [PC]: the GS overlay capture (texture slot `ov`) already holds retail's static over
/// the 4:3 centre (FUN_001ecaf8 draws it as GIF sprites) [C: weapon_fx.md stage 7], so the overlay is drawn over the centre
/// only, uv 0..1, and each side bar gets copies of it translated by whole centre widths (the static is noise, so the
/// copies read as more of it; the scene shows through them as through the centre). Nothing when the window is not wider
/// than 4:3.
pub fn rc_bar_quads(ov: usize, sw: f32, sh: f32, out: &mut Vec<crate::overlay::Quad>) {
	let cw = sh * 4.0 / 3.0;
	let x0 = (sw - cw) * 0.5;
	if x0 <= 0.0 || cw <= 0.0 {
		return;
	}
	let mut quad = |a: f32, b: f32, shift: f32| {
		let (a, b) = (a.max(0.0), b.min(sw));
		if a < b {
			let (u0, u1) = ((a - x0 - shift) / cw, (b - x0 - shift) / cw);
			out.push(crate::overlay::Quad { tex: ov, rect: [a, 0.0, b - a, sh], uv: [u0, 0.0, u1, 1.0], colour: [1.0; 4], angle: 0.0, corners: None });
		}
	};
	quad(x0, x0 + cw, 0.0);
	let mut i = 1.0;
	while x0 - (i - 1.0) * cw > 0.0 {
		quad(x0 - i * cw, x0 - (i - 1.0) * cw, -i * cw);
		quad(x0 + i * cw, x0 + (i + 1.0) * cw, i * cw);
		i += 1.0;
	}
}


/// Blarg (level 6) energy barrier draw (queue A, queued by the barrier moby's updater FUN_00304320 state 2 while the
/// moby is within 48 of the camera) [C: nebula overlay.elf.c 168207-168327, disassembly scratch_bfx/dis].
const BARRIER_LEVEL: u32 = 6;
const BARRIER_FN: u32 = 0x304a98;
/// The strip helper it calls at +0x1e4 (FUN_002166b8: 4 lanes of `n` points, a 0.1 wide z-ribbon through the points,
/// VU1 strip, texture s along the lane, t across).
const BARRIER_RIB: u32 = 0x2166b8;
const BARRIER2_FN: u32 = 0x2f66f8;
/// Its four Euler angle vectors (DAT_001fe790, stride 0x10), the lane colours (gp-0x4aa0 / -0x4a9c), the texture step
/// and offset (gp-0x4a8c / -0x4a88), the uniform scale (gp-0x4a78 = 1.6), the colour-fade ages A / B (0x1621b4 / b8).
const BARRIER_EULER: u32 = 0x1fe790;
const BARRIER_C1: u32 = 0x162160;
const BARRIER_C2: u32 = 0x162164;
const BARRIER_STEP: u32 = 0x162174;
const BARRIER_OFF: u32 = 0x162178;
const BARRIER_SCALE: u32 = 0x162188;
const BARRIER_AGE: u32 = 0x1621b4;
/// fx texture of the barrier lanes (FUN_00210d80(0xe), the lightning texture Tesla and Walloper use).
const BARRIER_TEX: usize = 0x0e;

fn is_barrier(m: &Machine, level: u32, f: u32) -> bool {
	level == BARRIER_LEVEL && f == crate::retail::disc::va(BARRIER_FN) && jal_to(m, f + 0x1e4) == Some(crate::retail::disc::va(BARRIER_RIB))
}

fn is_barrier2(m: &Machine, level: u32, f: u32) -> bool {
	level == BARRIER_LEVEL && f == crate::retail::disc::va(BARRIER2_FN) && jal_to(m, f + 0x1e4) == Some(crate::retail::disc::va(BARRIER_RIB))
}

/// `v * M` for the row-vector matrix `m` (rows `m[0..3]`).
fn row_mul(v: [f32; 3], m: [[f32; 3]; 3]) -> [f32; 3] {
	add(add(scale(m[0], v[0]), scale(m[1], v[1])), scale(m[2], v[2]))
}

/// One `FUN_002166b8` strip-helper call set: for each world matrix (rows `M`, translation `t`) the helper draws lanes 3..0
/// of 30 points at `pts` (vec4, lane stride 0x1e0) [C]: per point the pair (p + z 0.05, p - z 0.05) transformed by
/// `p.x M0 + p.y M1 + p.z M2 + t`, s = off + step (k + 1), t = 1 / 0, ONE colour per lane for every vertex = lerp(c1, c2, f)
/// per channel (truncated) with f = 1 - age / B for age <= B else (age - B) / (A - B), age = i32 at `ages + 4 lane`;
/// texture 0xe, ALPHA_1 data A0 B2 C0 D1 FIX 0x80 = word 0x48 = Cs As + Cd, additive [C: the 0x42 is the REGISTER address of FUN_002ad5c8(0x42, ..) in FUN_002f66f8, the data comes from gp-0x4eb0..-0x4ea4 (RAM of fg1/6_0166: 0, 2, 0, 1); corrected 2026-10-16: the audit read 0x42 as a subtractive blend].
struct Lanes {
	mats: Vec<([[f32; 3]; 3], [f32; 3])>,
	pts: u32,
	ages: u32,
	age_a: f32,
	age_b: f32,
	c1: u32,
	c2: u32,
	step: f32,
	off: f32,
}

fn lanes(m: &Machine, l: &Lanes, out: &mut Vec<WorldQuad>) {
	const N: u32 = 30;
	for (mat, t) in &l.mats {
		for lane in (0..4u32).rev() {
			let age = m.rd32(l.ages + 4 * lane) as i32 as f32;
			let f = if age - l.age_b > 0.0 { (age - l.age_b) * (1.0 / (l.age_a - l.age_b)) } else { 1.0 - age * (1.0 / l.age_b) };
			let ch = |sh: u32| (((l.c1 >> sh & 0xff) as f32 * (1.0 - f) + (l.c2 >> sh & 0xff) as f32 * f) as u32 & 0xff) << sh;
			let col = ch(0) | ch(8) | ch(16) | ch(24);
			let w = |k: u32, dz: f32| {
				let p = m.rdv3(l.pts + lane * N * 0x10 + 0x10 * k);
				add(row_mul([p[0], p[1], p[2] + dz], *mat), *t)
			};
			for k in 0..N - 1 {
				let (s0, s1) = (l.off + l.step * (k + 1) as f32, l.off + l.step * (k + 2) as f32);
				let p = [w(k, -0.05), w(k, 0.05), w(k + 1, -0.05), w(k + 1, 0.05)];
				out.push(strip(p, [[s0, 0.0], [s0, 1.0], [s1, 0.0], [s1, 1.0]], [col; 4], BARRIER_TEX, true));
			}
		}
	}
}

/// Barrier 1 (FUN_00304a98): arg = the barrier moby, pvar = moby +0x78 holds 4 lanes of 30 points and the lane ages at
/// pvar + 0xf00. World matrix i = euler(table 0x1fe790[i]) x (1.6 euler(moby +0x40)) (FUN_002179a8 = b x a), translation
/// moby +0x10 + (0, 0, 1.7 i + 0.6) [C].
fn barrier(m: &Machine, arg: u32, out: &mut Vec<WorldQuad>) {
	let pv = m.rd32(arg + 0x78) & 0x01ff_ffff;
	if arg == 0 || pv == 0 {
		return;
	}
	let obj = euler_rows(m.rdv3(arg + 0x40)).map(|r| scale(r, m.rdf(crate::retail::disc::va(BARRIER_SCALE))));
	let pos = m.rdv3(arg + 0x10);
	let mats = (0..4u32).map(|i| (euler_rows(m.rdv3(crate::retail::disc::va(BARRIER_EULER) + 0x10 * i)).map(|r| row_mul(r, obj)), add(pos, [0.0, 0.0, i as f32 * 1.7 + 0.6]))).collect();
	let l = Lanes {
		mats,
		pts: pv,
		ages: pv + 0xf00,
		age_a: m.rd32(crate::retail::disc::va(BARRIER_AGE)) as i32 as f32,
		age_b: m.rd32(crate::retail::disc::va(BARRIER_AGE) + 4) as i32 as f32,
		c1: m.rd32(crate::retail::disc::va(BARRIER_C1)),
		c2: m.rd32(crate::retail::disc::va(BARRIER_C2)),
		step: m.rdf(crate::retail::disc::va(BARRIER_STEP)),
		off: m.rdf(crate::retail::disc::va(BARRIER_OFF)),
	};
	lanes(m, &l, out);
}

/// Barrier 2 (nebula FUN_002f66f8, queued by FUN_002f5d98): 9 matrices euler(table 0x1daf60[i]) x (1.5 euler(moby
/// +0x40)), translation moby +0x10 + table 0x1daed0[i] x R, fixed 4 x 30 point table 0x1daff0, ages 0x161dc0, A / B
/// 0x161dd0 / 0x161dd4, c2 gp-0x4e8c, step gp-0x4e88, offset gp-0x4e84, c1 = lerp(pvar +0x18, gp-0x4e90 & 0xffffff,
/// gp-0x4e90) (FUN_00217e68) [C: disassembly scratch_bfx/dis/bar2_2f66f8.s].
fn barrier2(m: &Machine, arg: u32, out: &mut Vec<WorldQuad>) {
	let pv = m.rd32(arg + 0x78) & 0x01ff_ffff;
	if arg == 0 || pv == 0 {
		return;
	}
	let obj = euler_rows(m.rdv3(arg + 0x40)).map(|r| scale(r, m.rdf(crate::retail::disc::va(0x161d8c))));
	let pos = m.rdv3(arg + 0x10);
	let mats = (0..9u32).map(|i| (euler_rows(m.rdv3(crate::retail::disc::va(0x1daf60) + 0x10 * i)).map(|r| row_mul(r, obj)), add(pos, row_mul(m.rdv3(crate::retail::disc::va(0x1daed0) + 0x10 * i), obj)))).collect();
	let (t, c) = (m.rdf(pv + 0x18), m.rd32(crate::retail::disc::va(0x161d70)));
	let c0 = c & 0x00ff_ffff;
	let ch = |sh: u32| (((c0 >> sh & 0xff) as f32 * (1.0 - t) + (c >> sh & 0xff) as f32 * t) as u32 & 0xff) << sh;
	let l = Lanes {
		mats,
		pts: crate::retail::disc::va(0x1daff0),
		ages: crate::retail::disc::va(0x161dc0),
		age_a: m.rd32(crate::retail::disc::va(0x161dd0)) as i32 as f32,
		age_b: m.rd32(crate::retail::disc::va(0x161dd4)) as i32 as f32,
		c1: ch(0) | ch(8) | ch(16) | ch(24),
		c2: m.rd32(crate::retail::disc::va(0x161d74)),
		step: m.rdf(crate::retail::disc::va(0x161d78)),
		off: m.rdf(crate::retail::disc::va(0x161d7c)),
	};
	lanes(m, &l, out);
}

/// Euler rotation rows (the retail euler matrix, VU0 0xd18; `particles::euler_m` images of the axes).
fn euler_rows(rot: [f32; 3]) -> [[f32; 3]; 3] {
	crate::particles::euler_m(rot)
}

/// One entry of the frame's particle walk (`pool_walk`): an index into the sprites, the pool ribbons (mode 3) or the
/// pool lines (mode 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolItem {
	Sprite(usize),
	Ribbon(usize),
	Line(usize),
}

/// Bucket of a view depth: `min(ftoi12(vz) >> 10, 0x3ff)` = floor(4 vz) (negative depths are culled before).
fn bucket(vz: f32) -> u32 {
	((vz * 4096.0) as i32 >> 10).clamp(0, 0x3ff) as u32
}

/// Draw order of the frame's particles: every non-pool sprite first, in list order (the queue-A glows run before the
/// pool, weapon_fx.md root cause 2), then every pool slot in retail's order, whatever its mode. Retail pool draw Eudora
/// FUN_00259e18 (= Veldin FUN_00267628) [C: overlay.elf.c 81836-81985 sort, 81992-82005 drain, 82087-82380 build]:
/// the sort pass walks the slots by rising index; modes 0/1 (flags +1 bit 1 clear) key on the slot position's view
/// depth, modes 2/3 (bit 1 set) on the depth of the midpoint of +0x10 and +0x20 (vf25.z = 0.5 on both points); BOTH
/// branches push the slot onto the head of the same bucket list `min(ftoi12(key) >> 10, 0x3ff)` at SPR 0x3000. The
/// drain reads the heads from bucket 0x3ff down to 0 into ONE index list, and the packet build walks that list once,
/// dispatching each slot by `flags & 3` (sprite, flat, line, ribbon) into one GIF stream. So the pool is drawn back to
/// front and, within one bucket, by falling slot index, with lines and ribbons interleaved among the sprites (stage 9;
/// stage 4 left modes 2/3 in separate passes). `sprite_slots` maps the n-th pool sprite to its slot (retail.rs
/// `queue_fx`); without it (or past its end) the list index stands in. Blended and additive slots interleave in that
/// order; drawing them grouped by blend put the Pyrocitor's additive glows over its blended flame frames (stage 4).
pub fn pool_walk(
	sprites: &[crate::particles::Sprite],
	sprite_slots: &[u16],
	ribbons: &[crate::retail::PoolRibbon],
	lines: &[crate::particles::Line],
	line_slots: &[u16],
	eye: [f32; 3],
	f: [f32; 3],
) -> Vec<PoolItem> {
	let mut order: Vec<PoolItem> = (0..sprites.len()).filter(|&i| sprites[i].pool.is_none()).map(PoolItem::Sprite).collect();
	let mid = |a: [f32; 3], b: [f32; 3]| dot(sub([(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5, (a[2] + b[2]) * 0.5], eye), f);
	let mut pool: Vec<(u32, u32, PoolItem)> = Vec::new();
	for (n, i) in (0..sprites.len()).filter(|&i| sprites[i].pool.is_some()).enumerate() {
		let slot = sprite_slots.get(n).map_or(0x800 + i as u32, |&s| s as u32);
		pool.push((bucket(dot(sub(sprites[i].pos, eye), f)), slot, PoolItem::Sprite(i)));
	}
	for (i, r) in ribbons.iter().enumerate() {
		pool.push((bucket(mid(r.head, r.tail)), r.slot as u32, PoolItem::Ribbon(i)));
	}
	for (i, l) in lines.iter().enumerate() {
		let slot = line_slots.get(i).map_or(0x800 + i as u32, |&s| s as u32);
		pool.push((bucket(mid(l.a, l.b)), slot, PoolItem::Line(i)));
	}
	pool.sort_by(|a, b| (b.0, b.1).cmp(&(a.0, a.1)));
	order.extend(pool.into_iter().map(|(_, _, it)| it));
	order
}

/// One rect of the screen filter FUN_001ec660 (frame draw FUN_001eae20, block 0x15f3f4 & 0x40): retail frame rows
/// [y0, y1) across the frame width, colour `rgba` (R low byte, 0x80 = 1.0 alpha) under GS ALPHA_1 mode `alpha`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenRect {
	pub y0: i32,
	pub y1: i32,
	/// The frame height sRam001518d2 the rows are counted in (448 PAL).
	pub rows: i32,
	pub rgba: u32,
	pub alpha: u8,
}

/// Frame draw flags and the HUD-pass enable (Eudora 0x15f3f4 bit 0x40, 0x16a134) [C Eudora, I others: same addresses as
/// `COUNT_B`].
const DRAW_FLAGS: u32 = 0x15f3f4;
const PASS_2D: u32 = 0x16a134;

/// The screen filter of this field (the Visibomb remote view's green tint and scanlines), FUN_001ec660 [C: overlay.elf.c
/// 11556-11608, frame draw 10760-10765]. It runs last in the frame (after the HUD and the fades) when the 0x40 block is
/// on, 0x16a134 != 0, gp-0x78f4 != 0 and the record pointer gp-0x78f0 != 0. Record: +4 colour / +8 ALPHA_1 of a rect
/// over the whole frame, then bands down the frame: rows [y, min(y + +0x10, h - 1)) in colour +0x14 / ALPHA +0x18,
/// then [y', min(y' + +0x20, h - 1)) in +0x24 / +0x28 (y' = y + +0x10), y = y' + +0x20, while y < h. A zero ALPHA word
/// keeps the previous ALPHA_1 (the block starts at 0x44); the word is masked to 0xff000000ff (mode byte, FIX) and a
/// colour whose alpha byte is 0 draws nothing. Rect FUN_001ec948(y0, y1, x0, x1): untextured sprite strip, XY
/// `n * 16 + offset - 8`, so it covers pixel rows y0..y1-1 [C]. Eudora's Visibomb record 0x16c880: full rect 0x80500050
/// under 0x42 (Cd - Cs * As: R and B lose 0x50), bands 2 rows 0x17ffffff under 0x42 (all channels lose 45), 3 rows clear
/// [C: dump/w13_60].
pub fn screen_filter(m: &Machine, level: u32) -> Vec<ScreenRect> {
	let g = gp(level);
	let rec = m.rd32(crate::retail::disc::va(g - 0x78f0)) & 0x01ff_ffff;
	if m.rd32(DRAW_FLAGS) & 0x40 == 0 || m.rd32(crate::retail::disc::va(PASS_2D)) == 0 || m.rd32(crate::retail::disc::va(g - 0x78f4)) == 0 || rec == 0 {
		return Vec::new();
	}
	let rows = m.rd16(0x1518d2) as i16 as i32;
	let mut alpha = 0x44u8;
	let mut out = Vec::new();
	let mut emit = |aw: u32, ahi: u32, rgba: u32, y0: i32, y1: i32| {
		if aw != 0 || ahi != 0 {
			alpha = aw as u8;
		}
		if rgba & 0xff00_0000 != 0 {
			out.push(ScreenRect { y0, y1, rows, rgba, alpha });
		}
	};
	emit(m.rd32(rec + 8), m.rd32(rec + 0xc), m.rd32(rec + 4), 0, rows);
	let mut y = 0i32;
	// Retail loops forever on two zero heights; the port stops after one band then [C: no guard in retail].
	while y < rows {
		let y2 = y.wrapping_add(m.rd32(rec + 0x10) as i32);
		emit(m.rd32(rec + 0x18), m.rd32(rec + 0x1c), m.rd32(rec + 0x14), y, y2.min(rows - 1));
		let y3 = y2.wrapping_add(m.rd32(rec + 0x20) as i32);
		emit(m.rd32(rec + 0x28), m.rd32(rec + 0x2c), m.rd32(rec + 0x24), y2, y3.min(rows - 1));
		if y3 <= y {
			break;
		}
		y = y3;
	}
	out
}

/// The subtractive filter rects (ALPHA 0x42, out = Cd - Cs*As, As 0x80 = 1.0) as HUD quads over the whole window, drawn
/// after the overlay quad (viewer `RETAIL_FILTER_TEX`: reverse subtract on the unorm target, so each channel clamps at
/// 0 like the GS with COLCLAMP on). The overlay capture leaves these prims out (`retail/gscap/draw.rs`: `ks < 0`
/// skip [merge M8]): it holds premultiplied colour clamped at 0, so a subtract over the overlay's own colour (the RC static)
/// was applied there and here again (normal R/B mean 2 vs gsfull 14 at e06_13rx tick 150) [C: weapon_fx.md stage 12].
/// Other modes are linear in Cd, so the overlay holds them exactly and they are not drawn here [C: the Visibomb record
/// uses 0x42 only]. Rows scale to the window height like the overlay; the frame width spans the window width [PC:
/// widescreen].
pub fn screen_quads(rects: &[ScreenRect], sw: f32, sh: f32, out: &mut Vec<crate::overlay::Quad>) {
	for r in rects.iter().filter(|r| r.rows > 0 && r.y1 > r.y0 && r.alpha == 0x42) {
		let a = (r.rgba >> 24) as f32 / 128.0;
		let c = [r.rgba & 0xff, r.rgba >> 8 & 0xff, r.rgba >> 16 & 0xff].map(|v| (v as f32 / 255.0 * a).min(1.0));
		let (y0, y1) = (r.y0 as f32 * sh / r.rows as f32, r.y1 as f32 * sh / r.rows as f32);
		out.push(crate::overlay::Quad { tex: crate::viewer::RETAIL_FILTER_TEX, rect: [0.0, y0, sw, y1 - y0], uv: [0.0; 4], colour: [c[0], c[1], c[2], 1.0], angle: 0.0, corners: None });
	}
}


#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn rand_back_inverts_the_lcg() {
		let mut s = 0x1234_5678u32;
		let mut fwd = Vec::new();
		for _ in 0..510 {
			s = s.wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039);
			fwd.push(s >> 16 & 0x7fff);
		}
		assert_eq!(rand_back(s, 510), fwd);
	}

	#[test]
	fn rc_bars_copy_the_centre() {
		let mut out = Vec::new();
		rc_bar_quads(9, 1280.0, 720.0, &mut out);
		assert_eq!(out.len(), 3);
		assert_eq!((out[0].rect, out[0].uv), ([160.0, 0.0, 960.0, 720.0], [0.0, 0.0, 1.0, 1.0]));
		assert_eq!(out[1].rect, [0.0, 0.0, 160.0, 720.0]);
		assert!((out[1].uv[0] - 800.0 / 960.0).abs() < 1e-6 && (out[1].uv[2] - 1.0).abs() < 1e-6);
		assert_eq!(out[2].rect, [1120.0, 0.0, 160.0, 720.0]);
		assert!(out[2].uv[0].abs() < 1e-6 && (out[2].uv[2] - 160.0 / 960.0).abs() < 1e-6);
		let mut none = Vec::new();
		rc_bar_quads(9, 960.0, 720.0, &mut none);
		assert!(none.is_empty());
	}

	#[test]
	fn screen_quads_subtract_only() {
		// Visibomb record: full rect 0x80500050 and a 2-row band 0x17ffffff under 0x42; a 0x44 rect stays in the overlay.
		let rects = [
			ScreenRect { y0: 0, y1: 448, rows: 448, rgba: 0x8050_0050, alpha: 0x42 },
			ScreenRect { y0: 0, y1: 2, rows: 448, rgba: 0x17ff_ffff, alpha: 0x42 },
			ScreenRect { y0: 2, y1: 5, rows: 448, rgba: 0x80ff_ffff, alpha: 0x44 },
		];
		let mut out = Vec::new();
		screen_quads(&rects, 1280.0, 896.0, &mut out);
		assert_eq!(out.len(), 2);
		assert!(out.iter().all(|q| q.tex == crate::viewer::RETAIL_FILTER_TEX));
		assert_eq!(out[0].rect, [0.0, 0.0, 1280.0, 896.0]);
		assert_eq!(out[0].colour, [80.0 / 255.0, 0.0, 80.0 / 255.0, 1.0]);
		assert_eq!(out[1].rect, [0.0, 0.0, 1280.0, 4.0]);
		assert!((out[1].colour[1] - 23.0 / 128.0).abs() < 1e-6);
	}

	#[test]
	fn strip_to_fan_order() {
		let q = strip([[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]], [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]], [0x80ff_0000, 0, 0, 0], 8, true);
		assert_eq!(q.pos[2], [1.0, 1.0, 0.0]);
		assert_eq!(q.uv[3], [1.0, 0.0]);
		assert_eq!(q.colour[0], [0.0, 0.0, 255.0 / 128.0, 1.0]);
	}

	#[test]
	fn pool_order_back_to_front_then_falling_slot() {
		let sp = |x: f32, pool: Option<u8>| crate::particles::Sprite { pos: [x, 0.0, 0.0], size: 1.0, rot: 0.0, colour: [1.0; 4], tex: 0, additive: false, pool, flat: false };
		// Depths 1.1 and 1.2 share bucket 4, 3.0 is bucket 12; the glow (no pool) goes first.
		let v = [sp(1.1, Some(0x44)), sp(3.0, Some(0x44)), sp(9.0, None), sp(1.2, Some(0x44))];
		use PoolItem::Sprite as S;
		assert_eq!(pool_walk(&v, &[], &[], &[], &[], [0.0; 3], [1.0, 0.0, 0.0]), vec![S(2), S(1), S(3), S(0)]);
	}

	#[test]
	fn pool_walk_interleaves_ribbons_and_lines_by_midpoint() {
		let sp = |x: f32| crate::particles::Sprite { pos: [x, 0.0, 0.0], size: 1.0, rot: 0.0, colour: [1.0; 4], tex: 0, additive: false, pool: Some(0x44), flat: false };
		let rb = |a: f32, b: f32, slot: u16| crate::retail::PoolRibbon { head: [a, 0.0, 0.0], tail: [b, 0.0, 0.0], half: 0.1, scale: 0.5, c_head: 0, c_tail: 0, tex: 0, additive: true, b9: 0x72, slot };
		let ln = |a: f32, b: f32| crate::particles::Line { a: [a, 0.0, 0.0], b: [b, 0.0, 0.0], col_a: [1.0; 4], col_b: [1.0; 4], near: 0.5, far: 224.0 };
		// Sprite slots 5 (depth 2.0, bucket 8) and 9 (depth 6.0, bucket 24); ribbon slot 7 midpoint 2.1 (bucket 8),
		// ribbon slot 2 midpoint 6.1 (bucket 24); line slot 4 midpoint 4.0 (bucket 16).
		let v = [sp(2.0), sp(6.0)];
		let r = [rb(2.0, 2.2, 7), rb(6.0, 6.2, 2)];
		let l = [ln(3.0, 5.0)];
		use PoolItem::*;
		assert_eq!(pool_walk(&v, &[5, 9], &r, &l, &[4], [0.0; 3], [1.0, 0.0, 0.0]), vec![Sprite(1), Ribbon(1), Line(0), Ribbon(0), Sprite(0)]);
	}

	#[test]
	fn set_len_keeps_zero() {
		assert_eq!(set_len([0.0; 3], -1.0), [0.0; 3]);
		assert_eq!(set_len([0.0, 2.0, 0.0], -1.0), [0.0, -1.0, 0.0]);
	}
}

/// Effects layer claimed set (retail/efx_draw.rs): whether `fill` (or a port it dispatches to) draws callback `f`.
pub fn claims(m: &Machine, level: u32, f: u32) -> bool {
	let i = li(level);
	if i >= 19 || f == 0 {
		return false;
	}
	is_swing_rope(m, f)
		|| f == (*crate::retail::disc::pick(&TAUNTER, &TAUNTER_USA))[i]
		|| f == (*crate::retail::disc::pick(&SUCK_TUBE, &SUCK_TUBE_USA))[i]
		|| is_barrier(m, level, f)
		|| is_barrier2(m, level, f)
		|| is_tesla(m, f)
		|| is_morph(m, f)
		|| is_walloper(m, f)
		|| is_hero_fx(m, f)
		|| (*crate::retail::disc::pick(&VENDOR_FN, &VENDOR_FN_USA))[i] == f
		|| arc_fn(level) == Some(f)
		|| super::queue_q2::claims(level, f)
		|| super::queue_fxb::claims(level, f)
}
