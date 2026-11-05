//! What a modern screen asks of the retail menu, and how it reaches the game. A screen builds an `Intent`; `resolve` turns it into
//! a `Command` for the menu on screen (a row request for `mouse_instant`, or a pad button pulse); `submit` queues it from the main
//! thread. The worker side (`feed`, called once per field by `Retail::tick_keys`) hands the request to `mouse_instant` (its one send
//! stays in input.rs) and adds the pulses to the pad. With nothing queued the pad and the requests are untouched.
use crate::retail::menu_keys;
use crate::retail::mouse_instant::{Cmd, Request};
use crate::retail::mouse_menu::{Kind, MenuSnap, L1, LEFT, R1, RIGHT};
use crate::retail::mouse_slider::Slide;
use std::collections::VecDeque;
use std::sync::Mutex;

/// What the player did on a modern screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Intent {
	/// Select row `row` of the item at `item` (its address); `confirm` also presses Cross (a click, not a hover).
	Select { item: u32, row: u32, confirm: bool },
	/// Set a Sound Options bar.
	Slide { item: u32, row: u32, slide: Slide },
	/// The screen's confirm / back button (the Quit Game page answers Yes on Circle, the root closes on Start, ...).
	Confirm,
	Back,
	/// L1 / R1 (the planet selector, the ring).
	Prev,
	Next,
	/// A pad button, by its bit.
	Press(u16),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
	Request(Request),
	Press(u16),
}

/// The command for an intent on the menu on screen; None when nothing should be sent (no such item or row, a disabled row).
pub fn resolve(m: &MenuSnap, intent: Intent) -> Option<Command> {
	let item = |addr: u32| m.items.iter().find(|i| i.addr == addr);
	let press = |bit: u16| (bit != 0).then_some(Command::Press(bit));
	match intent {
		Intent::Select { item: addr, row, confirm } => {
			let it = item(addr)?;
			match &it.kind {
				Kind::Label { rows, .. } if !rows.get(row as usize).is_some_and(|r| r.1) => return None,
				Kind::Label { .. } | Kind::Value { .. } | Kind::Grid { .. } => {}
				_ => return None,
			}
			Some(Command::Request(Request { page_id: m.id, item: addr, sub: row, confirm, slide: None }))
		}
		Intent::Slide { item: addr, row, slide } => {
			matches!(item(addr)?.kind, Kind::Value { .. }).then_some(Command::Request(Request { page_id: m.id, item: addr, sub: row, confirm: false, slide: Some(slide) }))
		}
		Intent::Confirm => press(menu_keys::screen(m).confirm),
		Intent::Back => press(menu_keys::screen(m).back),
		Intent::Prev => press(L1),
		Intent::Next => press(R1),
		Intent::Press(b) => press(b),
	}
}

/// Fields a pulse holds its button, then the fields it is released before the next one (mouse_menu's PRESS / RELEASE).
const PRESS: u8 = 1;
const RELEASE: u8 = 2;
/// A queue that finds no settled menu for this many fields is dropped (the screen that asked is gone).
const STALE: u16 = 180;
/// The most pulses waiting.
const QUEUE_MAX: usize = 8;
/// Fields a shop screen stays spent (`Spent`) when the game never leaves it (a Cross on a row the player cannot afford only buzzes):
/// the next click then presses again. Retail shows the screen a Cross opened 4 fields later (pp run 2026-11-02: Cross pressed in
/// field 50, the list still read at fields 51..53, the prompt from field 54), longer than the 3 fields one pulse takes.
pub(super) const SPENT_MAX: u16 = 30;
/// The pulses that keep the shop on its screen: the buy prompt's quantity arrows. Every other button leaves it (Cross opens the
/// prompt or buys, Triangle backs out, Square buys all ammo).
const STAY: u16 = LEFT | RIGHT;

/// What a pulse or a request is made for: the menu's page id, and inside the shop (one id, 0xfe) its screen
/// (`mouse_vendor::stage`: the list, the buy prompt or animating); the second word is 0 on every other menu.
pub type Key = (u32, u32);

/// The key of the menu on screen.
pub fn key(m: &MenuSnap) -> Key {
	(m.id, crate::retail::mouse_vendor::stage(m))
}

/// The pad pulses queued by the screens, one button after the other. A pulse is made for ONE page: it carries the key of the menu the
/// screen built from (`stamp`) and fires only while that page is on screen; it is dropped when the page changed or the menu closed.
/// While the menu animates only the page id must match (the pulse waits); once it settles the whole key must, so a Cross queued on
/// the shop's list never confirms the buy prompt it opened (a double click on Buy), and a Triangle queued on the prompt never leaves
/// the shop from the list it went back to. The key alone is not enough there: the game keeps showing the old screen for a few fields
/// after the press that leaves it, so that press also spends the screen (`Spent`).
pub struct Pulses {
	queue: VecDeque<(u16, Option<Key>, bool)>,
	/// The page the screens last built from (what a new pulse is made for); None = unknown (the pulse fires on any settled menu).
	page: Option<Key>,
	cur: u16,
	left: u8,
	idle: u16,
	/// The game's pad is in a neutral stretch (`neutral`): the queue was dropped at its first field.
	quiet: bool,
	/// The shop screen a press went to that the game has not left yet (`Spent`).
	spent: Option<Spent>,
}

