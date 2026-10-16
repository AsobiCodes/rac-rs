//! Effects the mirror draws from rac-rs's OLD simulation state, read from retail RAM instead (lane "old").
//!
//! In retail mode `Game::update` never runs, so every draw state that only the old tick advanced stayed frozen: the
//! level frame counter (`Game.frames`: ocean texture blend, sky shell spin, waterfall, Clank pulse), the glint pool
//! (pickup twinkles) and the Aridia sky sprites. The retail interpreter already runs all of them; these readers copy
//! their state out of RAM once per field (`retail.rs queue_fx`), so the mirror draws exactly what retail holds.
//! Facts: decomp/notes/mirror_old_sim_fx.md [C].

use crate::game::Glint;
use crate::ocean::{Desc, STRIP_LEN};
use crate::sky::SkySprite;
use ee_oracle::Machine;

/// Frame counter (DAT_0015f5ac): +1 per sim tick at FUN_002ab290, the `Game.frames` the ported effects are keyed on [C].
pub const FRAME_COUNTER: u32 = 0x15f5ac;

/// `glint_pool_draw` (FUN_00220518 and its binary twins) per level: the draw's entry address. The pool base is read from
/// the draw's own code (`lui` / `addiu` pair at +0x6c / +0x70 on every twin), so a level whose code differs draws nothing [C].
const GLINT_DRAW: [u32; 19] = [
	0x1fe688, 0x220518, 0x20f640, 0x1f7f70, 0x1f1070, 0x22d340, 0x2162f0, 0x226440, 0x211118, 0x21dcd0, 0x1f1c70, 0x22ff70, 0x2214d8, 0x20a0f0, 0x211c28, 0x1f7d40, 0x1fce70, 0x1fb670, 0x2073f0,
];
/// `GLINT_DRAW` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const GLINT_DRAW_USA: [u32; 19] = [
	0x1fead8, 0x2209e8, 0x20fb10, 0x1f8440, 0x1f1540, 0x22d790, 0x2167c0, 0x226910, 0x2115e8, 0x21e1a0, 0x1f2140, 0x230440, 0x2219a8, 0x20a5c0, 0x212078, 0x1f8210, 0x1fd340, 0x1fbb40, 0x2078c0,
];
/// Pool entries: 16 x 0x20 bytes (pos +0, life i16 +0x10, fade i16 +0x12, angle f32 +0x18, size f32 +0x1c) [C].
const GLINTS: u32 = 16;

fn sext(v: u32) -> u32 {
	v as u16 as i16 as i32 as u32
}

/// The glint pool base of `level`, decoded from the draw function in RAM (None when the code is not the expected twin).
pub fn glint_pool(m: &Machine, level: u32) -> Option<u32> {
	let f = *(*crate::retail::disc::pick(&GLINT_DRAW, &GLINT_DRAW_USA)).get(level as usize)?;
	let lo = m.rd32(f + 0x70);
	// addiu rt, rs, imm
	if lo >> 26 != 9 {
		return None;
	}
	let rs = (lo >> 21) & 31;
	(1..=4).find_map(|i| {
		let w = m.rd32(f + 0x70 - 4 * i);
		(w >> 26 == 0xf && (w >> 16) & 31 == rs).then(|| ((w & 0xffff) << 16).wrapping_add(sext(lo)))
	})
}

/// The live glints retail's pool holds this field (entries with life > 0, culled ones were already freed by retail's own draw).
pub fn glints(m: &Machine, level: u32) -> [Glint; 16] {
	let mut out = [Glint::default(); 16];
	let Some(base) = glint_pool(m, level) else { return out };
	for (i, g) in out.iter_mut().enumerate().take(GLINTS as usize) {
		let a = base + 0x20 * i as u32;
		let life = m.rd16(a + 0x10) as i16;
		if life > 0 {
			*g = Glint { pos: m.rdv3(a), life, fade: m.rd16(a + 0x12) as i16, owner: 0, angle: m.rdf(a + 0x18), size: m.rdf(a + 0x1c) };
		}
	}
	out
}

