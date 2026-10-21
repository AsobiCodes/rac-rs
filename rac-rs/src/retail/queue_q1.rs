//! Queue-A callbacks of the early planets that the mirror did not draw (census rows 6-8, decomp/notes/draw_producers.md):
//! the class 1060 "red orb" glows (Veldin, Aridia, Rilgar), the Aridia / Gaspar / Orxon transport glows (classes 1212 /
//! 1213) and the Novalis strip surfaces (classes 676 / 678 / 761 / 1225).
//!
//! Like `queue_b`, every callback is found by its queue-A entry and identified by its address in a per-level twin table
//! plus a check of its own code in RAM; the mirror snapshot holds the draw-time state (retail's strip drawer advances its
//! scroll words before it draws, so the record words are what the draw used).

use super::draw_queue;
use crate::particles::WorldStrip;
use ee_oracle::Machine;
use crate::retail::disc::va;

/// Mask a moby / pvar pointer to a physical address.
const fn phys(a: u32) -> u32 {
	a & 0x01ff_ffff
}

/// (callback, moby) of every live queue-A entry.
fn entries(m: &Machine, level: u32) -> Vec<(u32, u32)> {
	let Some((fns, args)) = draw_queue(level) else { return Vec::new() };
	(0..m.rd32(0x15f424).min(0x40)).map(|i| (m.rd32(fns + 4 * i), phys(m.rd32(args + 4 * i)))).collect()
}

/// Whether the function at `f` holds a `lui` with immediate `hi` in its first 0x100 bytes (the float constants the
/// callbacks pass: the twins are binary equal, so the immediates identify the body).
fn has_lui(m: &Machine, f: u32, hi: u32) -> bool {
	(0..0x40).any(|k| {
		let w = m.rd32(f + 4 * k);
		w >> 26 == 0x0f && w & 0xffff == hi
	})
}

/// The target of the `jal` at `at`.
fn jal_to(m: &Machine, at: u32) -> Option<u32> {
	let w = m.rd32(at);
	(w >> 26 == 3).then(|| (at & 0xf000_0000) | (w & 0x03ff_ffff) << 2)
}

/// The mobys a callback scans: retail walks the type list of the argument moby's byte +0x21 (FUN_00258de0 with the
/// dead flag clear: byte +0x20 sign bit 0) and keeps the visible ones (+0x31 != 0) of the argument's class (+0xa6).
fn same_class(m: &Machine, level: u32, arg: u32) -> Vec<u32> {
	let (s, e) = super::table(level);
	let (s, e) = (phys(m.rd32(s)), phys(m.rd32(e)));
	let (kind, class) = (m.rd8(arg + 0x21), m.rd16(arg + 0xa6));
	(s..e)
		.step_by(0x100)
		.filter(|&a| m.rd8(a + 0x21) == kind && (m.rd8(a + 0x20) as i8) >= 0 && m.rd8(a + 0x31) != 0 && m.rd16(a + 0xa6) == class)
		.collect()
}

/// Class 1060 glow callback (Veldin FUN_002ded80, queued by FUN_002dee50; binary twins Aridia 0x2ea408, Rilgar 0x319388)
/// [C: overlay.elf.c 157274-157311]: for each visible class-1060 moby, FUN_00263228(scale, 0.5, moby +0x10, pvar +4):
/// one additive camera-facing glow quad (the hero-glow primitive). scale = 1.0 when the level id 0x15ed84 is 0 or 0x12
/// (Veldin), else 1.2. Veldin 2 (level 18, id 0x12) FUN_002ec310 is a binary twin (the red orbs of the finale, effects census E9).
const ORB_FN: [(u32, u32); 4] = [(0, 0x2ded80), (2, 0x2ea408), (5, 0x319388), (18, 0x2ec310)];
/// `ORB_FN` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const ORB_FN_USA: [(u32, u32); 4] = [(0, 0x2df3d8), (2, 0x2eaad8), (5, 0x3199c8), (18, 0x2ec9e0)];

