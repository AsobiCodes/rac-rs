//! Mouse in the Gadgetron vendor's shop (lane mmenu stage 6, v0.4.2). The shop is no page menu (mode 5, its own update, the same code
//! on all 19 levels and on the USA disc, addresses in `vendor.rs`), so it is shown to the mouse as a one-item menu (id 0xfe =
//! `menu_keys::VENDOR`) and then everything the page menus do applies unchanged: hover and click are the `mouse_instant` Grid rule, the
//! wheel and the right click are `mouse_menu::State`'s.
//!
//! * The browse strip (stage 0, no exit pending; the update in the shop function, Novalis FUN_002afce8 OV:111700..111790): with fewer than
//!   8 rows Right / Left are NEW presses (`0x13cb04` against the last field's word) that move `sel` by one, clamped to 0 .. count - 1, and
//!   run the row's side effects (sound, text box, ammo bar); Cross (row state word +8 == 0) starts a purchase, Triangle leaves. The strip
//!   is a Grid of ONE row whose cursor word is `sel`: the item address is `sel - 0x3c`, so `mouse_instant::plan` writes `sel` (item +
//!   0x3c) to the icon next to the target and adds the ONE real Right / Left press, exactly the page grids' rule. Icons are 56 x 56
//!   cells at overlay (144 + 56 i, 360) (PAL, measured on a captured frame: panel origin (136, 358) + icon origin (12, 6) - frame (8, 2)).
//! * The quantity prompt (stage 2): the only target is the selected icon (a click there = Cross: buys the quantity on show, or the plain
//!   "buy?" of a non-ammo row); a click anywhere else, a click while the shop animates (stage 1 / 3 / 4, leaving, fly-in) or with 8 rows
//!   or more (the cyclic list keys only) does nothing: a stray click never reaches Cross. The right click is Triangle (the prompt's cancel;
//!   in browse the shop's exit). The wheel steps the quantity: up = Right (more), down = Left.
//!
//! Nothing here acts without the mouse: `read` only builds a snapshot; the pad and RAM are touched only by a request the mouse made.

use super::mouse_instant::Request;
use super::mouse_menu::{Item, Kind, MenuSnap, LEFT, RIGHT};
use super::vendor;

/// The snapshot's page id (= `menu_keys::VENDOR.id`).
pub const ID: u32 = 0xfe;
/// `MenuSnap::state`: the strip is live / the quantity prompt is up / the shop animates or is not for the mouse (`MenuSnap::busy`).
const BROWSE: u32 = 2;
const PROMPT: u32 = 3;
const BUSY: u32 = 1;
/// The icon strip: first cell's top-left and the cell size, overlay pixels.
const STRIP: (i32, i32) = (144, 360);
const CELL: i32 = 56;
/// Rows the strip shows without cycling; the update wraps (and scrolls) from 8 rows on.
const MAX_ROWS: u32 = 7;
/// The grid update's cursor word sits at item + 0x3c: the item "address" is chosen so that this word is the shop's `sel`.
const CURSOR_OFF: u32 = 0x3c;

/// The shop as the mouse sees it this field (`rd` = 32-bit word read). Only call it while the shop screen is up (`shop_snap`).
pub fn read(rd: &dyn Fn(u32) -> u32, level: u32) -> Option<MenuSnap> {
	let w = vendor::words(level)?;
	let (sel, stage, count, exitp) = (rd(w.sel), rd(w.stage), rd(w.count), rd(w.exitp));
	let state = match (rd(w.phase), stage, exitp) {
		(1, 0, 0) => BROWSE,
		(1, 2, 0) => PROMPT,
		_ => BUSY,
	};
	let mut m = MenuSnap { page: w.sel, id: ID, state, cur: w.sel - CURSOR_OFF, items: Vec::new(), front: false };
	if !(1..=MAX_ROWS).contains(&count) || sel >= count || state == BUSY {
		m.state = BUSY;
		return Some(m);
	}
	let (rect, kind) = if state == BROWSE {
		([STRIP.0, STRIP.1, CELL * count as i32, CELL], Kind::Grid { rows: 1, cols: count, cursor: sel })
	} else {
		([STRIP.0 + CELL * sel as i32, STRIP.1, CELL, CELL], Kind::Grid { rows: 1, cols: 1, cursor: 0 })
	};
	m.items.push(Item { addr: m.cur, slot: 0, rect, kind, texts: Vec::new() });
	Some(m)
}

/// Is `m` the shop? (A click on no icon there does nothing instead of pressing the screen's confirm button.)
pub fn strict(m: &MenuSnap) -> bool {
	m.id == ID
}

