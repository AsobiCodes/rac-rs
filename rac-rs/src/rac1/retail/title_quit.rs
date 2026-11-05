//! "Quit to Desktop" on the title screen's main menu (v0.5 menu lane; plan `scratch_perf_menu/QUIT_PLAN.md`, handoff
//! `HANDOFF_QUIT_STAGE_B.md`). The front end's root page has exactly three retail panels (New Game, Load Game, Options) bound to
//! three animation sequences, so a fourth retail item is not possible; this is a fourth, VIRTUAL entry owned by the host. No retail
//! RAM is written and the game never hears of it. ONE state machine serves both title styles:
//! - `FOCUS`: the virtual entry (row `ROW`) is the highlighted one. Only meaningful on the root page.
//! - `ASK`: its question ("Quit to Desktop? ...") is up. Circle = Yes, Triangle = No, Cross does nothing; on the keyboard Enter =
//!   Yes and Esc = No. Yes asks the main thread to end the program (`rac_engine::quit::request`; the flag stays set).
//! The Modern style (`modern_menu/title_host.rs`) draws it as a fourth menu button and the engine's dialog and calls `answer`. The
//! Classic style has the pad: `classic_field` (worker-side pad filter, once per field) walks the same machine on the real pad
//! (`step`, pure) and hands the game a neutral pad while the entry or its question is up; the mouse wrappers do the same for the
//! cursor. Everything here is a no-op until the player moves Down from the last retail item: the pad is returned bit for bit.
use crate::retail::disc;
use crate::retail::menu_model::addrs::P_ROOT;
use crate::retail::mouse_menu::{stick_bits, MenuSnap, CIRCLE, CROSS, DOWN, TRIANGLE, UP};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// The virtual entry's row on the root page (the retail panels are rows 0..3).
pub const ROW: usize = 3;
/// The sentinel item address of the entry in the Modern page's targets: no retail item has it (`title::Built::menu`).
pub const NATIVE_ITEM: u32 = 0xFFFF_FFFE;
/// The entry's panel in the front end's 512 x 448 space: the root panels' x, width and height (155, 203 x 39) one pitch below Options.
pub const PANEL: [f32; 4] = [155.0, 316.0, 203.0, 39.0];
/// The question's box, retail's Quit box (`frontend.rs` `Page::Quit`).
pub const BOX: [f32; 4] = [173.0, 125.0, 166.0, 168.0];
/// The answers' zones in the box: "No" on the left, "Yes" on the right (retail's label places, a little larger for the mouse).
pub const NO_ZONE: [f32; 4] = [173.0, 255.0, 83.0, 38.0];
pub const YES_ZONE: [f32; 4] = [256.0, 255.0, 83.0, 38.0];

/// The entry is highlighted (MAIN and the worker; only read on the root page).
static FOCUS: AtomicBool = AtomicBool::new(false);
/// The question is up.
static ASK: AtomicBool = AtomicBool::new(false);
/// The mouse cursor last seen, in the front end's 512 x 448 space (a click answers where the cursor is). (-1, -1) = none yet.
static CURSOR: Mutex<(f32, f32)> = Mutex::new((-1.0, -1.0));
/// The Classic pad filter's memory between fields.
static CLASSIC: Mutex<Cls> = Mutex::new(Cls::new());

pub fn focus() -> bool {
	FOCUS.load(Ordering::Relaxed)
}

pub fn ask() -> bool {
	ASK.load(Ordering::Relaxed)
}

pub fn set_focus(on: bool) {
	FOCUS.store(on, Ordering::Relaxed);
}

pub fn set_ask(on: bool) {
	ASK.store(on, Ordering::Relaxed);
}

/// Forgets everything (the title page went away, another page opened, the style changed).
pub fn clear() {
	set_focus(false);
	set_ask(false);
	*CLASSIC.lock().unwrap_or_else(|e| e.into_inner()) = Cls::new();
	*CURSOR.lock().unwrap_or_else(|e| e.into_inner()) = (-1.0, -1.0);
}

