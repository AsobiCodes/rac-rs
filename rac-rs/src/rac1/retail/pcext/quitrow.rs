//! The "Quit to Desktop" row of the pause Options list (v0.5 menu lane; plan `scratch_perf_menu/QUIT_PLAN.md`). The game has no way
//! out but Alt+F4; this row ends the program cleanly (`rac_engine::quit`: achievements flushed, then the window loop ends).
//!
//! The row is a retail type-3 record ("go to the page at arg") placed between Achievements and Quit Game, and its arg is the retail
//! Quit Game page (0xd), exactly like the Achievements row's. Nothing is swallowed on the way in: the game opens its own Quit Game
//! page, with its question, its Circle / Triangle answers, Cross doing nothing, the mouse's answer zones (`mouse_answers`) and the
//! modern style's dialog. Only the words and the meaning of "Yes" change, through one flag, `PENDING`, which says "the page that is
//! (about to be) open was asked for by THIS row":
//! - ARM: the worker, once per field (`filter`, called from `worker/field_run.rs` after every pad source has been merged, so the
//!   keyboard, the pad, a mouse click and the modern page's Enter all count): Cross with the cursor on this row of the idle Options
//!   page sets `PENDING`.
//! - WORDS: while `PENDING`, the text lookup (`pcext::text_hook`) answers the page's two retail texts (0x4f3f "Quit Game?" and
//!   0x4f6d "Are you sure you want to quit your current game?") with `ID_QUITD_TITLE` / `ID_QUITD_BODY`; the modern pause page
//!   asks `pending()` for its dialog (`modern_menu/pause.rs`). The words and the action come from the same flag, so they always agree.
//! - YES: `PENDING`, page 0xd settled, Circle: the field's menu buttons are cleared (the game never sees Yes, so it does not go back to
//!   the title) and `rac_engine::quit::request()` is made. The main thread ends the program (`host/handler.rs`).
//! - NO and everything else: Triangle goes to the game and the page closes; Cross does nothing; the flag clears when the page is
//!   gone (another page after page 0xd was seen), when the menu is gone (unpaused, level load, title), when the cursor left this row
//!   before the page ever opened, and after `ARM_FIELDS` fields without page 0xd. Without the flag the pad is returned bit for bit and
//!   the retail Quit Game row works as it always did (Yes = back to the title).
//! No retail RAM is written but the three records `patch_graphics` builds (`achrow::with_row`, `record`).
use super::achrow;
use crate::retail::mouse_answers::QUIT_GAME;
use crate::retail::mouse_menu::{MenuSnap, CIRCLE, CROSS, MENU_BTNS};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Texts: the row label, the description under the list, and the question's title and body (`texts.rs`).
pub const ID_QUITD: u32 = 0x7f30;
pub const ID_QUITD_DESC: u32 = 0x7f31;
pub const ID_QUITD_TITLE: u32 = 0x7f32;
pub const ID_QUITD_BODY: u32 = 0x7f33;

/// The row's label in the game's own (Classic) Options list, per language: the retail list clips a label wider than its box, so it
/// gets a short one; the modern pages and the title draw the full `ID_QUITD` text of `texts.rs` [C: m_opt_cls.png 2026-11-02, German
/// "Zum Desktop beenden" (195 font units) clipped on both sides, retail "Spiel beenden" (123) fits; widths = the disc's font 1
/// advances, test `the_classic_label_fits_the_retail_list_in_the_games_font`]. `texts::text_for` hands it to the text lookup.
pub(super) const CLASSIC_LABEL: [&[u8]; 5] = [b"To Desktop", b"Vers le bureau", b"Zum Desktop", b"Ir al escritorio", b"Vai al desktop"];

/// The widest label the Classic list shows whole, in font 1 units (its box is about 131; retail's own "Spiel beenden" is 123).
#[cfg(test)]
const CLASSIC_MAX_W: i32 = 125;