/// Wheel notches on the browse strip as ONE instant request: the highlight moved `|wheel|` icons (up = left), clamped, no wrap. None at
/// the ends and in the prompt (the pulse of `wheel_button` stays).
pub fn wheel_request(m: &MenuSnap, wheel: i32) -> Option<Request> {
	let it = m.items.first().filter(|_| m.id == ID && m.state == BROWSE)?;
	let Kind::Grid { cols, cursor, .. } = it.kind else { return None };
	let n = wheel.unsigned_abs();
	let to = if wheel > 0 { cursor.saturating_sub(n) } else { (cursor + n).min(cols - 1) };
	(to != cursor).then_some(Request { page_id: ID, item: it.addr, sub: to, confirm: false, slide: None })
}

/// The pad button of one wheel notch: on the strip up = Left (the previous icon) and down = Right; in the quantity prompt up = Right
/// (more) and down = Left (fewer).
pub fn wheel_button(m: &MenuSnap, up: bool) -> u16 {
	if (m.state == PROMPT) == up { RIGHT } else { LEFT }
}

/// Is the quantity prompt up? (Its notches are queued one by one: each is a key press of the game's own quantity step.)
pub fn in_prompt(m: &MenuSnap) -> bool {
	m.id == ID && m.state == PROMPT
}

/// Tests only: the shop's words as they are at the start of every field, for the in-game checks (`mouse_vendor_test.rs`):
/// [sel, stage, exit pending, quantity, bolts, ammo of item 10, ammo of item 16, ammo-row mask (bits 0..6) | ammo bar present << 8].
#[cfg(test)]
pub static SEEN: [std::sync::atomic::AtomicU32; 8] = [const { std::sync::atomic::AtomicU32::new(0) }; 8];

#[cfg(test)]
pub fn publish(m: &ee_oracle::Machine) {
	use std::sync::atomic::Ordering;
	let Some(w) = vendor::words(m.level()) else { return };
	let kinds = (0..MAX_ROWS).fold(0u32, |acc, i| acc | ((m.rd32(w.rows + i * 0x14 + 4) == 1) as u32) << i);
	let bar = (m.rd32(w.sel - 8) != u32::MAX) as u32;
	let ammo = |item: u32| m.rd32(super::disc::va(0x13d428) + 4 * item);
	let v = [m.rd32(w.sel), m.rd32(w.stage), m.rd32(w.exitp), m.rd32(w.exitp + 0x1c), m.rd32(super::disc::va(0x15ed98)), ammo(10), ammo(16), kinds | bar << 8];
	for (slot, x) in SEEN.iter().zip(v) {
		slot.store(x, Ordering::Relaxed);
	}
}

