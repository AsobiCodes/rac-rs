//! R&C1's smoke puffs as rows for the engine's volumetric smoke (`Pack::smoke`, engine/smoke_rows.rs; viewer/volsmoke.rs).
//! A row names an effect-pool type (slot byte +0, decomp/notes/particles.md "Common slot header" and the type lists) and says how big
//! and thick the volume is. Types seen in the Novalis savestates (fg1/1_*, play_runs fullgame1 0016..0039; slot histograms 2026-11-02):
//! - 4: smoke4, the grey (alpha-over, rgb ~0.9) puffs of explosions, burning enemies and the Thruster-Pack exhaust; the same type
//!   carries the orange additive fire ball of a blast, so the row is `grey_only` (rgb saturation under `GREY_LIMIT`);
//! - 6: the big dark-grey column puffs (size 200k..380k raw = 1.4..2.6 m side, rgb ~0.4, alpha ~0.8) that rise from the Novalis
//!   burning buildings and the crash trail (hundreds of slots at once), merged into at most the quality's volume cap;
//! - 0x16: the lofting smoke (NOV-9.3), faint (alpha ~0.1), rising slowly;
//! - 0x17: the soft puff (Veldin types: crate fluff 0x0d is not smoke).
//! Texture is `ANY_TEX` for all: the particle texture index of a type differs per level's def table, the type does not.
//!
//! The level fires' own smoke (efx class 760, rac1/fire.rs) is not in the pool: its grey columns (the fire's flame texture, alpha-blended,
//! vertex alpha `COLUMN_ALPHA` on the two middle corners) and volume sheets (the second texture, alpha `VOLUME_ALPHA`) are world quads.
//! `quad_rows` names them per level (`Pack::smoke_quads`) and the engine turns each into a volume while Smoke is on. The fx table keys are
//! the fire lane's scan of every class 760 placement (pvar +0x54 / +0x58 + 0x28; rac1/fire_look.rs): Veldin 41 / 42, Novalis 46 / 47,
//! Oltanis 45 / 42; the 16 other levels place no level fire.
use crate::fire::{COLUMN_ALPHA, VOLUME_ALPHA};
use rac_engine::smoke_rows::{QuadRow, SmokeRow, ANY_TEX};

pub const ROWS: &[SmokeRow] = &[
	SmokeRow { kind: 0x04, tex: ANY_TEX, grey_only: true, radius: 0.5, density: 1.1, rise: 1.2, billow: 0.35 },
	SmokeRow { kind: 0x06, tex: ANY_TEX, grey_only: false, radius: 0.55, density: 0.9, rise: 1.6, billow: 0.45 },
	SmokeRow { kind: 0x16, tex: ANY_TEX, grey_only: true, radius: 0.5, density: 1.4, rise: 1.0, billow: 0.4 },
	SmokeRow { kind: 0x17, tex: ANY_TEX, grey_only: true, radius: 0.5, density: 1.0, rise: 1.0, billow: 0.35 },
];

/// A level fire's grey column: thin, dark, rising slowly in tall billows.
const fn column(key: u32) -> QuadRow {
	QuadRow { key, alpha: COLUMN_ALPHA, radius: 0.45, density: 0.9, rise: 1.6, billow: 0.45 }
}

/// A level fire's volume sheet: the wide plume the six sheets of an emitter make, thinner per square metre.
const fn sheet(key: u32) -> QuadRow {
	QuadRow { key, alpha: VOLUME_ALPHA, radius: 0.4, density: 0.6, rise: 1.3, billow: 0.5 }
}

static VELDIN: &[QuadRow] = &[column(41), sheet(42)];
static NOVALIS: &[QuadRow] = &[column(46), sheet(47)];
static OLTANIS: &[QuadRow] = &[column(45), sheet(42)];

/// The pack's `Pack::smoke_quads`: the smoke quads of loaded level `level`.
pub fn quad_rows(level: usize) -> &'static [QuadRow] {
	match level {
		0 => VELDIN,
		1 => NOVALIS,
		14 => OLTANIS,
		_ => &[],
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn rows_are_distinct_types_with_sane_numbers() {
		for (i, r) in ROWS.iter().enumerate() {
			assert!(ROWS[i + 1..].iter().all(|o| o.kind != r.kind), "type {:#x} twice", r.kind);
			assert!((0.2..1.0).contains(&r.radius) && (0.1..4.0).contains(&r.density) && (0.0..8.0).contains(&r.rise) && (0.0..=1.0).contains(&r.billow));
		}
		assert!(rac_engine::smoke_rows::find(ROWS, 6, 0).is_some() && rac_engine::smoke_rows::find(ROWS, 0x0b, 0).is_none());
	}

	/// Every quad row sits on a level fire's own texture: the column on the flame row's key (the fire lane's table), the sheet on the
	/// other. Smoke never touches the additive flame quad, and a level without a level fire has no quad rows.
	#[test]
	fn quad_rows_follow_the_levels_fires_and_never_take_the_flame() {
		use rac_engine::fire_option::{quad_kind, FireKind};
		use rac_engine::smoke_rows::quad_row;
		for (level, column, sheet) in [(0, 41, 42), (1, 46, 47), (14, 45, 42)] {
			assert_eq!(quad_kind(crate::fire_look::rows(level), column, true), Some(FireKind::Ground), "level {level}: the flame row");
			assert_eq!(quad_row(quad_rows(level), column, false).map(|r| r.alpha), Some(COLUMN_ALPHA));
			assert_eq!(quad_row(quad_rows(level), sheet, false).map(|r| r.alpha), Some(VOLUME_ALPHA));
			assert!(quad_row(quad_rows(level), column, true).is_none() && quad_row(quad_rows(level), sheet, true).is_none(), "level {level}: additive quads stay");
		}
		for level in (0..19).filter(|l| ![0, 1, 14].contains(l)) {
			assert!(quad_rows(level).is_empty(), "level {level}: no level fire, no smoke quads");
		}
		for l in [0, 1, 14] {
			for r in quad_rows(l) {
				assert!((0.2..1.0).contains(&r.radius) && (0.1..4.0).contains(&r.density) && (0.0..8.0).contains(&r.rise) && (0.0..=1.0).contains(&r.billow) && r.alpha > 0.0);
			}
		}
		assert!(rac_engine::pack::NONE.smoke_quads.clone()(1).is_empty(), "a game without fire smoke keeps the quads");
	}
}
