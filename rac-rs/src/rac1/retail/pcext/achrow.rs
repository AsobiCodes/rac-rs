//! The Achievements row of the pause Options list (v0.5): between Controls / Mods and Quit Game, it opens the achievements list
//! (`rac_engine::achievements::open`, the same list F6 opens). The title screen's Options has no row: the list is drawn over the
//! pause menu only (`achievements::draw` closes it in a field that is not paused).
//!
//! The row is a retail type-3 record ("go to the page at arg") whose arg is the Quit Game page, so the list stays a plain retail
//! list the mouse, the keyboard and the pad already work (`mouse_menu` reads its text ids and its row cursor). A press is taken
//! BEFORE the game sees it, and decided on the WORKER side from CURRENT RAM (fix round B1 / M3): `worker_field` runs in
//! worker/field_run.rs after `mouse_instant::field` (so an instant mouse click's own Cross is in the pad it sees), reads the list
//! item's cursor out of RAM as the game will see it in this field, and `decide` says whether this field's Cross is a press on the
//! row. If so the Cross is removed from the pad (no Cross reaches retail on that row, so the Quit Game page can never open) and
//! `OPEN_REQ` is raised; `Retail::tick_keys` (input.rs) turns it into `achievements::open()` via `take_open`. A snapshot of the
//! menu (two fields old) is never used. The detour is the host's, not RAM: nothing here writes RAM but the records
//! `patch_graphics` builds (`with_row`), and the two texts are pool slots of `texts.rs` (`ID_ACH` and its description).
use crate::retail::mouse_menu::{Kind, MenuSnap, CROSS};
use std::sync::atomic::{AtomicBool, Ordering};

/// Text: the row label and the description under the list while the row is selected.
pub const ID_ACH: u32 = 0x7f2b;
pub const ID_ACH_DESC: u32 = 0x7f2c;
/// Text: the description instead of `ID_ACH_DESC` once the session cheated (Debug mode was on): why nothing unlocks any more.
pub const ID_ACH_OFF: u32 = 0x7f2f; // 0x7f2d / 0x7f2e = the menu lane's Menu style row (mstyle.rs)

/// The Options page's id in the menu snapshot (state = the same id while it is idle).
const OPTIONS_PAGE: u32 = 8;

/// The first `n` of `base` (the Graphics, Controls and Mods entries that exist), then the entries of `tail`: the Achievements entry
/// and the Quit to Desktop entry (`quitrow.rs`), in this order. `patch_graphics` uses it for the list records, their descriptions
/// and their pictures, so the three stay in step.
pub(super) fn with_row<T, const K: usize>(base: impl IntoIterator<Item = T>, n: usize, tail: [T; K]) -> Vec<T> {
	base.into_iter().take(n).chain(tail).collect()
}

/// What the row does with one field's pad.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Verdict {
	/// The Cross of this field is a press on the row: the game must not see it.
	pub swallow: bool,
	/// The list should open (the host's `take_open` turns it into `achievements::open`).
	pub open: bool,
}

/// The ONE decision of the Achievements row. `live` = the menu read out of RAM in this very field, before the game acts (never a
/// snapshot from the main thread); `pad` = the pad the game is about to get, including a Cross the instant mouse selection added.
/// A Cross is a press on the row when the Options page is open and idle, its list item is the current one and its cursor is on the
/// row of `ID_ACH`; a Cross on any other row, a pad without Cross and any other page are untouched. Self-contained on purpose: the
/// menu lane's worker-side swallow (`modern_swallow::decide`) will call it.
pub fn decide(live: Option<&MenuSnap>, pad: u16) -> Verdict {
	let on_row = row_pressed_on(live.filter(|m| !m.front), pad, ID_ACH);
	Verdict { swallow: on_row, open: on_row }
}

