//! Lane q2: draw-queue callbacks of the later planets that the mirror did not draw (fx_census rows 10, 14-17).
//!
//! Same method as `queue_b`: the callback's own code in RAM gives its data addresses (lui/addiu pairs, gp-relative
//! loads), so the twin levels share one port and a callback whose code does not have the expected shape draws nothing.
//! The mirror snapshot is taken after the field drew, so the RAM state the callback consumed (colour word, pvar fields)
//! is its draw-time value.


use crate::particles::{WorldQuad, FX_CLAMP};
use ee_oracle::Machine;
use std::collections::HashMap;
use std::sync::Mutex;

/// `MobyClass::colour_start` (first vertex of the class-tint packets) of the classes the viewer keeps as static meshes only (no
/// skeleton animation, so absent from `prop_anims`), by class id: retail's tail-packet colour +0x90 (flag +0x34 & 0x10) applies to them
/// too (moby_format.md CLASS-tint), e.g. the Oltanis street lamp (class 1511, globe = tail packet 6, +0x90 = 0x7f6e6e32 cyan). [C]
static STATIC_TINT: Mutex<Option<HashMap<u32, usize>>> = Mutex::new(None);

/// Loader hook (`app.rs`, static-class arm): remember the class's `colour_start`.
pub fn note_static_tint(class: u32, colour_start: usize) {
	if super::deferring() {
		super::defer(Box::new(move || note_static_tint(class, colour_start)));
		return;
	}
	if let Ok(mut g) = STATIC_TINT.lock() {
		g.get_or_insert_with(HashMap::new).insert(class, colour_start);
	}
}

/// Draw hook (`handler.rs`, PROP_TINTS): the static class's `colour_start`, if the loader noted one.
pub fn static_tint_start(class: u32) -> Option<usize> {
	STATIC_TINT.lock().ok()?.as_ref()?.get(&class).copied()
}

/// fx textures bound with GS CLAMP_1 = 5 here (`particles::FX_CLAMP` key variants): the lamp glow's 0xb [C: Rilgar
/// 0x31c918 `ori a2,zero,5; sd a2,0x70(sp)`; the other ported contexts send CLAMP_1 = 0].
pub const FX_CLAMPED: [usize; 1] = [0x0b];

/// Street-lamp glow draw (class 1511, update FUN_00307488 on Oltanis; Oltanis FUN_00307b00, Rilgar twin FUN_0031c918, queue A)
/// [C: nebula-style decode of overlay.elf.c 174324-174449 and the Rilgar disassembly].
const LAMP_FN: [(u32, u32); 2] = [(5, 0x31c918), (14, 0x307b00)];
/// `LAMP_FN` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const LAMP_FN_USA: [(u32, u32); 2] = [(5, 0x31cf58), (14, 0x308150)];
/// ON (2026-10-12): the first port read the gp floats at gp + 0x40 and drew the glow on the ground, where retail shows none; with gp 0x166c00 (K1 = 4.0,
/// K2 = -0.1) the halo sits on the globe and agrees with retail's GS picture (fg1/14_0692, shots q2c vs q2i).
const LAMP_ENABLED: bool = true;

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn unit(v: [f32; 3]) -> [f32; 3] {
	let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
	if l == 0.0 { [0.0; 3] } else { v.map(|x| x / l) }
}
fn rgba(w: u32) -> [f32; 4] {
	[(w & 0xff) as f32 / 128.0, (w >> 8 & 0xff) as f32 / 128.0, (w >> 16 & 0xff) as f32 / 128.0, (w >> 24) as f32 / 128.0]
}

/// The gp the callbacks run with: 0x166c00 on every level (regs.bin of fg1 states of levels 0 5 10 14 all hold gp = 0x166c00; read at 0x166c40
/// the lamp's first gp float is the integer 4 (a denormal) while at 0x166c00 it is 4.0 and the second -0.1; corrected 2026-10-12: was
/// 0x166c40 for levels 0 5 14 16 as in `queue_b::gp`, which put the glow on the ground) [C].
fn gp(_level: u32) -> u32 {
	0x166c00
}

/// Every `lui rt` + first following `addiu rd, rt` (within 8 insns; rd may differ: `addiu s2, v1, 0x5bf0`) address of the function in the overlay-data / globals range.
fn lui_pairs(m: &Machine, f: u32, insns: u32) -> Vec<u32> {
	let mut out = Vec::new();
	for k in 0..insns {
		let w = m.rd32(f + 4 * k);
		if w >> 26 != 0x0f {
			continue;
		}
		let rt = w >> 16 & 31;
		let hit = (1..=8).map(|j| m.rd32(f + 4 * (k + j))).find(|x| x >> 26 == 9 && (x >> 21 & 31) == rt);
		if let Some(x) = hit {
			let v = ((w & 0xffff) << 16).wrapping_add(x as i16 as i32 as u32);
			if (0x15_ef00..0x40_0000).contains(&v) {
				out.push(v);
			}
		}
	}
	out
}