/// Transport glow callbacks (Aridia FUN_002ebc38 class 1212 / FUN_002ec598 class 1213, binary twins Gaspar 0x305538 /
/// 0x305e98 and Orxon 0x2eb280 for 1213) [C: overlay.elf.c 157091-157143, 157507-157559]: for each visible same-class
/// moby whose pvar colour word (+0xb0 / +0xe0) has a low byte above 0x80, FUN_002646e8 (= FUN_00263228) at the `joints`
/// joint records pvar +0x80 + 0x10 k with that word as colour. (level, callback, joints, colour offset, size hi, bias hi).
const TRANSPORT_FN: [(u32, u32, u32, u32, u32, u32); 5] = [
	(2, 0x2ebc38, 3, 0xb0, 0x3f66, 0x3ee6),
	(2, 0x2ec598, 6, 0xe0, 0x3f99, 0x3f11),
	(9, 0x305538, 3, 0xb0, 0x3f66, 0x3ee6),
	(9, 0x305e98, 6, 0xe0, 0x3f99, 0x3f11),
	(10, 0x2eb280, 6, 0xe0, 0x3f99, 0x3f11),
];
/// `TRANSPORT_FN` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const TRANSPORT_FN_USA: [(u32, u32, u32, u32, u32, u32); 5] = [
	(2, 0x2ec308, 3, 0xb0, 0x3f66, 0x3ee6),
	(2, 0x2ecc68, 6, 0xe0, 0x3f99, 0x3f11),
	(9, 0x305c10, 3, 0xb0, 0x3f66, 0x3ee6),
	(9, 0x306570, 6, 0xe0, 0x3f99, 0x3f11),
	(10, 0x2eb958, 6, 0xe0, 0x3f99, 0x3f11),
];

/// Novalis strip-surface callbacks: `FUN_002b8ff0(); FUN_002b9010(count, table)` (676 x8 at 0x1e2ec0, 678 x2 at 0x1e3280,
/// 761 x4 at 0x1fbb80, 1225 x3 at 0x202c80) [C: overlay.elf.c 152240, 152250, 157743, 165192]. (callback, count, table).
const STRIP_FN: [(u32, usize, u32); 4] = [(0x2f5a28, 8, 0x1e2ec0), (0x2f5a80, 2, 0x1e3280), (0x2fe458, 4, 0x1fbb80), (0x309528, 3, 0x202c80)];
/// `STRIP_FN` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const STRIP_FN_USA: [(u32, usize, u32); 4] = [(0x2f60f8, 8, 0x1e2fc0), (0x2f6150, 2, 0x1e3380), (0x2feb28, 4, 0x1fbc80), (0x309bf8, 3, 0x202d80)];
/// Novalis strip drawer FUN_002b9010.
const STRIP_DRAW: u32 = 0x2b9010;
const NOVALIS: u32 = 1;

/// The class-1225 pulse I = 0.5 + 0.5 sin((float(tick & 0x3f) - 32) * 0.09817477) the callback writes after drawing.
fn pulse_1225(tick: u32) -> f32 {
	crate::strips::pulse_1225(tick)
}


/// Hook (retail.rs `queue_fx`): the queue-A callbacks above. Glows go to `glows` (pos, half-size, bias, rgba) like the hero
/// glow, strips to `strips`.
pub fn fill(m: &Machine, level: u32, glows: &mut Vec<([f32; 3], f32, f32, u32)>, strips: &mut Vec<WorldStrip>, pools: &mut Vec<WorldStrip>) {
	let mut done: Vec<u32> = Vec::new();
	for (f, arg) in entries(m, level) {
		// The glass patches take no argument and draw their fixed tables once per callback.
		if let Some(&(_, _, hash, patches)) = (*crate::retail::disc::pick(&GLASS_FN, &GLASS_FN_USA)).iter().find(|g| g.0 == level && g.1 == f) {
			if !done.contains(&f) {
				done.push(f);
				if code_hash(m, f) == hash {
					glass(m, level, f, patches, strips);
				}
			}
			continue;
		}
		// A callback scans a whole class: one scan per callback even when several entries carry it.
		if arg == 0 || done.contains(&f) {
			continue;
		}
		if (*crate::retail::disc::pick(&ORB_FN, &ORB_FN_USA)).contains(&(level, f)) && has_lui(m, f, 0x3f00) {
			done.push(f);
			orbs(m, level, arg, glows);
		} else if let Some(&(_, _, joints, off, size, bias)) = (*crate::retail::disc::pick(&TRANSPORT_FN, &TRANSPORT_FN_USA)).iter().find(|t| t.0 == level && t.1 == f) {
			done.push(f);
			if has_lui(m, f, size) && has_lui(m, f, bias) {
				transports(m, level, arg, joints, off, glows);
			}
		} else if level == NOVALIS {
			if let Some(&(_, count, table)) = (*crate::retail::disc::pick(&STRIP_FN, &STRIP_FN_USA)).iter().find(|t| t.0 == f) {
				done.push(f);
				if (0..16).any(|k| jal_to(m, f + 4 * k) == Some(va(STRIP_DRAW))) {
					surfaces(m, f, count, table, strips);
				}
			}
		}
	}
	if level == HOVEN {
		hoven_pools(m, pools);
	}
}

