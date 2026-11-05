//! Tests of engine/omni/data_map.rs (engine split stage E2, 2026-10-26): they run with R&C1's part of the mod host (rac1/omni_bind.rs: binding,
//! spec files, RAM words), so they live in the R&C1 pack; moved from the engine file's test modules, each test hands that part over first.
#![allow(unused_imports)]
mod tests {
	use crate::omni::data_map::*;
	use std::sync::OnceLock;
	use crate::toc;

	#[test]
	fn parse_reads_sections_and_runs() {
		crate::omni_bind::for_tests();
		let m = DataMap::parse("# c\npal 3\n0x100 0x200 16\n0x300 0x400 -32\nusa -1\n0x10 0x20 0\n").unwrap();
		assert_eq!(m.lookup(false, 3, 0x100), Some(0x110));
		assert_eq!(m.lookup(false, 3, 0x1ff), Some(0x20f));
		assert_eq!(m.lookup(false, 3, 0x200), None);
		assert_eq!(m.lookup(false, 3, 0x310), Some(0x2f0));
		assert_eq!(m.lookup(true, -1, 0x10), Some(0x10));
		assert_eq!(m.lookup(true, 3, 0x100), None);
		assert_eq!(m.lookup(false, 4, 0x100), None);
	}

	#[test]
	fn parse_refuses_damage() {
		crate::omni_bind::for_tests();
		assert!(DataMap::parse("0x1 0x2 0\n").is_err());
		assert!(DataMap::parse("pal 1\n0x5 0x5 0\n").is_err());
		assert!(DataMap::parse("pal 1\nnonsense\n").is_err());
		assert!(DataMap::parse("pal x\n").is_err());
	}

	/// The camera yaw and the other lock-strafe globals, level by level (PAL v1.00): found by the same code in two levels, checked against the
	/// decomp's reference lists (the hero code of Kerwan reads 0x166e58, Eudora 0x166ed8, Novalis 0x1671d8, Veldin finale 0x167758).
	#[test]
	fn lock_strafe_globals() {
		crate::omni_bind::for_tests();
		let m = builtin();
		let yaw = |l| m.lookup(false, l, 0x16_6DD8);
		assert_eq!((yaw(0), yaw(1), yaw(3), yaw(4), yaw(18)), (Some(0x16_6DD8), Some(0x16_71D8), Some(0x16_6E58), Some(0x16_6ED8), Some(0x16_7758)));
		for l in 0..19 {
			assert!(yaw(l).is_some(), "camera yaw unmapped in level {l}");
			assert_eq!(m.lookup(false, l, 0x15_F5C4), Some(0x15_F5A4), "level mode, level {l}");
			assert_eq!(m.lookup(false, l, 0x15_EF40), Some(0x15_EF40), "camera pan, level {l}");
			assert_eq!(m.lookup(true, l, 0x15_F5C4), Some(0x15_F5C4), "USA level mode, level {l}");
		}
	}

	#[test]
	fn no_level_loaded_maps_only_the_head() {
		crate::omni_bind::for_tests();
		let m = builtin();
		assert_eq!(m.lookup(false, -1, 0x15_F5C4), Some(0x15_F5A4));
		assert_eq!(m.lookup(false, -1, 0x16_6DD8), None);
		assert!(m.runs(false, 0) > 10 && m.runs(true, 3) > 10);
	}
}