/// A shop screen (list or buy prompt) the game was sent a leaving press for: a Cross pulse or a Cross request (`spend`). Retail keeps
/// showing it for a few fields, so a second click made on it meanwhile (a double click on Buy, the list row or the hint's Enter, or on
/// Cancel) still matches its key, and would press on the screen the first one opens (R4-1 / R2b-1: the double click bought one ammo with
/// no prompt). Every pulse and every Cross request made for it is dropped until the game settles on a screen after leaving it, or
/// `SPENT_MAX` fields pass while it stays (the press did nothing).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Spent {
	key: Key,
	age: u16,
	/// The game showed another screen (or animated) since.
	gone: bool,
}

impl Pulses {
	pub const fn new() -> Pulses {
		Pulses { queue: VecDeque::new(), page: None, cur: 0, left: 0, idle: 0, quiet: false, spent: None }
	}

	/// The game was sent a leaving press on shop screen `k` (other menus, `k.1 == 0`: nothing, their pages change on the press).
	fn spend(&mut self, k: Key) {
		if k.1 != 0 {
			self.spent = Some(Spent { key: k, age: 0, gone: false });
		}
	}

	/// Is shop screen `k` spent (a press for it now would be a second click on a screen the game is leaving)?
	fn spent_on(&self, k: Key) -> bool {
		self.spent.is_some_and(|s| s.key == k)
	}

	/// One field of the spent screen: gone once the game shows another; free once it settled after that, or after `SPENT_MAX` fields
	/// on it, or when the menu closed.
	fn age_spent(&mut self, menu: Option<Key>, ready: bool) {
		let Some(s) = self.spent.as_mut() else { return };
		s.gone |= menu != Some(s.key);
		s.age = s.age.saturating_add(1);
		if menu.is_none() || (s.gone && ready) || (!s.gone && s.age > SPENT_MAX) {
			self.spent = None;
		}
	}

	/// A field the game's pad is neutral (a PC page has the pad, input.rs). The first field of the stretch drops what was queued
	/// before it (R2-3: a Back queued just before the Achievements list opened must not press on the menu when the list closes); a
	/// pulse pushed during the stretch (a click on a page that is up meanwhile, e.g. the pause page while the key that closed the
	/// Options page is still held) waits for its end, then steps like any pulse (page stamp check, STALE count).
	pub fn neutral(&mut self) {
		if !self.quiet {
			self.clear();
			self.quiet = true;
		}
	}

	/// The page the pulses pushed from now on are made for (a plain page id: the key of a menu other than the shop).
	pub fn stamp(&mut self, page: Option<u32>) {
		self.stamp_key(page.map(|id| (id, 0)));
	}

	pub fn stamp_key(&mut self, page: Option<Key>) {
		self.page = page;
	}

	/// Drops the queue and a pulse being pressed; the stamp stays (the screen's page did not change).
	pub fn clear(&mut self) {
		(self.queue, self.cur, self.left, self.idle) = (VecDeque::new(), 0, 0, 0);
	}

	pub fn push(&mut self, bit: u16) {
		self.push_with(bit, false);
	}

	/// `quiet`: the pulse plays no game sound (`submit_quiet`).
	pub fn push_with(&mut self, bit: u16, quiet: bool) {
		if self.queue.len() < QUEUE_MAX {
			self.queue.push_back((bit, self.page, quiet));
		}
	}

	/// One field: the bits to add to the pad. `menu` = the id of the menu on screen (None = no menu: the queue is dropped, the screen
	/// that asked is gone). A new pulse starts only on a settled menu (`ready`) while the player holds nothing, and only if it was
	/// made for that menu's page (a pulse for another page is dropped, never sent to the one that came instead).
	pub fn step(&mut self, menu: Option<u32>, ready: bool, held: bool) -> u16 {
		self.step_key(menu.map(|id| (id, 0)), ready, held)
	}

	pub fn step_key(&mut self, menu: Option<Key>, ready: bool, held: bool) -> u16 {
		self.quiet = false;
		self.age_spent(menu, ready);
		if self.left == 0 {
			match menu {
				None => self.queue.clear(),
				Some((id, sub)) => {
					let spent = self.spent.map(|s| s.key);
					while self.queue.front().is_some_and(|p| p.1.is_some_and(|k| k.0 != id || (ready && k.1 != sub) || spent == Some(k))) {
						self.queue.pop_front();
					}
				}
			}
			if self.queue.is_empty() {
				self.idle = 0;
				return 0;
			}
			if !ready || held {
				self.idle += 1;
				if self.idle > STALE {
					self.queue.clear();
				}
				return 0;
			}
			self.idle = 0;
			let (bit, _, quiet) = self.queue.pop_front().unwrap_or((0, None, false));
			self.cur = bit;
			self.left = PRESS + RELEASE;
			if quiet {
				crate::retail::snd::hush(HUSH_FIELDS); // a step that only walks the cursor: no sound of its own
			}
			if let (Some(k), true) = (menu, self.cur & !STAY != 0) {
				self.spend(k);
			}
		}
		self.left -= 1;
		if self.left >= RELEASE { self.cur } else { 0 }
	}
}

