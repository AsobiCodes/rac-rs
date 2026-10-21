//! Lane fxb: ribbon trails of flying mobys (Rilgar class 79 FUN_002d72b0, Pokitaru class 1319 FUN_00318bf8), queue A.
//!
//! Same method as `queue_q2`: the callback's data (history ring, ST table, colours) is read from the RAM snapshot taken after the field drew,
//! so the update function's ring buffer and the gp words are their draw-time values. Retail facts: decomp/notes/draw_producers.md
//! (rows Rilgar 0x2d72b0 / Pokitaru 0x318bf8, 2026-10-12 fxb).


use crate::particles::WorldQuad;
use ee_oracle::Machine;
use crate::retail::disc::va;

/// gp of every level's callbacks (see `queue_q2::gp`).
const GP: u32 = 0x166c00;

/// One ribbon-trail family. Both callbacks are the same loop: for every member moby of the controller's class list, 16-entry history ring
/// of positions per strand; segment k (k < length) is a 4-corner strip over ring entries a = (head - k + 15) & 15 and a + 1, each corner
/// shifted by +-w along z (corner & 1), with the ST pair of table[corner] and the colour lerp(frac, c1, c2) (frac below).
struct Cfg {
	/// Level (0-based) and callback address.
	level: u32,
	f: u32,
	/// gp-relative float L = trail length limit (5.0, or 16.0 when byte 0x15edb2 is set; the callback stores it before use).
	l_at: u32,
	/// Class-list table (u32 per controller +0x21: pointer to u16 member ids, bit 15 = last) and the moby-table base pointer.
	list_tbl: u32,
	base_ptr: u32,
	/// ST table (4 x (s, t)) of the strip corners.
	st_at: u32,
	/// Strands per member, ring offset in the pvar, half width w.
	strands: u32,
	ring: u32,
	w: f32,
	/// `frac` formula: Rilgar x / (L + a / (4 L)), Pokitaru x / L + a / (4 L).
	rilgar: bool,
}

const RILGAR: Cfg = Cfg { level: 5, f: 0x2d72b0, l_at: GP - 0x5860, list_tbl: 0x1abcc0, base_ptr: 0x15ff98, st_at: 0x1cdcf0, strands: 3, ring: 0x20, w: 0.15, rilgar: true };
/// `RILGAR` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const RILGAR_USA: Cfg = Cfg { level: 5, f: 0x2d7920, l_at: GP - 0x5860, list_tbl: 0x1abcc0, base_ptr: 0x15ffd8, st_at: 0x1cdd70, strands: 3, ring: 0x20, w: 0.15, rilgar: true };
const POKITARU: Cfg = Cfg { level: 11, f: 0x318bf8, l_at: GP - 0x4890, list_tbl: 0x1ac1c0, base_ptr: 0x15ff58, st_at: 0x1f1390, strands: 2, ring: 0xc0, w: 0.25, rilgar: false };
/// `POKITARU` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const POKITARU_USA: Cfg = Cfg { level: 11, f: 0x3192c8, l_at: GP - 0x4890, list_tbl: 0x1ac240, base_ptr: 0x15ffd8, st_at: 0x1f1490, strands: 2, ring: 0xc0, w: 0.25, rilgar: false };

fn rgba(w: u32) -> [f32; 4] {
	[(w & 0xff) as f32 / 128.0, (w >> 8 & 0xff) as f32 / 128.0, (w >> 16 & 0xff) as f32 / 128.0, (w >> 24) as f32 / 128.0]
}

/// FUN_0022eb30 / FUN_00231730 (VU0 colour lerp): per byte trunc(a (1 - t) + b t).
fn lerp_colour(t: f32, a: u32, b: u32) -> u32 {
	(0..4).fold(0u32, |acc, i| {
		let (x, y) = ((a >> (8 * i) & 0xff) as f32, (b >> (8 * i) & 0xff) as f32);
		acc | (((x * (1.0 - t) + y * t) as u32 & 0xff) << (8 * i))
	})
}

/// Hook (`queue_b::fill`, queue A loop): draw `f(arg)` when it is a ribbon-trail callback of this module.
pub fn fill_a(m: &Machine, level: u32, f: u32, arg: u32, out: &mut Vec<WorldQuad>) {
	for c in [&(*crate::retail::disc::pick(&RILGAR, &RILGAR_USA)), &(*crate::retail::disc::pick(&POKITARU, &POKITARU_USA))] {
		if c.level == level && c.f == f {
			trail(m, c, arg, out);
		}
	}
}