/// Hoven (level 12): the water pools of the queue-C callback FUN_002e6ae0 [C: overlay.elf.c 141878-141892, 116328-116368,
/// disassembly of 0x266394 / 0x222d20 in RAM; decomp/notes/fx_census_late.md]. Queue C = count 0x15f42c, callbacks 0x16e080 (FUN_0021be18,
/// queued by the class update FUN_002e6b38); the callback is FUN_002bbb40(0x1f5380, 47, TEX0 of fx 0x2c, TEX0 of fx 0x2d), whose GS
/// block 0x1cb6a0 sets TEST 0x50000, CLAMP 0, ALPHA_1 FIX 0x80, ALPHA_2 FIX 0x40, and whose vertex colours are 0x80808080. Per
/// 0x20-byte record (+0 bounding sphere, +0x10 positions 12 B, +0x14 uv 8 B, +0x18 normals 12 B, +0x1c count): FUN_00222d20(256.0,
/// record) == -1 skips it, else FUN_00266394 builds the sphere-map uv and FUN_00220588 (Novalis strip_submit2 FUN_0021f5c8) draws the
/// strip twice: context 1 with the table uv, context 2 with the sphere-map uv. (function, FNV-1a of its first 0x100 bytes).
const HOVEN: u32 = 12;
const POOL_FN: [(u32, u32); 4] = [(0x2e6ae0, 0x012d_e745), (0x2bbb40, 0x85ef_dfcc), (0x266394, 0x5f5b_76c1), (0x222d20, 0xe462_ea3a)];
/// `POOL_FN` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const POOL_FN_USA: [(u32, u32); 4] = [(0x2e71b0, 0x012d_e745), (0x2bc210, 0x85ef_dfcc), (0x2667fc, 0x5f5b_76c1), (0x2231f0, 0xe462_ea3a)];
/// (record table, records, fx texture of pass 1 / 2, FIX of pass 1 / 2, camera vec4 of FUN_00266394, cull far).
const POOLS: (u32, u32, [usize; 2], [u8; 2], u32, f32) = (0x1f5380, 0x2f, [0x2c, 0x2d], [0x80, 0x40], 0x1671c0, 256.0);
/// `POOLS` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const POOLS_USA: (u32, u32, [usize; 2], [u8; 2], u32, f32) = (0x1f5480, 0x2f, [0x2c, 0x2d], [0x80, 0x40], 0x167240, 256.0);

fn hoven_pools(m: &Machine, out: &mut Vec<WorldStrip>) {
	let queued = (0..m.rd32(0x15f42c).min(0x40)).any(|i| m.rd32(va(0x16e080) + 4 * i) == (*crate::retail::disc::pick(&POOL_FN, &POOL_FN_USA))[0].0);
	if !queued || (*crate::retail::disc::pick(&POOL_FN, &POOL_FN_USA)).iter().any(|&(f, h)| code_hash(m, f) != h) {
		return;
	}
	let (table, count, tex, fix, cam, far) = *crate::retail::disc::pick(&POOLS, &POOLS_USA);
	let cam = m.rdv3(cam);
	for k in 0..count {
		let r = table + 0x20 * k;
		let n = m.rd32(r + 0x1c) as usize;
		let (vp, up, np) = (phys(m.rd32(r + 0x10)), phys(m.rd32(r + 0x14)), phys(m.rd32(r + 0x18)));
		// FUN_002bbb40 fills 0xb4 colour words: the strips stay within them.
		if n < 3 || n > 0xb4 || vp == 0 || up == 0 || np == 0 {
			continue;
		}
		if sphere_visible(m, far, [m.rdf(r), m.rdf(r + 4), m.rdf(r + 8), m.rdf(r + 0xc)]) < 0 {
			continue;
		}
		let pos: Vec<[f32; 3]> = (0..n as u32).map(|j| m.rdv3(vp + 12 * j)).collect();
		let uv_a = (0..n as u32).map(|j| [m.rdf(up + 8 * j), m.rdf(up + 8 * j + 4)]).collect();
		let uv_b = (0..n as u32).map(|j| pool_uv(pos[j as usize], m.rdv3(np + 12 * j), cam)).collect();
		let colour = vec![[1.0, 1.0, 1.0, 1.0]; n];
		out.push(WorldStrip { pos: pos.clone(), uv: uv_a, colour: colour.clone(), tex: tex[0], fix: fix[0], add: false });
		out.push(WorldStrip { pos, uv: uv_b, colour, tex: tex[1], fix: fix[1], add: false });
	}
}