/// The gp-relative `lwc1` offsets of the function in order of appearance (first `insns` words).
fn gp_floats(m: &Machine, level: u32, f: u32, insns: u32) -> Vec<u32> {
	(0..insns).map(|k| m.rd32(f + 4 * k)).filter(|x| x >> 26 == 0x31 && (x >> 21 & 31) == 28).map(|x| gp(level).wrapping_add(x as i16 as i32 as u32)).collect()
}


/// Hook (`queue_b::fill`, queue A loop): draw `f(arg)` when it is a callback of this module.
pub fn fill_a(m: &Machine, level: u32, f: u32, arg: u32, out: &mut Vec<WorldQuad>) {
	if LAMP_ENABLED && (*crate::retail::disc::pick(&LAMP_FN, &LAMP_FN_USA)).contains(&(level, f)) {
		lamp(m, level, f, arg, out);
	} else if (*crate::retail::disc::pick(&BEAM_FN, &BEAM_FN_USA)).contains(&(level, f)) {
		beam(m, f, arg, out);
	}
}

/// Searchlight beam draw (class 823): Rilgar FUN_0030bbb0, Umbris twin FUN_003041d8 (queue A) [C: overlay C 7_umbris 156263-156482, the
/// tables read from RAM].
const BEAM_FN: [(u32, u32); 2] = [(5, 0x30bbb0), (7, 0x3041d8)];
/// `BEAM_FN` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const BEAM_FN_USA: [(u32, u32); 2] = [(5, 0x30c220), (7, 0x3048b0)];

/// The gp-relative `lw` addresses of the function in order of appearance (first `insns` words).
fn gp_words(m: &Machine, f: u32, insns: u32) -> Vec<u32> {
	(0..insns).map(|k| m.rd32(f + 4 * k)).filter(|x| x >> 26 == 0x23 && (x >> 21 & 31) == 28).map(|x| 0x16_6c00u32.wrapping_add(x as i16 as i32 as u32)).collect()
}

fn lerp_colour(t: f32, a: u32, b: u32) -> u32 {
	// FUN_00227c30 (VU0 colour lerp): per byte trunc(a (1 - t) + b t).
	(0..4).fold(0u32, |acc, i| {
		let (x, y) = ((a >> (8 * i) & 0xff) as f32, (b >> (8 * i) & 0xff) as f32);
		acc | (((x * (1.0 - t) + y * t) as u32 & 0xff) << (8 * i))
	})
}

/// One searchlight (moby = the light, class 823) [C]: moby joint 0 world position J (the moby origin [I]: joint 0 of the class has no offset), euler
/// E = euler(0, sin(pvar+0x48) * pi/6 + pi/6, moby +0x48), t = unit(camera - J), basis rows r0 = E2 x t, r1 = r0 x E2, r2 = E2 and translation J. Ten slices
/// k = 0..9 of two nested strips (tables: strip A 4 vec4 `x 0.8 / -0.8, z 0 / 10`, strip B `x 0.4 / -0.4, z 0 / 12`), corner i (strip order,
/// i < 2 is the + side): s = (k + (i & 1)) 0.1, x += s * 8 (B: 6) * (+1 | -1), z += k * 10 (B: 12); world = x r0 + y r1 + z r2 + w J; colour =
/// colour_lerp(s, c1, c2) per corner (c1 0x40807070 -> c2 0x00804040: the alpha fades to the tip); the 4 (s, t) pairs of the UV table are shared
/// by both strips; fx texture slot gp word, ALPHA (A,B,C,D) = (0,2,0,1) = 0x48 additive; each strip is submitted by submit_strip4_ctx
/// (FUN_00224b90 / Novalis 0x21e958) with the basis as its matrix. The gp words sit at the texture slot's address - 0x18 (A) .. - 0xc (D), - 8 (c1), - 4 (c2).
fn beam(m: &Machine, f: u32, arg: u32, out: &mut Vec<WorldQuad>) {
	let pv = m.rd32(arg + 0x78) & 0x01ff_ffff;
	if arg == 0 || pv == 0 {
		return;
	}
	let pairs = lui_pairs(m, f, 0x100);
	let (Some(&cam_at), Some(&uv_at), Some(&tb), Some(&ta)) = (pairs.first(), pairs.get(1), pairs.get(2), pairs.get(3)) else { return };
	let Some(&tex_at) = gp_words(m, f, 0x100).first() else { return };
	let tex = m.rd32(tex_at) as usize;
	let alpha = [0x18u32, 0x14, 0x10, 0x0c].map(|d| m.rd32(tex_at - d));
	if alpha != [0, 2, 0, 1] || tex > 0x80 {
		return;
	}
	let (c1, c2) = (m.rd32(tex_at - 8), m.rd32(tex_at - 4));
	let ang = m.rdf(pv + 0x48);
	let pitch = crate::camera::fast_sin(ang) * 0.5235988 + 0.5235988;
	let e = crate::particles::euler_m([0.0, pitch, m.rdf(arg + 0x48)]);
	let j = m.rdv3(arg + 0x10);
	let t = unit(sub(m.rdv3(cam_at), j));
	let r2 = e[2];
	let r0 = cross(r2, t);
	let r1 = cross(r0, r2);
	let uv: [[f32; 2]; 4] = std::array::from_fn(|i| [m.rdf(uv_at + 8 * i as u32), m.rdf(uv_at + 8 * i as u32 + 4)]);
	for k in 0..10u32 {
		for (tbl, kx, kz) in [(ta, 8.0f32, 10.0f32), (tb, 6.0, 12.0)] {
			let mut p = [[0.0f32; 3]; 4];
			let mut col = [[0.0f32; 4]; 4];
			for i in 0..4u32 {
				let v = m.rdv3(tbl + 0x10 * i);
				let w = m.rdf(tbl + 0x10 * i + 0xc);
				let s = (k + (i & 1)) as f32 * 0.1;
				let sign = if i > 1 { -1.0 } else { 1.0 };
				let x = v[0] + s * kx * sign;
				let z = v[2] + k as f32 * kz;
				p[i as usize] = std::array::from_fn(|a| x * r0[a] + v[1] * r1[a] + z * r2[a] + w * j[a]);
				col[i as usize] = rgba(lerp_colour(s, c1, c2));
			}
			// strip order (v0 v1 v2 v3) -> fan order (0 1 3 2).
			out.push(WorldQuad { pos: [p[0], p[1], p[3], p[2]], uv: [uv[0], uv[1], uv[3], uv[2]], colour: [col[0], col[1], col[3], col[2]], tex, additive: true });
		}
	}
}

