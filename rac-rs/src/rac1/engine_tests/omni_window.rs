//! Tests of engine/omni/window.rs (engine split stage E2, 2026-10-26): they run with R&C1's part of the mod host (rac1/omni_bind.rs: binding,
//! spec files, RAM words), so they live in the R&C1 pack; moved from the engine file's test modules, each test hands that part over first.
#![allow(unused_imports)]
mod tests {
	use crate::omni::window::*;
	use crate::omni::native_host::loaded_level_of;
	use crate::omni::native_host::relocate_in;
	use crate::toc;

	/// The price rows (retail ammo caps live at row + 0xe) on the levels the data map covers; Veldin 0x1c4030 and Novalis 0x1c4430 are the numbers
	/// of the weapon notes (omni_weapons.md), the others come from the same map.
	#[test]
	fn price_row_per_level() {
		crate::omni_bind::for_tests();
		let row = OVERLAY_WINDOW + 0x1C_40B0 + 10 * 0x18 + 0xE;
		assert_eq!(translate(false, 0, row, 2), Some(0x1C_4030 + 10 * 0x18 + 0xE));
		assert_eq!(translate(false, 1, row, 2), Some(0x1C_4430 + 10 * 0x18 + 0xE));
		assert_eq!(translate(false, 3, row, 2), Some(0x1C_40B0 + 10 * 0x18 + 0xE));
		assert_eq!(translate(true, 0, row, 2), Some(0x1C_40B0 + 10 * 0x18 + 0xE));
	}

	#[test]
	fn outside_the_window_or_unmapped() {
		crate::omni_bind::for_tests();
		assert_eq!(translate(false, 0, 0x1C_40B0, 2), None);
		assert_eq!(translate(false, 0, OVERLAY_WINDOW + 0xFF_FFF0, 2), None);
		// title screen / menu: only the head of the overlay data is mapped
		assert_eq!(translate(false, 99, OVERLAY_WINDOW + 0x1C_40B0, 2), None);
		assert_eq!(translate(false, -1, OVERLAY_WINDOW + 0x15_F5C4, 4), Some(0x15_F5A4));
	}

	#[test]
	fn executable_globals_are_not_moved() {
		crate::omni_bind::for_tests();
		assert_eq!(translate(false, 3, OVERLAY_WINDOW + 0x14_0408, 4), Some(0x14_0408));
	}
}