/// The player answered the question. Yes asks the main thread to end the program (the question stays up while the window closes);
/// No closes the question and leaves the entry highlighted.
pub fn answer(yes: bool) {
	if yes {
		rac_engine::quit::request();
	} else {
		set_ask(false);
	}
}

/// The two flags, as the pure core reads and writes them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Flags {
	pub focus: bool,
	pub ask: bool,
}

/// The Classic filter's memory: last field's buttons and keys (for edges) and the buttons that were held when the entry was left or
/// the question closed (the game gets a neutral pad until they are released, so the press that ended it never also acts on the menu).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Cls {
	prev: u16,
	prev_yes: bool,
	prev_no: bool,
	latch: u16,
	latch_yes: bool,
	latch_no: bool,
}

impl Cls {
	pub(crate) const fn new() -> Cls {
		Cls { prev: 0, prev_yes: false, prev_no: false, latch: 0, latch_yes: false, latch_no: false }
	}
}

/// What one field becomes: `neutral` = the game gets a neutral pad; `quit` = Yes was answered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Out {
	pub neutral: bool,
	pub quit: bool,
}

/// The pure core, one field. `root` = None when the front end is not on its root page (or the filter is off), else whether the
/// game's cursor is on the LAST retail item; `bits` = the pad plus the pushed stick as D-pad bits; `yes_key` / `no_key` = the
/// keyboard's Enter / Esc are down. Edges are presses since the last field.
/// - not on the root: everything cleared, the pad passes;
/// - question up: always neutral; No (Triangle, Esc) closes it, else Yes (Circle, Enter) quits; Cross does nothing. A question
///   opened in THIS field is not answered in it, and when both answers come in one field No wins;
/// - entry highlighted: always neutral; Up leaves it, Cross (Enter) asks;
/// - a button latched by leaving / closing is still held: neutral until it is released;
/// - Down on the last retail item highlights the entry (the game does not wrap, so it never sees that Down);
/// - everything else passes untouched.
pub(crate) fn step(c: &mut Cls, f: &mut Flags, root: Option<bool>, bits: u16, yes_key: bool, no_key: bool) -> Out {
	let (edge, yes_e, no_e) = (bits & !c.prev, yes_key && !c.prev_yes, no_key && !c.prev_no);
	(c.prev, c.prev_yes, c.prev_no) = (bits, yes_key, no_key);
	let Some(last) = root else {
		*f = Flags::default();
		(c.latch, c.latch_yes, c.latch_no) = (0, false, false);
		return Out::default();
	};
	c.latch &= bits;
	(c.latch_yes, c.latch_no) = (c.latch_yes && yes_key, c.latch_no && no_key);
	let neutral = Out { neutral: true, quit: false };
	if f.ask {
		if edge & TRIANGLE != 0 || no_e {
			f.ask = false;
			(c.latch, c.latch_yes, c.latch_no) = (bits, yes_key, no_key);
			return neutral;
		}
		return Out { quit: edge & CIRCLE != 0 || yes_e, ..neutral };
	}
	if f.focus {
		if edge & UP != 0 {
			f.focus = false;
			(c.latch, c.latch_yes, c.latch_no) = (bits, yes_key, no_key);
		} else if edge & CROSS != 0 || yes_e {
			f.ask = true;
		}
		return neutral;
	}
	if c.latch != 0 || c.latch_yes || c.latch_no {
		return neutral;
	}
	if edge & DOWN != 0 && last {
		f.focus = true;
		return neutral;
	}
	Out::default()
}

/// The front end is showing its root page (New Game / Load Game / Options).
pub fn is_root(m: &MenuSnap) -> bool {
	m.front && disc::unva(m.page) == P_ROOT
}

/// The game's cursor is on the last visible root item (Options): Down from there leaves the retail list.
pub fn cursor_last(m: &MenuSnap) -> bool {
	let mut v: Vec<_> = m.items.iter().filter(|i| i.rect[2] > 0).collect();
	v.sort_by_key(|i| i.slot);
	v.last().is_some_and(|i| i.addr == m.cur)
}