/// Street-lamp glow (Oltanis FUN_00307b00 / Rilgar FUN_0031c918) [C]: one camera-facing quad, texture 0xb (FUN_0020c408(0xb),
/// CLAMP_1 = 5), ALPHA 0x48 additive, UV (0,0) (0,1) (1,0) (1,1) (strip order). With d = moby +0x10 + K1 * (row +0xe0) (K1 the first
/// gp float the code loads), t = unit(camera - d) (camera = the vec3 at the first lui/addiu address below 0x170000), right =
/// unit(t x (0,0,1)), up = right x t and centre = d + K2 * t (K2 the second gp float), the corners are the 4 vec4 of the table
/// (the last lui/addiu address, >= 0x170000) with xyz * pvar[+0x70] times the rows t, right, up, centre (vec4_mul_mat4). Colour word
/// = (i32 pvar[+0x60] << 24) | pvar[+0x68], the same on all 4 corners; pvar = moby +0x78. When pvar byte +0x67 is set the draw
/// halves pvar[+0x60] one time in 8 (rand, written back), so the snapshot's value is already the drawn one.
fn lamp(m: &Machine, level: u32, f: u32, arg: u32, out: &mut Vec<WorldQuad>) {
	let pv = m.rd32(arg + 0x78) & 0x01ff_ffff;
	if arg == 0 || pv == 0 {
		return;
	}
	let pairs = lui_pairs(m, f, 0x60);
	let (Some(&cam_at), Some(&corners)) = (pairs.iter().find(|&&v| v < 0x17_0000), pairs.iter().rev().find(|&&v| v >= 0x17_0000)) else { return };
	let g = gp_floats(m, level, f, 0x60);
	let [k1, k2] = [g.first(), g.get(1)].map(|a| a.map_or(f32::NAN, |&a| m.rdf(a)));
	let size = m.rdf(pv + 0x70);
	if !(k1.is_finite() && k2.is_finite() && size.is_finite()) {
		return;
	}
	let z = m.rdv3(arg + 0xe0);
	let pos = m.rdv3(arg + 0x10);
	let d: [f32; 3] = std::array::from_fn(|i| pos[i] + k1 * z[i]);
	let t = unit(sub(m.rdv3(cam_at), d));
	let right = unit(cross(t, [0.0, 0.0, 1.0]));
	let up = cross(right, t);
	let centre: [f32; 3] = std::array::from_fn(|i| d[i] + k2 * t[i]);
	let p: [[f32; 3]; 4] = std::array::from_fn(|q| {
		let a = corners + 0x10 * q as u32;
		let v = m.rdv3(a).map(|x| x * size);
		let w = m.rdf(a + 0xc);
		std::array::from_fn(|i| v[0] * t[i] + v[1] * right[i] + v[2] * up[i] + w * centre[i])
	});
	let c = rgba(m.rd32(pv + 0x60) << 24 | m.rd32(pv + 0x68));
	let uv = [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]];
	// strip order (v0 v1 v2 v3) -> fan order (0 1 3 2), as queue_b::strip.
	out.push(WorldQuad { pos: [p[0], p[1], p[3], p[2]], uv: [uv[0], uv[1], uv[3], uv[2]], colour: [c; 4], tex: 0x0b | FX_CLAMP, additive: true });
}

/// Effects layer claimed set (retail/efx_draw.rs): the callbacks `fill_a` draws.
pub fn claims(level: u32, f: u32) -> bool {
	(LAMP_ENABLED && (*crate::retail::disc::pick(&LAMP_FN, &LAMP_FN_USA)).contains(&(level, f))) || (*crate::retail::disc::pick(&BEAM_FN, &BEAM_FN_USA)).contains(&(level, f))
}