/// Whether this field's pad presses the row whose text id is `id`: Cross is down and `cursor_on` (the Quit to Desktop row asks it
/// at the worker, `quitrow.rs`).
pub fn row_pressed_on(menu: Option<&MenuSnap>, pad: u16, id: u32) -> bool {
	pad & CROSS != 0 && cursor_on(menu, id)
}

/// The Options page is open and idle and the cursor of its current list item is on the row whose text id is `id`.
pub fn cursor_on(menu: Option<&MenuSnap>, id: u32) -> bool {
	let Some(m) = menu.filter(|m| m.id == OPTIONS_PAGE && m.state == OPTIONS_PAGE) else { return false };
	m.items.iter().find(|i| i.addr == m.cur).is_some_and(|it| match it.kind {
		Kind::Label { cursor, .. } => it.texts.get(cursor as usize) == Some(&(id as u16)),
		_ => false,
	})
}

/// The row's text id in a description table: `want` replaces whichever of the two descriptions is there. Returns the index changed.
fn swap_desc(words: &mut [u32], want: u32) -> Option<usize> {
	let k = words.iter().position(|&w| w == ID_ACH_DESC || w == ID_ACH_OFF)?;
	(words[k] != want).then(|| {
		words[k] = want;
		k
	})
}

/// Worker, each field after `patch_graphics`: once the session cheated (`achievements::cheated`, fix round M2) the row's description
/// says why nothing unlocks any more (5 languages, `ID_ACH_OFF`). The id is swapped in place in the description table at `table`
/// (PC region, not retail RAM); idempotent.
pub(super) fn sync_desc(mem: &mut ee_oracle::mem::Mem, table: u32) {
	let mut words: [u32; 15] = std::array::from_fn(|k| mem.rd32(table + 4 * k as u32));
	let want = if crate::achievements::cheated() { ID_ACH_OFF } else { ID_ACH_DESC };
	if let Some(k) = swap_desc(&mut words, want) {
		mem.wr32(table + 4 * k as u32, want);
	}
}

/// Raised by the worker, taken by the main thread (`take_open`).
static OPEN_REQ: AtomicBool = AtomicBool::new(false);

/// Worker, each field after `mouse_instant::field` and before the pad is set: the pad with the Cross taken out when it presses the
/// Achievements row. Reads RAM only when Cross is down in the pause menu of a game with a list (no cost in other fields).
pub fn worker_field(m: &ee_oracle::Machine, front: bool, pad: u16) -> u16 {
	if front || pad & CROSS == 0 || !crate::achievements::available() {
		return pad;
	}
	let rd = |a: u32| m.rd32(a);
	let live = crate::retail::mouse_menu::read(&rd, m.level(), rd(crate::retail::disc::va(0x15f5a4)));
	apply(decide(live.as_ref(), pad), pad)
}

/// The pad after a verdict, and the open request raised.
fn apply(v: Verdict, pad: u16) -> u16 {
	if v.open {
		OPEN_REQ.store(true, Ordering::Release);
	}
	if v.swallow { pad & !CROSS } else { pad }
}

/// Main thread, once per field: the worker swallowed a Cross on the row since the last call; true = open the list.
pub fn take_open() -> bool {
	OPEN_REQ.swap(false, Ordering::AcqRel)
}

#[cfg(test)]
mod tests {
	use crate::retail::mouse_menu::Item;
	use super::*;

	fn list(cursor: u32, texts: Vec<u16>) -> MenuSnap {
		let rows = texts.iter().enumerate().map(|(i, _)| (30.0 + 12.0 * i as f32, true)).collect();
		let it = Item { addr: 0x40, slot: 1, rect: [0, 0, 100, 100], kind: Kind::Label { rows, rowh: 12.0, cursor }, texts, planet_texts: Vec::new() };
		MenuSnap { page: 1, id: 8, state: 8, cur: 0x40, items: vec![it], front: false }
	}