/// Aridia (level 2): the sky sprite records the sky pass FUN_00289d78 keeps (header pointer DAT_0016049c, count i16 +8,
/// record array pointer +0x1c, 0x20 bytes each: kind +0, texture +2, RGBA +4, angle +8, camera-relative position +0x10,
/// size +0x1c). Retail creates and updates them in RAM every frame (stars twinkle, ten lights orbit) [C].
pub fn sky_sprites(m: &Machine, level: u32) -> Vec<SkySprite> {
	if level != 2 {
		return Vec::new();
	}
	let hdr = m.rd32(crate::retail::disc::va(0x16049c)) & 0x01ff_ffff;
	let (count, recs) = (m.rd16(hdr + 8) as i16, m.rd32(hdr + 0x1c) & 0x01ff_ffff);
	if hdr == 0 || recs == 0 || !(0..=0x400).contains(&count) {
		return Vec::new();
	}
	(0..count as u32)
		.map(|i| {
			let a = recs + 0x20 * i;
			SkySprite { kind: m.rd8(a), tex: m.rd8(a + 2), colour: m.rd32(a + 4), base: 0, phase: [0; 2], angle: m.rdf(a + 8), pos: m.rdv3(a + 0x10), size: m.rdf(a + 0x1c) }
		})
		.collect()
}

/// The ocean draw of Rilgar, Umbris and Gaspar (the levels the mirror has no `ocean.rs` def for and `queue_q1::ocean` does not
/// cover: Batalia 8 and Oltanis 14 are drawn there, never here, so a level has one ocean source), from the disassembly of each level's overlay [C]:
/// (level, `ocean_draw` wrapper, `ocean_st_scale` fn, `ocean_draw_setup`, descriptor VAs). The seven levels run byte-identical
/// copies of the same code (Aridia and Kerwan keep their `OceanDef`s); Umbris queues two oceans (callbacks 0x2f7960 and
/// 0x30cd48), both drawn every field (calls.tsv: 2 patch-draw calls per field). Descriptor layout (the setup's argument block):
/// origin +0, radius^2 +0xc, spacing +0x10, u16 cells +0x18, colours ptr +0x34, mask ptr +0x38, fog words +0x20..0x2c,
/// FOGCOL +0x30, texture step +0x33, first fx record u16 +0x3c, record count +0x3e, ALPHA FIX +0x3f (written by the update's
/// `ocean_st_scale` call, so 0 = not initialised yet).
const OCEANS: [(u32, u32, u32, u32, &[(u32, u32)]); 3] = [
	(5, 0x2ce1c0, 0x2cda28, 0x2cdac0, &[(0x211aa0, 0x315a00)]),
	(7, 0x2cc6c8, 0x2cbf30, 0x2cbfc8, &[(0x1d3300, 0x2f7960), (0x208460, 0x30cd48)]),
	(9, 0x2c1248, 0x2c0ac8, 0x2c0b48, &[(0x1fc640, 0x2ef078)]),
];
/// `OCEANS` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const OCEANS_USA: [(u32, u32, u32, u32, &[(u32, u32)]); 3] = [
	(5, 0x2ce830, 0x2ce098, 0x2ce130, &[(0x211b20, 0x316070)]),
	(7, 0x2ccda0, 0x2cc608, 0x2cc6a0, &[(0x1d3400, 0x2f8038), (0x208560, 0x30d420)]),
	(9, 0x2c1920, 0x2c11a0, 0x2c1220, &[(0x1fc740, 0x2ef750)]),
];

/// Whether `cb` is in draw queue C this field (count 0x15f42c, callbacks at the queue-A callback array + 0x400: Rilgar 0x16e080,
/// Umbris 0x16dc80, Gaspar 0x16dd80, read from each overlay's add-to-queue-C twin FUN_00221190). Retail draws an ocean only
/// when its updater queued the callback: Umbris queues 0x2f7960 every field but 0x30cd48 (the big dark sea, FOGCOL 13,13,13)
/// only while `FUN_00287f10(0x166dc0, ...)` is 0, so in the Snagglebeast arena it is not drawn [C: states fg1 7_0284 / 7_0303
/// queue both, fullgame1 0314..0323 queue only 0x2f7960].
fn queued_c(m: &Machine, level: u32, cb: u32) -> bool {
	let Some((fns, _)) = crate::retail::draw_queue(level) else { return false };
	(0..m.rd32(0x15f42c).min(0x40)).any(|i| m.rd32(fns + 0x400 + 4 * i) == cb)
}

/// One retail ocean as RAM holds it this field: the patch descriptor, the scaled ST table and strip index of the draw, the FIX
/// byte, the cross-faded 64x64 texture of the field's frame counter, and the fog the setup installs (words / 1024, FOGCOL).
#[derive(Clone, Debug)]
pub struct RamOcean {
	pub desc: Desc,
	pub st: [[f32; 2]; STRIP_LEN],
	pub idx: [u8; STRIP_LEN],
	pub fix: u8,
	pub rgba: Vec<[u8; 4]>,
	/// near, far (world units), intensity near, intensity far.
	pub fog: [f32; 4],
	/// FOGCOL r | g << 8 | b << 16.
	pub fogcol: u32,
	/// DAT_0015f458 == 1 (Visibomb live): the level fog and colour stay instead of the ocean's.
	pub level_fog: bool,
	/// The second lava pass (`lava_patches`, Gaspar only): the triangle strips the first pass leaves out, drawn with this texture and fog.
	pub patches: Vec<RamPatch>,
	/// Vertex colour of every patch vertex (RGB bytes of the fill word / 128, as the strips' colours).
	pub patch_colour: [f32; 4],
}

