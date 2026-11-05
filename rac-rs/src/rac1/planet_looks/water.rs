//! R&C1's water tables (engine/looks.rs `Looks`, read by engine/viewer/waterfx/levels.rs and shore_ref.rs; engine split stage
//! E2 batch 5, 2026-10-26: moved out of those): which textures and terrain of a level are sea, pond, river, waterfall, cold,
//! frozen, lava, mud or sludge, and the measured tide of its sea.

use crate::looks::{Frozen, Liquid};

/// Gaspar (its "ocean" is lava): its RAM ocean takes `LAVA_SLOT` (`WaterDraw::liquid`), the lava look of `fs_water`.
const LAVA_LEVEL: usize = 9;

/// The liquids that are no water, by level: the look (`fs_water`'s flow slot) their ocean draws take (`WaterDraw::liquid`). Gaspar's
/// RAM ocean is lava; Aridia's ocean (its "level ocean", the brown plane at z 24.75 [C: RAC_WATER_LOG at fg1/2_0091 / 2_0138,
/// 2026-10-16: its only water draw]) is the mud the hero sinks in (decomp/notes/water.md ARI-50: Aridia has no water faces).
pub(super) fn ocean_liquid(level: usize) -> Option<Liquid> {
	match level {
		LAVA_LEVEL => Some(Liquid::Lava),
		2 => Some(Liquid::Mud),
		_ => None,
	}
}

/// [PC] Level oceans that are FOG sheets, not liquids (user 2026-10-24, fg1 3_0086 hero at 265.5 151.4 50.0 looking down the city
/// chasm: "we flooded the whole city"). Kerwan's class-994 "ocean" (ocean.rs `KERWAN`: FIX 0x20, fog words with FOGCOL 96 96 96)
/// is the soft grey fog layer at the chasm bottom; the Enhanced water pass must leave it as retail draws it (and the volumetric
/// fog may build on it). A fog sheet is never water on any level: Aridia's ocean is mud (`liquid_slot`), Batalia's and Oltanis'
/// oceans are sea.
pub(super) fn fog_sheet_ocean(level: usize) -> bool {
	level == 3
}

/// A liquid drawn as level terrain, by level: its terrain texture index (tfrag face key) and kind. Orxon's green toxic sea is
/// terrain texture 12 (flat faces, z 27-62; it goes when that texture is left out, and retail draws it still: Classic fields
/// 60 / 80 / 100 at fullgame1 0520 differ in no sludge pixel) [C: the terrain textures logged and left out one at a time, 2026-10-16].
pub(super) fn terrain_liquid(level: usize) -> Option<(u32, Liquid)> {
	match level {
		10 => Some((12, Liquid::Sludge)),
		_ => None,
	}
}

/// The fx textures (level fx table index, `WorldQuad.tex`) of a sea drawn as effect quads: Pokitaru's sea is queue-A
/// callback 0x30a238 output (decomp/notes/ocean.md); at fg1/11_0449 it is fx texture 22 (15 fans out to the horizon) under
/// the additive layers 40 and 41 (flat at z 223.16 around the beach) [C: RAC_WATER_LOG bounds and RAC_DRAWSKIP=quads, 2026-10-15].
/// The queue-C pool strips (`Gpu.pool_draws`) that are water, by fx texture: Hoven's door pools (FUN_002e6ae0: tex 0x2c at FIX
/// 0x80, its second layer 0x2d at FIX 0x40) and its abyss water (tex 0x2c, 0x2f and 0x30; `cold_pool` gives 0x2f / 0x30 the cold look). Orxon's tex 41 (FIX
/// 32) and Batalia's tex 59 (additive, FIX 0) are no water surfaces.
/// (corrected 2026-10-16: was "tex 47 / 48 strips are the snow"; at fg1/12_0554 they are drawn but hidden under the ground,
/// in the abyss they are its teal wave plane [C: RAC_DRAWSKIP=pools on play/compare/Hoven.racpad field 1950].)
pub(super) fn pool_water_tex(level: usize) -> &'static [usize] {
	match level {
		12 => &[44, 45, 47, 48],
		_ => &[],
	}
}

/// Hoven's abyss water, the queue-C callback FUN_0030b798 (overlay.elf.c 165949-166004): tex 0x2c strips while the camera is
/// inside its cuboid and the wave plane tex 0x2f, both at FIX 0x7f (ALPHA 0x7f00000064), then tex 0x30 additive (Cs * FIX +
/// Cd). The two wave-plane layers get the cold look (`COLD_SLOT`). The tex 0x2c sheet is the flooded floor around the doors
/// (fg1/12_0582, plane z 45) and keeps the door pools' look, as the door pools themselves (tex 0x2c FIX 0x80, 0x2d FIX 0x40).
pub(super) fn cold_pool(level: usize, tex: usize, add: bool, fix: u8) -> bool {
	level == 12 && (!add && fix == 127 && tex == 47 || add && tex == 48)
}

