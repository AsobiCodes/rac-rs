//! Instant mouse selection in the page menus (lane mmenu, v0.4.2): the highlight is on the hovered row in the field the pointer
//! asks for it, instead of crawling after the pointer on D-pad pulses (3-5 fields each, up to 24 of them).
//!
//! `mouse_menu::State` (main thread) only says WHAT should be selected (`Request`: a row of an item of a page). The worker thread
//! (`field`, called from worker/field_run.rs before the pad is set) reads the menu out of RAM as it is right now (no snapshot lag),
//! and takes the shortest way the game's own code allows [C: Novalis list update FUN_0028df18, multi-choice update FUN_002945e0,
//! page engine FUN_00218da0; the same code on all 19 levels and on the USA disc, items are found by pointers]:
//!
//! * the row is on ANOTHER item: the engine's own "next item" word, page+0x80, is set to the item (what a D-pad edge of the
//!   current item writes: `*(page + 0x80) = item + 0x38 / 0x3c`), and the row cursor of the target item is set. The engine
//!   swaps the items at the end of the pass (deselect / select callbacks run as for a D-pad move) and the current item's
//!   update plays the move sound because page+0x80 is non-zero. No pad press is needed.
//! * the row is on the CURRENT item: its cursor is set to the row next to the target and ONE D-pad edge in the right direction
//!   is added to the pad of the same field, so the game itself moves the last step (state, fade counters and the move sound
//!   exactly as for a key press; a cursor written under the update's feet would move silently). Any distance is one field.
//! * the row is already selected: a click adds Cross in the next field (the field after the move, never the same one: the
//!   update acts on the cursor it had at the start of the field).
//! * a grid cell (Weapons, Quick Select, Items, the Gadgets groups: update FUN_00278e30, cursor +0x3c, `cols` +0x44) is the same as a
//!   list row: the cell next to the target is preset and ONE D-pad edge (Down / Up when the row differs, else Right / Left) makes the
//!   last step, so any distance is one field and the game plays its own move sound.
//! * a Sound Options bar (`mouse_slider.rs`, `Request::slide`): once the row is selected the volume word is preset to one step short
//!   of the wanted value and Right / Left is held for ONE field (the update adds / subtracts 3 per held field and refreshes the
//!   mixer variables itself), so the value is one a run of key presses reaches and every other word is the game's own.
//!
//! Nothing here runs unless the mouse asked (no `Request`): with no mouse input the pad and RAM are untouched. Other item kinds
//! (grid, ring, panel, hint lines) keep the pulses of `mouse_menu::State`. RAC_MOUSE_INSTANT=0 turns it all off.
//! Recording / replay: the added pad bits are in the recorded pad (`rec::field`), the cursor writes are not (a mouse session
//! replays its presses, not its jumps).

use super::mouse_menu::{Kind, MenuSnap, CROSS, DOWN, LEFT, MENU_BTNS, RIGHT, UP};
use super::mouse_slider::{self, Slide};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// Offset of the page's "next current item" word (set by an item's update on a D-pad edge, taken by the engine at the pass end).
const PAGE_NEXT: u32 = 0x80;
/// Fields a request may take before it is dropped (a page flying in, a row the game does not follow).
const MAX_AGE: u8 = 30;

/// A row of an item of a page that should be selected (and clicked).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Request {
	pub page_id: u32,
	pub item: u32,
	pub sub: u32,
	/// Press Cross once the row is selected (a left click).
	pub confirm: bool,
	/// A Sound Options bar: set the volume of the selected row (`mouse_slider.rs`) instead of pressing Cross.
	pub slide: Option<Slide>,
}

/// What `State::step` hands the worker: a new request (replaces the old), or drop it (the keyboard took the menu, the pointer aims
/// elsewhere, another button was fired, the menu is gone).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cmd {
	Submit(Request),
	Cancel,
}

struct Active {
	req: Request,
	age: u8,
	/// The pad bits this request added last field: the same direction is not pressed twice in a row (the game needs a release for
	/// an edge).
	last: u16,
}

static ACTIVE: Mutex<Option<Active>> = Mutex::new(None);
/// `ACTIVE` is set: lets the worker skip the lock in every field without a request.
static LIVE: AtomicBool = AtomicBool::new(false);

