//! [PC] R&C1's fire for the Fire option (engine/fire_option.rs, viewer/firefx.rs): which textures are flames, explosions and the
//! level fires' billboards. The engine draws them as realistic procedural fire when Fire is on; this table only names them.
//!
//! - Level fires (efx class 760, rac1/fire.rs): the fx table index of the flame billboard. Its ADDITIVE quads are the flame (drawn as a
//!   tongue); its alpha-blended quads, the grey column and the volume sheet, are the Smoke option's (rac1/smoke_table.rs `quad_rows`).
//!   Veldin 41, Novalis 46, Oltanis 45 [C: pvar +0x54 + 0x28 of every
//!   class 760 placement in extracted/levels/*/gameplay.instances; the three images are byte-identical]. The volume textures (42 / 47)
//!   stay retail. The other 16 levels place no class 760 at all (scan 2026-11-02, HANDOFF scratch_perf_fire), so their level-fire
//!   list is empty ("none", test `levels_without_class_760_have_no_level_fire_rows`) and only the pool-sprite rows apply.
//! - Effect fire: the rows are the pool's particle EFFECT TYPES (the slot's byte +0; the same numbers in every level and on PAL and USA,
//!   they are code-defined; the extracted `particles/N` folder = the def of type N [C: all 19 PAL levels, test `fire_defs_tests`]).
//!   Types 19, 44, 72, 75 (one flame blob / orange streak), 15, 21 (the embers' fire image) are flames; 55 (red-yellow burst) is an
//!   explosion. The Pyrocitor's effects (decomp/notes/particles.md "Pyrocitor effect-pool", weapons_framework.md "Pyrocitor interrupted-
//!   session recovery trace"): 12 = the stream's segments (the fireball def, 16 frames: orange, then green for the upgraded Pyrocitor,
//!   `Jet`: joined into one flame jet), 2 = its sub-bursts (`JetBurst`) and 25 (0x19) = its embers (`JetSpark`); types 2 and 25 are
//!   generic (ship exhaust puffs, other sparks), so the engine takes them over only within reach of a jet. 4 = the burn puffs of an
//!   enemy set alight (`FUN_00270df0`, a type-4 ctor, orange 0x7000a0ff or green 0x7000ffa0 for the upgraded gun, [C] particles.md
//!   PAR-6): `Burn`, a flame while the colour is warm or green (type 4 is also grey smoke, Smoke's row). A type is one def, so a flipbook is fire
//!   in all its frames or in none (an image hash would not do: the embers' images are also frames of the explosion types 5 / 8 and the
//!   puff 16). Type 14 is no row: on 13 of 19 levels its def is unset and falls back to the level's first image. Sparks and the other
//!   types stay retail.
use rac_engine::fire_option::{FireKind, FireRow};

/// A pool sprite row: the effect type `ty` (`PoolFx::sprite_types`).
const fn ty(ty: u32, kind: FireKind) -> FireRow {
	FireRow { key: ty, kind, dark: false }
}

/// A flame row that is drawn as fire but never lights (`FireRow::dark`): type 15 = the fire image's flying embers (also the
/// Blaster's impact embers on Batalia, the R.Y.N.O. blast fire's embers; fire_pick_log 2026-11-07: up to 0.27 big, lane rfire).
const fn ember(ty: u32) -> FireRow {
	FireRow { key: ty, kind: FireKind::Flame, dark: true }
}

macro_rules! table {
	($($quad:expr),*) => {
		&[
			$($quad,)*
			ty(2, FireKind::JetBurst), ty(4, FireKind::Burn), ty(12, FireKind::Jet), ember(15), ty(19, FireKind::Flame), ty(21, FireKind::Flame),
				ty(25, FireKind::JetSpark), ty(5, FireKind::Blast), ty(8, FireKind::Blast),
			ty(44, FireKind::Flame), ty(55, FireKind::Burst), ty(72, FireKind::Flame), ty(75, FireKind::Flame),
		]
	};
}

static SPRITES: &[FireRow] = table!();
static VELDIN: &[FireRow] = table!(FireRow { key: 41, kind: FireKind::Ground, dark: false });
static NOVALIS: &[FireRow] = table!(FireRow { key: 46, kind: FireKind::Ground, dark: false });
static OLTANIS: &[FireRow] = table!(FireRow { key: 45, kind: FireKind::Ground, dark: false });

/// Moby classes that are fire and draw as meshes (not as pool sprites): 112 the blast fireball (decomp/notes/particles.md "Fireball 112 and
/// shards 122 draw like every moby") and 179 the Pyrocitor's pilot flame (viewer/smooth.rs `PYRO_PILOT`). The same in every level.
static MOBYS: &[u32] = &[112, 179];