/// The question's title on the game's own (Classic) Quit Game page where the full `ID_QUITD_TITLE` text is too wide: retail's header
/// box clips at about 165 font 1 units [C: Classic pause dialog shot 2026-11-03, German "Zum Desktop beenden?" (206) shown as
/// "um Desktop beende"; retail's own "Spiel beenden?" is 134], so German and French ("Retour au bureau ?", 166, at the edge) get a
/// short title: German the short label as a question, French "Au bureau ?" (the body's own "Retour au bureau ?" without "Retour");
/// English 151, Spanish 155, Italian 158 keep theirs. `texts::text_for` hands it to the text lookup.
pub(super) const CLASSIC_TITLE: [Option<&[u8]>; 5] = [None, Some(b"Au bureau ?"), Some(b"Zum Desktop?"), None, None];

/// The widest question title the Classic header box shows whole, in font 1 units (a margin under the ~165 it clips at).
#[cfg(test)]
const CLASSIC_TITLE_MAX_W: i32 = 160;

/// Retail's Quit Game page texts the flag replaces: the header and the question.
const RETAIL_TITLE: u32 = 0x4f3f;
const RETAIL_BODY: u32 = 0x4f6d;

/// Fields the flag waits for page 0xd to open before it gives up (a press the game did not act on).
const ARM_FIELDS: u32 = 120;

/// The row's record in the Options list: "go to the Quit Game page" (`quit_page` = the retail Quit Game row's arg).
pub(super) fn record(quit_page: u32) -> [u32; 3] {
	[ID_QUITD | 3 << 16, quit_page, 0]
}

/// The worker's memory of the flag. `pending` = the Quit Game page belongs to this row; `seen` = page 0xd has been on screen since;
/// `age` = fields spent waiting for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct State {
	pending: bool,
	seen: bool,
	age: u32,
}

/// What one field's pad becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Out {
	pub pad: u16,
	pub quit: bool,
}

/// The pure core: one field. `menu` is the pause menu read from RAM (None = no menu on screen).
pub(super) fn step(s: &mut State, menu: Option<&MenuSnap>, pad: u16) -> Out {
	let mut out = Out { pad, quit: false };
	let Some(m) = menu.filter(|m| !m.front) else {
		*s = State::default();
		return out;
	};
	if !s.pending {
		if achrow::row_pressed_on(menu, pad, ID_QUITD) {
			*s = State { pending: true, seen: false, age: 0 };
		}
		return out;
	}
	if m.id == QUIT_GAME {
		s.seen = true;
		if m.state == QUIT_GAME && pad & CIRCLE != 0 {
			out.pad &= !MENU_BTNS;
			out.quit = true;
		}
	} else if s.seen {
		*s = State::default(); // the page closed (Triangle, or the game left it)
	} else {
		s.age += 1;
		if s.age > ARM_FIELDS || (m.id == 8 && m.state == 8 && !achrow::cursor_on(menu, ID_QUITD)) {
			*s = State::default(); // the game never opened the page, or the cursor moved to another row first
		}
	}
	out
}

/// The flag as the text lookup and the modern page read it (any thread).
static PENDING: AtomicBool = AtomicBool::new(false);
/// The worker's `State` (one thread writes it; the mutex only makes the static legal).
static STATE: Mutex<State> = Mutex::new(State { pending: false, seen: false, age: 0 });

/// The Quit Game page that is open (or opening) was asked for by the Quit to Desktop row.
pub fn pending() -> bool {
	PENDING.load(Ordering::Relaxed)
}

/// Text lookup (`pcext::text_hook`): the id to answer for `id`. Retail's Quit Game header and question become the Quit to Desktop ones
/// while the flag is set; every other id, and every id while it is clear, is returned unchanged.
pub(super) fn swapped(id: u32) -> u32 {
	if !pending() {
		return id;
	}
	match id {
		RETAIL_TITLE => ID_QUITD_TITLE,
		RETAIL_BODY => ID_QUITD_BODY,
		_ => id,
	}
}

/// One field with the globals: the state, the flag and the quit request.
fn apply(menu: Option<&MenuSnap>, pad: u16) -> u16 {
	let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
	let out = step(&mut s, menu, pad);
	PENDING.store(s.pending, Ordering::Relaxed);
	if out.quit {
		rac_engine::quit::request();
	}
	out.pad
}

