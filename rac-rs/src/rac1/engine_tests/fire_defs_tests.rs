//! Fire rows against the disc (review A21): the rows of `fire_look.rs` are particle EFFECT TYPES; here each is checked against the 19
//! PAL levels' particle defs. USA needs no table of its own: type numbers are code-defined, the same on both discs. Without the disc
//! the test returns early.

use crate::fire_look::rows;
use crate::level::Level;
use rac_engine::fire_option::sprite_kind;
use std::path::Path;

const ISO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso");

/// Every fire row type is a real def (not the unset fallback) on at least one level with its first frame a particle texture that
/// exists, and no other type (the explosion flipbooks 5 / 8, the puff 16, type 64, the unset type 14) is classified as fire.
#[test]
fn fire_row_types_are_real_defs_and_nothing_else_is_fire() {
	if !Path::new(ISO).is_file() {
		return;
	}
	let mut d = crate::iso::Disc::open(Path::new(ISO)).unwrap();
	let levels = crate::toc::read_levels(&mut d).unwrap();
	assert!(levels.len() >= 19);
	let fire: Vec<usize> = rows(5).iter().filter(|r| r.kind.is_sprite()).map(|r| r.key as usize).collect();
	assert!(!fire.is_empty());
	let mut real = std::collections::BTreeSet::new();
	let mut flipbooks = std::collections::BTreeSet::new();
	for (i, lv) in levels.iter().enumerate().take(19) {
		let l = Level::load(&mut d, lv).unwrap();
		let (runs, texs) = (l.particle_def_runs().unwrap(), l.particle_textures().unwrap());
		for (ty, r) in runs.iter().enumerate() {
			let is_fire = sprite_kind(rows(i), ty as u8).is_some();
			assert_eq!(is_fire, fire.contains(&ty), "level {i}: type {ty} is classified like the table says");
			if is_fire && !r.unset {
				let first = r.run[0] as usize;
				assert!(texs.get(first).is_some_and(|t| t.is_some()), "level {i}: fire type {ty} frame 0 = texture {first} exists");
				real.insert(ty);
			}
			if [5, 8].contains(&ty) && !r.unset && r.run.len() >= 6 {
				flipbooks.insert(ty);
			}
		}
	}
	for ty in &fire {
		assert!(real.contains(ty), "fire row type {ty} has a real def on at least one level");
	}
	assert_eq!(flipbooks.len(), 2, "the explosion flipbooks 5 and 8 exist as real multi-frame defs, and are not fire rows");
}