/// FUN_00266394 (Hoven) per vertex V with normal N: s = (cam - V) * 0.45 / |cam - V| (VRSQRT), i = 0 - s, d = i . N; d > 0 keeps
/// s, else i + 2 (N d - i); uv = xy + 0.5. Dot products as the VU sums them: (x x + y y) + z z.
fn pool_uv(v: [f32; 3], n: [f32; 3], cam: [f32; 3]) -> [f32; 2] {
	let e = [cam[0] - v[0], cam[1] - v[1], cam[2] - v[2]];
	let q = f32::from_bits(0x3ee6_6666) / ((e[0] * e[0] + e[1] * e[1]) + e[2] * e[2]).sqrt();
	let s = e.map(|x| x * q);
	let i = s.map(|x| 0.0 - x);
	let d = (i[0] * n[0] + i[1] * n[1]) + i[2] * n[2];
	let r = if d > 0.0 {
		[s[0], s[1]]
	} else {
		let t = [n[0] * d - i[0], n[1] * d - i[1]];
		[i[0] + (t[0] + t[0]), i[1] + (t[1] + t[1])]
	};
	[r[0] + 0.5, r[1] + 0.5]
}

/// FUN_00222d20 (Hoven sphere_visible; frustum block 0x16d0c0) on sphere `s` (x, y, z, r): -1 rejects, 0 straddles, 1 inside.
/// k = block +0x3c; p = s * k; v = view rows (+0, +0x10, +0x20) applied to p.xyz - block +0x30; reject when z + r <= 0 (sign of
/// 0 - (z + r) clear), far * k - (z - r) < 0, or a side test a.c z - (|c| - b.c r) < 0 (c = x, y; a = +0x40, b = +0x60).
fn sphere_visible(m: &Machine, far: f32, s: [f32; 4]) -> i32 {
	let q = |a: u32| [m.rdf(a), m.rdf(a + 4), m.rdf(a + 8), m.rdf(a + 0xc)];
	let b = va(0x16d0c0u32);
	let (r0, r1, r2, c, a, w, h, g) = (q(b), q(b + 0x10), q(b + 0x20), q(b + 0x30), q(b + 0x40), q(b + 0x50), q(b + 0x60), q(b + 0x70));
	let k = c[3];
	let p = s.map(|x| x * k);
	let d = [p[0] - c[0], p[1] - c[1], p[2] - c[2]];
	let v: [f32; 3] = std::array::from_fn(|i| r0[i] * d[0] + r1[i] * d[1] + r2[i] * d[2]);
	let rad = p[3];
	let zz = [rad + v[2], (0.0 - rad) + v[2]];
	let far_k = far * k;
	let behind = 0.0 - zz[0];
	let beyond = far_k - zz[1];
	let side: [f32; 2] = std::array::from_fn(|i| a[i] * v[2] - (v[i].abs() - h[i] * rad));
	if beyond.is_sign_negative() || !behind.is_sign_negative() || side[0].is_sign_negative() || side[1].is_sign_negative() {
		return -1;
	}
	let outer: [f32; 2] = std::array::from_fn(|i| a[i] * v[2] * w[i] - (v[i].abs() + h[i] * rad * h[2]));
	let near = [zz[0] - g[0], zz[1] - g[1]];
	if outer[1].is_sign_negative() || near[1].is_sign_negative() || outer[0].is_sign_negative() || !near[0].is_sign_negative() {
		0
	} else {
		1
	}
}

