//! The title menu's Language page rows (v0.4.1, issue #26: German and Spanish were swapped).
//!
//! The text ids are numbered English 0x4f2c, French 0x4f2d, Spanish 0x4f2e, German 0x4f2f, Italian 0x4f30, but the game's own
//! page (boot ELF 0x1d48c0, label records at +0x34) lists English (arg 0), French (arg 2), German 0x4f2f (arg 3), Spanish 0x4f2e
//! (arg 4), Italian (arg 5): the row order of `retail::pcopts::Lang` and of the language words 0x15ed88 (0, 2, 3, 4, 5). The native
//! mirror drew the rows in id order, so the row labelled "Spanish" selected German and the row labelled "German" Spanish.

/// The game's page order, for when the page's own records are not at hand (the old non-retail menu, tests).
pub const ROWS: [u32; 5] = [0x4f2c, 0x4f2d, 0x4f2f, 0x4f2e, 0x4f30];

/// The row text ids to draw: the records of the game's page (retail mode, `Menu::lang_rows`) whenever it listed any, else `ROWS`.
/// Both discs list the same five (label records in the boot ELF: PAL 0x1d49a0, USA 0x1d4a20, ids 0x4f2c 0x4f2d 0x4f2f 0x4f2e 0x4f30); a disc with another count keeps the game's rows too.
pub fn rows(from_game: &[u32]) -> Vec<u32> {
	if !from_game.is_empty() { from_game.to_vec() } else { ROWS.to_vec() }
}

/// Takes the rows from the Language page's list item (slot 3) of the game's snapshot, kept while the item is not on screen.
pub fn follow(rows: &mut Vec<u32>, items: &[crate::retail::mouse_menu::Item]) {
	if let Some(it) = items.iter().find(|i| i.slot == 3 && !i.texts.is_empty()) {
		*rows = it.texts.iter().map(|&t| t as u32).collect();
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::frontend::{Menu, Page, Text};

	fn texts() -> Text {
		let mut t = Text::default();
		for (id, s) in [(0x4f2c, "English"), (0x4f2d, "French"), (0x4f2e, "Spanish"), (0x4f2f, "German"), (0x4f30, "Italian")] {
			t.insert(id, s.to_string());
		}
		t
	}

	#[test]
	fn row_order_is_the_games_not_the_text_ids() {
		// Row 3 selects word 3 = German, row 4 word 4 = Spanish (Lang::word): the labels must say so.
		assert_eq!(ROWS[2], 0x4f2f);
		assert_eq!(ROWS[3], 0x4f2e);
		for (i, l) in crate::retail::pcopts::Lang::ALL.into_iter().enumerate() {
			assert_eq!(l.word(), [0, 2, 3, 4, 5][i]);
		}
	}

	#[test]
	fn the_menu_draws_the_rows_in_that_order() {
		let t = texts();
		let mut m = Menu::new(0);
		m.page = Page::Language;
		let labels = |m: &Menu| m.items(&t).into_iter().map(|(s, _)| s).collect::<Vec<_>>();
		assert_eq!(labels(&m), ["English", "French", "German", "Spanish", "Italian"]);
		// The game's own records win whenever it listed any (a disc with another order or count); only an empty list falls back.
		m.lang_rows = vec![0x4f2c, 0x4f2d, 0x4f2e, 0x4f2f, 0x4f30];
		assert_eq!(labels(&m), ["English", "French", "Spanish", "German", "Italian"]);
		m.lang_rows = vec![0x4f2c, 0x4f2d, 0x4f30];
		assert_eq!(labels(&m), ["English", "French", "Italian"]);
		m.lang_rows = Vec::new();
		assert_eq!(labels(&m), ["English", "French", "German", "Spanish", "Italian"]);
	}
}
