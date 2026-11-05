//! The Graphics rows' menu bytes <-> levels (`swap_level`, `swap_rows`) and the packed word of the rows that fit in it (`pack_gfx`,
//! `unpack_gfx`). Moved out of pcext.rs 2026-10-31 (lane furopt, a pure move plus the two Fur rows: 17 rows, the word still holds
//! rows 0..=12; Fog, Clouds, Fur, Fur style, Fire and Smoke live in their own modules; 19 rows since the Fire and Smoke rows).
use super::*;

/// The menu byte of an effect row (ambient occlusion, shadows, water, particles, sun rays) and the level it stands for are the
/// same table both ways (an involution): the bytes keep today's values so the retail menu RAM does not change by default (0 Off or
/// Classic, 1 High = today's On / Enhanced, 2 Medium, 3 Low; the menu cycles Off, High, Medium, Low); the levels count up for
/// the renderer and the file (0 Off, 1 Low, 2 Medium, 3 High). The ONE place the two meet.
pub(super) const fn swap_level(v: u8) -> u8 {
	match v {
		1 => 3,
		2 => 2,
		3 => 1,
		_ => 0,
	}
}

/// `g` (`GROWS` order, bytes or levels) with the effect rows (Fur, Fire and Smoke included, not Fur style) through `swap_level`.
pub(super) const fn swap_rows(mut g: [u8; 19]) -> [u8; 19] {
	g[1] = swap_level(g[1]);
	g[2] = swap_level(g[2]);
	g[4] = swap_level(g[4]);
	g[5] = swap_level(g[5]);
	g[6] = swap_level(g[6]);
	g[10] = swap_level(g[10]);
	g[13] = swap_level(g[13]);
	g[14] = swap_level(g[14]);
	g[15] = swap_level(g[15]);
	g[17] = swap_level(g[17]);
	g[18] = swap_level(g[18]);
	g
}

/// The row bytes (`GROWS` order) as one word: lighting in bits 0..8, then one "on" bit per effect row (ambient occlusion, shadows,
/// water, particles, sun rays: level > 0; frame smoothing; display mode excluded: `FULLSCREEN`), vegetation in bits 14..16,
/// the five effect levels in two bits each from bit 16 (ambient occlusion, shadows, water, particles, sun rays), culling Off
/// in bit 26, the global illumination level in bits 27..28 (no "on" bit: 0 = Off), waves Off in bit 29 and the draw distance
/// byte (0 Medium, 1 High, 2 Low) in bits 30..31. A word with an "on" bit and a zero level (the packing before the levels
/// existed) reads as High (`unpack_gfx`).
pub(super) const fn pack_gfx(g: [u8; 19]) -> u32 {
	let veg = if g[8] > 2 { 2 } else { g[8] } as u32;
	let draw = if g[12] > 2 { 2 } else { g[12] } as u32;
	let g = swap_rows(g);
	let (ao, sh, wa, pa, su) = (g[1] as u32, g[2] as u32, g[4] as u32, g[5] as u32, g[6] as u32);
	g[0] as u32
		| ((ao != 0) as u32) << 8
		| ((sh != 0) as u32) << 9
		| ((wa != 0) as u32) << 10
		| ((pa != 0) as u32) << 11
		| ((su != 0) as u32) << 12
		| ((g[7] == 1) as u32) << 13
		| veg << 14
		| ao << 16
		| sh << 18
		| wa << 20
		| pa << 22
		| su << 24
		| ((g[9] == 1) as u32) << 26
		| ((g[10] & 3) as u32) << 27
		| ((g[11] == 1) as u32) << 29
		| draw << 30
}

/// The effect level (0..3) the word holds for the row whose "on" bit is `bit` and whose level sits at `shift`.
pub(super) fn gfx_level(g: u32, bit: u32, shift: u32) -> u8 {
	match (g >> bit & 1 != 0, (g >> shift & 3) as u8) {
		(false, _) => 0,
		(true, 0) => 3,
		(true, l) => l,
	}
}

/// `pack_gfx`'s inverse: the row bytes (display mode not in the word: 0).
pub(super) fn unpack_gfx(g: u32) -> [u8; 19] {
	swap_rows([(g & 0xff).min(1) as u8, gfx_level(g, 8, 16), gfx_level(g, 9, 18), 0, gfx_level(g, 10, 20), gfx_level(g, 11, 22), gfx_level(g, 12, 24), (g >> 13 & 1) as u8, (g >> 14 & 3).min(2) as u8, (g >> 26 & 1) as u8, (g >> 27 & 3) as u8, (g >> 29 & 1) as u8, (g >> 30 & 3).min(2) as u8, 0, 0, 0, 0, 0, 0])
}