fn trail(m: &Machine, c: &Cfg, arg: u32, out: &mut Vec<WorldQuad>) {
	if arg == 0 {
		return;
	}
	let b21 = m.rd8(arg + 0x21) as u32;
	// Colours and ALPHA: Rilgar reads them from gp words (gate words must be 0), Pokitaru has them as immediates.
	let (alpha, c1, c2) = if c.rilgar {
		if m.rd32(va(GP - 0x5850)) != 0 || m.rd32(va(GP - 0x584c)) != 0 {
			return;
		}
		([GP - 0x5848, GP - 0x5844, GP - 0x5840, GP - 0x583c].map(|a| m.rd32(va(a))), m.rd32(va(GP - 0x5838)), m.rd32(va(GP - 0x5834)))
	} else {
		if b21 == 0xff {
			return;
		}
		([0, 2, 0, 1], 0x8c28_aa28, 0x0014_4646)
	};
	// Additive only (ALPHA 0x48), as the other ported callbacks; the packet's FIX 0x80 is irrelevant there.
	if alpha != [0, 2, 0, 1] {
		return;
	}
	let l = m.rdf(c.l_at);
	if !(l.is_finite() && l >= 1.0 && l <= 16.0) {
		return;
	}
	let base = m.rd32(c.base_ptr);
	let list = m.rd32(c.list_tbl + 4 * b21);
	if base == 0 || list == 0 {
		return;
	}
	let ctrl_class = m.rd16(arg + 0xa6);
	let a = ((m.rd32(va(0x15_f5ac)).wrapping_add(3)) & 3) as f32;
	let st: [[f32; 2]; 4] = std::array::from_fn(|i| [m.rdf(c.st_at + 8 * i as u32), m.rdf(c.st_at + 8 * i as u32 + 4)]);
	// colour of corner i of segment k
	let col = |k: i32, i: i32| -> [f32; 4] {
		let x = (k - ((i >> 1) - 1)) as f32;
		let mut t = if c.rilgar { x / (l + a / (l * 4.0)) } else { x / l + a / (l * 4.0) };
		if 1.0 < t {
			t = 1.0;
		} else if t < 0.0 {
			t = 0.0;
		}
		rgba(lerp_colour(t, c1, c2))
	};
	let mut e = list;
	for _ in 0..64 {
		let id = m.rd16(e) as u32;
		e += 2;
		let mob = (id & 0x7fff) * 0x100 + base;
		let member = if c.rilgar { m.rd16(mob + 0xa6) == 0x4f && m.rd8(mob + 0x20) == 1 } else { m.rd8(mob + 0x31) != 0 && m.rd16(mob + 0xa6) == ctrl_class };
		let pv = m.rd32(mob + 0x78) & 0x01ff_ffff;
		if member && pv != 0 {
			let (len, head) = if c.rilgar { (m.rd16(pv + 0x320) as i16 as i32, m.rd16(pv + 0x322) as i16 as i32) } else { (m.rd32(pv + 0x2c0) as i32, m.rd32(pv + 0x2c4) as i32) };
			for k in 0..len.clamp(0, 16) {
				let a0 = (head - k + 15).rem_euclid(16);
				for s in 0..c.strands {
					let mut p = [[0.0f32; 3]; 4];
					let mut cc = [[0.0f32; 4]; 4];
					for i in 0..4i32 {
						let idx = ((a0 + (i >> 1)) & 15) as u32;
						let mut v = m.rdv3(pv + c.ring + s * 0x100 + idx * 0x10);
						v[2] += if i & 1 == 0 { c.w } else { -c.w };
						p[i as usize] = v;
						cc[i as usize] = col(k, i);
					}
					// strip order (v0 v1 v2 v3) -> fan order (0 1 3 2).
					out.push(WorldQuad { pos: [p[0], p[1], p[3], p[2]], uv: [st[0], st[1], st[3], st[2]], colour: [cc[0], cc[1], cc[3], cc[2]], tex: 0x13, additive: true });
				}
			}
		}
		if id & 0x8000 != 0 {
			break;
		}
	}
}

/// Effects layer claimed set (retail/efx_draw.rs): the callbacks `fill_a` draws.
pub fn claims(level: u32, f: u32) -> bool {
	[&(*crate::retail::disc::pick(&RILGAR, &RILGAR_USA)), &(*crate::retail::disc::pick(&POKITARU, &POKITARU_USA))].iter().any(|c| c.level == level && c.f == f)
}