/// Worker, once per field after `modern_swallow::filter` (`worker/field_run.rs`): the pad for the game. The menu is read from CURRENT
/// RAM (what the game acts on in this field), and only while it matters: the flag is set, or Cross is down (the Options row's press).
pub fn filter(m: &ee_oracle::Machine, front: bool, pad: u16) -> u16 {
	if front {
		return if pending() { apply(None, pad) } else { pad }; // the title: nothing to quit from, the flag clears
	}
	if !pending() && pad & CROSS == 0 {
		return pad;
	}
	let rd = |a: u32| m.rd32(a);
	apply(live(&rd, m.level(), rd(crate::retail::disc::va(0x15f5a4))).as_ref(), pad)
}

/// The menu `step` sees in this field. While the game changes pages, the menu cannot be read: the kind word is 1 and the current-page
/// word 0 for 12 fields (Options -> Quit Game included) [C: `mouse_menu::read_flying`]; those fields are a placeholder that is
/// neither page (id 0, state 1 = flying), so the flag waits for page 0xd instead of clearing (the 2026-11-03 bug: the flag cleared on
/// the way, page 0xd opened with retail's words and its Yes went back to the title). None only when there is no menu: another mode,
/// or the menu closing (kind word 0x14).
fn live(rd: &dyn Fn(u32) -> u32, level: u32, mode: u32) -> Option<MenuSnap> {
	use crate::retail::mouse_menu::{closing, read, MODE};
	read(rd, level, mode).or_else(|| (mode == MODE && !closing(rd, level, mode)).then(|| MenuSnap { state: 1, ..MenuSnap::default() }))
}

#[cfg(test)]
mod tests {
	use super::super::{NEW_IDS, OPT_DESC, OPT_RECS, TEXTS};
	use super::*;
	use crate::retail::menu_model::cmd::TEST_LOCK;
	use crate::retail::mouse_menu::{Item, Kind, TRIANGLE};
	use ee_oracle::cpu::Cpu;
	use ee_oracle::mem::Mem;

	/// The Options list as the game holds it with Mods on: the retail six, Graphics, Controls, Mods, Achievements, Quit to Desktop, Quit Game.
	fn ids() -> Vec<u16> {
		vec![0x4f1e, 0x4f1f, 0x4f20, 0x4f22, 0x4f23, 0x4f24, 0x7f06, 0x7f13, 0x7f70, achrow::ID_ACH as u16, ID_QUITD as u16, 0x4f26]
	}

	fn options(cursor: u32) -> MenuSnap {
		let texts = ids();
		let rows = texts.iter().enumerate().map(|(i, _)| (20.0 + 12.0 * i as f32, true)).collect();
		let it = Item { addr: 0x40, slot: 1, rect: [0, 0, 100, 200], kind: Kind::Label { rows, rowh: 12.0, cursor }, texts, planet_texts: Vec::new() };
		MenuSnap { page: 1, id: 8, state: 8, cur: 0x40, items: vec![it], front: false }
	}

	/// The retail Quit Game page: id 0xd, one item that is not a list.
	fn quit_page(state: u32) -> MenuSnap {
		let it = Item { addr: 0x80, slot: 0, rect: [0, 0, 100, 100], kind: Kind::Other, texts: Vec::new(), planet_texts: Vec::new() };
		MenuSnap { page: 2, id: QUIT_GAME, state, cur: 0x80, items: vec![it], front: false }
	}

	const ROW: u32 = 10;
	const RETAIL_QUIT_ROW: u32 = 11;

	fn armed() -> State {
		let mut s = State::default();
		step(&mut s, Some(&options(ROW)), CROSS);
		assert!(s.pending);
		s
	}

