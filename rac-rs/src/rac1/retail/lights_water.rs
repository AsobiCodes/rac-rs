//! The dynamic point lights, the reserved level lights and the water patch table, read from the interpreter's RAM.

use super::*;

/// The effect pool and draw-queue callbacks retail draws this field (`RetailFx`). Pool: every occupied slot up to the
/// high-water index gp-0x6b0c (inclusive: the alloc keeps the highest slot index, the draw walks index + 1 slots) with
/// the hidden flag (+1 bit 7) clear, by mode +1 & 3 (decomp/notes/particles.md "Draw size").
/// Dynamic light allocator FUN_002522c0 (Novalis; twins.py: binary twins on all 19 levels, Eudora FUN_00230c90) [C].
const LIGHT_ALLOC_FN: [u32; 19] = [
	0x23e320, 0x2522c0, 0x23fdf8, 0x22c580, 0x230c90, 0x2672d0, 0x24cf60, 0x2658f8, 0x247e98, 0x25d390, 0x230d18, 0x263658, 0x256498, 0x249520,
	0x245eb8, 0x22d8a8, 0x239960, 0x238998, 0x240590,
];

/// `LIGHT_ALLOC_FN` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const LIGHT_ALLOC_FN_USA: [u32; 19] = [
	0x23e738, 0x252750, 0x240288, 0x22ca10, 0x231128, 0x267700, 0x24d3f8, 0x265d90, 0x248328, 0x25d828, 0x2311b0, 0x263ae8, 0x256928, 0x2499b8,
	0x2462c8, 0x22dd38, 0x239e10, 0x238e30, 0x240a20,
];

/// The 8 dynamic point-light slots (decomp/notes/lighting_fog.md 5b) as MobyProc reads them. Table 8 x 0x20: rgb
/// floats +0, intensity +0xc, position +0x10, radius +0x1c (Eudora 0x1803c0, Rilgar 0x180740, Novalis 0x1806c0: not
/// the camera-globals shift); its address comes from the allocator's own `lui v0 / addiu v0` at +0x78 / +0x80 (+0x30 / +0x38 load the state table, +0x200) (same
/// layout in the Eudora and Rilgar twins [C], the others [I]; a mismatch gives no lights). Eudora's moby pass
/// FUN_00248288 copies the slots whose position x/y doubleword is nonzero to SPR 0x2400 (free FUN_002523c0 zeroes a
/// record) and lights every moby from them (overlay.elf.c 67779-67806) [C]; `dynlight::moby_light` is the per-moby sum.
pub fn dyn_lights(m: &ee_oracle::Machine, level: u32) -> [Option<crate::particles::PointLight>; 8] {
	let mut out = [None; 8];
	let Some(&f) = (*crate::retail::disc::pick(&LIGHT_ALLOC_FN, &LIGHT_ALLOC_FN_USA)).get(level as usize) else { return out };
	let (hi, lo) = (m.rd32(f + 0x78), m.rd32(f + 0x80));
	if hi >> 16 != 0x3c02 || lo >> 16 != 0x2442 {
		return out;
	}
	let t = (hi << 16).wrapping_add(lo as i16 as i32 as u32);
	for (k, o) in out.iter_mut().enumerate() {
		let a = t + 0x20 * k as u32;
		if m.rd32(a + 0x10) | m.rd32(a + 0x14) != 0 {
			*o = Some(crate::particles::PointLight { pos: m.rdv3(a + 0x10), radius: m.rdf(a + 0x1c), rgb: [m.rdf(a), m.rdf(a + 4), m.rdf(a + 8)], intensity: m.rdf(a + 0xc) });
		}
	}
	out
}

/// Level-light slots 12..15 (decomp/notes/lighting_fog.md "Reserved level-light slots 12-14"), as the uniform rows
/// (colour A + wrap, to-light A = -dir A, colour B + wrap, to-light B). The loader copies at most 12 level lights; code
/// fills slot 12 (hero-attached / gold models), 13 (camera-yaw light, every frame) and 14 (menu and shop models: the
/// vendor's purchase prompt class 1137 has light word 0x0e0e) [C]. Records 0x40 each at the 16-slot table that ends
/// where the dynamic slots (`dyn_lights`) begin: Eudora 0x17ffc0 (slot 13 (0.8, 0.8, 0.8, -0.3) at 0x180300 and slot 14
/// (0.9, 0.9, 0.6, -0.25) at 0x180340 in s12 / fg1 4_0159 / s07 RAM), Novalis 0x1802c0 = 0x1806c0 - 0x400 [C]; the
/// other levels use the same layout [I].
pub fn reserved_lights(m: &ee_oracle::Machine, level: u32) -> Option<[[[f32; 4]; 4]; 4]> {
	let f = *(*crate::retail::disc::pick(&LIGHT_ALLOC_FN, &LIGHT_ALLOC_FN_USA)).get(level as usize)?;
	let (hi, lo) = (m.rd32(f + 0x78), m.rd32(f + 0x80));
	if hi >> 16 != 0x3c02 || lo >> 16 != 0x2442 {
		return None;
	}
	let t = (hi << 16).wrapping_add(lo as i16 as i32 as u32).wrapping_sub(0x400);
	Some(std::array::from_fn(|k| {
		let a = t + 0x40 * (12 + k as u32);
		let v = |o: u32, neg: f32| [m.rdf(a + o) * neg, m.rdf(a + o + 4) * neg, m.rdf(a + o + 8) * neg, if neg > 0.0 { m.rdf(a + o + 0xc) } else { 0.0 }];
		[v(0, 1.0), v(0x10, -1.0), v(0x20, 1.0), v(0x30, -1.0)]
	}))
}