/// [PC] Hoven's frozen floor (user 2026-10-16 at fg1/12_0582: "the ice here should be more static, currently it moves like
/// water"): the door puddles (FUN_002e6ae0 / FUN_002bbb40: tex 0x2c FIX 0x80, then tex 0x2d FIX 0x40) and the tex 0x2c FIX 0x7f
/// sheet around the doors (plane z 45) keep their Enhanced look held still: `Frozen::Opaque` for the opaque layers,
/// `Frozen::Clear` for the FIX 0x40 one. Retail blends the puddle texture over time itself (Classic fields 60 / 100
/// differ there), which stays. The abyss (`cold_pool`, tex 0x2f / 0x30) stays liquid.
pub(super) fn frozen_pool(level: usize, tex: usize, add: bool, fix: u8) -> Option<Frozen> {
	match (level, add, tex, fix) {
		(12, false, 44, 127..=128) => Some(Frozen::Opaque),
		(12, false, 45, 64) => Some(Frozen::Clear),
		_ => None,
	}
}

/// Quartu's tank water (the flooded round tank room, decomp/notes/draw_coverage.md 6h: queue B callbacks 0x2ea5c0 / 0x2ed510 /
/// 0x2ed8f0, a wobbling grid drawn twice at ALPHA 0x44) is fx texture 45 (646 quads, z 39.40-39.60 at fullgame2 0058)
/// [C: RAC_WATER_LOG fx quad bounds, 2026-10-16].
pub(super) fn sea_fx_tex(level: usize) -> &'static [usize] {
	match level {
		11 => &[22, 40, 41],
		16 => &[22, 40, 41],
		15 => &[45],
		_ => &[],
	}
}

/// [PC] A pond drawn as effect quads, by level: its fx textures. Pokitaru's waterfall ponds (fullgame1 0447, z 231-233 at
/// x 445-476 y 549-572) are fx texture 44 (non-additive) under the additive layer 43 [C: RAC_DRAWSKIP=quads leaves them out,
/// fx quad bounds logged, 2026-10-17]. They get their own draw: their plane is not the sea's.
pub(super) fn pond_fx_tex(level: usize) -> &'static [usize] {
	match level {
		11 => &[43, 44],
		_ => &[],
	}
}

/// [PC] Falling water drawn as effect quads, by level: its fx textures. Pokitaru's waterfalls are fx texture 42 (additive,
/// z 222-252 at fullgame1 0447) [C: as `pond_fx_tex`]. Their feet churn the water (`WaterFx::sheet_falls`).
pub(super) fn fall_fx_tex(level: usize) -> &'static [usize] {
	match level {
		11 => &[42],
		_ => &[],
	}
}

/// A river drawn as effect quads, by level: its fx texture. Batalia's jungle river is fx texture 58 (non-additive quads from
/// the waterfall down to the sea, z 14-31, sloped) under the additive layers 60 and 61 [C: RAC_WATER_LOG bounds at fullgame1
/// 0345 / 0348, 2026-10-16]; retail scrolls its uv the same step every tick on every quad (`WaterFx::quad_flows`).
/// (corrected 2026-10-24: Kalebo III's fx texture 16 was listed too; it is the glowing conveyor belt channels on the walkways
/// (user at 165 168 124: "why does this conveyor belt here has water shader"), not water: its sea is `sea_fx_tex`.)
pub(super) fn river_fx_tex(level: usize) -> Option<usize> {
	match level {
		8 => Some(58),
		_ => None,
	}
}

/// Levels whose world strips (`Gpu::strip_draws`) are water: Novalis, whose strip classes 676 / 678 / 761 / 1225 are its
/// river, waterfall pool and cascade pools (retail/queue_q1.rs `STRIP_FN`; decomp/notes/water.md NOV-23.1).
pub(super) fn strips_are_water(level: usize) -> bool {
	level == 1
}

/// The measured tide (lowest, highest plane) of a level's sea, by level [C: RAC_WATER_LOG]: Pokitaru 222.758 .. 223.275.
pub(super) fn known_tide(level: usize) -> Option<(f32, f32)> {
	match level {
		11 => Some((222.758, 223.275)),
		_ => None,
	}
}