	/// Ids as the Options list now holds them: the retail six, Graphics, Controls, Achievements, Quit Game.
	fn ids() -> Vec<u16> {
		vec![0x4f1e, 0x4f1f, 0x4f20, 0x4f22, 0x4f23, 0x4f24, 0x7f13, 0x7f70, ID_ACH as u16, 0x4f26]
	}

	const TAKEN: Verdict = Verdict { swallow: true, open: true };
	const NONE: Verdict = Verdict { swallow: false, open: false };

	#[test]
	fn cross_on_the_row_is_taken_and_nothing_else_is() {
		assert_eq!(decide(Some(&list(8, ids())), CROSS), TAKEN);
		assert_eq!(decide(Some(&list(8, ids())), CROSS | 0x10), TAKEN, "Cross with another button down");
		assert_eq!(decide(Some(&list(8, ids())), 0), NONE, "no press");
		assert_eq!(decide(Some(&list(8, ids())), 0x20), NONE, "Circle is back, not a press");
		for other in [0, 1, 2, 3, 4, 5, 6, 7, 9] {
			assert_eq!(decide(Some(&list(other, ids())), CROSS), NONE, "row {other}: the game's own Cross");
		}
		assert_eq!(decide(None, CROSS), NONE);
	}

	#[test]
	fn only_the_idle_options_page_counts() {
		let mut m = list(8, ids());
		m.state = 1;
		assert_eq!(decide(Some(&m), CROSS), NONE, "flying in");
		m.state = 0x14;
		assert_eq!(decide(Some(&m), CROSS), NONE, "closing");
		let mut m = list(8, ids());
		m.id = 7;
		m.state = 7;
		assert_eq!(decide(Some(&m), CROSS), NONE, "another page that has such a row id");
		let mut m = list(8, ids());
		m.cur = 0x99;
		assert_eq!(decide(Some(&m), CROSS), NONE, "the list is not the current item");
		let mut m = list(8, ids());
		m.items[0].kind = Kind::Other;
		assert_eq!(decide(Some(&m), CROSS), NONE);
		let mut m = list(8, ids());
		m.front = true;
		assert_eq!(decide(Some(&m), CROSS), NONE, "the title's menu has no such row");
	}

	/// B1: an instant mouse click (the worker adds the Cross itself, `mouse_instant::plan`) on the row opens the list and no Cross
	/// reaches retail; a click on any other row keeps its Cross.
	#[test]
	fn an_instant_mouse_click_on_the_row_is_swallowed_and_other_rows_keep_their_cross() {
		use crate::retail::mouse_instant::{plan, Request};
		let req = |sub| Request { page_id: 8, item: 0x40, sub, confirm: true, slide: None };
		let live = list(8, ids());
		let p = plan(&live, &req(8), 0, 0);
		assert_eq!((p.pad, p.done), (CROSS, true), "the plan adds the Cross on the selected row");
		assert_eq!(decide(Some(&live), p.pad), TAKEN);
		let live = list(9, ids());
		let p = plan(&live, &req(9), 0, 0);
		assert_eq!(p.pad, CROSS);
		assert_eq!(decide(Some(&live), p.pad), NONE, "Quit Game's click is the game's");
		// The click that still has to move the selection adds no Cross: nothing to take (the Cross comes a field later).
		let p = plan(&list(3, ids()), &req(8), 0, 0);
		assert_eq!(p.pad & CROSS, 0);
		// The flag and the pad: the worker's glue (`apply`) removes the Cross and raises the request once.
		let _ = take_open();
		assert_eq!(apply(decide(Some(&list(8, ids())), CROSS | 0x10), CROSS | 0x10), 0x10, "only the Cross is taken");
		assert!(take_open());
		assert!(!take_open(), "taken once");
		assert_eq!(apply(decide(Some(&list(9, ids())), CROSS), CROSS), CROSS);
		assert!(!take_open(), "no request for another row");
	}