	#[test]
	fn the_press_arms_only_on_cross_with_the_cursor_on_the_row_of_the_idle_options_page() {
		let mut s = State::default();
		for (menu, pad, why) in [
			(options(ROW), 0, "no press"),
			(options(ROW), CIRCLE, "Circle is back"),
			(options(ROW), TRIANGLE, "Triangle is back"),
			(options(RETAIL_QUIT_ROW), CROSS, "the retail Quit Game row is the game's"),
			(options(ROW - 1), CROSS, "the Achievements row"),
			(options(0), CROSS, "another row"),
			(quit_page(QUIT_GAME), CROSS, "another page"),
			(MenuSnap { state: 1, ..options(ROW) }, CROSS, "flying in"),
			(MenuSnap { state: 0x14, ..options(ROW) }, CROSS, "closing"),
			(MenuSnap { front: true, ..options(ROW) }, CROSS, "the title's menu"),
		] {
			assert_eq!(step(&mut s, Some(&menu), pad), Out { pad, quit: false }, "{why}");
			assert!(!s.pending, "{why}");
		}
		assert_eq!(step(&mut s, None, CROSS), Out { pad: CROSS, quit: false }, "no menu");
		assert!(!s.pending);
		// The press: pending, and the pad is still the game's own (the game opens its Quit Game page).
		assert_eq!(step(&mut s, Some(&options(ROW)), CROSS | 0x4000), Out { pad: CROSS | 0x4000, quit: false });
		assert!(s.pending && !s.seen);
	}

	#[test]
	fn yes_on_the_settled_page_asks_to_quit_and_the_game_never_sees_it() {
		let mut s = armed();
		assert_eq!(step(&mut s, Some(&options(ROW)), 0).pad, 0, "the page is opening");
		let pad = CIRCLE | 0x4000 | 0x1000 | 0x400; // Circle, Down, Up and a bit that is not a menu button
		let o = step(&mut s, Some(&quit_page(QUIT_GAME)), pad);
		assert!(o.quit, "Circle on the settled page is Yes");
		assert_eq!(o.pad, 0x400, "every menu button is cleared, the rest is left alone");
		assert!(s.pending, "the program ends next; the words stay");
		assert!(!step(&mut s, Some(&quit_page(0x14)), CIRCLE).quit, "not while the page closes");
		assert!(!step(&mut s, Some(&quit_page(1)), CIRCLE).quit, "not while it flies in");
	}

	#[test]
	fn cross_triangle_and_the_other_buttons_are_the_games_on_the_question() {
		let mut s = armed();
		for pad in [0, CROSS, TRIANGLE, 0x4000, CROSS | TRIANGLE, 0x800] {
			assert_eq!(step(&mut s, Some(&quit_page(QUIT_GAME)), pad), Out { pad, quit: false }, "pad {pad:#x}");
			assert!(s.pending && s.seen);
		}
		// Triangle sends the page away: the flag clears when the Options list is back.
		assert!(step(&mut s, Some(&quit_page(0x14)), TRIANGLE).pad == TRIANGLE && s.pending, "closing still shows the words");
		step(&mut s, Some(&options(ROW)), 0);
		assert_eq!(s, State::default(), "page change after page 0xd was seen");
	}

	#[test]
	fn the_flag_clears_on_every_other_path() {
		// The menu closes (unpaused, a level load, the title): no snapshot.
		let mut s = armed();
		step(&mut s, None, 0);
		assert_eq!(s, State::default());
		// The page is gone in one step (a different page, even one that is not Options).
		let mut s = armed();
		step(&mut s, Some(&quit_page(QUIT_GAME)), 0);
		step(&mut s, Some(&MenuSnap { id: 2, state: 2, ..options(0) }), 0);
		assert_eq!(s, State::default());
		// The game never opened the page: the cursor moved to another row first ...
		let mut s = armed();
		step(&mut s, Some(&options(RETAIL_QUIT_ROW)), 0);
		assert_eq!(s, State::default(), "retail's own Quit Game row opened later keeps its retail words and Yes");
		// ... or nothing happened for ARM_FIELDS fields.
		let mut s = armed();
		for _ in 0..ARM_FIELDS {
			step(&mut s, Some(&options(ROW)), 0);
			assert!(s.pending);
		}
		step(&mut s, Some(&options(ROW)), 0);
		assert_eq!(s, State::default(), "safety timeout");
		// The title's front end.
		let mut s = armed();
		step(&mut s, Some(&MenuSnap { front: true, ..quit_page(QUIT_GAME) }), CIRCLE);
		assert_eq!(s, State::default());
	}