/// Environment-mapped glass patches (Kalebo III FUN_002e8650 / 2e8b80 / 2e90b0 / 2e98f8, Veldin 2 hangar curtain FUN_002f4fb8 /
/// 2f52d0 / 2f55e8 / 2f5900 / 2f5c18 / 2f5f30, effects census E2 / E3) [C: overlay.elf.c 165950-165995, 174088-174137, disassembly of
/// the RAM words]. Each callback sets the GS (fx texture, ALPHA = A Cs, B Cd, C FIX, D Cd: 0x40 on Kalebo, 0x20 on the Fleet), then for
/// every patch k (sphere test left to the GPU clip) builds the sphere-map UV of its vertices and submits them as one strip
/// (FUN_001fc230 / FUN_001faa30 = the strip submit the Novalis strip surfaces use). Five gp slots (the first five `addiu rt, gp, imm`
/// of the callback, in this order): colour pointers, unused, normal pointers, position pointers, vertex counts, one word per patch
/// (word stride 4 x patches). (level, callback, FNV-1a of its first 0x100 bytes, patches).
const GLASS_FN: [(u32, u32, u32, u32); 10] = [
	(16, 0x2e8650, 0x9c28a011, 3),
	(16, 0x2e8b80, 0x7e953950, 1),
	(16, 0x2e90b0, 0xea65985b, 2),
	(16, 0x2e98f8, 0x72a3ca4d, 3),
	(17, 0x2f4fb8, 0xeb2f8f15, 2),
	(17, 0x2f52d0, 0xd9485226, 1),
	(17, 0x2f55e8, 0xd857cb0f, 2),
	(17, 0x2f5900, 0xbff301aa, 1),
	(17, 0x2f5c18, 0xe5313eba, 2),
	(17, 0x2f5f30, 0x4aaf107f, 1),
];
/// `GLASS_FN` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const GLASS_FN_USA: [(u32, u32, u32, u32); 10] = [
	(16, 0x2e8d40, 0x9c28a011, 3),
	(16, 0x2e9270, 0x7e953950, 1),
	(16, 0x2e97a0, 0xea65985b, 2),
	(16, 0x2e9fe8, 0x72a3ca4d, 3),
	(17, 0x2f5690, 0xeb2f8f15, 2),
	(17, 0x2f59a8, 0xd9485226, 1),
	(17, 0x2f5cc0, 0xd857cb0f, 2),
	(17, 0x2f5fd8, 0xbff301aa, 1),
	(17, 0x2f62f0, 0xe5313eba, 2),
	(17, 0x2f6608, 0x4aaf107f, 1),
];

/// FNV-1a over the first 0x100 bytes (0x40 words) of the function at `f`.
pub(super) fn code_hash(m: &Machine, f: u32) -> u32 {
	let mut h = 0x811c_9dc5u32;
	for k in 0..0x40 {
		for b in m.rd32(f + 4 * k).to_le_bytes() {
			h = (h ^ b as u32).wrapping_mul(0x0100_0193);
		}
	}
	h
}

/// (fx texture, GS FIX, UV scale, camera position address) of the glass of `level`: the camera is the vec4 the UV builder subtracts
/// (Kalebo 0x167140, Fleet 0x167640), the scale 2.0 / 4.0 its `(x / 2L + 0.5) * scale`.
fn glass_level(level: u32) -> Option<(usize, u8, f32, u32)> {
	match level {
		16 => Some((0x29, 0x40, 2.0, va(0x167140))),
		17 => Some((0x30, 0x20, 4.0, va(0x167640))),
		_ => None,
	}
}

/// The fx textures of `glass_level` (sampled without mips, `particles::FX_NOMIP`).
pub const GLASS_TEX: [usize; 2] = [0x29, 0x30];

/// FUN_001fc0d8: `v * len / |v|`, the zero vector stays zero.
fn set_len(v: [f32; 3], len: f32) -> [f32; 3] {
	let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
	if n == 0.0 { [0.0; 3] } else { v.map(|x| x * len / n) }
}

/// The sphere-map UV of the glass vertex builders (Kalebo FUN_002e84a8, Fleet FUN_002f4e00): e = unit(V - cam), N = unit(N),
/// r = unit(e - 2 (N . e) N), r.z += 1, (r.x / 2|r| + 0.5) * scale and the same for y.
fn glass_uv(v: [f32; 3], n: [f32; 3], cam: [f32; 3], scale: f32) -> [f32; 2] {
	let e = set_len([v[0] - cam[0], v[1] - cam[1], v[2] - cam[2]], 1.0);
	let n = set_len(n, 1.0);
	let d = n[0] * e[0] + n[1] * e[1] + n[2] * e[2];
	let mut r = set_len([e[0] - n[0] * (d + d), e[1] - n[1] * (d + d), e[2] - n[2] * (d + d)], 1.0);
	r[2] += 1.0;
	let l = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt();
	[(r[0] / (l + l) + 0.5) * scale, (r[1] / (l + l) + 0.5) * scale]
}

