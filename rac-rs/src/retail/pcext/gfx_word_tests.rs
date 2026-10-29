//! The packed Graphics word and the row tables' layout (moved out of pcext.rs 2026-10-24, lane yfog: a pure move plus the Fog row;
//! `use super::*` reaches the same private items).
use super::tests::SERIAL;
use super::*;

/// The packed Graphics word: every level of every effect row round-trips, a word from before the levels existed (an "on" bit
/// and no level) reads as High, and the effect rows have the right number of values. Fog and Clouds (rows 13, 14) are not in the word.
#[test]
fn gfx_word_packs_levels_and_reads_the_old_packing() {
	let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
	let no_fog = |mut g: [u8; 15]| {
		g[13] = 0;
		g[14] = 0;
		g
	};
	for lvl in 0..4u8 {
		for row in [1usize, 2, 4, 5, 6, 10] {
			let mut g = no_fog(DEFAULT_ROWS);
			g[row] = lvl;
			g[3] = 0;
			assert_eq!(unpack_gfx(pack_gfx(g)), g, "row {row} level {lvl}");
		}
	}
	let all = [1, 2, 3, 0, 1, 2, 3, 1, 2, 1, 2, 1, 2, 0, 0];
	assert_eq!(unpack_gfx(pack_gfx(all)), all);
	for b in 0..3u8 {
		let mut g = no_fog(DEFAULT_ROWS);
		g[12] = b;
		assert_eq!(unpack_gfx(pack_gfx(g)), g, "draw distance byte {b}");
		assert_eq!(pack_gfx(g) >> 30, b as u32);
	}
	// The old word: lighting 1, "on" bits for AO, shadows, water, particles, sun rays and smoothing, vegetation Lush, no levels.
	let old = 1 | 0x3f << 8 | 2 << 14;
	assert_eq!(unpack_gfx(old), [1, 1, 1, 0, 1, 1, 1, 1, 2, 0, 0, 0, 0, 0, 0], "old on = High = byte 1");
	// An old word with some rows off.
	assert_eq!(unpack_gfx(1 | 1 << 8 | 1 << 11 | 1 << 14), [1, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
	// The new default packs to the same flags as the old default plus the High levels.
	assert_eq!(pack_gfx([0, 1, 1, 0, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0]) & 0xffff, pack_gfx(DEFAULT_ROWS) & 0xffff, "flag bits are unchanged");
	assert_eq!(pack_gfx(DEFAULT_ROWS) >> 16, 3 | 3 << 2 | 3 << 4 | 3 << 6 | 3 << 8, "the word holds levels: bytes of 1 are High = 3");
	assert_eq!([0u8, 1, 2, 3].map(swap_level), [0, 3, 2, 1]);
	let counts: Vec<u8> = (0..15).map(row_values).collect();
	assert_eq!(counts, [2, 4, 4, 2, 4, 4, 4, 2, 3, 2, 4, 2, 3, 4, 4]);
	// Every value of every row has a text, and the new level texts are in the pool.
	for r in ROWS {
		assert!(r[2..].iter().filter(|&&v| v != 0).all(|v| *v < 0x7f00 || NEW_IDS.contains(v)), "row {:#x}", r[0]);
	}
	let slots: std::collections::HashSet<u32> = NEW_IDS.iter().copied().collect();
	assert_eq!(slots.len(), NEW_IDS.len(), "no id twice");
	assert_eq!(NEW_IDS.len(), TEXTS.len());
	assert!(POOL + 0x40 * POOL_SLOTS as u32 <= OPT_RECS, "the string pool ends before the Graphics page copies");
	// The title's table holds the stepper, the longest section's six rows and the zero record, padded to `titlegfx::TAB_RECS`;
	// the pause page's table has `pcctl::TABLE_RECS` records.
	assert!(TGTAB + 20 * titlegfx::TAB_RECS as u32 <= POOL2 && GTAB + 24 * super::super::pcctl::TABLE_RECS as u32 <= TRECS, "the row tables end before the next copies");
	assert!(pool_slot(POOL_SLOTS + POOL2_SLOTS - 1) + 0x40 <= BASE + 0x1000, "the second pool ends before the camopts block");
	assert!(pool_slot(NEW_IDS.len() - 1) + 0x40 <= camopts::CO_POOL_END && POOL3 >= TAB + 9 * 24, "the third pool is inside the camopts block, after the row table copy");
}
