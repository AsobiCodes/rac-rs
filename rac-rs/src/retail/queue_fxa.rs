//! Lane fxa: draw-queue B callbacks of the later planets that the mirror did not draw (fx_census rows 16-17).
//!
//! Same method as `queue_q2`: the callback's own code in RAM gives its data addresses (lui/addiu pairs, gp-relative loads), so
//! twin levels share one port; code of another shape draws nothing.


use crate::particles::{WorldQuad, FX_CLAMP};
use ee_oracle::Machine;
use crate::retail::disc::va;

/// Class-252 spark glow: Batalia FUN_002d3a38 (queue B), binary twin Oltanis FUN_002da8b8 [C: overlay C 8_batalia 139867-139990].
const SPARK_FN: [(u32, u32); 2] = [(8, 0x2d3a38), (14, 0x2da8b8)];
/// `SPARK_FN` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const SPARK_FN_USA: [(u32, u32); 2] = [(8, 0x2d4108), (14, 0x2daf08)];
/// First instruction of the callback: `addiu sp, sp, -0x170`.
const SPARK_HEAD: u32 = 0x27bd_fe90;
/// The gp the callbacks run with (queue_q2::gp).
const GP: u32 = 0x166c00;

/// OFF (2026-10-12, lane fxa): the quad is decoded from the code, but the retail GS picture (fg1/8_0328, shots fxa1 gsf / fxa4o gsf) shows no
/// glow on the three queued class-252 mobys (hex nuts, 9-18 m ahead of the hero) and neither does the mirror with the port on (fxa3 / fxa4n):
/// a 0.35 m additive quad of alpha 0.25 at most. Not drawing what retail does not show; flip to true only with a picture that shows the glow.
const SPARK_ENABLED: bool = false;

/// Hook (`queue_b::fill`, queue B loop): draw `f(arg)` when it is a callback of this module.
pub fn fill_b(m: &Machine, level: u32, f: u32, arg: u32, out: &mut Vec<WorldQuad>) {
	if SPARK_ENABLED && (*crate::retail::disc::pick(&SPARK_FN, &SPARK_FN_USA)).contains(&(level, f)) {
		spark(m, f, arg, out);
	} else if level == BOLT_LEVEL && f == va(BOLT_FN) {
		bolt(m, out);
	}
}

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

/// Every `lui rt` + first following `addiu rd, rt` (within 8 insns) address of the function in the overlay-data / globals range.
fn lui_pairs(m: &Machine, f: u32, insns: u32) -> Vec<u32> {
	let mut out = Vec::new();
	for k in 0..insns {
		let w = m.rd32(f + 4 * k);
		if w >> 26 != 0x0f {
			continue;
		}
		let rt = w >> 16 & 31;
		if let Some(x) = (1..=8).map(|j| m.rd32(f + 4 * (k + j))).find(|x| x >> 26 == 9 && (x >> 21 & 31) == rt) {
			let v = ((w & 0xffff) << 16).wrapping_add(x as i16 as i32 as u32);
			if (0x15_ef00..0x40_0000).contains(&v) {
				out.push(v);
			}
		}
	}
	out
}

/// The gp-relative `lwc1` addresses of the function in order of appearance.
fn gp_floats(m: &Machine, f: u32, insns: u32) -> Vec<u32> {
	(0..insns).map(|k| m.rd32(f + 4 * k)).filter(|x| x >> 26 == 0x31 && (x >> 21 & 31) == 28).map(|x| GP.wrapping_add(x as i16 as i32 as u32)).collect()
}