/// One element of the Gaspar second lava pass as RAM holds it: a triangle strip at lava height, ST already halved.
#[derive(Clone, Debug)]
pub struct RamPatch {
	pub pos: Vec<[f32; 3]>,
	pub uv: Vec<[f32; 2]>,
}

/// `lui` at `hi`, `addiu` at `lo` -> the 32-bit constant.
fn lui_addiu(m: &Machine, hi: u32, lo: u32) -> u32 {
	((m.rd32(hi) & 0xffff) << 16).wrapping_add(sext(m.rd32(lo)))
}

fn read_desc(m: &Machine, va: u32) -> Option<Desc> {
	let cells = [m.rd16(va + 0x18), m.rd16(va + 0x1a)];
	let slots = cells[0].div_ceil(7) as usize * cells[1].div_ceil(7) as usize;
	let (colp, maskp) = (m.rd32(va + 0x34) & 0x01ff_ffff, m.rd32(va + 0x38) & 0x01ff_ffff);
	if slots == 0 || slots > 0x400 || colp == 0 || maskp == 0 {
		return None;
	}
	let mask = (0..slots as u32).map(|i| m.rd8(maskp + i)).collect();
	let colours = (0..slots as u32).flat_map(|s| (0..STRIP_LEN as u32).map(move |k| colp + s * 0x200 + 0x10 + k * 4)).map(|a| [m.rd8(a), m.rd8(a + 1), m.rd8(a + 2), m.rd8(a + 3)]).collect();
	Some(Desc { origin: m.rdv3(va), spacing: [m.rdf(va + 0x10), m.rdf(va + 0x14)], cells, radius2: m.rdf(va + 0xc), mask, colours })
}

/// FUN_0024f838 twin (`ocean_tex_blend`): record A = first + q % count, B = first + (q + 1) % count with q = n / step, t =
/// n / step - q; per texel palA[pixA] * (1 - t) + palB[pixB] * t, truncated per channel. The fx record table (16 bytes: pixel
/// offset u16 +8, palette offset u16 +10, both >> 4 from DAT_0015f420) is addressed by the setup's own `lui t4` / `addiu t4`.
fn ocean_texture(m: &Machine, va: u32, table: u32, n: u32) -> Option<Vec<[u8; 4]>> {
	let (step, first, count) = (m.rd8(va + 0x33) as u32, m.rd16(va + 0x3c) as u32, m.rd8(va + 0x3e) as u32);
	if step == 0 || count == 0 {
		return None;
	}
	let base = m.rd32(0x15f420) & 0x01ff_ffff;
	let q = n / step;
	let t = n as f32 / step as f32 - q as f32;
	let rec = |k: u32| {
		let r = table + 16 * (first + k % count);
		(base + 16 * m.rd16(r + 8) as u32, base + 16 * m.rd16(r + 10) as u32)
	};
	let ((pa, qa), (pb, qb)) = (rec(q), rec(q + 1));
	Some(
		(0..4096u32)
			.map(|i| {
				let (a, b) = (m.rd8(pa + i) as u32, m.rd8(pb + i) as u32);
				std::array::from_fn(|c| crate::ocean::mix(m.rd8(qa + 4 * a + c as u32), m.rd8(qb + 4 * b + c as u32), t))
			})
			.collect(),
	)
}