/// Instant selection on (the default): RAC_MOUSE_INSTANT=0 gives the pulses of retail-style pad steering only.
pub fn enabled() -> bool {
	static ON: OnceLock<bool> = OnceLock::new();
	*ON.get_or_init(|| std::env::var("RAC_MOUSE_INSTANT").map_or(true, |v| v != "0"))
}

/// The main thread's command for the next worker field.
pub fn send(c: Cmd) {
	if c == Cmd::Cancel && !LIVE.load(Ordering::Acquire) {
		return;
	}
	let Ok(mut a) = ACTIVE.lock() else { return };
	match c {
		Cmd::Submit(req) => {
			*a = Some(Active { req, age: 0, last: 0 });
			LIVE.store(true, Ordering::Release);
		}
		Cmd::Cancel => {
			if a.take().is_some() {
				LIVE.store(false, Ordering::Release);
			}
		}
	}
}

/// The command for a field in which the game's pad is neutral on purpose (a Controls page waiting for a key, the key that ended it still
/// down, the dev free camera): the worker must not press or move anything either, so every command becomes "drop it".
pub fn when_neutral(c: Cmd, neutral: bool) -> Cmd {
	if neutral { Cmd::Cancel } else { c }
}

/// What one field does for a request.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
	/// (address, value) words to write before the field.
	pub writes: Vec<(u32, u32)>,
	/// Pad bits to add to this field's pad.
	pub pad: u16,
	/// The request is done (or void): drop it.
	pub done: bool,
	/// The row is selected and the request wants its slider moved: the worker reads the volume and takes `mouse_slider::step`.
	pub slide: bool,
}

/// The cursor word of a Value page's item: Sound Options (page 0x20) keeps it at +0x40, the multi-choice update at +0x38.
fn value_cursor_off(m: &MenuSnap) -> u32 {
	if !m.front && m.id == 0x20 { 0x40 } else { 0x38 }
}

/// This field's plan for `r` on the menu as it is in RAM now. `pending` = the page's next-item word (page+0x80), `last` = the
/// pad bits the request added last field.
pub fn plan(m: &MenuSnap, r: &Request, pending: u32, last: u16) -> Plan {
	let done = Plan { done: true, ..Plan::default() };
	let wait = Plan::default();
	let Some(it) = m.items.iter().find(|i| i.addr == r.item) else { return done };
	if m.id != r.page_id {
		return done;
	}
	if m.busy() {
		return wait;
	}
	let (cursor, off, selectable) = match &it.kind {
		Kind::Label { cursor, rows, .. } => (*cursor, 0x40, rows.get(r.sub as usize).is_some_and(|x| x.1)),
		Kind::Value { cursor, n } => (*cursor, value_cursor_off(m), r.sub < *n),
		Kind::Grid { cursor, rows, cols } => (*cursor, GRID_CURSOR, r.sub < rows * cols),
		_ => return done,
	};
	if !selectable {
		return done;
	}
	if m.cur == r.item && cursor == r.sub {
		if r.slide.is_some() {
			return Plan { slide: true, ..Plan::default() };
		}
		// Selected. A click presses Cross in this field (the move happened in an earlier one).
		return Plan { pad: if r.confirm { CROSS } else { 0 }, done: true, ..Plan::default() };
	}
	if m.cur != r.item {
		if pending != 0 {
			return wait;
		}
		let mut writes = vec![(m.page + PAGE_NEXT, r.item)];
		if cursor != r.sub {
			writes.push((r.item + off, r.sub));
		}
		return Plan { writes, ..Plan::default() };
	}
	// The current item: the last step is the game's own.
	let (prev, dir) = match &it.kind {
		Kind::Grid { cols, .. } => grid_step(*cols, cursor, r.sub),
		_ if cursor < r.sub => (r.sub - 1, DOWN),
		_ => (r.sub + 1, UP),
	};
	if last & dir != 0 {
		return wait;
	}
	let writes = if cursor == prev { Vec::new() } else { vec![(r.item + off, prev)] };
	Plan { writes, pad: dir, ..Plan::default() }
}