/// One additive glow quad per queued class-252 moby (Batalia / Oltanis) [C: code]. pvar = moby +0x78; P = pvar +0x220 (vec4);
/// d = unit(V - P) with V the vec3 at the first lui/addiu address below 0x170000 (0x167540); basis rows d, c = unit(up x d)
/// (up = (0,0,1)), b = d x c; centre = P + K1 * d (K1 = the first gp float, 0.2); the 4 corners of the table at the last lui/addiu
/// address (>= 0x170000, 0x1d1c80, vec4 (0, +-1, +-1, 1)) are scaled by K2 (the second gp float, 0.175) in xyz and multiplied by the matrix
/// (d, c, b, centre). Colour = (moby +0x90 & 0xffffff) | 0x40 << 24 on all corners, fx texture 0xb, CLAMP_1 5, ALPHA 0x48 (additive),
/// UV (0,0) (0,1) (1,0) (1,1); drawn by submit_strip4_ctx FUN_0020f868.
fn spark(m: &Machine, f: u32, arg: u32, out: &mut Vec<WorldQuad>) {
	if m.rd32(f) != SPARK_HEAD {
		return;
	}
	let pv = m.rd32(arg + 0x78) & 0x01ff_ffff;
	if arg == 0 || pv == 0 {
		return;
	}
	let pairs = lui_pairs(m, f, 0x70);
	let (Some(&v_at), Some(&corners)) = (pairs.iter().find(|&&v| v < 0x17_0000), pairs.iter().rev().find(|&&v| v >= 0x17_0000)) else { return };
	let g = gp_floats(m, f, 0x70);
	let [k1, k2] = [g.first(), g.get(1)].map(|a| a.map_or(f32::NAN, |&a| m.rdf(a)));
	if !(k1.is_finite() && k2.is_finite()) {
		return;
	}
	let p = m.rdv3(pv + 0x220);
	let d = unit(sub(m.rdv3(v_at), p));
	let c = unit(cross([0.0, 0.0, 1.0], d));
	let b = cross(d, c);
	let centre: [f32; 3] = std::array::from_fn(|i| p[i] + k1 * d[i]);
	let q: [[f32; 3]; 4] = std::array::from_fn(|n| {
		let a = corners + 0x10 * n as u32;
		let v = m.rdv3(a).map(|x| x * k2);
		let w = m.rdf(a + 0xc);
		std::array::from_fn(|i| v[0] * d[i] + v[1] * c[i] + v[2] * b[i] + w * centre[i])
	});
	let col = rgba((m.rd32(arg + 0x90) & 0x00ff_ffff) | 0x4000_0000);
	let uv = [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]];
	// strip order (v0 v1 v2 v3) -> fan order (0 1 3 2), as queue_q2::lamp.
	out.push(WorldQuad { pos: [q[0], q[1], q[3], q[2]], uv: [uv[0], uv[1], uv[3], uv[2]], colour: [col; 4], tex: 0x0b | FX_CLAMP, additive: true });
}

/// Oltanis lightning bolt: FUN_002ed5c8, a queue-B callback registered by FUN_002ecee0 while a strike runs [C: overlay C 14_oltanis
/// 157035-157533 and the RAM disassembly]. Nothing is read from the moby: the bolt is all RAM tables.
const BOLT_LEVEL: u32 = 14;
const BOLT_FN: u32 = 0x2ed5c8;
/// First instruction `addiu sp, sp, -0x310`.
const BOLT_HEAD: u32 = 0x27bd_fcf0;
type V4 = [f32; 4];

fn rd4(m: &Machine, a: u32) -> V4 {
	[m.rdf(a), m.rdf(a + 4), m.rdf(a + 8), m.rdf(a + 12)]
}
/// vec4 * mat4 (FUN_002128b8): v.x M0 + v.y M1 + v.z M2 + v.w M3, one rounding per VU op.
fn mul_mat(v: V4, mt: &[V4; 4]) -> V4 {
	std::array::from_fn(|c| ((v[0] * mt[0][c] + v[1] * mt[1][c]) + v[2] * mt[2][c]) + v[3] * mt[3][c])
}
/// FUN_00212468: a - b on 4 lanes.
fn sub4(a: V4, b: V4) -> V4 {
	std::array::from_fn(|i| a[i] - b[i])
}
/// FUN_002126c0 with f12 = 1: xyz scaled by 1 / |xyz|, w kept.
fn norm4(v: V4) -> V4 {
	let s = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
	if s == 0.0 {
		return [0.0, 0.0, 0.0, 2.0];
	}
	let q = 1.0 / s.sqrt();
	[v[0] * q, v[1] * q, v[2] * q, v[3]]
}
/// FUN_00212580(out, a, b): out = b x a (VU opmula / opmsub), w left at 0.
fn cross4(a: V4, b: V4) -> V4 {
	[b[1] * a[2] - b[2] * a[1], b[2] * a[0] - b[0] * a[2], b[0] * a[1] - b[1] * a[0], 0.0]
}
/// FUN_0025ff38(-1, 0, 1, 0, t): the cubic of the call, in the code's operation order.
fn curve(t: f32) -> f32 {
	let (p1, p2, p3, p4) = (-1.0f32, 0.0f32, 1.0f32, 0.0f32);
	let (a, b) = (p1 - p2, p4 - p3);
	let c = b - a;
	let (t2, t3) = (t * t, t * t * t);
	let d = (p3 - p1) * t;
	let e = a - c;
	(c * t3 + e * t2 + d) + p2
}

