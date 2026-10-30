//! Yes / No answers with the mouse (lane mmenu, v0.4.2). Some page-menu screens ask a question whose answers are PAD BUTTONS drawn as
//! text (' Yes' = Circle, ' No' = Triangle), not rows with a cursor: the in-level Quit Game page (0xd, item update 0x27bda0 draws
//! 'No' and 'Yes' itself, Circle quits, Triangle goes back, Cross does nothing) and the title's New Game overwrite box (one label item
//! whose two records are the hint texts 0x524f / 0x524b, `menu_keys::screen`). There is nothing to select, so a hover has no say; a
//! click on an answer presses its button and a click anywhere else does nothing (before, a click on no row pressed the screen's
//! confirm button, which is Yes: a stray click quit the game or overwrote the save).
//!
//! The zones: a label item of hint texts gets one zone per record row (the same row bands as `MenuSnap::hit`, record text -> button
//! through `mouse_menu::hint_button`); the Quit Game item draws 'No' at y = h - 0x28 and 'Yes' at y = h - 0x14 (item-local, font 12) from
//! x = (w - widest text) / 2 [C: Veldin FUN_0027be68]. Measured on a captured frame (PAL German, item rect 173,125,166,168): the item's
//! origin is its rect's top-left, the 'No' line is centred at overlay y 261 and the 'Yes' line at y 281 (= y + 8: the 16 px text hangs
//! from y), both 72 px wide centred on the item; the zones are the two 20 px bands (y - 2 .. y + 18) 120 px wide centred on the item.

use super::mouse_menu::{hint_button, Kind, MenuSnap, CIRCLE, TRIANGLE};

/// The in-level Quit Game page (and the question boxes the title front end shares the idea with).
pub const QUIT_GAME: u32 = 0xd;

/// A zone: x0, y0, x1, y1 in overlay pixels, and the pad button a click on it presses.
pub type Zone = ([f32; 4], u16);

/// The answer zones of `m` (empty: not a question screen).
pub fn zones(m: &MenuSnap) -> Vec<Zone> {
	let mut out = Vec::new();
	for it in &m.items {
		let [rx, ry, rw, rh] = it.rect.map(|v| v as f32);
		let btns: Vec<Option<u16>> = it.texts.iter().map(|&t| hint_button(t)).collect();
		if let (Kind::Label { rows, rowh, .. }, true) = (&it.kind, btns.len() >= 2 && btns.iter().all(Option::is_some)) {
			for (&(cy, _), b) in rows.iter().zip(&btns) {
				out.push(([rx, ry + cy - rowh * 0.5 - 0.5, rx + rw, ry + cy + rowh * 0.5 + 0.5], b.unwrap_or(0)));
			}
		} else if !m.front && m.id == QUIT_GAME && matches!(it.kind, Kind::Other) && rw > 0.0 {
			let (cx, no_y, yes_y) = (rx + rw * 0.5, ry + rh - 0x28 as f32, ry + rh - 0x14 as f32);
			out.push(([cx - 60.0, no_y - 2.0, cx + 60.0, yes_y - 2.0], TRIANGLE));
			out.push(([cx - 60.0, yes_y - 2.0, cx + 60.0, yes_y + 18.0], CIRCLE));
		}
	}
	out
}

/// The button of the answer under (x, y).
pub fn press_at(m: &MenuSnap, x: f32, y: f32) -> Option<u16> {
	zones(m).into_iter().find(|(z, _)| x >= z[0] && x < z[2] && y >= z[1] && y < z[3]).map(|z| z.1)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::retail::mouse_menu::Item;

	fn snap(id: u32, front: bool, items: Vec<Item>) -> MenuSnap {
		MenuSnap { page: 0x1b0000, id, state: id, cur: 0, items, front }
	}

	#[test]
	fn a_label_of_hint_texts_answers_with_its_rows_buttons() {
		// The overwrite box: records 0x524f ('Yes', Circle) and 0x524b ('No', Triangle), rows not selectable.
		let it = Item { addr: 0x100, slot: 0, rect: [100, 200, 200, 60], kind: Kind::Label { rows: vec![(20.0, false), (40.0, false)], rowh: 20.0, cursor: 0 }, texts: vec![0x524f, 0x524b] };
		let m = snap(0x1a, true, vec![it]);
		assert_eq!(press_at(&m, 150.0, 220.0), Some(CIRCLE));
		assert_eq!(press_at(&m, 150.0, 240.0), Some(TRIANGLE));
		assert_eq!(press_at(&m, 150.0, 300.0), None);
		assert_eq!(press_at(&m, 50.0, 220.0), None);
	}

	#[test]
	fn quit_game_has_two_bands_and_nothing_else_does() {
		let it = Item { addr: 0x200, slot: 0, rect: [100, 100, 300, 200], kind: Kind::Other, texts: Vec::new() };
		let q = snap(QUIT_GAME, false, vec![it.clone()]);
		// y = 100 + 200 - 0x28 = 260: 'No' band 258..278, 'Yes' 278..298.
		assert_eq!(press_at(&q, 250.0, 262.0), Some(TRIANGLE));
		assert_eq!(press_at(&q, 250.0, 285.0), Some(CIRCLE));
		assert_eq!(press_at(&q, 250.0, 150.0), None);
		assert_eq!(press_at(&q, 120.0, 285.0), None);
		assert!(zones(&snap(8, false, vec![it.clone()])).is_empty());
		assert!(zones(&snap(QUIT_GAME, true, vec![it])).is_empty());
	}
}
