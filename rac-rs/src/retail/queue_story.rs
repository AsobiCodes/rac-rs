//! Queue callbacks that only run in story moments (in-engine cutscenes) and that the weapon / gameplay ports do not cover.
//! Census 2026-10-12 (decomp/notes/draw_producers.md "story + weapons census").
//!
//! Ship gloss outside mode 6: the cutscene ship's env-mapped canopy overlay (Eudora FUN_00283dd8 = Novalis 0x2a69d8, binary
//! twin on every level) is queued on queue A or B for the ship actors (classes 0x213 / 0x214) during mode-2 cutscenes
//! (Novalis landing, Blarg, Orxon, Hoven) and mode 0. `retail.rs snap` draws the mode-6 gloss from the ship type; here the
//! class comes from the queued moby (+0xa6) [C: Eudora OV:109458 `*(short *)(arg + 0xa6) - 0x212`]. The callback draws with
//! fx texture 0x15 outside mode 6 (OV:109466-109477) and recomputes the shared vertex / UV buffers (0x1c06e0 / 0x1c0d40
//! Eudora) on every call, so RAM at field end holds the LAST call's points: only the last queued entry can be redrawn.

use crate::particles::WorldQuad;
use ee_oracle::Machine;
use crate::retail::disc::va;

/// The gloss callback per level (where.py LEVELS order: veldin novalis aridia kerwan eudora rilgar blarg umbris batalia
/// gaspar orxon pokitaru hoven oltanis_orbit oltanis quartu kalebo veldin_orbit veldin_finale; `twins.py`, binary twins) [C].
const GLOSS_FN: [u32; 19] = [
	0x292f60, 0x2a69d8, 0x292658, 0x27f830, 0x283dd8, 0x2bb698, 0x29ffd8, 0x2b9b88, 0x29b270, 0x2af830, 0x2839e0, 0x2b5de0, 0x2a9770, 0x29d2f0, 0x299310,
	0x280e30, 0x28bda0, 0x28b140, 0x292a90,
];
/// `GLOSS_FN` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const GLOSS_FN_USA: [u32; 19] = [
	0x2935b8, 0x2a70a8, 0x292d28, 0x27ff00, 0x2844b0, 0x2bbd08, 0x2a06b0, 0x2ba260, 0x29b940, 0x2aff08, 0x2840b8, 0x2b64b0, 0x2a9e40, 0x29d9c8, 0x299960,
	0x281500, 0x28c490, 0x28b818, 0x293160,
];
/// First code word of the callback (`addiu sp, sp, -0x1d0`); a level whose RAM differs draws nothing [C].
const PROLOGUE: u32 = 0x27bd_fe30;

/// The gloss quads retail drew this field outside mode 6 (empty when no ship is queued).
pub fn cutscene_gloss(m: &Machine, level: u32) -> Vec<WorldQuad> {
	let (Some(&f), Some((fa, aa))) = ((*crate::retail::disc::pick(&GLOSS_FN, &GLOSS_FN_USA)).get(level as usize), super::draw_queue(level)) else { return Vec::new() };
	if m.rd32(f) != PROLOGUE {
		return Vec::new();
	}
	// Queue B runs after queue A, so its last entry is the field's last call.
	let last = [(0x15f428u32, fa + 0x200, aa + 0x200), (0x15f424, fa, aa)].into_iter().find_map(|(count, fns, args)| {
		(0..m.rd32(count).min(0x40)).rev().find(|&i| m.rd32(fns + 4 * i) == f).map(|i| m.rd32(args + 4 * i) & 0x01ff_ffff)
	});
	let Some(moby) = last.filter(|&a| a != 0) else { return Vec::new() };
	let class = m.rd16(moby + 0xa6) as u32;
	let q = gloss_for(m, level, class, 0x15);
	if std::env::var_os("RAC_SCENE_LOG").is_some() {
		eprintln!("retail: story ship gloss level {level} mode {} class {class:#x} quads {} q0 {:?}", m.rd32(va(0x15f5a4)), q.len(), q.first().map(|q| (q.pos, q.uv, q.colour[0])));
	}
	q
}

/// The gloss quads of ship class `class` (0x212..0x215) from the level's RAM, texture `tex` (retail/warp.rs `gloss` with
/// the per-level addresses). Tables are gp-relative: colour 0x160560, quad count 0x160580, index table pointer 0x1605b0,
/// +0x40 on Veldin, Rilgar, Oltanis and Kalebo III (those overlays' immediates are gp-0x6660.. / gp-0x6610 where the
/// others use gp-0x66a0.. / gp-0x6650; RAM check: states fg1/0_0001 and 5_0216 hold them at +0x40) [C]. The vertex / UV
/// buffers (Novalis 0x1c09e0 / 0x1c1040) move with `LEVEL_SHIFT`, Rilgar and Kalebo III a further 0x80 [C: overlay
/// DAT_001c0a60 / DAT_001c09e0 at the vertex loop].
pub fn gloss_for(m: &Machine, level: u32, class: u32, tex: usize) -> Vec<WorldQuad> {
	let Some(idx) = class.checked_sub(0x212).filter(|&i| i < 4) else { return Vec::new() };
	let t = if matches!(level, 0 | 5 | 14 | 16) { 0x40 } else { 0 };
	let d = super::LEVEL_SHIFT[level as usize] + if matches!(level, 5 | 16) { 0x80 } else { 0 };
	let at = |a: u32| va(a.wrapping_add_signed(d));
	let nq = m.rd32(va(0x160580 + t) + 4 * idx).min(128);
	let tab = m.rd32(va(0x1605b0 + t) + 4 * idx) & 0x01ff_ffff;
	let w = m.rd32(va(0x160560 + t) + 4 * idx);
	let colour: [f32; 4] = std::array::from_fn(|k| ((w >> (8 * k)) & 0xff) as f32 / 128.0);
	if tab == 0 {
		return Vec::new();
	}
	(0..nq)
		.map(|q| {
			let v: [u32; 4] = std::array::from_fn(|k| (m.rd16(tab + 16 * q + 4 * k as u32) as u32).min(101));
			let p = v.map(|i| m.rdv3(at(0x1c09e0) + 16 * i));
			let uv = v.map(|i| [m.rdf(at(0x1c1040) + 8 * i), m.rdf(at(0x1c1040) + 8 * i + 4)]);
			WorldQuad { pos: [p[1], p[0], p[2], p[3]], uv: [uv[1], uv[0], uv[2], uv[3]], colour: [colour; 4], tex, additive: false }
		})
		.collect()
}

/// Effects layer claimed set (retail/efx_draw.rs): the gloss callback this module draws.
pub fn claims(level: u32, f: u32) -> bool {
	f != 0 && (*crate::retail::disc::pick(&GLOSS_FN, &GLOSS_FN_USA)).get(level as usize) == Some(&f)
}