/// One strike: 40 trunk points (table 0x1dfd00, local x along the bolt axis, y / z jitter) joined by camera-facing ribbons, and three
/// 15-segment branches that start at a trunk point and walk their own offset table, ribbons in the bolt frame. Every segment is two
/// strips: core (fx 0x2d, colour word 0x161c38 | alpha << 24) and glow (fx 0x2c, 0x161c3c | alpha / 2 << 24), ALPHA 0x48 additive, UV
/// (0,0) (0,1) (1,0) (1,1) over (previous-left, previous-right, current-left, current-right); the previous ring carries the previous
/// segment's colour. Alpha = clamp(int(age / scale_ticks(0x161c30) * 255) - u8 0x161ca0, 0, 255).
fn bolt(m: &Machine, out: &mut Vec<WorldQuad>) {
	if m.rd32(va(BOLT_FN)) != BOLT_HEAD {
		return;
	}
	let (p0, p1, cam) = (rd4(m, va(0x161c80)), rd4(m, va(0x161c90)), rd4(m, va(0x1674c0)));
	let k = m.rdf(va(GP - 0x7e98));
	let scale = (m.rd32(va(0x161c30)) as i32 as f32 * k + 0.5) as i32;
	let sub = m.rd8(va(0x161ca0)) as i32;
	let (col_core, col_glow) = (m.rd32(va(0x161c38)), m.rd32(va(0x161c3c)));
	let (w0, w1, kg) = (m.rdf(va(0x161c44)), m.rdf(va(0x161c40)), m.rdf(va(0x161c48)));
	if scale <= 0 || !(w0.is_finite() && w1.is_finite() && kg.is_finite()) {
		return;
	}
	let alpha = |age: i16| -> u32 { (((age as f32 / scale as f32) * 255.0) as i32 - sub).clamp(0, 255) as u32 };
	let cols = |a: u32| -> (u32, u32) { (col_core | a << 24, col_glow | (a >> 1) << 24) };
	let d = norm4(sub4(p0, p1));
	let r1 = norm4(cross4(d, [1.0, 0.0, 0.0, 0.0]));
	let r2 = cross4(r1, d);
	let frame: [V4; 4] = [[d[0], d[1], d[2], 0.0], [r1[0], r1[1], r1[2], 0.0], [r2[0], r2[1], r2[2], 0.0], p1];
	let tpl = |z: f32| -> [V4; 2] { [[0.0, 0.0, z, 1.0], [-0.0, -0.0, -z, 1.0]] };
	// Emit one segment: previous ring [l0, r0] with colour pc, current ring [l1, r1] with colour cc, both strips.
	let mut seg = |core: [V4; 4], glow: [V4; 4], pc: (u32, u32), cc: (u32, u32)| {
		let uv = [[0.0f32, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]];
		for (v, tex, c0, c1) in [(core, 0x2du32, pc.0, cc.0), (glow, 0x2c, pc.1, cc.1)] {
			let p: [[f32; 3]; 4] = v.map(|q| [q[0], q[1], q[2]]);
			let col = [rgba(c0), rgba(c0), rgba(c1), rgba(c1)];
			out.push(WorldQuad { pos: [p[0], p[1], p[3], p[2]], uv: [uv[0], uv[1], uv[3], uv[2]], colour: [col[0], col[1], col[3], col[2]], tex: tex as usize, additive: true });
		}
	};
	let world = |local: V4| mul_mat(local, &frame);
	let point = |i: usize| -> V4 { if i == 39 { p0 } else { world(rd4(m, va(0x1dfd00) + 16 * i as u32)) } };
	let facing = |pt: V4| -> [V4; 4] {
		let tc = norm4(sub4(cam, pt));
		[frame[0], tc, cross4(tc, frame[0]), pt]
	};
	// Trunk. The first ring sits at table entry 1 (retail starts the walk there), width w0, colour of age[0].
	let age = |i: usize| m.rd16(va(0x1dff80) + 2 * i as u32) as i16;
	let (mut w, mut prev) = (w0, cols(alpha(age(0))));
	let pt = point(1);
	let mt = facing(pt);
	let [t, g] = [tpl(w), tpl(w * kg)];
	let mut ring = [mul_mat(t[0], &mt), mul_mat(t[1], &mt), mul_mat(g[0], &mt), mul_mat(g[1], &mt)];
	for i in 1..40usize {
		let [t, g] = [tpl(w), tpl(w * kg)];
		let pt = point(i);
		let mt = facing(pt);
		let cur = [mul_mat(t[0], &mt), mul_mat(t[1], &mt), mul_mat(g[0], &mt), mul_mat(g[1], &mt)];
		let cc = if i == 39 { cols(0) } else { cols(alpha(age(i))) };
		seg([ring[0], ring[1], cur[0], cur[1]], [ring[2], ring[3], cur[2], cur[3]], prev, cc);
		ring = cur;
		prev = cc;
		w = w0 + (w1 - w0) * curve((i as f32) / 40.0);
	}
	// Branches: ribbons in the bolt frame (rows d, r1, r2) from the world position of trunk point `idx`.
	for b in 0..3u32 {
		let br_age = |j: u32| m.rd16(va(0x1e02d0) + 0x20 * b + 2 * j) as i16;
		let tab = va(0x1dffd0) + 0x100 * b;
		let idx = m.rd32(va(0x161cb8) + 4 * b) as i32;
		if !(0..40).contains(&idx) {
			continue;
		}
		let mut prev = cols(alpha(br_age(0)));
		let mut w = (w0 + (w1 - w0) * curve(idx as f32 / 40.0)) * 0.75;
		let mut s = rd4(m, va(0x1dfd00) + 16 * idx as u32);
		let mut e = world(s);
		s[0] += m.rdf(tab);
		s[2] += m.rdf(tab + 8);
		let mut wp = world(s);
		let mt = |row3: V4| [frame[0], frame[1], frame[2], row3];
		let tg = |w: f32| [tpl(w), tpl(w * 4.0)];
		let [t, g] = tg(w);
		let mm = mt(e);
		let mut ring = [mul_mat(t[0], &mm), mul_mat(t[1], &mm), mul_mat(g[0], &mm), mul_mat(g[1], &mm)];
		for j in 0..15u32 {
			let [t, g] = tg(w);
			e = wp;
			s[0] += m.rdf(tab + 16 * (j + 1));
			s[2] += m.rdf(tab + 16 * (j + 1) + 8);
			wp = world(s);
			let mm = mt(e);
			let cur = [mul_mat(t[0], &mm), mul_mat(t[1], &mm), mul_mat(g[0], &mm), mul_mat(g[1], &mm)];
			let cc = cols(alpha(br_age(j + 1)));
			seg([ring[0], ring[1], cur[0], cur[1]], [ring[2], ring[3], cur[2], cur[3]], prev, cc);
			ring = cur;
			prev = cc;
			w *= 0.95;
		}
	}
}