/// Classic style, worker side, once per field in `Retail::tick_keys` (after the pad sources are merged, before the Modern page's
/// filter): walks the machine on this field's pad `b`, sticks `s` and the Enter / Esc keys. `active` = the front end runs and no
/// card message box is up. True = a neutral pad was applied (`b = 0`, sticks centred); false = the pad is untouched. A Yes asks the
/// program to quit. Inactive or off the root page it only clears.
pub fn classic_field(active: bool, menu: Option<&MenuSnap>, b: &mut u16, s: &mut [u8; 4], yes_key: bool, no_key: bool) -> bool {
	let root = menu.filter(|m| active && is_root(m)).map(cursor_last);
	let mut fl = Flags { focus: focus(), ask: ask() };
	let out = step(&mut CLASSIC.lock().unwrap_or_else(|e| e.into_inner()), &mut fl, root, *b | stick_bits(*s), yes_key, no_key);
	(set_focus(fl.focus), set_ask(fl.ask));
	if out.quit {
		rac_engine::quit::request();
	}
	if out.neutral {
		(*b, *s) = (0, [0x80; 4]);
	}
	out.neutral
}

fn cursor() -> (f32, f32) {
	*CURSOR.lock().unwrap_or_else(|e| e.into_inner())
}

fn inside(r: [f32; 4], (x, y): (f32, f32)) -> bool {
	x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3]
}

/// Classic mouse: the cursor moved to (x, y) in the front end's 512 x 448 space. True = taken (the game's hover must not also steer):
/// the question is up (the hover does nothing then), or the cursor is on the entry's panel (which highlights it). A cursor on a
/// retail panel takes the highlight back and the game steers as usual.
pub fn mouse_move(menu: Option<&MenuSnap>, x: f32, y: f32) -> bool {
	*CURSOR.lock().unwrap_or_else(|e| e.into_inner()) = (x, y);
	let Some(m) = menu.filter(|m| is_root(m)) else { return false };
	if ask() {
		return true;
	}
	if inside(PANEL, (x, y)) {
		set_focus(true);
		return true;
	}
	if m.items.iter().any(|i| i.rect[2] > 0 && inside(i.rect.map(|v| v as f32), (x, y))) {
		set_focus(false);
	}
	false
}

/// Classic mouse: the cursor moved while hidden (a pad / keyboard player's desk bump): the place is kept for a click, nothing aims.
pub fn mouse_track(x: f32, y: f32) {
	*CURSOR.lock().unwrap_or_else(|e| e.into_inner()) = (x, y);
}

/// Classic mouse: a left click. True = taken. With the question up it answers on the zone under the cursor (Yes quits, No closes,
/// anywhere else nothing) and is always taken; on the entry's panel it asks; elsewhere the game gets the click.
pub fn mouse_click(menu: Option<&MenuSnap>) -> bool {
	if !menu.is_some_and(is_root) {
		return false;
	}
	let at = cursor();
	if ask() {
		if inside(YES_ZONE, at) {
			answer(true);
		} else if inside(NO_ZONE, at) {
			answer(false);
		}
		return true;
	}
	if inside(PANEL, at) {
		(set_focus(true), set_ask(true));
		return true;
	}
	false
}

/// Classic mouse: a right click. True = taken: with the question up it closes it (No).
pub fn mouse_back() -> bool {
	if ask() {
		answer(false);
		return true;
	}
	false
}