/// A grid's cursor word (FUN_00278e30: item +0x3c; rows +0x40, columns +0x44, cells +0x48).
const GRID_CURSOR: u32 = 0x3c;

/// The last step into cell `sub` of a grid `cols` wide with the cursor on `cursor`: the cell to preset and the one D-pad press that
/// moves from it to `sub` without a wrap or a neighbour item (Up / Down need the row above / below, Left / Right the column next to
/// it: the target's own row / column side that faces the cursor always has one). A different row goes by Down / Up in the target's
/// column, the same row by Right / Left.
pub fn grid_step(cols: u32, cursor: u32, sub: u32) -> (u32, u16) {
	let (cr, tr, tc) = (cursor / cols, sub / cols, sub % cols);
	if cr < tr {
		((tr - 1) * cols + tc, DOWN)
	} else if cr > tr {
		((tr + 1) * cols + tc, UP)
	} else if cursor < sub {
		(sub - 1, RIGHT)
	} else {
		(sub + 1, LEFT)
	}
}

/// Worker, each field before the pad is set: the pad with the request's bits, and the cursor writes applied. `front` = the title
/// front end's menu (its own mode word), else the level's pause / map menu.
pub fn field(m: &mut ee_oracle::Machine, front: bool, pad: u16) -> u16 {
	// Tests only (no-op otherwise): the volumes as they are at the start of every field, request or not.
	mouse_slider::publish(m);
	super::mouse_vendor::publish(m);
	if !LIVE.load(Ordering::Acquire) {
		return pad;
	}
	let snap = {
		let rd = |a: u32| m.rd32(a);
		if front {
			super::mouse_menu::read_front(&rd, rd(super::title_boot::front_mode()))
		} else {
			super::mouse_menu::read(&rd, m.level(), rd(super::disc::va(0x15f5a4))).or_else(|| super::mouse_vendor::read(&rd, m.level()).filter(|v| !v.busy()))
		}
	};
	let Ok(mut g) = ACTIVE.lock() else { return pad };
	let Some(a) = g.as_mut() else { return pad };
	let Some(snap) = snap else {
		*g = None;
		LIVE.store(false, Ordering::Release);
		return pad;
	};
	// A key or button held by the player (or a pulse in flight): the menu is theirs; wait, the main thread cancels on a key.
	let held = pad & MENU_BTNS != 0;
	let mut p = if held { Plan::default() } else { plan(&snap, &a.req, m.rd32(snap.page + PAGE_NEXT), a.last) };
	if p.slide {
		p = slide_plan(m, &a.req);
	}
	for &(addr, v) in &p.writes {
		m.cpu.mem.wr32(addr, v);
	}
	a.last = p.pad;
	a.age += 1;
	if p.done || a.age > MAX_AGE {
		*g = None;
		LIVE.store(false, Ordering::Release);
	}
	pad | p.pad
}