	#[test]
	fn without_the_flag_the_pad_is_returned_bit_for_bit_and_retails_quit_game_is_retails() {
		let menus = [None, Some(options(ROW)), Some(options(RETAIL_QUIT_ROW)), Some(quit_page(QUIT_GAME)), Some(quit_page(0x14)), Some(MenuSnap { front: true, ..quit_page(QUIT_GAME) })];
		for menu in &menus {
			for pad in 0..=0xffffu32 {
				let pad = pad as u16;
				let mut s = State::default();
				let o = step(&mut s, menu.as_ref(), pad);
				let armed_now = menu.as_ref().is_some_and(|m| m.id == 8 && !m.front) && pad & CROSS != 0 && menu.as_ref().is_some_and(|m| achrow::cursor_on(Some(m), ID_QUITD));
				assert_eq!((o.pad, o.quit, s.pending), (pad, false, armed_now), "menu {menu:?} pad {pad:#x}");
			}
		}
		// Circle on retail's Quit Game page, no flag: Yes is the game's (back to the title), no quit request.
		let mut s = State::default();
		assert_eq!(step(&mut s, Some(&quit_page(QUIT_GAME)), CIRCLE), Out { pad: CIRCLE, quit: false });
	}

	/// The 2026-11-03 bug: the 12 fields of the page change Options -> Quit Game read no menu; the flag must survive them, so page 0xd
	/// shows the Quit to Desktop words and its Yes quits. Leaving page 0xd (Triangle) through the same kind of fields clears it.
	#[test]
	fn the_flag_survives_the_page_change_to_the_quit_game_page_and_clears_on_the_way_back() {
		let flying = MenuSnap { state: 1, ..MenuSnap::default() };
		let mut s = armed();
		for _ in 0..12 {
			assert_eq!(step(&mut s, Some(&flying), 0), Out { pad: 0, quit: false });
			assert!(s.pending && !s.seen, "on the way to page 0xd");
		}
		step(&mut s, Some(&quit_page(1)), 0);
		assert!(s.pending && s.seen, "landing");
		assert_eq!(step(&mut s, Some(&quit_page(QUIT_GAME)), CIRCLE), Out { pad: 0, quit: true }, "Yes quits, the game never sees it");
		// No instead: Triangle, then the page change back to the Options list.
		let mut s = armed();
		step(&mut s, Some(&flying), 0);
		step(&mut s, Some(&quit_page(QUIT_GAME)), TRIANGLE);
		step(&mut s, Some(&flying), 0);
		assert_eq!(s, State::default(), "page 0xd left");
	}

	/// `live`: a readable page is that page; a page change (kind word 1, current-page word 0) is the flying placeholder; a closing
	/// menu (kind word 0x14) and another mode are no menu.
	#[test]
	fn the_live_menu_reads_a_page_change_as_flying_and_a_closing_menu_as_none() {
		use crate::retail::mouse_menu::{page_ptr, MODE};
		let pp = page_ptr(4).unwrap();
		let mut w = std::collections::HashMap::<u32, u32>::from([(pp - 4, 1), (pp, 0), (pp + 4, 0x1b2608)]);
		let at = |w: &std::collections::HashMap<u32, u32>, mode| live(&|a| w.get(&a).copied().unwrap_or(0), 4, mode);
		assert_eq!(at(&w, MODE), Some(MenuSnap { state: 1, ..MenuSnap::default() }), "a page change");
		assert_eq!(at(&w, 0), None, "not the page-menu mode");
		w.insert(pp - 4, 0x14);
		assert_eq!(at(&w, MODE), None, "the menu closes");
		// A page that reads: that page (the Quit Game page, its kind word = its id while idle).
		let page = 0x1b2608u32;
		w.extend([(pp - 4, QUIT_GAME), (pp, page), (pp + 4, 0), (page + 0x3c, QUIT_GAME)]);
		assert_eq!(at(&w, MODE).map(|m| (m.page, m.id)), Some((page, QUIT_GAME)));
	}