/// FUN_0021faf0 twin: the sphere (x, y, z, r at `a`, world units) against the camera block at 0x16cdc0 (columns 0x16cdc0 / d0 / e0,
/// camera and scale 0x16cdf0 (w = 1024), frustum slopes 0x16ce00..0x16ce30), with `far` (the caller's f12) as the far limit in
/// metres. -1 = outside (not drawn), 0 = crosses a plane (drawn through the clipping VU1 entry 0xe), 1 = fully inside (entry 0xc) [C
/// disassembly 0x21faf0: vf1 = sphere * w; c = M * (xyz - cam); depth bounds cz +- r against 0 and far * w; side planes
/// |c.xy| against cz * slope]. Only the -1 answer changes the picture, the GPU clips the rest.
fn sphere_vis(m: &Machine, a: u32, far: f32) -> i32 {
	let rv = |p: u32| [m.rdf(p), m.rdf(p + 4), m.rdf(p + 8), m.rdf(p + 12)];
	let (c0, c1, c2, k) = (rv(crate::retail::disc::va(0x16cdc0)), rv(crate::retail::disc::va(0x16cdd0)), rv(crate::retail::disc::va(0x16cde0)), rv(crate::retail::disc::va(0x16cdf0)));
	let (v20, v21, v22, v23) = (rv(crate::retail::disc::va(0x16ce00)), rv(crate::retail::disc::va(0x16ce10)), rv(crate::retail::disc::va(0x16ce20)), rv(crate::retail::disc::va(0x16ce30)));
	let w = k[3];
	let s = rv(a).map(|x| x * w);
	let d = [s[0] - k[0], s[1] - k[1], s[2] - k[2]];
	let c: [f32; 3] = std::array::from_fn(|i| c0[i] * d[0] + c1[i] * d[1] + c2[i] * d[2]);
	let (cz, r) = (c[2], s[3]);
	let v1 = [r + cz, -r + cz];
	let v3 = [0.0 - v1[0], far * w - v1[1]];
	if v3[1].is_sign_negative() || !v3[0].is_sign_negative() {
		return -1;
	}
	let ab = [c[0].abs(), c[1].abs()];
	let v5 = [v20[0] * cz, v20[1] * cz];
	let v8 = [v22[0] * r, v22[1] * r];
	let v7 = [v8[0] * v22[2], v8[1] * v22[2]];
	let v9 = [v5[0] * v21[0], v5[1] * v21[1]];
	let v6 = [ab[0] - v8[0], ab[1] - v8[1]];
	let v8b = [ab[0] + v7[0], ab[1] + v7[1]];
	let v4 = [v1[0] - v23[0], v1[1] - v23[1]];
	let v7b = [v5[0] - v6[0], v5[1] - v6[1]];
	let v8c = [v9[0] - v8b[0], v9[1] - v8b[1]];
	if v7b[0].is_sign_negative() || v7b[1].is_sign_negative() {
		return -1;
	}
	// a0 = vf4 with the sign of x flipped: x counts as "outside" when cz + r has not stayed below vf23.x.
	i32::from(!(v8c[0].is_sign_negative() || v8c[1].is_sign_negative() || v4[1].is_sign_negative() || !v4[0].is_sign_negative()))
}

/// The second lava pass of Gaspar (level 9): the ocean callback 0x2ef078 calls FUN_002c12a0(descriptor 0x1fc640, elements, count)
/// right after the first pass (`jal 0x2c12a0` at 0x2ef108, a1 = lui/addiu at 0x2ef0fc / 0x2ef104, a2 = 10 at 0x2ef10c) [C]. Per element
/// (0x20 bytes: sphere xyz r +0, vertex ptr +0x10, ST ptr +0x14, count +0x1c) that FUN_0021faf0(256.0) does not cull, the function
/// copies the ST pairs * 0.5 (lui 0x3f00 at 0x2c16d4) and draws the xyz triples as one triangle strip (VU1 prog10 entry 0xc / 0xe,
/// GIF PRIM 0x7c = strip, Gouraud, TME, FGE, ABE) with the constant vertex colour 0x757c8e (lui / ori at 0x2c1630 / 0x2c1640), the first
/// pass's texture (TEX0 0x161270), fog words and FOGCOL, ALPHA FIX = descriptor +0x3f, Z test GEQUAL (TEST 0x5360a: no alpha test).
/// Empty on other levels or when the code is not the expected copy.
fn lava_patches(m: &Machine, level: u32, desc: u32) -> (Vec<RamPatch>, [f32; 4]) {
	let none = (Vec::new(), [0.0; 4]);
	let gate = [(crate::retail::disc::va(0x2ef108), 0x0c0b_04a8), (crate::retail::disc::va(0x2ef0fc), 0x3c05_001f), (crate::retail::disc::va(0x2c12a0), 0x27bd_ff40), (crate::retail::disc::va(0x2c1630), 0x3c03_0075), (crate::retail::disc::va(0x2c16d4), 0x3c01_3f00), (crate::retail::disc::va(0x2c16a0), 0x3c01_4380)];
	if level != 9 || desc != crate::retail::disc::va(0x1fc640) || gate.iter().any(|&(a, w)| m.rd32(a) != w) {
		return none;
	}
	let elems = lui_addiu(m, crate::retail::disc::va(0x2ef0fc), crate::retail::disc::va(0x2ef104));
	let count = m.rd32(crate::retail::disc::va(0x2ef10c)) & 0xffff;
	let fill = (m.rd32(crate::retail::disc::va(0x2c1630)) & 0xffff) << 16 | (m.rd32(crate::retail::disc::va(0x2c1640)) & 0xffff);
	let far = f32::from_bits((m.rd32(crate::retail::disc::va(0x2c16a0)) & 0xffff) << 16);
	let colour = [fill & 255, fill >> 8 & 255, fill >> 16 & 255].map(|b| b as f32 / 128.0);
	let patches = (0..count.min(0x40))
		.filter_map(|i| {
			let a = elems + 0x20 * i;
			let n = m.rd32(a + 0x1c) as i32;
			if sphere_vis(m, a, far) < 0 || !(3..=256).contains(&n) {
				return None;
			}
			let (vp, sp) = (m.rd32(a + 0x10) & 0x01ff_ffff, m.rd32(a + 0x14) & 0x01ff_ffff);
			let pos = (0..n as u32).map(|k| m.rdv3(vp + 12 * k)).collect();
			let uv = (0..n as u32).map(|k| [m.rdf(sp + 8 * k) * 0.5, m.rdf(sp + 8 * k + 4) * 0.5]).collect();
			Some(RamPatch { pos, uv })
		})
		.collect();
	(patches, [colour[0], colour[1], colour[2], 0.0])
}