/// The slider step of a selected Sound Options row: the volume word and the held direction (`mouse_slider::step`); the request ends.
fn slide_plan(m: &ee_oracle::Machine, r: &Request) -> Plan {
	let (Some(slide), Some(addr)) = (r.slide, mouse_slider::volume_addr(r.sub)) else { return Plan { done: true, ..Plan::default() } };
	match mouse_slider::step(m.rd32(addr) as i32, slide) {
		Some((preset, dir)) => Plan { writes: vec![(addr, preset as u32)], pad: dir, done: true, ..Plan::default() },
		None => Plan { done: true, ..Plan::default() },
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::retail::mouse_menu::Item;

	fn label(addr: u32, rows: usize, cursor: u32) -> Item {
		Item { addr, slot: 0, rect: [150, 40, 200, 100], kind: Kind::Label { rows: (0..rows).map(|i| (10.0 + 12.0 * i as f32, i != 2)).collect(), rowh: 12.0, cursor }, texts: Vec::new() }
	}

	fn menu(cur: u32) -> MenuSnap {
		MenuSnap { page: 0x1b0000, id: 8, state: 8, cur, items: vec![label(0x100, 1, 0), label(0x200, 7, 3), label(0x300, 1, 0)], front: false }
	}

	fn req(item: u32, sub: u32, confirm: bool) -> Request {
		Request { page_id: 8, item, sub, confirm, slide: None }
	}

	#[test]
	fn another_item_is_one_next_item_write_and_the_row_cursor() {
		let p = plan(&menu(0x100), &req(0x200, 5, false), 0, 0);
		assert_eq!(p, Plan { writes: vec![(0x1b0080, 0x200), (0x200 + 0x40, 5)], ..Plan::default() });
		// The row already there: only the item hop. A hop already pending: wait.
		assert_eq!(plan(&menu(0x100), &req(0x200, 3, false), 0, 0).writes, vec![(0x1b0080, 0x200)]);
		assert_eq!(plan(&menu(0x100), &req(0x200, 5, false), 0x300, 0), Plan::default());
	}

	#[test]
	fn a_row_of_the_current_item_is_one_game_step() {
		// Cursor 3, target 6: preset 5, one Down. Cursor 3, target 4: no write, one Down. Target 0: preset 1, one Up.
		assert_eq!(plan(&menu(0x200), &req(0x200, 6, false), 0, 0), Plan { writes: vec![(0x240, 5)], pad: DOWN, ..Plan::default() });
		assert_eq!(plan(&menu(0x200), &req(0x200, 4, false), 0, 0), Plan { writes: vec![], pad: DOWN, ..Plan::default() });
		assert_eq!(plan(&menu(0x200), &req(0x200, 0, false), 0, 0), Plan { writes: vec![(0x240, 1)], pad: UP, ..Plan::default() });
		// The same direction twice in a row has no edge: wait a field.
		assert_eq!(plan(&menu(0x200), &req(0x200, 6, false), 0, DOWN), Plan::default());
	}

	#[test]
	fn selected_rows_click_in_the_next_field_and_dead_requests_end() {
		assert_eq!(plan(&menu(0x200), &req(0x200, 3, false), 0, 0), Plan { done: true, ..Plan::default() });
		assert_eq!(plan(&menu(0x200), &req(0x200, 3, true), 0, 0), Plan { pad: CROSS, done: true, ..Plan::default() });
		// A row the list does not let the pointer select (row 2 above), another page, an item that left, a page flying in.
		assert!(plan(&menu(0x200), &req(0x200, 2, false), 0, 0).done);
		assert!(plan(&menu(0x200), &Request { page_id: 9, ..req(0x200, 3, false) }, 0, 0).done);
		assert!(plan(&menu(0x200), &req(0x999, 0, false), 0, 0).done);
		let mut flying = menu(0x200);
		flying.state = 1;
		assert_eq!(plan(&flying, &req(0x200, 5, false), 0, 0), Plan::default());
	}

	#[test]
	fn value_rows_use_the_item_cursor_word_of_their_page() {
		let v = |id: u32| MenuSnap { page: 0x1b0000, id, state: id, cur: 0x400, items: vec![Item { addr: 0x400, slot: 3, rect: [150, 40, 200, 100], kind: Kind::Value { n: 3, cursor: 0 }, texts: Vec::new() }], front: false };
		let on = |id: u32| Request { page_id: id, ..req(0x400, 2, false) };
		assert_eq!(plan(&v(0x1d), &on(0x1d), 0, 0).writes, vec![(0x438, 1)]);
		assert_eq!(plan(&v(0x20), &on(0x20), 0, 0).writes, vec![(0x440, 1)]);
	}

	fn grid_menu(cur: u32, cursor: u32) -> MenuSnap {
		let g = Item { addr: 0x500, slot: 2, rect: [20, 40, 200, 100], kind: Kind::Grid { rows: 3, cols: 5, cursor }, texts: Vec::new() };
		MenuSnap { page: 0x1b0000, id: 3, state: 3, cur, items: vec![label(0x100, 1, 0), g], front: false }
	}

	#[test]
	fn a_neutral_pad_field_drops_every_instant_command() {
		let r = Request { page_id: 3, item: 0x500, sub: 1, confirm: true, slide: None };
		assert_eq!(when_neutral(Cmd::Submit(r), true), Cmd::Cancel, "key capture / free camera: no worker press either");
		assert_eq!(when_neutral(Cmd::Cancel, true), Cmd::Cancel);
		assert_eq!(when_neutral(Cmd::Submit(r), false), Cmd::Submit(r), "otherwise untouched");
		assert_eq!(when_neutral(Cmd::Cancel, false), Cmd::Cancel);
	}

	/// input.rs `tick_keys` hands the worker the command only through `when_neutral`, after the three states that give the game a neutral pad
	/// (Controls key capture, the key that ended it still down, the dev free camera) have been decided (mmenu F3).
	#[test]
	fn the_viewer_sends_the_instant_command_only_after_the_neutral_pad_states() {
		let src = include_str!("input.rs");
		assert_eq!(src.matches("mouse_instant::send(").count(), 1, "one place sends commands to the worker");
		let send = src.find("mouse_instant::send(mouse_instant::when_neutral(c, neutral))").expect("the send goes through when_neutral");
		let take = src.find("self.mm.take_cmd()").expect("the command is taken from the state");
		assert!(take < send);
		let mut states = 0;
		for (at, _) in src.match_indices("neutral = true;") {
			assert!(at < send, "every neutral state is decided before the command goes out");
			states += 1;
		}
		assert_eq!(states, 3, "key capture, the capture's key still held, the free camera");
	}

	#[test]
	fn grid_cells_are_one_game_step_from_the_neighbour_cell() {
		// 3 rows x 5 columns. Cursor (0,0): a cell in another row is entered from the row above it (Down) in its column.
		assert_eq!(grid_step(5, 0, 12), (7, DOWN));
		assert_eq!(grid_step(5, 0, 5), (0, DOWN));
		// Below the target: Up from the row under it.
		assert_eq!(grid_step(5, 14, 1), (6, UP));
		// The same row: Right from the cell left of the target, Left from the cell right of it.
		assert_eq!(grid_step(5, 5, 8), (7, RIGHT));
		assert_eq!(grid_step(5, 9, 6), (7, LEFT));
		// The plan writes the preset cursor word (+0x3c) and adds the press; no write when the cursor is already next to the target.
		let r = Request { page_id: 3, item: 0x500, sub: 12, confirm: false, slide: None };
		assert_eq!(plan(&grid_menu(0x500, 0), &r, 0, 0), Plan { writes: vec![(0x53c, 7)], pad: DOWN, ..Plan::default() });
		let near = Request { sub: 5, ..r };
		assert_eq!(plan(&grid_menu(0x500, 0), &near, 0, 0), Plan { writes: vec![], pad: DOWN, ..Plan::default() });
		// From another item: the hop writes page+0x80 and the grid's cursor; a cell past the grid ends the request.
		assert_eq!(plan(&grid_menu(0x100, 0), &r, 0, 0).writes, vec![(0x1b0080, 0x500), (0x53c, 12)]);
		assert!(plan(&grid_menu(0x500, 0), &Request { sub: 15, ..r }, 0, 0).done);
		// Selected: a click presses Cross; a slider request waits for the worker's volume.
		assert_eq!(plan(&grid_menu(0x500, 12), &Request { confirm: true, ..r }, 0, 0), Plan { pad: CROSS, done: true, ..Plan::default() });
	}

	#[test]
	fn every_grid_step_ends_on_the_target_without_a_wrap() {
		// Model of FUN_00278e30's four moves (interior only): the preset cell plus the press must give exactly the target cell.
		for (rows, cols) in [(1u32, 3u32), (2, 3), (3, 5), (1, 2), (2, 2)] {
			for c in 0..rows * cols {
				for t in 0..rows * cols {
					if c == t {
						continue;
					}
					let (p, d) = grid_step(cols, c, t);
					assert!(p < rows * cols, "preset {p} outside {rows}x{cols}");
					let (pr, pc) = (p / cols, p % cols);
					let after = match d {
						DOWN => { assert!(pr + 1 < rows); p + cols }
						UP => { assert!(pr > 0); p - cols }
						RIGHT => { assert!(pc + 1 < cols); p + 1 }
						_ => { assert!(pc > 0); p - 1 }
					};
					assert_eq!(after, t, "{rows}x{cols}: {c} -> {t}");
				}
			}
		}
	}
}