// ---- Drek's Fleet (level 17) hangar water: the queue-B callbacks FUN_002efdf8 / 002f09f0 / 002f0270 / 002f0630 ----
//
// [C: overlay C 17_veldin_orbit 170735-171490, gp 0x166c00, gp-relative words = 0x156c00 + 0xbXXX; decomp/notes/fx_census_late.md]
// Each callback draws one water surface (class 1405 / 1426 mobys; the two siblings are not queued in the shot field) with GS state
// TEX0 = fx table entry (gp word), ALPHA (FIX << 32) | 0x44 (C = As), CLAMP 0, TEX1 0xff9000000260; the frame's TEST 0x5380b stays
// (alpha test GEQUAL 0x80 with AFAIL RGB_ONLY: the surface blends and writes no z). Per base vertex (x, y, z):
//   t = fmod(vsqrt(x x + y y), L); a = angle_add(wrap(t 2pi / L), phase); pos = (x + mx, y + my, z + mz + amp vu0_sin(a))
// phase = the moby pvar word +4 (the callback has already advanced it when the field's RAM is read). Vertex colour = the moby bytes
// +0x3c..0x3e as r g b, alpha = the gp alpha word (0x40). TWO passes over the same positions, pass p scrolling the base uv by
// fmod(frames speed dt s[2p], 1) / fmod(frames speed dt s[2p+1], 1) (s = four floats after the callback's gp block); each pass
// submits every set as a prog10 strip (FUN_001faa30, fogged), triangles (i, i+1, i+2).

/// Where a set's base table lives: a pointer in a gp slot or a fixed address.
#[derive(Clone, Copy)]
enum Base {
	Slot(u32),
	At(u32),
}