/// One glass callback (see `GLASS_FN`).
fn glass(m: &Machine, level: u32, f: u32, patches: u32, out: &mut Vec<WorldStrip>) {
	let Some((tex, fix, scale, cam)) = glass_level(level) else { return };
	let gp = 0x166c00u32; // regs.bin of every level holds gp = 0x166c00 (see queue_q2::gp)
	let slots: Vec<u32> = (0..0x30u32)
		.filter_map(|k| {
			let w = m.rd32(f + 4 * k);
			(w >> 26 == 9 && (w >> 21 & 31) == 28).then(|| gp.wrapping_add(w as i16 as i32 as u32))
		})
		.take(5)
		.collect();
	let &[colours, _, normals, positions, counts] = slots.as_slice() else { return };
	let cam = m.rdv3(cam);
	for k in 0..patches {
		let n = m.rd32(counts + 4 * k) as usize;
		let (pp, np, cp) = (phys(m.rd32(positions + 4 * k)), phys(m.rd32(normals + 4 * k)), phys(m.rd32(colours + 4 * k)));
		if n < 3 || n > 4096 || pp == 0 || np == 0 || cp == 0 {
			continue;
		}
		let pos: Vec<[f32; 3]> = (0..n as u32).map(|j| m.rdv3(pp + 12 * j)).collect();
		let uv = (0..n as u32).map(|j| glass_uv(pos[j as usize], m.rdv3(np + 12 * j), cam, scale)).collect();
		let colour = (0..n as u32).map(|j| [0, 8, 16].map(|s| ((m.rd32(cp + 4 * j) >> s) & 0xff) as f32 / 128.0)).map(|c| [c[0], c[1], c[2], 0.0]).collect();
		out.push(WorldStrip { pos, uv, colour, tex: tex | crate::particles::FX_NOMIP, fix, add: false });
	}
}

/// Class 1060 (see `ORB_FN`).
fn orbs(m: &Machine, level: u32, arg: u32, out: &mut Vec<([f32; 3], f32, f32, u32)>) {
	let level_id = m.rd32(0x15ed84);
	let size = if level_id == 0 || level_id == 0x12 { 1.0 } else { f32::from_bits(0x3f99_999a) };
	for a in same_class(m, level, arg) {
		let pv = phys(m.rd32(a + 0x78));
		if pv != 0 {
			out.push((m.rdv3(a + 0x10), size, 0.5, m.rd32(pv + 4)));
		}
	}
}

/// Classes 1212 / 1213 (see `TRANSPORT_FN`).
fn transports(m: &Machine, level: u32, arg: u32, joints: u32, off: u32, out: &mut Vec<([f32; 3], f32, f32, u32)>) {
	let (size, bias) = transport_consts(joints);
	for a in same_class(m, level, arg) {
		let pv = phys(m.rd32(a + 0x78));
		if pv == 0 || m.rd8(pv + off) <= 0x80 {
			continue;
		}
		let word = m.rd32(pv + off);
		for k in 0..joints {
			out.push((m.rdv3(pv + 0x80 + 0x10 * k), size, bias, word));
		}
	}
}

/// (size, bias) of the transport glows: 1212 (3 joints) 0.9 / 0.45 = 0x3f666666 / 0x3ee66666, 1213 (6 joints) 1.2 / 0.57 =
/// 0x3f99999a / 0x3f11eb85 [C: the callbacks' lui/ori immediates].
fn transport_consts(joints: u32) -> (f32, f32) {
	if joints == 3 { (f32::from_bits(0x3f66_6666), f32::from_bits(0x3ee6_6666)) } else { (f32::from_bits(0x3f99_999a), f32::from_bits(0x3f11_eb85)) }
}

/// One strip record of the table at `r` as retail RAM holds it, with the scroll words (+0x40..+0x4c), the wave phase
/// (+0x50) and the pulse (+0x5c).
fn record(m: &Machine, r: u32) -> Option<(crate::strips::StripRec, [f32; 4], u32, f32)> {
	let n = m.rd32(r + 0xc) as usize;
	if n < 3 || n > 4096 {
		return None;
	}
	let (vp, up) = (phys(m.rd32(r)), phys(m.rd32(r + 4)));
	if vp == 0 || up == 0 {
		return None;
	}
	let verts = (0..n as u32).map(|j| [m.rdf(vp + 16 * j), m.rdf(vp + 16 * j + 4), m.rdf(vp + 16 * j + 8), m.rdf(vp + 16 * j + 12)]).collect();
	let uv = (0..n as u32).map(|j| [m.rdf(up + 8 * j), m.rdf(up + 8 * j + 4)]).collect();
	let rec = crate::strips::StripRec {
		verts,
		uv,
		sphere: [0.0; 4],
		gain: [m.rdf(r + 0x10), m.rdf(r + 0x14)],
		amp: m.rdf(r + 0x18),
		step: m.rd32(r + 0x1c),
		rate: [m.rdf(r + 0x20), m.rdf(r + 0x24), m.rdf(r + 0x28), m.rdf(r + 0x2c)],
		colour: m.rd32(r + 0x30),
		tex: [m.rd32(r + 0x34) as usize, m.rd32(r + 0x38) as usize],
		fix: [m.rd8(r + 0x3c), m.rd8(r + 0x3d)],
	};
	let scroll = [m.rdf(r + 0x40), m.rdf(r + 0x44), m.rdf(r + 0x48), m.rdf(r + 0x4c)];
	Some((rec, scroll, m.rd32(r + 0x50) & 0xff_ffff, m.rdf(r + 0x5c)))
}

