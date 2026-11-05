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
//! replays its presses, not its jumps). Neither is the Quick Select keep write (`post_field`): a recording made with an Equip on
//! the modern Quick Select page replays retail's turn one slot on, so such a session is not replay-exact from that Equip on.

use super::menu_model::extra::RING_CURSOR;
use super::mouse_menu::{slot_page, Kind, MenuSnap, CROSS, DOWN, LEFT, MENU_BTNS, RIGHT, SLOT_CURSOR, UP};
use super::mouse_slider::{self, Slide};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

/// Offset of the page's "next current item" word (set by an item's update on a D-pad edge, taken by the engine at the pass end).
const PAGE_NEXT: u32 = 0x80;
/// Fields a request may take before it is dropped (a page flying in, a row the game does not follow). Fields it waits (`held`) do
/// not count: they have their own cap, `WAIT_MAX`.
const MAX_AGE: u8 = 30;
/// Fields a request may wait (`held`: a button held, the Quick Select ring's L1 / R1 pulses still queued) before it is dropped: 120
/// fields (2 s at NTSC 60 Hz, 2.4 s at PAL 50 Hz), five times what a full pulse queue takes (8 pulses x 3 fields = 24), so an Equip
/// behind queued turns is never lost, and a button held for good still ends the request.
const WAIT_MAX: u8 = 120;

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
	/// Fields spent waiting (`held`), counted apart from `age`.
	waited: u8,
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
			*a = Some(Active { req, age: 0, waited: 0, last: 0 });
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
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
	/// (address, value) words to write before the field.
	pub writes: Vec<(u32, u32)>,
	/// Pad bits to add to this field's pad.
	pub pad: u16,
	/// The request is done (or void): drop it.
	pub done: bool,
	/// The row is selected and the request wants its slider moved: the worker reads the volume and takes `mouse_slider::step`.
	pub slide: bool,
	/// Quick Select: this field's Cross equips into the ring slot under the cursor and retail then turns the cursor one slot on
	/// (FUN_00279d78); (the ring's cursor word, the slot) for `post_field`, which turns it back on the modern page (`kept`).
	pub keep: Option<(u32, u32)>,
}

/// The cursor word of a Value page's item: Sound Options (page 0x20) and the Save / Load slot list keep it at +0x40, the multi-choice
/// update at +0x38.
fn value_cursor_off(m: &MenuSnap) -> u32 {
	if !m.front && m.id == 0x20 || slot_page(m) { 0x40 } else { 0x38 }
}