/// One water callback: (function, FNV-1a of its first 0x100 bytes, sets (count word, base xyz, base uv), fix, tex, alpha, L, amp, speed, scroll).
struct WaterFn {
	f: u32,
	hash: u32,
	sets: &'static [(u32, Base, Base)],
	words: [u32; 6],
	scroll: u32,
}

const FLEET_LEVEL: u32 = 17;
const WATER_FN: [WaterFn; 4] = [
	WaterFn {
		f: 0x2efdf8,
		hash: 0xb452_b8ea,
		sets: &[(0x162190, Base::Slot(0x1621b0), Base::Slot(0x1621b8)), (0x162194, Base::Slot(0x1621b4), Base::Slot(0x1621bc))],
		words: [0x1621c8, 0x1621cc, 0x1621d0, 0x1621d4, 0x1621dc, 0x1621e0],
		scroll: 0x1621e8,
	},
	WaterFn { f: 0x2f09f0, hash: 0x1a85_106f, sets: &[(0x1622c0, Base::At(0x1dd660), Base::At(0x1dd880))], words: [0x1622f4, 0x1622f8, 0x1622fc, 0x162300, 0x162308, 0x16230c], scroll: 0x162310 },
	WaterFn { f: 0x2f0270, hash: 0xa7dd_15b0, sets: &[(0x162200, Base::At(0x1dbd20), Base::At(0x1dc150))], words: [0x162234, 0x162238, 0x16223c, 0x162240, 0x162248, 0x16224c], scroll: 0x162250 },
	WaterFn { f: 0x2f0630, hash: 0x7bc0_b2b0, sets: &[(0x162260, Base::At(0x1dcde0), Base::At(0x1dd000))], words: [0x162294, 0x162298, 0x16229c, 0x1622a0, 0x1622a8, 0x1622ac], scroll: 0x1622b0 },
];
/// `WATER_FN` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const WATER_FN_USA: [WaterFn; 4] = [
	WaterFn {
		f: 0x2f04d0,
		hash: 0xb452_b8ea,
		sets: &[(0x162210, Base::Slot(0x162230), Base::Slot(0x162238)), (0x162214, Base::Slot(0x162234), Base::Slot(0x16223c))],
		words: [0x162248, 0x16224c, 0x162250, 0x162254, 0x16225c, 0x162260],
		scroll: 0x162268,
	},
	WaterFn { f: 0x2f10c8, hash: 0x1a85_106f, sets: &[(0x162340, Base::At(0x1dd760), Base::At(0x1dd980))], words: [0x162374, 0x162378, 0x16237c, 0x162380, 0x162388, 0x16238c], scroll: 0x162390 },
	WaterFn { f: 0x2f0948, hash: 0xa7dd_15b0, sets: &[(0x162280, Base::At(0x1dbe20), Base::At(0x1dc250))], words: [0x1622b4, 0x1622b8, 0x1622bc, 0x1622c0, 0x1622c8, 0x1622cc], scroll: 0x1622d0 },
	WaterFn { f: 0x2f0d08, hash: 0x7bc0_b2b0, sets: &[(0x1622e0, Base::At(0x1dcee0), Base::At(0x1dd100))], words: [0x162314, 0x162318, 0x16231c, 0x162320, 0x162328, 0x16232c], scroll: 0x162330 },
];

/// FUN_001fce38: `(q - trunc q) * b` with `q = a / b` (VDIV, VFTOI0 / VITOF0).
fn fmod_vu(a: f32, b: f32) -> f32 {
	let q = crate::physics::vdiv(a, b);
	(q - q.trunc()) * b
}

/// FUN_001fcd50 / FUN_001fccc0: wrap to [-pi, pi) (loops, then one step for angle_add).
fn wrap_pi(mut a: f32) -> f32 {
	const PI: f32 = 3.1415927;
	while a >= PI {
		a = (a - PI) - PI;
	}
	while a < -PI {
		a = a + PI + PI;
	}
	a
}

fn angle_add(a: f32, b: f32) -> f32 {
	const PI: f32 = 3.1415927;
	let mut x = a + b;
	let low = x < -PI;
	if PI <= x {
		x = (x - PI) - PI;
	}
	if low {
		x = x + PI + PI;
	}
	x
}