/// FUN_002b9010 over `count` records at `table` (each: scroll step, then two passes over the waved vertices; the frustum
/// sphere reject is left to the GPU clip). The strip vertex builder FUN_002621b0 takes z = w8 * I + wc * (1 - I) with I the
/// record's +0x5c; the class-1225 callback rewrites I after drawing, so the draw used the value written one call earlier:
/// the stored word is I(frame counter at the write), found against the snapshot's counter, and the draw's I is the one
/// before it [I: one call per frame].
fn surfaces(m: &Machine, f: u32, count: usize, table: u32, out: &mut Vec<WorldStrip>) {
	let frames = m.rd32(va(0x15f5ac));
	for k in 0..count as u32 {
		let Some((rec, scroll, phase, stored)) = record(m, table + 0x60 * k) else { continue };
		let pulse = if f == va(0x309528) {
			(0..3u32).find(|d| stored == pulse_1225(frames.wrapping_sub(*d))).map_or(stored, |d| pulse_1225(frames.wrapping_sub(d + 1)))
		} else {
			stored
		};
		let (pos, a, b) = crate::strips::strip_wave(&rec, &scroll, phase, pulse);
		let c = [0, 8, 16].map(|s| ((rec.colour >> s) & 0xff) as f32 / 128.0);
		let colour = vec![[c[0], c[1], c[2], 0.0]; pos.len()];
		out.push(WorldStrip { pos: pos.clone(), uv: a, colour: colour.clone(), tex: rec.tex[0], fix: rec.fix[0], add: false });
		out.push(WorldStrip { pos, uv: b, colour, tex: rec.tex[1], fix: rec.fix[1], add: false });
	}
}

/// Ocean wrappers (census row 9, class 994 / 327 / 1260: Kerwan 0x2dc2c0, Batalia 0x2d9a00, Oltanis 0x302d00): ten
/// instructions `lui a0,hi; jal draw; addiu a0,a0,lo` = the ocean draw of the descriptor at (hi << 16) + lo [C: RAM words]. The
/// draw reads everything else from the descriptor (origin +0, radius^2 +0xc, spacing +0x10, cells +0x18, fog words +0x20..
/// +0x2c, FOGCOL +0x30, colour block +0x34, mask +0x38, first fx record +0x3c, count +0x3e, FIX +0x3f) and from three
/// per-level tables. (level, wrapper, ST table, strip index, fx table). Kerwan keeps the WAD-built ocean (`ocean.rs`
/// KERWAN): its fx table address is not decoded here.
pub(super) const OCEAN_FN: [(u32, u32, u32, u32, u32); 2] = [(8, 0x2d9a00, 0x1cb610, 0x1cb500, 0x16d908), (14, 0x302d00, 0x1cb590, 0x1cb480, 0x16d888)];
/// `OCEAN_FN` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
pub(super) const OCEAN_FN_USA: [(u32, u32, u32, u32, u32); 2] = [(8, 0x2da0d0, 0x1cb710, 0x1cb600, 0x16d988), (14, 0x303350, 0x1cb610, 0x1cb500, 0x16d888)];

/// Cache of the last ocean built: (key, ocean). The key holds the descriptor words and the first fx record, which pin the
/// level data in RAM (the tables are overlay data; the fx pool base 0x15f420 moves per level).
static OCEAN_CACHE: std::sync::Mutex<Option<(Vec<u32>, std::sync::Arc<crate::ocean::Ocean>)>> = std::sync::Mutex::new(None);