	/// M3: the decision is the LIVE cursor, not a snapshot of earlier fields: Down then Cross within two fields presses the row the
	/// game is on now.
	#[test]
	fn a_cross_right_after_a_cursor_move_follows_the_live_cursor() {
		// Onto the row (Controls was selected in the snapshot, the game moved to Achievements): the Cross is taken.
		assert_eq!(decide(Some(&list(8, ids())), CROSS), TAKEN);
		// Off the row (Achievements was selected, the game moved up to Controls): the Cross is the game's.
		assert_eq!(decide(Some(&list(7, ids())), CROSS), NONE);
		// The cursor word is the item's record index; the id at it is what counts (a list without Mods has the row one earlier).
		let mut short = ids();
		short.remove(7);
		assert_eq!(decide(Some(&list(7, short.clone())), CROSS), TAKEN, "Achievements is the 8th row without Controls");
		assert_eq!(decide(Some(&list(8, short)), CROSS), NONE);
	}

	/// The wiring: input.rs has no snapshot-based test any more, and the worker asks after the instant selection added its Cross.
	#[test]
	fn the_decision_is_made_on_the_worker_after_the_instant_cross() {
		assert!(!include_str!("../input.rs").contains("row_pressed"), "no snapshot path in tick_keys");
		assert!(include_str!("../input.rs").contains("achrow::take_open()"));
		let w = include_str!("../worker/field_run.rs");
		let (a, b) = (w.find("mouse_instant::field(").unwrap(), w.find("achrow::worker_field(").unwrap());
		assert!(a < b && b < w.find("m.set_pad(").unwrap(), "after mouse_instant::field, before set_pad");
	}

	#[test]
	fn the_tail_rows_wear_the_last_slots_before_quit_game_and_the_other_entries_keep_their_order() {
		assert_eq!(with_row([1, 2, 3], 3, [9, 8]), vec![1, 2, 3, 9, 8]);
		assert_eq!(with_row([1, 2, 3], 2, [9, 8]), vec![1, 2, 9, 8], "no Mods row");
		assert_eq!(with_row([[1u32, 2], [3, 4]], 1, [[9, 9], [8, 8]]), vec![[1, 2], [9, 9], [8, 8]]);
	}

	#[test]
	fn the_description_says_why_once_the_session_cheated() {
		let mut t = [0x4f1e, 0x7f13, 0x7f70, ID_ACH_DESC, 0x4f26, 0, 0, 0, 0, 0, 0, 0];
		assert_eq!(swap_desc(&mut t, ID_ACH_DESC), None, "nothing to change");
		assert_eq!(swap_desc(&mut t, ID_ACH_OFF), Some(3));
		assert_eq!(t[3], ID_ACH_OFF);
		assert_eq!(swap_desc(&mut t, ID_ACH_OFF), None, "idempotent");
		assert_eq!(swap_desc(&mut t, ID_ACH_DESC), Some(3), "and back");
		assert_eq!(swap_desc(&mut [0x4f1e, 0x4f26], ID_ACH_OFF), None, "no such row");
	}

	#[test]
	fn the_texts_exist_in_five_languages_and_fit_the_row() {
		for id in [ID_ACH, ID_ACH_DESC, ID_ACH_OFF] {
			let k = super::super::NEW_IDS.iter().position(|&n| n == id).expect("listed in NEW_IDS");
			assert!(super::super::TEXTS[k].iter().all(|t| !t.is_empty()), "text {id:#x}");
			let max = if id == ID_ACH { 22 } else { 40 };
			assert!(super::super::TEXTS[k].iter().all(|t| t.len() <= max), "text {id:#x} fits");
			assert_eq!(super::super::texts::pool_slot(k) % 0x40, 0);
		}
		let k = super::super::NEW_IDS.iter().position(|&n| n == ID_ACH).unwrap();
		assert_eq!((super::super::TEXTS[k][0], super::super::TEXTS[k][1]), (&b"Achievements"[..], &b"Succ\x8ds"[..]));
	}
}