/// A root-page menu snapshot for tests (here and in the Modern host's tests): three one-row panels at retail's places, slots 0..3,
/// the game's cursor on panel `cur`.
#[cfg(test)]
pub(crate) fn test_root(cur: usize) -> MenuSnap {
	use crate::retail::mouse_menu::{Item, Kind};
	let item = |i: usize| Item {
		addr: P_ROOT + 0x100 * (i as u32 + 1),
		slot: i,
		rect: [155, [153, 207, 262][i], 203, 39],
		kind: Kind::Label { rows: vec![(19.5, true)], rowh: 39.0, cursor: 0 },
		texts: vec![[0x4f2a, 0x4f28, 0x4ee7][i]],
		planet_texts: Vec::new(),
	};
	MenuSnap { page: P_ROOT, id: 0x2e, state: 2, cur: item(cur).addr, items: (0..3).map(item).collect(), front: true }
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::retail::menu_model::cmd::TEST_LOCK;
	use std::sync::MutexGuard;

	/// The statics (this file's, the quit request) are shared with every test that touches them: `cmd::TEST_LOCK` is the one lock.
	fn world() -> MutexGuard<'static, ()> {
		let g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
		clear();
		let _ = rac_engine::quit::take();
		g
	}

	const NEUTRAL: [u8; 4] = [0x80; 4];

	/// One field of the pure core with no keys down.
	fn field(c: &mut Cls, f: &mut Flags, last: bool, bits: u16) -> Out {
		step(c, f, Some(last), bits, false, false)
	}

	fn focused() -> (Cls, Flags) {
		let (mut c, mut f) = (Cls::new(), Flags::default());
		field(&mut c, &mut f, true, 0);
		assert!(field(&mut c, &mut f, true, DOWN).neutral && f.focus, "set up: the entry is highlighted");
		field(&mut c, &mut f, true, 0);
		(c, f)
	}

	fn asking() -> (Cls, Flags) {
		let (mut c, mut f) = focused();
		assert!(field(&mut c, &mut f, true, CROSS).neutral && f.ask, "set up: the question is up");
		field(&mut c, &mut f, true, 0);
		(c, f)
	}

	#[test]
	fn down_on_the_last_item_highlights_the_entry_and_down_elsewhere_passes() {
		let (mut c, mut f) = (Cls::new(), Flags::default());
		assert_eq!(field(&mut c, &mut f, false, 0), Out::default());
		assert_eq!(field(&mut c, &mut f, false, DOWN), Out::default(), "Down on New Game / Load Game is the game's");
		assert!(!f.focus);
		field(&mut c, &mut f, true, 0);
		let out = field(&mut c, &mut f, true, DOWN);
		assert!(out.neutral && !out.quit && f.focus && !f.ask, "Down on Options moves to the entry, the game does not see it");
		assert!(field(&mut c, &mut f, true, DOWN).neutral, "held Down: still neutral");
		assert!(field(&mut c, &mut f, true, DOWN | CROSS).neutral && f.ask, "Cross while Down is held asks");
	}

	#[test]
	fn up_leaves_the_entry_and_the_game_stays_neutral_until_the_button_is_up() {
		let (mut c, mut f) = focused();
		assert!(field(&mut c, &mut f, true, 0x2000).neutral, "other buttons on the entry: neutral, nothing else");
		field(&mut c, &mut f, true, 0);
		let out = field(&mut c, &mut f, true, UP);
		assert!(out.neutral && !f.focus && !f.ask, "Up leaves");
		assert!(field(&mut c, &mut f, true, UP).neutral, "Up still held: the game must not step the list with it");
		assert!(field(&mut c, &mut f, true, UP | 0x20).neutral);
		assert_eq!(field(&mut c, &mut f, true, 0), Out::default(), "all released: the game has its pad back");
		assert_eq!(field(&mut c, &mut f, true, UP), Out::default(), "and the next Up is the game's");
	}

	#[test]
	fn cross_on_the_entry_asks_and_the_question_answers_as_designed() {
		let (mut c, mut f) = focused();
		let out = field(&mut c, &mut f, true, CROSS);
		assert!(out.neutral && !out.quit && f.ask && f.focus, "Cross asks, nothing quits yet");
		// Cross does nothing on the question, held or pressed again.
		assert_eq!(field(&mut c, &mut f, true, CROSS), Out { neutral: true, quit: false });
		field(&mut c, &mut f, true, 0);
		assert_eq!(field(&mut c, &mut f, true, CROSS), Out { neutral: true, quit: false });
		assert!(f.ask);
		field(&mut c, &mut f, true, 0);
		// Circle = Yes.
		let out = field(&mut c, &mut f, true, CIRCLE);
		assert_eq!(out, Out { neutral: true, quit: true });
		assert!(f.ask, "the question stays up while the window closes");
		// Triangle = No.
		let (mut c, mut f) = asking();
		let out = field(&mut c, &mut f, true, TRIANGLE);
		assert_eq!(out, Out { neutral: true, quit: false });
		assert!(!f.ask && f.focus, "No closes the question, the entry stays highlighted");
		assert!(field(&mut c, &mut f, true, TRIANGLE).neutral);
	}

	#[test]
	fn enter_means_yes_and_esc_means_no_on_the_keyboard() {
		let (mut c, mut f) = asking();
		let out = step(&mut c, &mut f, Some(true), CROSS, true, false);
		assert!(out.quit && out.neutral, "Enter (with the Cross it maps to) is Yes");
		let (mut c, mut f) = asking();
		let out = step(&mut c, &mut f, Some(true), TRIANGLE, false, true);
		assert!(!out.quit && out.neutral && !f.ask, "Esc (with the Triangle it maps to) is No");
		// Both in one field: No wins.
		let (mut c, mut f) = asking();
		let out = step(&mut c, &mut f, Some(true), CIRCLE | TRIANGLE, true, true);
		assert!(!out.quit && !f.ask, "a double press closes, never quits");
	}

	#[test]
	fn the_enter_that_opens_the_question_does_not_answer_it() {
		let (mut c, mut f) = focused();
		let out = step(&mut c, &mut f, Some(true), CROSS, true, false);
		assert!(f.ask && !out.quit && out.neutral, "Enter on the entry asks");
		for _ in 0..5 {
			let out = step(&mut c, &mut f, Some(true), CROSS, true, false);
			assert!(!out.quit && f.ask, "the same Enter, still held, is not Yes");
		}
		step(&mut c, &mut f, Some(true), 0, false, false);
		let out = step(&mut c, &mut f, Some(true), CROSS, true, false);
		assert!(out.quit, "a fresh Enter is");
	}

	#[test]
	fn leaving_the_root_page_clears_everything() {
		let (mut c, mut f) = asking();
		let out = step(&mut c, &mut f, None, DOWN | CIRCLE, false, false);
		assert_eq!((out, f), (Out::default(), Flags::default()), "another page: the pad passes, the flags are gone");
		assert_eq!(c.latch, 0);
		let (mut c, mut f) = focused();
		assert_eq!(step(&mut c, &mut f, None, 0, false, false), Out::default());
		assert_eq!(f, Flags::default());
		// Back on the root with the cursor on the first item: nothing is left over.
		assert_eq!(field(&mut c, &mut f, false, DOWN), Out::default());
	}

	#[test]
	fn the_pad_is_bit_for_bit_unchanged_unless_the_entry_is_used() {
		let _w = world();
		let stick = [0x11u8, 0x22, 0x33, 0x44];
		let first = test_root(0);
		let last = test_root(2);
		for b in 0..=u16::MAX {
			// Not on the root page (no menu, a pause menu, the filter off).
			let (mut p, mut s) = (b, stick);
			assert!(!classic_field(true, None, &mut p, &mut s, false, false), "no menu: pad {b:#06x}");
			assert_eq!((p, s), (b, stick));
			let (mut p, mut s) = (b, stick);
			assert!(!classic_field(false, Some(&first), &mut p, &mut s, true, true));
			assert_eq!((p, s), (b, stick), "inactive: pad {b:#06x}");
			let mut pause = first.clone();
			pause.front = false;
			let (mut p, mut s) = (b, stick);
			assert!(!classic_field(true, Some(&pause), &mut p, &mut s, false, false));
			assert_eq!((p, s), (b, stick), "pause menu: pad {b:#06x}");
			// On the root with the cursor not on the last item.
			let (mut p, mut s) = (b, stick);
			assert!(!classic_field(true, Some(&first), &mut p, &mut s, false, false));
			assert_eq!((p, s), (b, stick), "root, cursor on New Game: pad {b:#06x}");
			// On the last item, without Down.
			if b & DOWN == 0 {
				let (mut p, mut s) = (b, stick);
				assert!(!classic_field(true, Some(&last), &mut p, &mut s, false, false));
				assert_eq!((p, s), (b, stick), "root, cursor on Options, no Down: pad {b:#06x}");
			}
		}
		assert!(!focus() && !ask() && !rac_engine::quit::requested());
		// The same, with the pushed stick: an upward push is not a Down.
		let (mut p, mut s) = (0u16, [0x80u8, 0x80, 0x80, 0x00]);
		assert!(!classic_field(true, Some(&last), &mut p, &mut s, false, false) && !focus());
	}

	#[test]
	fn the_classic_filter_walks_the_machine_on_the_real_pad() {
		let _w = world();
		let (first, last) = (test_root(0), test_root(2));
		let pad = |m: &MenuSnap, b: u16, y: bool, n: bool| {
			let mut s = [0x12u8, 0x34, 0x56, 0x78];
			let mut b = b;
			let neutral = classic_field(true, Some(m), &mut b, &mut s, y, n);
			(neutral, b, s)
		};
		assert_eq!(pad(&last, 0, false, false), (false, 0, [0x12, 0x34, 0x56, 0x78]));
		assert_eq!(pad(&last, DOWN | 0x80, false, false), (true, 0, NEUTRAL), "Down on Options: the game sees a neutral pad");
		assert!(focus() && !ask());
		assert_eq!(pad(&last, 0, false, false), (true, 0, NEUTRAL), "the entry is highlighted: neutral");
		assert_eq!(pad(&last, CROSS, true, false), (true, 0, NEUTRAL));
		assert!(ask(), "Enter asks");
		assert_eq!(pad(&last, 0, false, false).0, true);
		assert_eq!(pad(&last, TRIANGLE, false, true), (true, 0, NEUTRAL));
		assert!(!ask() && focus(), "Esc closes the question");
		assert!(!rac_engine::quit::requested(), "and asked for nothing");
		// Leaving the page clears.
		let mut other = first.clone();
		other.page = 0x1d0000;
		assert_eq!(pad(&other, 0, false, false).0, false);
		assert!(!focus() && !ask());
		// Yes quits.
		pad(&last, 0, false, false);
		pad(&last, DOWN, false, false);
		pad(&last, 0, false, false);
		pad(&last, CROSS, false, false);
		assert!(ask());
		pad(&last, 0, false, false);
		assert_eq!(pad(&last, CIRCLE, false, false), (true, 0, NEUTRAL));
		assert!(rac_engine::quit::take(), "Circle asked the program to quit");
		assert!(!rac_engine::quit::take());
	}

	#[test]
	fn a_pushed_stick_works_like_the_d_pad() {
		let _w = world();
		let last = test_root(2);
		let (mut b, mut s) = (0u16, [0x80u8, 0x80, 0x80, 0x80]);
		classic_field(true, Some(&last), &mut b, &mut s, false, false);
		(b, s) = (0, [0x80, 0x80, 0x80, 0xff]);
		assert!(classic_field(true, Some(&last), &mut b, &mut s, false, false), "stick down = Down");
		assert_eq!((b, s), (0, NEUTRAL));
		assert!(focus());
	}

	#[test]
	fn the_root_and_the_last_item_are_read_from_the_snapshot() {
		let m = test_root(2);
		assert!(is_root(&m) && cursor_last(&m) && !cursor_last(&test_root(1)) && !cursor_last(&test_root(0)));
		let mut pause = test_root(2);
		pause.front = false;
		assert!(!is_root(&pause), "the pause menu is not the title");
		let mut other = test_root(2);
		other.page = 0x1d0000;
		assert!(!is_root(&other), "another front page");
		let mut hidden = test_root(2);
		hidden.items[2].rect[2] = 0;
		assert!(!cursor_last(&hidden), "an item that is not on screen is not last");
		hidden.cur = hidden.items[1].addr;
		assert!(cursor_last(&hidden), "the last VISIBLE item counts");
	}

	#[test]
	fn the_zones_lie_inside_the_box_and_the_panel_keeps_the_roots_column() {
		let inside_box = |z: [f32; 4]| z[0] >= BOX[0] && z[1] >= BOX[1] && z[0] + z[2] <= BOX[0] + BOX[2] && z[1] + z[3] <= BOX[1] + BOX[3];
		assert!(inside_box(NO_ZONE) && inside_box(YES_ZONE));
		assert!(NO_ZONE[0] + NO_ZONE[2] <= YES_ZONE[0], "the zones do not overlap");
		assert_eq!((PANEL[0], PANEL[2], PANEL[3]), (155.0, 203.0, 39.0), "same column and size as the three retail panels");
		let root = test_root(0);
		assert_eq!(root.items[0].rect[0], PANEL[0] as i32);
		assert!(PANEL[1] >= (root.items[2].rect[1] + root.items[2].rect[3]) as f32, "below Options");
		assert!(PANEL[1] + PANEL[3] <= 448.0, "on screen");
	}

	#[test]
	fn the_mouse_highlights_asks_and_answers() {
		let _w = world();
		let m = test_root(2);
		let panel = (PANEL[0] + 10.0, PANEL[1] + 10.0);
		let opt = (170.0, 280.0);
		assert!(!mouse_move(Some(&m), opt.0, opt.1), "over Options: the game steers");
		assert!(!focus());
		assert!(mouse_move(Some(&m), panel.0, panel.1), "over the entry: taken");
		assert!(focus() && !ask());
		assert!(!mouse_move(Some(&m), opt.0, opt.1) && !focus(), "back over a retail panel: the highlight returns to the game");
		assert!(!mouse_click(Some(&m)), "a click on a retail panel is the game's");
		assert!(mouse_move(Some(&m), panel.0, panel.1));
		assert!(mouse_click(Some(&m)), "a click on the entry is taken");
		assert!(ask() && focus());
		// The question: the game's hover and clicks are swallowed, only the zones answer.
		assert!(mouse_move(Some(&m), opt.0, opt.1), "hover swallowed while asking");
		assert!(mouse_click(Some(&m)) && ask() && !rac_engine::quit::requested(), "a click off the zones does nothing");
		mouse_track(BOX[0] + 5.0, BOX[1] + 5.0);
		assert!(mouse_click(Some(&m)) && ask(), "a click on the box's text does nothing");
		mouse_track(NO_ZONE[0] + 5.0, NO_ZONE[1] + 5.0);
		assert!(mouse_click(Some(&m)) && !ask(), "No closes");
		assert!(!rac_engine::quit::requested());
		assert!(mouse_move(Some(&m), panel.0, panel.1) && mouse_click(Some(&m)) && ask());
		assert!(mouse_back() && !ask(), "a right click closes the question");
		assert!(!mouse_back(), "and is the game's otherwise");
		assert!(mouse_click(Some(&m)) && ask());
		mouse_track(YES_ZONE[0] + 5.0, YES_ZONE[1] + 5.0);
		assert!(mouse_click(Some(&m)));
		assert!(rac_engine::quit::take(), "Yes quits");
		let mut other = test_root(2);
		other.front = false;
		assert!(!mouse_click(Some(&other)) && !mouse_move(Some(&other), panel.0, panel.1) && !mouse_click(None));
	}

	#[test]
	fn answer_and_clear_work_on_the_shared_flags() {
		let _w = world();
		set_focus(true);
		set_ask(true);
		answer(false);
		assert!(!ask() && focus() && !rac_engine::quit::requested());
		set_ask(true);
		answer(true);
		assert!(ask() && rac_engine::quit::take(), "Yes keeps the question up and asks to quit");
		clear();
		assert!(!focus() && !ask());
		assert_eq!(cursor(), (-1.0, -1.0));
	}
}