/// Hook (retail.rs `queue_fx`): the level's ocean while the wrapper is in queue A, built from the descriptor in RAM.
/// None on levels without an entry in `OCEAN_FN` and when the wrapper or descriptor does not look as decoded.
pub fn ocean(m: &Machine, level: u32) -> Option<std::sync::Arc<crate::ocean::Ocean>> {
	let &(_, wrapper, st_table, strip_index, fx_table) = (*crate::retail::disc::pick(&OCEAN_FN, &OCEAN_FN_USA)).iter().find(|o| o.0 == level)?;
	let (fns, _) = draw_queue(level)?;
	if !(0..m.rd32(0x15f424).min(0x40)).any(|i| m.rd32(fns + 4 * i) == wrapper) {
		return None;
	}
	let (lui, call, addiu) = (m.rd32(wrapper + 4), m.rd32(wrapper + 12), m.rd32(wrapper + 16));
	if lui >> 16 != 0x3c04 || call >> 26 != 3 || addiu >> 16 != 0x2484 {
		return None;
	}
	let d = (lui & 0xffff) << 16;
	let d = d.wrapping_add(addiu as u16 as i16 as i32 as u32);
	// The shared fog words and the 255 -> 0 intensity: what `fs_ocean_f` bakes in.
	if (m.rd32(d + 0x20), m.rd32(d + 0x24), m.rd32(d + 0x28), m.rd32(d + 0x2c)) != (0x461c_4000, 0x487d_fc00, 0x437f_0000, 0) {
		return None;
	}
	let (first, count, fix) = (m.rd16(d + 0x3c) as u32, m.rd8(d + 0x3e) as u32, m.rd8(d + 0x3f) as u8);
	let pool = phys(m.rd32(0x15f420));
	let mut key: Vec<u32> = vec![level, pool];
	key.extend((0..16).map(|k| m.rd32(d + 4 * k)));
	key.extend((0..4).map(|k| m.rd32(fx_table + first * 16 + 4 * k)));
	if let Some((k, o)) = OCEAN_CACHE.lock().ok()?.as_ref() {
		if *k == key {
			return Some(o.clone());
		}
	}
	let (cells, spacing) = ([m.rd16(d + 0x18) as u16, m.rd16(d + 0x1a) as u16], [m.rdf(d + 0x10), m.rdf(d + 0x14)]);
	let slots = cells[0].div_ceil(7) as u32 * cells[1].div_ceil(7) as u32;
	let (colours_at, mask_at) = (phys(m.rd32(d + 0x34)), phys(m.rd32(d + 0x38)));
	if slots == 0 || slots > 4096 || count == 0 || pool == 0 || colours_at == 0 || mask_at == 0 {
		return None;
	}
	let mask: Vec<u8> = (0..slots).map(|s| m.rd8(mask_at + s)).collect();
	let colours = (0..slots).flat_map(|s| (0..124u32).map(move |k| colours_at + s * 0x200 + 0x10 + k * 4)).map(|a| m.rd32(a).to_le_bytes()).collect();
	let desc = crate::ocean::Desc { origin: m.rdv3(d), spacing, cells, radius2: m.rdf(d + 0xc), mask, colours };
	let scale = f32::from_bits(0x3f2a_aaab);
	let st = std::array::from_fn(|k| [m.rdf(st_table + 8 * k as u32) * scale, m.rdf(st_table + 8 * k as u32 + 4) * scale]);
	let idx = std::array::from_fn(|k| (m.rd16(strip_index + 2 * k as u32) as u16 as i16 / 8) as u8);
	let frames = (0..count)
		.map(|k| {
			let rec = fx_table + (first + k) * 16;
			let (pix, pal) = (pool + m.rd16(rec) as u32 * 16, pool + m.rd16(rec + 2) as u32 * 16);
			let palette = std::array::from_fn(|v| m.rd32(pal + 4 * v as u32).to_le_bytes());
			(palette, (0..4096).map(|i| m.rd8(pix + i)).collect())
		})
		.collect();
	let fog = [m.rd8(d + 0x30), m.rd8(d + 0x31), m.rd8(d + 0x32)];
	let o = std::sync::Arc::new(crate::ocean::Ocean { groups: [vec![desc], Vec::new(), Vec::new()], cuboid: [-1; 3], st, idx, frames, fix, gated: false, first_frame: 0, fog: Some(fog) });
	*OCEAN_CACHE.lock().ok()? = Some((key, o.clone()));
	Some(o)
}

/// Effects layer claimed set (retail/efx_draw.rs): the callbacks `fill` draws.
pub fn claims(level: u32, f: u32) -> bool {
	(*crate::retail::disc::pick(&ORB_FN, &ORB_FN_USA)).contains(&(level, f)) || (*crate::retail::disc::pick(&TRANSPORT_FN, &TRANSPORT_FN_USA)).iter().any(|t| t.0 == level && t.1 == f) || (*crate::retail::disc::pick(&STRIP_FN, &STRIP_FN_USA)).iter().any(|t| t.0 == f) || (*crate::retail::disc::pick(&POOL_FN, &POOL_FN_USA)).iter().any(|t| t.0 == f) || (*crate::retail::disc::pick(&GLASS_FN, &GLASS_FN_USA)).iter().any(|g| g.0 == level && g.1 == f)
}