	#[test]
	fn the_globals_follow_the_state_and_the_quit_request_is_made_once_per_ask() {
		let _t = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
		let _ = rac_engine::quit::take();
		*STATE.lock().unwrap() = State::default();
		PENDING.store(false, Ordering::Relaxed);
		assert_eq!(swapped(RETAIL_TITLE), RETAIL_TITLE);
		assert_eq!(apply(Some(&options(ROW)), CROSS), CROSS);
		assert!(pending(), "armed");
		assert_eq!((swapped(RETAIL_TITLE), swapped(RETAIL_BODY), swapped(0x524f), swapped(0x4f26)), (ID_QUITD_TITLE, ID_QUITD_BODY, 0x524f, 0x4f26), "only the two Quit Game texts");
		assert_eq!(apply(Some(&quit_page(QUIT_GAME)), CROSS), CROSS);
		assert!(!rac_engine::quit::requested(), "Cross does not quit");
		assert_eq!(apply(Some(&quit_page(QUIT_GAME)), CIRCLE), 0);
		assert!(rac_engine::quit::take(), "Yes asked to quit");
		assert!(!rac_engine::quit::take(), "one ask");
		assert_eq!(apply(None, 0), 0);
		assert!(!pending() && swapped(RETAIL_BODY) == RETAIL_BODY, "menu gone: the retail words are back");
		// The title: a flag left over is cleared, an idle field costs nothing.
		*STATE.lock().unwrap() = State { pending: true, seen: true, age: 0 };
		PENDING.store(true, Ordering::Relaxed);
		assert_eq!(apply(None, 0), 0);
		assert!(!pending());
		let _ = rac_engine::quit::take();
	}

	#[test]
	fn the_text_lookup_swaps_the_question_only_while_the_flag_is_set_in_every_language() {
		let _t = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
		let mut c = Cpu::new(Mem::new());
		for (word, l) in [(0u32, 0usize), (2, 1), (3, 2), (4, 3), (5, 4)] {
			c.mem.wr32(super::super::LANG_WORD, word);
			for (retail, ours) in [(RETAIL_TITLE, ID_QUITD_TITLE), (RETAIL_BODY, ID_QUITD_BODY)] {
				PENDING.store(false, Ordering::Relaxed);
				c.s(4, retail as u64);
				assert!(!super::super::text_hook(&mut c), "{retail:#x} not pending: retail's lookup runs (lang {word})");
				PENDING.store(true, Ordering::Relaxed);
				c.s(4, retail as u64);
				assert!(super::super::text_hook(&mut c), "{retail:#x} pending (lang {word})");
				let k = NEW_IDS.iter().position(|&n| n == ours).unwrap();
				let p = c.g(2) as u32;
				assert_eq!(p, super::super::texts::pool_slot(k), "the pool slot of the replacement");
				let mut got = Vec::new();
				while c.mem.rd8(p + got.len() as u32) != 0 {
					got.push(c.mem.rd8(p + got.len() as u32));
				}
				assert_eq!(got, super::super::texts::text_for(k, l), "language {l}");
			}
		}
		PENDING.store(false, Ordering::Relaxed);
	}

	/// The Classic list's label in the game's own font (read from the PAL disc when it is there; no disc: nothing to measure).
	#[test]
	fn the_classic_label_fits_the_retail_list_in_the_games_font() {
		let k = NEW_IDS.iter().position(|&n| n == ID_QUITD).unwrap();
		for l in 0..5 {
			assert_eq!(super::super::texts::text_for(k, l), CLASSIC_LABEL[l], "the game's lookup gets the short label");
		}
		let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso");
		let Ok(mut disc) = crate::iso::Disc::open(std::path::Path::new(path)) else { return };
		let font = &crate::toc::font_glyph_tables(&mut disc).unwrap()[0];
		let w = |t: &[u8]| -> i32 { t.iter().map(|&b| font.get(b as usize).map_or(0, |g| g.adv as i32)).sum() };
		assert_eq!(w(b"Spiel beenden"), 123, "the anchor: retail's own label that fits");
		for (l, t) in CLASSIC_LABEL.iter().enumerate() {
			assert!(w(t) <= CLASSIC_MAX_W, "language {l}: {} units", w(t));
		}
		assert!(w(TEXTS[k][2]) > CLASSIC_MAX_W, "the full German label is the one that was clipped");
	}