#[cfg(not(test))]
pub fn publish(_: &ee_oracle::Machine) {}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::retail::mouse_instant::{plan, Plan};
	use crate::retail::mouse_menu::{State, Target, CROSS, TRIANGLE};
	use std::collections::HashMap;

	/// Novalis-like words (PAL, level 1): sel 0x1ca898, stage 0x1ca89c, count 0x1caa50, phase 0x1ca840, exit pending 0x162218.
	fn ram(vals: &[(u32, u32)]) -> impl Fn(u32) -> u32 + use<> {
		let m: HashMap<u32, u32> = vals.iter().copied().collect();
		move |a| m.get(&a).copied().unwrap_or(0)
	}

	fn shop(sel: u32, stage: u32, count: u32) -> MenuSnap {
		let rd = ram(&[(0x1ca898, sel), (0x1ca89c, stage), (0x1caa50, count), (0x1ca840, 1)]);
		read(&rd, 1).unwrap()
	}

	#[test]
	fn browse_is_a_one_row_grid_whose_cursor_word_is_sel() {
		let m = shop(2, 0, 4);
		assert_eq!((m.id, m.state, m.cur, m.busy()), (ID, BROWSE, 0x1ca898 - 0x3c, false));
		let it = &m.items[0];
		assert_eq!((it.addr, it.rect, &it.kind), (m.cur, [144, 360, 224, 56], &Kind::Grid { rows: 1, cols: 4, cursor: 2 }));
		// The grid cursor word (item + 0x3c) IS sel; hover maps 56 px cells; the pointer off the strip hits nothing.
		assert_eq!(it.addr + 0x3c, 0x1ca898);
		assert_eq!(m.hit(144.0 + 56.0 * 3.0 + 1.0, 361.0), Some(Target::Sub { item: it.addr, sub: 3 }));
		assert_eq!(m.hit(144.0 + 55.9, 415.0), Some(Target::Sub { item: it.addr, sub: 0 }));
		assert_eq!(m.hit(143.0, 380.0), None);
		assert_eq!(m.hit(144.0 + 224.0, 380.0), None);
		assert_eq!(m.hit(200.0, 359.0), None);
	}

	#[test]
	fn the_prompt_has_only_the_selected_icon_and_everything_else_is_busy_or_empty() {
		let p = shop(2, 2, 4);
		assert_eq!((p.state, p.items.len(), p.items[0].rect), (PROMPT, 1, [144 + 112, 360, 56, 56]));
		assert!(in_prompt(&p) && !in_prompt(&shop(2, 0, 4)));
		assert_eq!(p.hit(144.0 + 112.0 + 5.0, 380.0), Some(Target::Sub { item: p.cur, sub: 0 }));
		assert_eq!(p.hit(144.0 + 5.0, 380.0), None);
		// Start, animation and resolve stages, a leaving shop, 8 rows or more, no rows, a fly-in: no target, busy.
		for m in [shop(2, 1, 4), shop(2, 3, 4), shop(2, 4, 4), shop(0, 0, 8), shop(0, 0, 0)] {
			assert!(m.busy() && m.items.is_empty(), "{m:?}");
		}
		let leaving = read(&ram(&[(0x1ca898, 1), (0x1caa50, 4), (0x1ca840, 1), (0x162218, 1)]), 1).unwrap();
		let flying = read(&ram(&[(0x1ca898, 1), (0x1caa50, 4), (0x1ca840, 0)]), 1).unwrap();
		assert!(leaving.busy() && leaving.items.is_empty() && flying.busy() && flying.items.is_empty());
	}

	#[test]
	fn hover_is_the_page_grid_rule_one_real_press_next_to_the_target() {
		let req = |sub, confirm| Request { page_id: ID, item: 0x1ca898 - 0x3c, sub, confirm, slide: None };
		// sel 0 -> icon 3: preset sel = 2 and ONE Right (the game moves 2 -> 3 and runs the row's own side effects).
		assert_eq!(plan(&shop(0, 0, 4), &req(3, false), 0, 0), Plan { writes: vec![(0x1ca898, 2)], pad: RIGHT, ..Plan::default() });
		// sel 3 -> icon 0: preset 1 and Left. Next door: no write, one press.
		assert_eq!(plan(&shop(3, 0, 4), &req(0, false), 0, 0), Plan { writes: vec![(0x1ca898, 1)], pad: LEFT, ..Plan::default() });
		assert_eq!(plan(&shop(1, 0, 4), &req(2, false), 0, 0), Plan { pad: RIGHT, ..Plan::default() });
		// Selected: a click presses Cross in the next field; the page word (sel + 0x80) is never written.
		assert_eq!(plan(&shop(2, 0, 4), &req(2, true), 0, 0), Plan { pad: CROSS, done: true, ..Plan::default() });
		// Busy shop: wait. The prompt's single cell: a click is Cross (the request ends at once, selected).
		assert_eq!(plan(&shop(2, 3, 4), &req(2, true), 0, 0), Plan { done: true, ..Plan::default() });
		assert_eq!(plan(&shop(2, 2, 4), &req(0, true), 0, 0), Plan { pad: CROSS, done: true, ..Plan::default() });
	}

	#[test]
	fn wheel_steps_the_strip_the_prompt_quantity_and_keeps_the_ends() {
		let rq = |sub| Some(Request { page_id: ID, item: 0x1ca898 - 0x3c, sub, confirm: false, slide: None });
		assert_eq!(wheel_request(&shop(1, 0, 4), -2), rq(3));
		assert_eq!(wheel_request(&shop(1, 0, 4), -9), rq(3));
		assert_eq!(wheel_request(&shop(2, 0, 4), 1), rq(1));
		assert_eq!(wheel_request(&shop(3, 0, 4), -1), None);
		assert_eq!(wheel_request(&shop(0, 0, 4), 1), None);
		assert_eq!(wheel_request(&shop(2, 2, 4), 1), None);
		let (b, p) = (shop(1, 0, 4), shop(1, 2, 4));
		assert_eq!((wheel_button(&b, true), wheel_button(&b, false), wheel_button(&p, true), wheel_button(&p, false)), (LEFT, RIGHT, RIGHT, LEFT));
	}

	#[test]
	fn clicks_hover_and_the_right_click_through_the_state() {
		let m = shop(0, 0, 4);
		// Hover icon 2: one request, no pad. A click on an unselected icon asks for the move and the Cross.
		let mut st = State::with_instant(true);
		st.cursor(144.0 + 56.0 * 2.5, 380.0);
		st.click();
		assert_eq!(st.step(Some(&m), 0), 0);
		assert_eq!(st.take_cmd(), Some(crate::retail::mouse_instant::Cmd::Submit(Request { page_id: ID, item: m.cur, sub: 2, confirm: true, slide: None })));
		// A click on the selected icon is NOT a Cross from this (two fields old) snapshot: the worker is asked, it checks the live
		// selection and presses Cross itself when the icon is still the selected one (mmenu F2).
		let mut st = State::with_instant(true);
		st.cursor(144.0 + 10.0, 380.0);
		st.click();
		assert_eq!(st.step(Some(&m), 0), 0, "no Cross from the snapshot");
		assert_eq!(st.take_cmd(), Some(crate::retail::mouse_instant::Cmd::Submit(Request { page_id: ID, item: m.cur, sub: 0, confirm: true, slide: None })));
		// Without the instant path (RAC_MOUSE_INSTANT=0) the pulse stays: Cross at once.
		let mut st = State::with_instant(false);
		st.cursor(144.0 + 10.0, 380.0);
		st.click();
		assert_eq!(st.step(Some(&m), 0), CROSS);
		assert_eq!(st.take_cmd(), None);
		// STRAY clicks: beside the strip in browse, off the selected icon in the prompt, anywhere while the shop animates: nothing at all.
		for (snap, x, y) in [(shop(0, 0, 4), 300.0, 100.0), (shop(0, 0, 4), 144.0 + 230.0, 380.0), (shop(1, 2, 4), 144.0 + 10.0, 380.0), (shop(1, 3, 4), 144.0 + 60.0, 380.0), (shop(1, 1, 4), 144.0 + 60.0, 380.0), (shop(0, 0, 9), 160.0, 380.0)] {
			for instant in [true, false] {
				let mut st = State::with_instant(instant);
				st.cursor(x, y);
				st.click();
				assert_eq!(st.step(Some(&snap), 0), 0, "a stray click at ({x}, {y}) in {snap:?}");
				assert!(!matches!(st.take_cmd(), Some(crate::retail::mouse_instant::Cmd::Submit(_))), "a stray click asks the worker for nothing");
			}
		}
		// The right click: the shop's exit in browse, the prompt's cancel in the prompt (Triangle both); not while animating.
		for (snap, want) in [(shop(0, 0, 4), TRIANGLE), (shop(1, 2, 4), TRIANGLE), (shop(1, 3, 4), 0)] {
			let mut st = State::with_instant(true);
			st.back();
			assert_eq!(st.step(Some(&snap), 0), want);
		}
		// The wheel on the strip is one request; in the prompt one pulse per notch (Right = more), the rest kept for later fields.
		let mut st = State::with_instant(true);
		st.wheel(-1.0);
		st.wheel(-1.0);
		assert_eq!(st.step(Some(&m), 0), 0);
		assert_eq!(st.take_cmd(), Some(crate::retail::mouse_instant::Cmd::Submit(Request { page_id: ID, item: m.cur, sub: 2, confirm: false, slide: None })));
		let prompt = shop(1, 2, 4);
		let mut st = State::with_instant(true);
		st.wheel(1.0);
		st.wheel(1.0);
		assert_eq!(st.step(Some(&prompt), 0), RIGHT);
		let rest: Vec<u16> = (0..8).map(|_| st.step(Some(&prompt), 0)).collect();
		assert_eq!(rest.iter().filter(|&&b| b == RIGHT).count(), 1, "the second notch is a second press: {rest:?}");
	}

	#[test]
	fn a_second_click_of_a_double_click_on_the_prompt_icon_does_not_buy() {
		// Click 1 on a shop icon opens the quantity prompt; click 2 (100 to 450 ms later) lands on the same icon, the prompt's only
		// target, and used to press Cross = buy (mmenu F1). The fields of this test run in microseconds, far inside the guard.
		let (browse, prompt) = (shop(1, 0, 4), shop(1, 2, 4));
		for instant in [true, false] {
			let mut st = State::with_instant(instant);
			st.step(Some(&browse), 0);
			st.step(Some(&prompt), 0);
			for _ in 0..5 {
				st.cursor(144.0 + 56.0 + 10.0, 380.0);
				st.click();
				assert_eq!(st.step(Some(&prompt), 0), 0, "instant={instant}: the click right after the prompt opened presses nothing");
				assert!(!matches!(st.take_cmd(), Some(crate::retail::mouse_instant::Cmd::Submit(_))), "instant={instant}: and asks the worker for nothing");
			}
		}
		// The first click, in the browse strip, is not held back: it asks for the Cross.
		let mut st = State::with_instant(true);
		st.cursor(144.0 + 56.0 + 10.0, 380.0);
		st.click();
		assert_eq!(st.step(Some(&browse), 0), 0);
		assert_eq!(st.take_cmd(), Some(crate::retail::mouse_instant::Cmd::Submit(Request { page_id: ID, item: browse.cur, sub: 1, confirm: true, slide: None })));
	}

	#[test]
	fn the_screen_keys_of_the_shop_are_cross_and_triangle() {
		let s = crate::retail::menu_keys::screen(&shop(0, 0, 4));
		assert_eq!((s.confirm, s.back), (CROSS, TRIANGLE));
	}
}