/// Hook (retail.rs `queue_fx`): the hangar water of the queue-B callbacks, into the fogged alpha quads (`RetailFx.qw`).
pub fn fleet_water(m: &Machine, level: u32, out: &mut Vec<WorldQuad>) {
	// Silent probe: RAC_FLEET_WATER_OFF=1 leaves the hangar water out (before / after pictures).
	if level != FLEET_LEVEL || std::env::var_os("RAC_FLEET_WATER_OFF").is_some() {
		return;
	}
	for (f, arg) in super::queue_b::entries(m, level, true) {
		let Some(w) = (*crate::retail::disc::pick(&WATER_FN, &WATER_FN_USA)).iter().find(|w| w.f == f) else { continue };
		if super::queue_q1::code_hash(m, f) == w.hash && arg != 0 {
			water(m, w, arg, out);
		}
	}
}

fn water(m: &Machine, w: &WaterFn, moby: u32, out: &mut Vec<WorldQuad>) {
	let phys = |a: u32| a & 0x01ff_ffff;
	let [_fix, tex, alpha, period, amp, speed] = w.words.map(|a| (m.rd32(a), m.rdf(a)));
	let pv = phys(m.rd32(moby + 0x78));
	if pv == 0 {
		return;
	}
	let phase = m.rdf(pv + 4);
	let word = m.rd32(moby + 0x3c);
	let (r, g, b) = ((word & 0xff) as f32, (word >> 8 & 0xff) as f32, (word >> 16 & 0xff) as f32);
	let colour = [r / 128.0, g / 128.0, b / 128.0, (alpha.0 & 0xff) as f32 / 128.0];
	let pos0 = m.rdv3(moby + 0x10);
	// frames * speed * dt, in that order (the callback's fVar19).
	let t = (m.rd32(va(0x15f5ac)) as i32 as f32) * speed.1 * m.rdf(0x15ed6c);
	let s: [f32; 4] = std::array::from_fn(|k| m.rdf(w.scroll + 4 * k as u32));
	let base = |b: Base| match b {
		Base::Slot(a) => phys(m.rd32(a)),
		Base::At(a) => a,
	};
	// Positions of each set (the same in both passes), with the base uv.
	let mut sets: Vec<(Vec<[f32; 3]>, Vec<[f32; 2]>)> = Vec::new();
	for &(count, xyz, uv) in w.sets {
		let n = (m.rd32(count) as i32).clamp(0, 0x200) as u32;
		let (xyz, uv) = (base(xyz), base(uv));
		if n < 3 || xyz == 0 || uv == 0 {
			sets.push((Vec::new(), Vec::new()));
			continue;
		}
		let pos = (0..n)
			.map(|j| {
				let p = m.rdv3(xyz + 12 * j);
				let q = crate::physics::vsqrt(p[0] * p[0] + p[1] * p[1]);
				let a = angle_add(wrap_pi(fmod_vu(q, period.1) * 6.2831855 / period.1), phase);
				[p[0] + pos0[0], p[1] + pos0[1], p[2] + pos0[2] + amp.1 * crate::camera::fast_sin(a)]
			})
			.collect();
		sets.push((pos, (0..n).map(|j| [m.rdf(uv + 8 * j), m.rdf(uv + 8 * j + 4)]).collect()));
	}
	let tex = tex.0 as usize;
	for pass in 0..2 {
		for (pos, base_uv) in &sets {
			let uv: Vec<[f32; 2]> = base_uv.iter().map(|b| [b[0] + fmod_vu(t * s[2 * pass], 1.0), b[1] + fmod_vu(t * s[2 * pass + 1], 1.0)]).collect();
			for i in 0..pos.len().saturating_sub(2) {
				let (a, b, c) = (i, i + 1, i + 2);
				out.push(WorldQuad { pos: [pos[a], pos[b], pos[c], pos[c]], uv: [uv[a], uv[b], uv[c], uv[c]], colour: [colour; 4], tex, additive: false });
			}
		}
	}
}

/// Effects layer claimed set (retail/efx_draw.rs): the callbacks `fill_b` / `fleet_water` draw.
pub fn claims(level: u32, f: u32) -> bool {
	(SPARK_ENABLED && (*crate::retail::disc::pick(&SPARK_FN, &SPARK_FN_USA)).contains(&(level, f))) || (level == BOLT_LEVEL && f == va(BOLT_FN)) || (level == FLEET_LEVEL && (*crate::retail::disc::pick(&WATER_FN, &WATER_FN_USA)).iter().any(|w| w.f == f))
}