/// The pack's `Pack::fire_mobys`: the fire mobys of loaded level `level`.
pub fn mobys(_level: usize) -> &'static [u32] {
	MOBYS
}

/// The pack's `Pack::fire_rows`: the fire textures of loaded level `level`.
pub fn rows(level: usize) -> &'static [FireRow] {
	match level {
		0 => VELDIN,
		1 => NOVALIS,
		14 => OLTANIS,
		_ => SPRITES,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use rac_engine::fire_option::{blast_type, quad_kind, sprite_kind};

	#[test]
	fn levels_name_their_fire_and_every_level_has_the_particle_rows() {
		assert_eq!(quad_kind(rows(1), 46, true), Some(FireKind::Ground));
		assert_eq!(quad_kind(rows(1), 46, false), None, "the grey column belongs to Smoke");
		assert_eq!(quad_kind(rows(1), 47, true), None, "the volume stays retail");
		assert_eq!(quad_kind(rows(0), 41, true), Some(FireKind::Ground));
		assert_eq!(quad_kind(rows(14), 45, true), Some(FireKind::Ground));
		assert_eq!(quad_kind(rows(14), 42, true), None, "the volume stays retail");
		assert_eq!(quad_kind(rows(5), 46, true), None);
		for l in 0..19 {
			assert_eq!(sprite_kind(rows(l), 19), Some(FireKind::Flame));
			assert_eq!(sprite_kind(rows(l), 12), Some(FireKind::Jet), "the Pyrocitor's stream segments");
			assert_eq!(sprite_kind(rows(l), 2), Some(FireKind::JetBurst), "its sub-bursts");
			assert_eq!(sprite_kind(rows(l), 25), Some(FireKind::JetSpark), "its embers (0x19)");
			assert_eq!(sprite_kind(rows(l), 4), Some(FireKind::Burn), "the burn puffs of an enemy set alight");
			assert_eq!(sprite_kind(rows(l), 55), Some(FireKind::Burst));
			for retail in [5u8, 8, 14, 16, 64] {
				assert_eq!(sprite_kind(rows(l), retail), None, "type {retail} stays retail");
			}
			// The explosions (5, 8) only start a ground fire; type 55 only with the dev knob; the Pyrocitor's types never.
			for blast in [5u8, 8] {
				assert!(blast_type(rows(l), blast, false), "type {blast} is an explosion");
			}
			assert!(!blast_type(rows(l), 55, false) && blast_type(rows(l), 55, true));
			for no in [12u8, 2, 25, 4, 19, 16, 14, 64, 15, 21, 44, 72, 75] {
				assert!(!blast_type(rows(l), no, true), "type {no} must not start a ground fire");
			}
		}
	}

	/// Lane rfire review: the embers (type 15) are drawn as flames but never light; the real flames do.
	#[test]
	fn embers_draw_as_flames_but_never_light() {
		use rac_engine::fire_option::sprite_dark;
		for l in 0..19 {
			assert_eq!(sprite_kind(rows(l), 15), Some(FireKind::Flame));
			assert!(sprite_dark(rows(l), 15), "level {l}: embers light nothing");
			for lit in [19u8, 21, 44, 72, 75, 4, 12, 55] {
				assert!(!sprite_dark(rows(l), lit), "level {l}: type {lit} lights");
			}
		}
	}

	/// Scan 2026-11-02 over all 19 levels' gameplay.instances: class 760 is placed only on Veldin (41), Novalis (46), Oltanis (45).
	#[test]
	fn levels_without_class_760_have_no_level_fire_rows() {
		for l in (0..19).filter(|l| ![0, 1, 14].contains(l)) {
			assert!(rows(l).iter().all(|r| r.kind != FireKind::Ground), "level {l}: a level fire row without a class 760 placement");
		}
	}

	#[test]
	fn fire_mobys_are_the_fireball_and_the_pilot_flame_in_every_level() {
		for l in 0..19 {
			assert_eq!(mobys(l), &[112, 179]);
		}
		assert!(rac_engine::pack::NONE.fire_mobys.clone()(1).is_empty(), "a game without fire mobys keeps the retail look");
	}

	#[test]
	fn rows_have_no_duplicates() {
		for l in 0..19 {
			let r = rows(l);
			for (i, a) in r.iter().enumerate() {
				assert!(!r[i + 1..].iter().any(|b| b.key == a.key && b.kind == a.kind), "level {l}: duplicate row {a:?}");
			}
		}
	}
}