	/// The question's title on the Classic Quit Game page in the game's own font (read from the PAL disc when it is there).
	#[test]
	fn the_classic_title_fits_the_retail_header_box_in_the_games_font() {
		let t = NEW_IDS.iter().position(|&n| n == ID_QUITD_TITLE).unwrap();
		for l in 0..5 {
			assert_eq!(super::super::texts::text_for(t, l), CLASSIC_TITLE[l].unwrap_or(TEXTS[t][l]), "language {l}: the short title only where set");
		}
		let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso");
		let Ok(mut disc) = crate::iso::Disc::open(std::path::Path::new(path)) else { return };
		let font = &crate::toc::font_glyph_tables(&mut disc).unwrap()[0];
		let w = |t: &[u8]| -> i32 { t.iter().map(|&b| font.get(b as usize).map_or(0, |g| g.adv as i32)).sum() };
		assert_eq!(w(b"Spiel beenden?"), 134, "the anchor: retail's own title that fits");
		for l in 0..5 {
			let s = super::super::texts::text_for(t, l);
			assert!(w(&s) <= CLASSIC_TITLE_MAX_W, "language {l}: {} units", w(&s));
		}
		assert!(w(TEXTS[t][2]) > CLASSIC_TITLE_MAX_W, "the full German title is the one that was clipped");
	}

	#[test]
	fn the_four_texts_exist_in_five_languages_inside_the_pool_and_fit_their_boxes() {
		let limits = [(ID_QUITD, 22), (ID_QUITD_DESC, 40), (ID_QUITD_TITLE, 20), (ID_QUITD_BODY, 60)];
		let mut slots = std::collections::HashSet::new();
		for (id, max) in limits {
			let k = NEW_IDS.iter().position(|&n| n == id).unwrap_or_else(|| panic!("{id:#x} is in NEW_IDS"));
			assert_eq!(NEW_IDS.iter().filter(|&&n| n == id).count(), 1);
			for (l, t) in TEXTS[k].iter().enumerate() {
				assert!(!t.is_empty() && t.len() <= max, "{id:#x} language {l}: {} bytes (max {max})", t.len());
				assert!(t.iter().all(|&b| b >= 0x20), "{id:#x} language {l}: only printable font bytes");
			}
			let at = super::super::texts::pool_slot(k);
			assert!(at + 0x40 <= super::super::camopts::CO_POOL_END && at % 0x40 == 0, "{id:#x} slot {at:#x} is inside the pool");
			assert!(slots.insert(at), "{id:#x} has a slot of its own");
		}
		assert_eq!((TEXTS[NEW_IDS.iter().position(|&n| n == ID_QUITD).unwrap()][0], TEXTS[NEW_IDS.iter().position(|&n| n == ID_QUITD).unwrap()][3]), (&b"Quit to Desktop"[..], &b"Salir al escritorio"[..]));
	}

	#[test]
	fn the_list_has_room_for_the_row_and_the_record_goes_to_the_quit_game_page() {
		// Mods on: 7 + Graphics, Controls, Mods, Achievements, Quit to Desktop = 12 rows + the zero record = 13 records of 12 bytes.
		assert!(OPT_RECS + 12 * 15 <= OPT_DESC, "the records end before the descriptions");
		assert!(OPT_DESC + 4 * 15 <= super::super::GPAGE, "the 14 descriptions + 0 end before the Graphics page");
		assert_eq!(record(0x1234), [ID_QUITD | 3 << 16, 0x1234, 0]);
		assert_eq!(record(0x1234)[0] >> 16, 3, "type 3: go to the page");
	}
}