/// Levels that run the Novalis patch-draw FUN_002b8af8 (binary twins, 230 lines each): (level, draw function, light-constants
/// delta from Novalis 0x1cac00, shift of the patch globals 0x161250 / 0x161258 / 0x1611a0). Novalis 0x2b8af8, Hoven 0x2bb5f8,
/// Oltanis Orbit 0x2af410, Pokitaru 0x2c7c68, Rilgar 0x2cd520, Umbris 0x2cba28 [C: read in fg1 states 1_0018, 12_0554, 13_0622, 11_0421, 5_0216, 7_0284].
const WATER_DRAW: [(u32, u32, i32, u32); 6] = [(1, 0x2b8af8, 0, 0), (12, 0x2bb5f8, 0, 0), (13, 0x2af410, -0x180, 0), (11, 0x2c7c68, 0x580, 0), (5, 0x2cd520, 0x80, 0x40), (7, 0x2cba28, -0x400, 0)];

/// `WATER_DRAW` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const WATER_DRAW_USA: [(u32, u32, i32, u32); 6] = [(1, 0x2b91c8, 0, 0), (12, 0x2bbcc8, 0, 0), (13, 0x2afae8, -0x180, 0), (11, 0x2c8338, 0x580, 0), (5, 0x2cdb90, 0x80, 0x40), (7, 0x2cc100, -0x400, 0)];

/// Retail's water patches (class 751 state) as the draw reads them [C]. Patch table pointer 0x161250, count 0x161258, output buffer
/// index 0x1611a0 (+ shift); record stride 0x1190: +0 pos, +0x14 / +0x18 fx textures (sphere / water pass), +0x1c / +0x1d ALPHA FIX,
/// +0x1e visible-subblock mask, +0x20 / +0x24 UV scroll, +0x30 / +0x34 wobble angles, +0x40 amplitude, +0x44 pin flags, three
/// 0x5c0-byte height buffers from +0x50 (the draw adds index * 0x5c0). The light vector sits at 0x1cac30 + the level delta.
pub(super) fn water_patches(m: &ee_oracle::Machine, level: u32) -> Option<([f32; 3], Vec<crate::water::RamPatch>)> {
	let &(_, f, d, s) = (*crate::retail::disc::pick(&WATER_DRAW, &WATER_DRAW_USA)).iter().find(|e| e.0 == level)?;
	if [m.rd32(f), m.rd32(f + 4), m.rd32(f + 8)] != [0x27bd_fee0, 0x7fb2_0090, 0x7fbf_0100] {
		return None;
	}
	let table = m.rd32(va(0x161250 + s)) & 0x01ff_ffff;
	let count = m.rd32(va(0x161258 + s));
	let idx = m.rd32(va(0x1611a0 + s));
	if table < 0x10_0000 || count == 0 || count > 64 || idx > 2 {
		return None;
	}
	let light = m.rdv3(va(0x1cac30u32.wrapping_add_signed(d)));
	let mut out = Vec::new();
	for k in 0..count {
		let b = table + k * 0x1190;
		let mask = m.rd16(b + 0x1e);
		let mut patch = crate::water::Patch {
			pos: m.rdv3(b),
			nb: [0xff; 8],
			tex_sphere: (m.rd32(b + 0x14) as i32).max(0) as usize,
			tex_water: (m.rd32(b + 0x18) as i32).max(0) as usize,
			fix_sphere: m.rd8(b + 0x1c),
			fix_water: m.rd8(b + 0x1d),
			mask,
			scroll: [m.rdf(b + 0x20), m.rdf(b + 0x24)],
			speed: [0.0; 2],
			ang: [m.rdf(b + 0x30), m.rdf(b + 0x34)],
			ang_rate: [0.0; 2],
			amp: m.rdf(b + 0x40),
			pin: m.rd8(b + 0x44),
		};
		let mut grid = Vec::new();
		if mask != 0 {
			let g = b + 0x50 + idx * 0x5c0;
			let buf: Vec<f32> = (0..368).map(|i| m.rdf(g + 4 * i)).collect();
			grid = crate::water::grid_from_buffer(&buf);
		} else {
			patch.mask = 0;
		}
		out.push(crate::water::RamPatch { patch, grid });
	}
	Some((light, out))
}
