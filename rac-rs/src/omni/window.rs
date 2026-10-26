//! The overlay window: binding fields for data that lives in the level overlay, whose address differs in every level.
//!
//! A field of the binding has one fixed address, but the overlay data (price rows, menu tables, ...) moves with the level (data_map.rs). A field
//! whose address is in the window `0x40000000 + <reference address>` (the reference is the format's numbering, the NTSC-U level-0 program, like
//! `relocate`) is read and written at the address that global has in the level loaded now: `read_mem` and `write_mem` (host.rs) translate it. The
//! game's RAM is 32 MiB, so no real address is in the window. Outside a level (title screen, a load: level word not 0..18) only the head of the
//! overlay data is mapped and most window addresses fail like an unreadable address.

use super::native_host::{loaded_level_of, relocate_in};

/// Start of the window; the reference address is added to it.
pub const OVERLAY_WINDOW: u32 = 0x4000_0000;
/// Size of the window (the reference addresses are below 16 MiB).
const WINDOW_SIZE: u32 = 0x0100_0000;

/// Whether `addr` is in the window.
pub fn in_window(addr: u32) -> bool {
	(OVERLAY_WINDOW..OVERLAY_WINDOW + WINDOW_SIZE).contains(&addr)
}

/// The real address for a window address with `len` bytes behind it, in the program of the level whose word is `level_word` on the PAL
/// (`usa` false) or USA disc; None when the global is not mapped in this level or the span crosses out of its run.
pub fn translate(usa: bool, level_word: i32, addr: u32, len: u32) -> Option<u32> {
	if !in_window(addr) {
		return None;
	}
	let reference = addr - OVERLAY_WINDOW;
	let level = loaded_level_of(level_word);
	let real = relocate_in(usa, level, reference)?;
	if len > 1 {
		// the last byte must sit in the same run (a span that crosses two runs is not one object)
		let last = relocate_in(usa, level, reference.checked_add(len - 1)?)?;
		if last.wrapping_sub(real) != len - 1 {
			return None;
		}
	}
	Some(real)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The price rows (retail ammo caps live at row + 0xe) on the levels the data map covers; Veldin 0x1c4030 and Novalis 0x1c4430 are the numbers
	/// of the weapon notes (omni_weapons.md), the others come from the same map.
	#[test]
	fn price_row_per_level() {
		let row = OVERLAY_WINDOW + 0x1C_40B0 + 10 * 0x18 + 0xE;
		assert_eq!(translate(false, 0, row, 2), Some(0x1C_4030 + 10 * 0x18 + 0xE));
		assert_eq!(translate(false, 1, row, 2), Some(0x1C_4430 + 10 * 0x18 + 0xE));
		assert_eq!(translate(false, 3, row, 2), Some(0x1C_40B0 + 10 * 0x18 + 0xE));
		assert_eq!(translate(true, 0, row, 2), Some(0x1C_40B0 + 10 * 0x18 + 0xE));
	}

	#[test]
	fn outside_the_window_or_unmapped() {
		assert_eq!(translate(false, 0, 0x1C_40B0, 2), None);
		assert_eq!(translate(false, 0, OVERLAY_WINDOW + 0xFF_FFF0, 2), None);
		// title screen / menu: only the head of the overlay data is mapped
		assert_eq!(translate(false, 99, OVERLAY_WINDOW + 0x1C_40B0, 2), None);
		assert_eq!(translate(false, -1, OVERLAY_WINDOW + 0x15_F5C4, 4), Some(0x15_F5A4));
	}

	#[test]
	fn executable_globals_are_not_moved() {
		assert_eq!(translate(false, 3, OVERLAY_WINDOW + 0x14_0408, 4), Some(0x14_0408));
	}
}