/// The words a preset of cursor word `at` to `v` writes: the word, and on the Save / Load slot list also the slot global its update
/// reloads the cursor from every field (`mouse_menu::SLOT_CURSOR`; the item's word alone would be overwritten before the D-pad edge
/// moves it).
fn cursor_writes(m: &MenuSnap, at: u32, v: u32) -> Vec<(u32, u32)> {
	let mut w = vec![(at, v)];
	if slot_page(m) {
		w.push((SLOT_CURSOR, v));
	}
	w
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
		let keep = ring_of(m).filter(|_| r.confirm && matches!(it.kind, Kind::Grid { .. })).map(|(addr, cursor)| (addr + RING_CURSOR, cursor));
		return Plan { pad: if r.confirm { CROSS } else { 0 }, done: true, keep, ..Plan::default() };
	}
	if m.cur != r.item {
		if pending != 0 {
			return wait;
		}
		let mut writes = vec![(m.page + PAGE_NEXT, r.item)];
		if cursor != r.sub {
			writes.extend(cursor_writes(m, r.item + off, r.sub));
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
	let writes = if cursor == prev { Vec::new() } else { cursor_writes(m, r.item + off, prev) };
	Plan { writes, pad: dir, ..Plan::default() }
}

/// A grid's cursor word (FUN_00278e30: item +0x3c; rows +0x40, columns +0x44, cells +0x48).
const GRID_CURSOR: u32 = 0x3c;

/// The Quick Select ring of the in-level page `m`: (its item, its cursor slot). None on every other page.
fn ring_of(m: &MenuSnap) -> Option<(u32, u32)> {
	if m.front {
		return None;
	}
	m.items.iter().find_map(|i| if let Kind::Ring { cursor } = i.kind { Some((i.addr, cursor)) } else { None })
}

/// The ring cursor to write back after an Equip on the modern Quick Select page: retail's Cross put the item into slot `slot` and
/// turned the cursor one on (`now` = `slot` + 1, FUN_00279d78), so the cursor goes back to the slot the player chose. Any other
/// `now` (an unowned cell: no equip, the cursor did not move) leaves the game's word alone.
pub fn kept(slot: u32, now: u32) -> Option<u32> {
	(now == (slot + 1) % 8).then_some(slot)
}

/// The request waits this field: a key or button held by the player (or a pulse in flight; the menu is theirs, the main thread cancels
/// on a key), or, on the Quick Select page, the ring's L1 / R1 pulses still queued (`pulses`: `cmd::pulses_pending`, asked only on
/// that page): a slot clicked just before Equip turns the cursor first, so the Cross goes to that slot.
pub fn held(pad: u16, m: &MenuSnap, pulses: impl FnOnce() -> bool) -> bool {
	pad & MENU_BTNS != 0 || (ring_of(m).is_some() && pulses())
}

/// The Equip whose cursor turn `post_field` takes back: only while the modern pause page is up (`modern`) on an in-level menu (a PC
/// deviation, ARCHITECTURE.md "modern pause menu"); the classic page keeps retail's turn one slot on (FUN_00279d78).
pub fn keep_on(keep: Option<(u32, u32)>, front: bool, modern: bool) -> Option<(u32, u32)> {
	keep.filter(|_| !front && modern)
}

/// A request's (age, waited) after one more field: a waiting field (`held`) counts toward `waited` only, any other toward `age`.
fn aged(age: u8, waited: u8, waits: bool) -> (u8, u8) {
	if waits { (age, waited.saturating_add(1)) } else { (age.saturating_add(1), waited) }
}

/// The modern Quick Select page's Equip waiting for `post_field`: (the ring's cursor word, the slot chosen).
static KEEP: Mutex<Option<(u32, u32)>> = Mutex::new(None);
static KEEPING: AtomicBool = AtomicBool::new(false);

/// [PC] The request whose cursor move is silent (`quiet`).
static QUIET: Mutex<Option<Request>> = Mutex::new(None);
/// Fields a quiet move hushes (its field and the next, as a hushed Back pulse: menu_model/cmd.rs HUSH_FIELDS).
const QUIET_FIELDS: u32 = 2;

/// Main thread, right before `r` is submitted: the cursor move of `r` plays no move sound and its Cross waits until that hush is
/// over, so the sound the Cross brings is the one heard (a modern pause tab switch's Select of the tab on the root:
/// pause_host.rs `select_quiet`; user 2026-11-04, test build 10: "i still hear the 'double' sound").
pub fn quiet(r: Request) {
	if let Ok(mut q) = QUIET.lock() {
		*q = Some(r);
	}
}

/// Tests: the request marked quiet.
#[cfg(test)]
pub fn quieted() -> Option<Request> {
	QUIET.lock().ok().and_then(|q| *q)
}

/// Does plan `p` move a cursor (a preset or a D-pad edge)?
fn moves(p: &Plan) -> bool {
	!p.writes.is_empty() || p.pad & (UP | DOWN | LEFT | RIGHT) != 0
}

/// A quiet request's plan: its Cross waits a field while the move's fields are still hushed (`hushing`), every other plan is kept.
fn quiet_plan(p: Plan, hushing: bool) -> Plan {
	if p.pad & CROSS != 0 && hushing { Plan::default() } else { p }
}

/// Worker, after EVERY field (before the snapshot, so no picture shows the turned cursor): the ring cursor of an Equip made on the
/// modern Quick Select page goes back to the slot it filled (`kept`). `live` = an in-level field with the PC extras on (not the
/// front end, not booting); a keep left from a field that was not is dropped unwritten, so it can never fire in a later field
/// (another level's overlay). Nothing to do in every other field.
pub fn post_field(m: &mut ee_oracle::Machine, live: bool) {
	if live && SKIP_FIELDS.load(Ordering::Acquire) > 0 {
		skip_page_change(m);
	}
	let Some((addr, v)) = due(live).and_then(|k| write_back(k, m.rd32(k.0))) else { return };
	m.cpu.mem.wr32(addr, v);
}

/// Fields the page-change countdown skip stays armed (`skip_page_changes`).
static SKIP_FIELDS: AtomicU32 = AtomicU32::new(0);
/// The countdown word of a page change, from the page pointer (Veldin 0x1b9cf4 -> 0x1b9d04; the block moves as one per level, as
/// the kind word 4 bytes before the pointer does). The kind word is 4 before the pointer.
const COUNTDOWN: u32 = 0x10;

/// Main thread, a modern pause tab switch (child_tabs.rs): for the next `fields` fields every page change of the in-level menu skips
/// its 12-field flight (page engine FUN_00276560: kind word 1, countdown 12; the pass where the countdown reaches 0 finishes the
/// change: clears the target word, sets the current page and runs its enters). 0 = off. Only the mouse's tab click arms it; a key or
/// pad Back never does, so they keep retail's pace.
pub fn skip_page_changes(fields: u32) {
	SKIP_FIELDS.store(fields, Ordering::Release);
}

/// The countdown value to write (1: the next pass finishes the change exactly as the last of the 12 would) for a page change in
/// flight: kind word 1 (flying), current-page word 0 (cleared at the start of a change), countdown still above 1.
pub fn skip_value(kind: u32, current: u32, countdown: u32) -> Option<u32> {
	(kind == 1 && current == 0 && (2..=12).contains(&countdown)).then_some(1)
}

/// Tests: the fields the skip is still armed.
#[cfg(test)]
pub fn skip_armed() -> u32 {
	SKIP_FIELDS.load(Ordering::Acquire)
}

fn skip_page_change(m: &mut ee_oracle::Machine) {
	let Some(pp) = super::mouse_menu::page_ptr(m.level()) else { return };
	let left = SKIP_FIELDS.load(Ordering::Acquire);
	SKIP_FIELDS.store(left.saturating_sub(1), Ordering::Release);
	if let Some(v) = skip_value(m.rd32(pp - 4), m.rd32(pp), m.rd32(pp + COUNTDOWN)) {
		m.cpu.mem.wr32(pp + COUNTDOWN, v);
	}
}

/// The Equip `field` stored for this field's `post_field`, taken in every field (one-shot); None when there is none or the field is
/// not `live` (then it is dropped).
fn due(live: bool) -> Option<(u32, u32)> {
	if !KEEPING.swap(false, Ordering::AcqRel) {
		return None;
	}
	KEEP.lock().ok().and_then(|mut k| k.take()).filter(|_| live)
}

/// The write of `post_field` for the keep (the ring's cursor word, the slot) and the word's value `now`: (address, value) when
/// retail turned the cursor one on (`kept`), else nothing.
fn write_back((addr, slot): (u32, u32), now: u32) -> Option<(u32, u32)> {
	kept(slot, now).map(|v| (addr, v))
}

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
	let waits = held(pad, &snap, super::menu_model::cmd::pulses_pending);
	let mut p = if waits { Plan::default() } else { plan(&snap, &a.req, m.rd32(snap.page + PAGE_NEXT), a.last) };
	if p.slide {
		p = slide_plan(m, &a.req);
	}
	let quiet = QUIET.lock().is_ok_and(|q| *q == Some(a.req));
	if quiet {
		p = quiet_plan(p, super::snd::hushing());
		if moves(&p) {
			super::snd::hush(QUIET_FIELDS); // before the field runs: its batch and the next start no voice
		}
	}
	for &(addr, v) in &p.writes {
		m.cpu.mem.wr32(addr, v);
	}
	if let Some(k) = keep_on(p.keep, front, crate::modern_menu::pause_host::up()) {
		if let Ok(mut g) = KEEP.lock() {
			*g = Some(k);
			KEEPING.store(true, Ordering::Release);
		}
	}
	a.last = p.pad;
	(a.age, a.waited) = aged(a.age, a.waited, waits);
	if p.done || a.age > MAX_AGE || a.waited > WAIT_MAX {
		if quiet {
			if let Ok(mut q) = QUIET.lock() {
				*q = None;
			}
		}
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
		Item { addr, slot: 0, rect: [150, 40, 200, 100], kind: Kind::Label { rows: (0..rows).map(|i| (10.0 + 12.0 * i as f32, i != 2)).collect(), rowh: 12.0, cursor }, texts: Vec::new(), planet_texts: Vec::new() }
	}

	fn menu(cur: u32) -> MenuSnap {
		MenuSnap { page: 0x1b0000, id: 8, state: 8, cur, items: vec![label(0x100, 1, 0), label(0x200, 7, 3), label(0x300, 1, 0)], front: false }
	}

	fn req(item: u32, sub: u32, confirm: bool) -> Request {
		Request { page_id: 8, item, sub, confirm, slide: None }
	}

	#[test]
	fn a_page_change_in_flight_finishes_on_the_next_pass_nothing_else_is_touched() {
		assert_eq!(skip_value(1, 0, 12), Some(1), "just started");
		assert_eq!(skip_value(1, 0, 7), Some(1), "half way");
		assert_eq!(skip_value(1, 0, 1), None, "already on its last pass");
		assert_eq!(skip_value(1, 0, 0), None);
		assert_eq!(skip_value(2, 0x1b9000, 12), None, "an idle page");
		assert_eq!(skip_value(0x14, 0, 10), None, "the close of the whole menu keeps its own wait");
		assert_eq!(skip_value(1, 0x1b9000, 12), None, "the current page is set: not a change in flight");
		assert_eq!(skip_value(1, 0, 500), None, "not a countdown");
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
		let v = |id: u32| MenuSnap { page: 0x1b0000, id, state: id, cur: 0x400, items: vec![Item { addr: 0x400, slot: 3, rect: [150, 40, 200, 100], kind: Kind::Value { n: 3, cursor: 0 }, texts: Vec::new(), planet_texts: Vec::new() }], front: false };
		let on = |id: u32| Request { page_id: id, ..req(0x400, 2, false) };
		assert_eq!(plan(&v(0x1d), &on(0x1d), 0, 0).writes, vec![(0x438, 1)]);
		assert_eq!(plan(&v(0x20), &on(0x20), 0, 0).writes, vec![(0x440, 1)]);
	}

	#[test]
	fn a_quiet_move_is_hushed_and_its_cross_waits_for_the_hush() {
		let mv = Plan { writes: vec![(0x1b0080, 0x400)], ..Plan::default() };
		assert!(moves(&mv) && moves(&Plan { pad: DOWN, ..Plan::default() }) && !moves(&Plan { pad: CROSS, done: true, ..Plan::default() }));
		let cross = Plan { pad: CROSS, done: true, ..Plan::default() };
		assert_eq!(quiet_plan(cross.clone(), true), Plan::default(), "the move's sound fields first: wait");
		assert_eq!(quiet_plan(cross.clone(), false), cross, "then the Cross: its open sound is heard");
		assert_eq!(quiet_plan(mv.clone(), true), mv);
	}

	#[test]
	fn save_and_load_slots_preset_the_slot_global_too() {
		// The slot list's update reloads +0x40 from 0x15ee34 every field: the preset goes to both, the last step is the game's Down.
		let v = |id: u32, cursor: u32| MenuSnap { page: 0x1b0000, id, state: id, cur: 0x400, items: vec![Item { addr: 0x400, slot: 1, rect: [10, 40, 100, 300], kind: Kind::Value { n: 5, cursor }, texts: Vec::new(), planet_texts: Vec::new() }], front: false };
		for id in [0x1e, 0x1f] {
			let on = |sub: u32, confirm: bool| Request { page_id: id, ..req(0x400, sub, confirm) };
			assert_eq!(plan(&v(id, 0), &on(4, false), 0, 0), Plan { writes: vec![(0x440, 3), (SLOT_CURSOR, 3)], pad: DOWN, ..Plan::default() });
			assert_eq!(plan(&v(id, 3), &on(4, false), 0, 0), Plan { writes: vec![], pad: DOWN, ..Plan::default() });
			assert_eq!(plan(&v(id, 4), &on(4, true), 0, 0), Plan { pad: CROSS, done: true, ..Plan::default() });
		}
		// Another page's value list keeps its one word.
		assert_eq!(plan(&v(0x20, 0), &Request { page_id: 0x20, ..req(0x400, 4, false) }, 0, 0).writes, vec![(0x440, 3)]);
	}

	fn grid_menu(cur: u32, cursor: u32) -> MenuSnap {
		let g = Item { addr: 0x500, slot: 2, rect: [20, 40, 200, 100], kind: Kind::Grid { rows: 3, cols: 5, cursor }, texts: Vec::new(), planet_texts: Vec::new() };
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
	/// (Controls key capture, the key that ended it still down, the dev free camera, the achievements list) have been decided (mmenu F3). The Achievements row's
	/// press is no longer one of them: the worker takes the Cross out of the pad itself (`pcext::achrow::worker_field`, fix round 3).
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
		assert_eq!(states, 5, "key capture, the capture's key still held, the achievements list, the modern Options page (host::pad_filter), the free camera");
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

	/// The Quick Select page (id 5): the ring item (slot 1, cursor 3) and the weapons grid 3 x 5 (cursor on cell `cell`), the grid current.
	fn quick_menu(cell: u32, front: bool) -> MenuSnap {
		let ring = Item { addr: 0x1b34e8, slot: 1, rect: [300, 40, 200, 200], kind: Kind::Ring { cursor: 3 }, texts: Vec::new(), planet_texts: Vec::new() };
		let g = Item { addr: 0x500, slot: 2, rect: [20, 40, 200, 100], kind: Kind::Grid { rows: 3, cols: 5, cursor: cell }, texts: Vec::new(), planet_texts: Vec::new() };
		MenuSnap { page: 0x1b0000, id: 5, state: 5, cur: 0x500, items: vec![ring, g], front }
	}

	#[test]
	fn an_equip_on_the_quick_select_page_carries_the_ring_cursor_to_keep() {
		let r = Request { page_id: 5, item: 0x500, sub: 12, confirm: true, slide: None };
		// Cross on the selected cell: the ring's cursor word (+0x50) and the slot it points at go to `post_field`.
		assert_eq!(plan(&quick_menu(12, false), &r, 0, 0), Plan { pad: CROSS, done: true, keep: Some((0x1b34e8 + 0x50, 3)), ..Plan::default() });
		// A hover (no Cross), a cell not yet selected (the move comes first), the front end, a grid page without a ring: nothing to keep.
		assert_eq!(plan(&quick_menu(12, false), &Request { confirm: false, ..r }, 0, 0).keep, None);
		assert_eq!(plan(&quick_menu(10, false), &r, 0, 0).keep, None);
		assert_eq!(plan(&quick_menu(12, true), &r, 0, 0).keep, None);
		assert_eq!(plan(&grid_menu(0x500, 12), &Request { page_id: 3, ..r }, 0, 0).keep, None);
	}

	#[test]
	fn only_the_modern_page_keeps_the_cursor() {
		let k = Some((0x1b3538, 3));
		assert_eq!(keep_on(k, false, true), k, "modern pause page up: the PC deviation");
		assert_eq!(keep_on(k, false, false), None, "classic page: retail's turn one slot on");
		assert_eq!(keep_on(k, true, true), None, "never on the front end");
		assert_eq!(keep_on(None, false, true), None);
	}

	#[test]
	fn the_kept_slot_is_the_one_retail_turned_away_from() {
		// FUN_00279d78: an Equip into slot s leaves the cursor on (s + 1) mod 8; that cursor goes back to s.
		assert_eq!(kept(5, 6), Some(5));
		assert_eq!(kept(7, 0), Some(7), "the turn wraps");
		assert_eq!(kept(0, 1), Some(0));
		// No turn (an unowned cell: no equip) or any other cursor (the player turned the ring): the game's word stays.
		assert_eq!(kept(5, 5), None);
		assert_eq!(kept(5, 4), None);
		assert_eq!(kept(2, 4), None);
	}

	#[test]
	fn a_quick_select_equip_waits_for_the_queued_ring_turns() {
		let q = quick_menu(12, false);
		assert!(!held(0, &q, || false), "nothing held, no pulse queued");
		assert!(held(0, &q, || true), "L1 / R1 turns still queued: the Cross waits for them");
		assert!(held(CROSS, &q, || false) && held(super::super::mouse_menu::L1, &q, || false), "a pad button held by the player");
		// Another page never asks the pulse queue (no lock taken), and its queued pulses do not hold the request.
		let asked = std::cell::Cell::new(false);
		assert!(!held(0, &grid_menu(0x500, 0), || { asked.set(true); true }));
		assert!(!asked.get(), "the queue is asked on the ring page only");
		assert!(!held(0, &quick_menu(12, true), || true), "the front end has no ring");
	}

	/// R19 minor 2: an Equip behind a full queue of L1 / R1 pulses (8 x 3 fields) plus its own steps is not dropped; waiting has its
	/// own cap; fields that do not wait still age.
	#[test]
	fn waiting_fields_do_not_age_a_request() {
		let (mut age, mut waited) = (0u8, 0u8);
		for _ in 0..24 {
			(age, waited) = aged(age, waited, true);
		}
		for _ in 0..3 {
			(age, waited) = aged(age, waited, false);
		}
		assert_eq!((age, waited), (3, 24), "the 24 waiting fields are not in the age");
		assert!(age <= MAX_AGE && waited <= WAIT_MAX, "still live");
		assert!(8 * 3 < WAIT_MAX, "a full pulse queue fits the wait cap");
		let (mut age, mut waited) = (0u8, 0u8);
		for _ in 0..=MAX_AGE {
			(age, waited) = aged(age, waited, false);
		}
		assert!(age > MAX_AGE && waited == 0, "a request the game does not follow still ends");
		for _ in 0..=WAIT_MAX {
			(age, waited) = aged(0, waited, true);
		}
		assert!(waited > WAIT_MAX && age == 0, "a button held for good ends it too");
		assert_eq!(aged(0, u8::MAX, true), (0, u8::MAX), "no overflow");
	}

	/// R19 nits 1 and 8: `post_field`'s keep is taken in every field (one-shot) and written only in a live field, and only when retail
	/// turned the cursor one on.
	#[test]
	fn the_keep_write_happens_once_in_a_live_field_and_is_dropped_otherwise() {
		// KEEP / KEEPING are process-wide: hold the lock the other tests of the menu globals take.
		let _t = crate::retail::menu_model::cmd::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
		let set = |k| {
			*KEEP.lock().unwrap() = Some(k);
			KEEPING.store(true, Ordering::Release);
		};
		assert_eq!(due(true), None, "nothing stored");
		set((0x1b3468 + RING_CURSOR, 5));
		assert_eq!(due(true), Some((0x1b3468 + RING_CURSOR, 5)));
		assert_eq!(due(true), None, "one-shot");
		set((0x1b3468 + RING_CURSOR, 5));
		assert_eq!(due(false), None, "a field that is not live drops it unwritten");
		assert_eq!(due(true), None, "and it does not fire in a later field");
		// The write: back to the slot when the cursor is slot + 1, else nothing.
		assert_eq!(write_back((0x1b34b8, 5), 6), Some((0x1b34b8, 5)));
		assert_eq!(write_back((0x1b34b8, 7), 0), Some((0x1b34b8, 7)), "the turn wraps");
		assert_eq!(write_back((0x1b34b8, 5), 5), None, "no turn: an unowned cell");
		assert_eq!(write_back((0x1b34b8, 5), 2), None);
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