/// Tests that queue commands share the two statics below: each takes this lock first (and leaves the queues empty).
#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());
/// The request waiting for the worker, with the page it was made for (the pulses' stamp when it was submitted).
static REQUEST: Mutex<Option<(Request, Option<Key>)>> = Mutex::new(None);
static PULSES: Mutex<Pulses> = Mutex::new(Pulses::new());

/// Drops every queued pad pulse (the window lost the focus: a held click or key must not finish its steps on a page nobody watches).
pub fn clear_pulses() {
	PULSES.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// Worker: pad pulses are still queued or being pressed (the Quick Select Equip waits for the ring's L1 / R1 turns first).
pub fn pulses_pending() -> bool {
	let p = PULSES.lock().unwrap_or_else(|e| e.into_inner());
	!p.queue.is_empty() || p.left > 0
}

/// Worker, a field `Retail::tick_keys` hands the game a neutral pad (instead of `feed`): `Pulses::neutral`.
pub fn neutral_field() {
	PULSES.lock().unwrap_or_else(|e| e.into_inner()).neutral();
}

/// Fields a hushed pulse silences: its press field and the next (a pulse takes 3, the following pulse's sound is not touched).
const HUSH_FIELDS: u32 = 2;

/// Main thread: queue a pad pulse that plays no menu sound (a step that only walks retail's cursor: the cards of `title_slots`, the
/// Back of a pause tab switch); the pulse that matters after it is heard.
pub fn submit_quiet(bit: u16) {
	PULSES.lock().unwrap_or_else(|e| e.into_inner()).push_with(bit, true);
}

/// Main thread: queue a command (a new request replaces the old one; pulses go in line).
pub fn submit(c: Command) {
	match c {
		Command::Request(r) => {
			let page = PULSES.lock().unwrap_or_else(|e| e.into_inner()).page;
			*REQUEST.lock().unwrap_or_else(|e| e.into_inner()) = Some((r, page));
		}
		Command::Press(b) => PULSES.lock().unwrap_or_else(|e| e.into_inner()).push(b),
	}
}

/// Worker: the request waiting for `mouse_instant`, if any.
pub fn take_request() -> Option<Cmd> {
	REQUEST.lock().unwrap_or_else(|e| e.into_inner()).take().map(|(r, _)| Cmd::Submit(r))
}

/// Worker: the waiting request for the menu on screen. One made for the shop's list is dropped once the shop settled on another of
/// its screens (a row's second click must not press Cross on the buy prompt it opened); an animating shop gets it as before
/// (`mouse_instant` drops a request while the shop is busy). A Cross request made for a spent shop screen is dropped too (`Spent`: a
/// second click on the list before the game shows the prompt), and one handed to a settled shop spends its screen. Every other menu:
/// as `take_request` (`mouse_instant` checks the page).
fn take_request_for(menu: Option<&MenuSnap>) -> Option<Cmd> {
	let (r, made_for) = REQUEST.lock().unwrap_or_else(|e| e.into_inner()).take()?;
	let mut p = PULSES.lock().unwrap_or_else(|e| e.into_inner());
	if r.confirm && made_for.is_some_and(|k| p.spent_on(k)) {
		return None;
	}
	if let Some(m) = menu.filter(|m| r.confirm && !m.busy() && made_for.is_none_or(|k| k == key(m))) {
		p.spend(key(m));
	}
	let stale = made_for.zip(menu).is_some_and(|(k, m)| k.1 != 0 && !m.busy() && key(m) != k);
	(!stale).then_some(Cmd::Submit(r))
}

/// Main thread, once per display frame in each modern host's `end`: the menu the page was built from. The pulses queued until the
/// next call are made for that page (`Pulses`); None = unknown.
pub fn stamp(menu: Option<&MenuSnap>) {
	PULSES.lock().unwrap_or_else(|e| e.into_inner()).stamp_key(menu.map(key));
}

/// The page the pulses are made for now (tests read it to play the worker's part).
#[cfg(test)]
pub(crate) fn stamped() -> Option<u32> {
	PULSES.lock().unwrap_or_else(|e| e.into_inner()).page.map(|k| k.0)
}

/// Worker: the pad bits the queued pulses add this field; `menu` = the page on screen, `b` = the pad the player makes.
pub fn pad_step(menu: Option<&MenuSnap>, b: u16) -> u16 {
	PULSES.lock().unwrap_or_else(|e| e.into_inner()).step_key(menu.map(key), menu.is_some_and(|m| !m.busy()), b != 0)
}

/// Worker, once per field in `Retail::tick_keys` after the neutral-pad cases: the screens' queued commands join the pad and the
/// instant request. A request already made by the mouse this field wins; the screen's stays queued.
pub fn feed(menu: Option<&MenuSnap>, b: &mut u16, instant: &mut Option<Cmd>) {
	*b |= pad_step(menu, *b);
	if instant.is_none() {
		*instant = take_request_for(menu);
	}
}