/// The oceans retail draws this field on Rilgar, Umbris and Gaspar, all read from RAM (the descriptors, tables
/// and fx records are plain overlay data; the FIX byte is the init flag). Empty on other levels or when the level's code is not
/// the expected copy. `frames` = the level frame counter of this field.
pub fn oceans(m: &Machine, level: u32, frames: u32) -> Vec<RamOcean> {
	let Some(&(_, wrap, st_fn, setup, descs)) = (*crate::retail::disc::pick(&OCEANS, &OCEANS_USA)).iter().find(|o| o.0 == level) else { return Vec::new() };
	// Code gate: the three functions start with the expected twin words and the setup loads its fx table with `lui t4`.
	if m.rd32(wrap) != 0x27bd_ffe0 || m.rd32(st_fn) != 0xa085_003f || m.rd32(setup) != 0x27bd_ff60 || m.rd32(setup + 0x200) != 0x3c0c_0017 {
		return Vec::new();
	}
	let Some(table) = (1..10).map(|k| setup + 0x200 + 4 * k).find(|&a| m.rd32(a) >> 16 == 0x258c).map(|a| lui_addiu(m, setup + 0x200, a)) else { return Vec::new() };
	let strip = lui_addiu(m, wrap + 0x14, wrap + 0x28);
	let st_data = lui_addiu(m, st_fn + 4, st_fn + 8) + 0x10;
	let idx: [u8; STRIP_LEN] = std::array::from_fn(|k| ((m.rd16(strip + 2 * k as u32) as i16) / 8) as u8);
	let st: [[f32; 2]; STRIP_LEN] = std::array::from_fn(|k| [m.rdf(st_data + 8 * k as u32), m.rdf(st_data + 8 * k as u32 + 4)]);
	descs
		.iter()
		.filter_map(|&(va, cb)| {
			let fix = m.rd8(va + 0x3f);
			if fix == 0 || !queued_c(m, level, cb) {
				return None;
			}
			let (desc, rgba) = (read_desc(m, va)?, ocean_texture(m, va, table, frames)?);
			let (n, f) = (m.rdf(va + 0x20) / 1024.0, m.rdf(va + 0x24) / 1024.0);
			let fogcol = m.rd8(va + 0x30) as u32 | (m.rd8(va + 0x31) as u32) << 8 | (m.rd8(va + 0x32) as u32) << 16;
			let (patches, patch_colour) = lava_patches(m, level, va);
			Some(RamOcean { desc, st, idx, fix, rgba, fog: [n, f, m.rdf(va + 0x28), m.rdf(va + 0x2c)], fogcol, level_fog: m.rd32(0x15f458) == 1, patches, patch_colour })
		})
		.collect()
}

#[cfg(test)]
mod tests {
	/// One ocean source per level: the levels `queue_q1::ocean` draws (Batalia, Oltanis) are never in `OCEANS`, and neither
	/// are the levels with a WAD-built `ocean.rs` def (Aridia 2, Kerwan 3).
	#[test]
	fn one_ocean_source_per_level() {
		for o in super::OCEANS.iter() {
			assert!(!super::super::queue_q1::OCEAN_FN.iter().any(|q| q.0 == o.0), "level {} has two RAM ocean sources", o.0);
			assert!(o.0 != 2 && o.0 != 3, "level {} keeps its ocean.rs def", o.0);
		}
	}
}
