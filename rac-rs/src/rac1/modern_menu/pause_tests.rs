//! Tests of the pure pause builder (`pause.rs`, v0.5 screens stage C) on synthetic pages: which pages are modern, what each page
//! builds, what a tab, row or button does, and that the words fit in all five languages. The fixtures (`view_of`, `snap`, `facts`)
//! are shared with the host tests (`pause_host/tests.rs`).
use super::pause::*;
use crate::modern_menu::hide::Screen;
use crate::retail::menu_keys;
use crate::retail::menu_model::extra::{Extra, Sound, Toggle};
use crate::retail::menu_model::pages::Page;
use crate::retail::menu_model::text::decode;
use crate::retail::menu_model::view::{Body, ItemView, PageView, RowView};
use crate::retail::mouse_menu::{Item, Kind, MenuSnap, CIRCLE, TRIANGLE};
use rac_engine::ui::font::FontMeasure;
use rac_engine::ui::input::{Device, GlyphSet};
use rac_engine::ui::kit::page::Panel;
use rac_engine::ui::kit::page_draw;
use rac_engine::ui::kit::text::run_width;
use rac_engine::ui::prim::FontRole;

/// The one item of every fixture page.
pub(crate) const ADDR: u32 = 0x1d0000;

pub(crate) fn text_of(id: u32) -> String {
	format!("T{id:x}")
}

fn row_views(rows: &[(u16, bool)]) -> Vec<RowView> {
	rows.iter().map(|&(id, enabled)| RowView { text_id: id, text: decode(&text_of(id as u32)), name: None, enabled }).collect()
}

fn item_view(body: Body) -> ItemView {
	ItemView { addr: ADDR, slot: 0, rect: [0, 0, 300, 300], body }
}

fn page_view(page: Page, body: Body) -> PageView {
	PageView { screen: Screen::Pause, page, id: 0, ready: true, front: false, keys: menu_keys::DEFAULT, cur: Some(0), items: vec![item_view(body)] }
}

/// A page with one label list: rows (text id, enabled), cursor row.
pub(crate) fn view_of(page: Page, rows: &[(u16, bool)], cursor: usize) -> PageView {
	page_view(page, Body::Rows { rows: row_views(rows), cursor })
}

/// A page with one value item of `n` rows (Sound, HelpDesk, Subtitles).
pub(crate) fn values_of(page: Page, n: u32, cursor: u32) -> PageView {
	page_view(page, Body::Values { n, cursor })
}

/// The retail menu the fixture page was read from (page id `id`, settled), its one item at `ADDR`.
pub(crate) fn snap(id: u32, rows: &[(u16, bool)], cursor: u32) -> MenuSnap {
	let kind = Kind::Label { rows: rows.iter().enumerate().map(|(i, r)| (10.0 + 20.0 * i as f32, r.1)).collect(), rowh: 20.0, cursor };
	let item = Item { addr: ADDR, slot: 0, rect: [0, 0, 300, 300], kind, texts: rows.iter().map(|r| r.0).collect(), planet_texts: Vec::new() };
	MenuSnap { page: 0x1d0000, id, state: id, cur: ADDR, items: vec![item], front: false }
}

/// A value page's menu (Sound, HelpDesk, Subtitles).
pub(crate) fn snap_values(id: u32, n: u32, cursor: u32) -> MenuSnap {
	let mut m = snap(id, &[], 0);
	m.items[0].kind = Kind::Value { n, cursor };
	m
}

pub(crate) fn facts<'a>(view: &'a PageView, extra: &'a Extra, text: &'a dyn Fn(u32) -> String) -> Facts<'a> {
	Facts { view, extra, text, goodies: false, level: 0, bolts: 123, local: None, now: 0, lang: 0, device: Device::Pad, glyphs: GlyphSet::PlayStation, quit_desktop: false, ach: None }
}

fn built(view: &PageView, extra: &Extra) -> Option<Built> {
	build(&facts(view, extra, &text_of))
}

const ROOT_IDS: [u16; 7] = [0x4ee2, 0x4ee3, 0x4ee4, 0x4ee5, 0x4ee6, 0x4ee7, 0x4fc2];

fn rows_of(ids: &[u16]) -> Vec<(u16, bool)> {
	ids.iter().map(|&i| (i, true)).collect()
}

fn gate() -> Gate {
	Gate { modern: true, ui_open: false, list_open: false, refused: false, front: false, paused: true, page: Some(Page::Root) }
}

const IN_SET: [Page; 15] = [Page::Root, Page::Weapons, Page::Gadgets, Page::QuickSelect, Page::Items, Page::Help, Page::Options, Page::Goodies, Page::HelpDesk, Page::Subtitles, Page::Sound, Page::Save, Page::Load, Page::Overwrite, Page::QuitGame];
const CLASSIC: [Page; 10] = [
	Page::Camera,
	Page::HelpLog,
	Page::Controls,
	Page::GalaxyMap,
	Page::GalaxyDetails,
	Page::PauseMap,
	Page::Missions,
	Page::Moves,
	Page::HelpWeapons,
	Page::Other(0x99),
];

#[test]
fn page_ids_route_to_the_modern_or_the_classic_set() {
	for p in IN_SET {
		assert!(in_set(p), "{p:?} is modern");
	}
	for p in CLASSIC {
		assert!(!in_set(p), "{p:?} stays classic");
		assert!(built(&view_of(p, &rows_of(&[1, 2, 3]), 0), &Extra::default()).is_none(), "{p:?} builds nothing");
	}
}

#[test]
fn wanted_needs_every_condition() {
	assert!(wanted(&gate()));
	let off: [fn(&mut Gate); 7] = [|g| g.modern = false, |g| g.ui_open = true, |g| g.list_open = true, |g| g.refused = true, |g| g.front = true, |g| g.paused = false, |g| g.page = None];
	for (i, f) in off.iter().enumerate() {
		let mut g = gate();
		f(&mut g);
		assert!(!wanted(&g), "condition {i}");
	}
	for p in IN_SET {
		assert!(wanted(&Gate { page: Some(p), ..gate() }), "{p:?}");
	}
	for p in CLASSIC {
		assert!(!wanted(&Gate { page: Some(p), ..gate() }), "{p:?}");
	}
}

#[test]
fn classic_style_wants_nothing() {
	for p in IN_SET {
		assert!(!wanted(&Gate { modern: false, page: Some(p), ..gate() }), "{p:?}");
	}
}

#[test]
fn hide_is_on_only_while_the_modern_page_is_wanted() {
	use crate::modern_menu::hide::hidden_in;
	// The glue sets the switch to `wanted && built`; the pure core hides the retail picture only for a set switch in the Modern style.
	let mask = 1u8 << Screen::Pause as u8;
	assert!(hidden_in(Screen::Pause, mask, true, None));
	assert!(!hidden_in(Screen::Pause, 0, true, None), "no page drawn: the retail pause shows");
	assert!(!hidden_in(Screen::Pause, mask, false, None), "Classic: never hidden");
	assert!(wanted(&Gate { page: Some(Page::QuickSelect), ..gate() }), "Quick Select is a modern page since v5i");
	assert!(!wanted(&Gate { page: Some(Page::Other(0x1b)), ..gate() }), "a card message box page: classic");
}

#[test]
fn the_root_is_a_tab_bar_with_the_cursor_selected() {
	let v = view_of(Page::Root, &rows_of(&ROOT_IDS), 5);
	let b = built(&v, &Extra::default()).unwrap();
	assert!(b.root && b.goodies);
	assert_eq!(b.screen.menu.len(), 8, "seven root rows and the Map tab");
	assert_eq!(b.screen.menu[0].text, "T4ee2");
	assert_eq!(b.screen.menu[6].text, "T4fc2");
	assert_eq!(b.screen.menu.iter().position(|m| m.selected), Some(5));
	assert!(b.screen.menu.iter().all(|m| m.live));
	assert_eq!(b.screen.menu[7].text, "Map");
	assert_eq!(b.menu, (0..7).map(|r| Some((ADDR, r))).chain([None]).collect::<Vec<_>>(), "the Map tab has no root row");
	assert!(b.items.is_empty() && b.buttons.is_empty() && b.screen.dialog.is_none());
	assert!(matches!(b.screen.panel, Panel::None));
	assert_eq!((b.rows(), b.target(5), b.target(7), b.target(9), b.sel), (8, Some((ADDR, 5)), None, None, 5));
	let sh = b.screen.shell.as_ref().unwrap();
	assert_eq!((sh.bolts.as_str(), sh.label.as_str()), ("123", "Paused"));
}

/// The root as retail builds it: one one-row label item per entry, the current item is the selected one (RAC_MENU_LOG of page 0x2,
/// 2026-11-02: six items at 0x1b2738.., each `rows: [(18.5, true)]`, cursor 0). Every tab gets its own item as the target.
#[test]
fn the_root_of_one_item_per_entry_targets_every_tab() {
	let items: Vec<ItemView> = ROOT_IDS[..6].iter().enumerate().map(|(k, &id)| ItemView { addr: ADDR + 0x50 * k as u32, slot: k, rect: [155, 43 + 54 * k as i32, 203, 39], body: Body::Rows { rows: row_views(&[(id, true)]), cursor: 0 } }).collect();
	let v = PageView { cur: Some(2), items, ..page_view(Page::Root, Body::Other) };
	let b = built(&v, &Extra::default()).unwrap();
	assert_eq!(b.screen.menu.len(), 7);
	assert_eq!(b.screen.menu.iter().position(|m| m.selected), Some(2), "the current item's tab");
	assert!(b.screen.menu.iter().all(|m| m.live));
	assert_eq!(b.menu, (0..6).map(|k| Some((ADDR + 0x50 * k, 0))).chain([None]).collect::<Vec<_>>(), "each tab selects its own item");
	assert_eq!(b.sel, 2);
}

#[test]
fn the_goodies_tab_appears_only_when_the_game_lists_it() {
	let b = built(&view_of(Page::Root, &rows_of(&ROOT_IDS[..6]), 0), &Extra::default()).unwrap();
	assert!(!b.goodies);
	assert_eq!(b.screen.menu.len(), 7);
	// A child page keeps the tab the host remembered.
	let v = view_of(Page::Options, &rows_of(&[1, 2]), 0);
	let ex = Extra::default();
	let mut f = facts(&v, &ex, &text_of);
	assert_eq!(build(&f).unwrap().screen.menu.len(), 7);
	f.goodies = true;
	assert_eq!(build(&f).unwrap().screen.menu.len(), 8);
}

#[test]
fn a_disabled_tab_is_not_live() {
	let mut rows = rows_of(&ROOT_IDS);
	rows[3].1 = false;
	let b = built(&view_of(Page::Root, &rows, 0), &Extra::default()).unwrap();
	assert_eq!(b.screen.menu.iter().map(|m| m.live).collect::<Vec<_>>(), [true, true, true, false, true, true, true, true]);
	assert_eq!(b.menu[3], None, "inert: the host never selects it");
}

#[test]
fn the_player_focus_wins_over_the_cursor_for_a_moment() {
	let v = view_of(Page::Root, &rows_of(&ROOT_IDS), 5);
	let ex = Extra::default();
	let mut f = facts(&v, &ex, &text_of);
	f.local = Some((1, 1000));
	f.now = 1100;
	assert_eq!(build(&f).unwrap().screen.menu.iter().position(|m| m.selected), Some(1));
	f.now = 1400;
	assert_eq!(build(&f).unwrap().screen.menu.iter().position(|m| m.selected), Some(5), "after the grace the game's cursor");
}

#[test]
fn help_and_options_list_the_games_rows_under_their_tab() {
	let ids: Vec<u16> = (0x4f00..0x4f0a).collect();
	for (page, tab) in [(Page::Help, 4), (Page::Options, 5)] {
		let b = built(&view_of(page, &rows_of(&ids), 2), &Extra::default()).unwrap();
		let Panel::Rows { rows, .. } = &b.screen.panel else { panic!("rows") };
		assert_eq!(rows.len(), 10, "all ten Options rows");
		assert_eq!(rows.iter().position(|r| r.selected), Some(2));
		assert_eq!(b.screen.menu.iter().position(|m| m.selected), Some(tab));
		assert!(b.screen.menu.iter().all(|m| !m.live), "the tabs are inert on a child page");
		assert!(b.menu.iter().all(|t| t.is_none()));
		assert_eq!(b.items[2], Some((ADDR, 2)));
		assert!(!b.root);
	}
}

#[test]
fn goodies_is_its_own_tab() {
	let b = built(&view_of(Page::Goodies, &rows_of(&[1, 2, 3]), 0), &Extra::default()).unwrap();
	assert_eq!(b.screen.menu.len(), 8);
	assert_eq!(b.screen.menu.iter().position(|m| m.selected), Some(6));
}

#[test]
fn a_disabled_row_is_dimmed_and_an_empty_label_keeps_the_page_classic() {
	let mut rows = rows_of(&[1, 2, 3]);
	rows[1].1 = false;
	let b = built(&view_of(Page::Options, &rows, 0), &Extra::default()).unwrap();
	let Panel::Rows { rows, .. } = &b.screen.panel else { panic!("rows") };
	assert_eq!(rows.iter().map(|r| r.dim).collect::<Vec<_>>(), [false, true, false]);
	let mut v = view_of(Page::Options, &rows_of(&[1, 2, 3]), 0);
	let Body::Rows { rows, .. } = &mut v.items[0].body else { unreachable!() };
	rows[1].text = decode("");
	assert!(built(&v, &Extra::default()).is_none());
}

fn toggles() -> Extra {
	let t = |text, on| Toggle { text, on, on_text: 0x5210, off_text: 0x5211 };
	Extra { toggles: vec![t(0x4f9f, true), t(0x4fa0, false)], ..Default::default() }
}

#[test]
fn toggle_pages_show_one_row_per_toggle_with_the_on_or_off_word() {
	for page in [Page::HelpDesk, Page::Subtitles] {
		let b = built(&values_of(page, 2, 1), &toggles()).unwrap();
		let Panel::Rows { rows, .. } = &b.screen.panel else { panic!("rows") };
		assert_eq!((rows[0].label.as_str(), rows[0].value.as_str()), ("T4f9f", "T5210"));
		assert_eq!((rows[1].label.as_str(), rows[1].value.as_str()), ("T4fa0", "T5211"));
		assert_eq!(rows.iter().position(|r| r.selected), Some(1));
		assert_eq!(b.items, vec![Some((ADDR, 0)), Some((ADDR, 1))]);
	}
	assert!(built(&values_of(Page::HelpDesk, 3, 0), &toggles()).is_none(), "row count differs from the list: classic");
	assert!(built(&values_of(Page::HelpDesk, 2, 0), &Extra::default()).is_none(), "no toggle facts: classic");
}

#[test]
fn sound_rows_carry_percent_bars_and_the_stereo_word() {
	let x = |stereo| Extra { sound: Some(Sound { sfx: 0x400, music: 0x200, stereo }), ..Default::default() };
	let b = built(&values_of(Page::Sound, 3, 0), &x(true)).unwrap();
	let Panel::Rows { rows, .. } = &b.screen.panel else { panic!("rows") };
	assert_eq!((rows[0].value.as_str(), rows[0].bar), ("100%", Some(1.0)));
	assert_eq!((rows[1].value.as_str(), rows[1].bar), ("50%", Some(0.5)));
	assert_eq!((rows[2].value.as_str(), rows[2].bar), ("T5216", None));
	assert_eq!((rows[0].label.as_str(), rows[2].label.as_str()), ("T5212", "T5214"));
	let Panel::Rows { rows, .. } = &built(&values_of(Page::Sound, 3, 0), &x(false)).unwrap().screen.panel else { panic!("rows") };
	assert_eq!(rows[2].value, "T5215", "mono");
	assert!(built(&values_of(Page::Sound, 3, 0), &Extra::default()).is_none());
	assert!(built(&values_of(Page::Sound, 2, 0), &x(true)).is_none(), "not the three-row list");
}

fn slots() -> Extra {
	let used = Some((0u32, 1234u32, 216_000 * 2 + 3600 * 5, [0x24u8, 0x11, 0x02]));
	Extra { slots: vec![used, None, None, used, None], ..Default::default() }
}

#[test]
fn save_and_load_show_five_cards_empty_ones_dimmed() {
	for page in [Page::Save, Page::Load] {
		let b = built(&view_of(page, &rows_of(&[1, 2, 3, 4, 5]), 3), &slots()).unwrap();
		let Panel::Cards { cards, extra, .. } = &b.screen.panel else { panic!("cards") };
		assert_eq!(cards.len(), 5);
		assert!(extra.is_none());
		assert_eq!(cards.iter().map(|c| c.dim).collect::<Vec<_>>(), [false, true, true, false, true]);
		assert_eq!(cards.iter().position(|c| c.selected), Some(3));
		assert_eq!(cards[0].label, format!("1  {}", crate::toc::LEVEL_NAMES[0]));
		assert_eq!(cards[1].label, "2  T5217");
		let values: Vec<&str> = cards[0].stats.iter().map(|s| s.value.as_str()).collect();
		assert_eq!(values, ["1234", "2:05", "11/24/02"]);
		assert_eq!(b.items.len(), 5);
		assert_eq!(b.items[4], Some((ADDR, 4)));
	}
	// Retail's slot list as the in-level reader gives it (mouse_menu `slot_list`: five value rows, the list's cursor).
	for page in [Page::Save, Page::Load] {
		let b = built(&values_of(page, 5, 2), &slots()).unwrap();
		let Panel::Cards { cards, .. } = &b.screen.panel else { panic!("cards") };
		assert_eq!((cards.len(), cards.iter().position(|c| c.selected)), (5, Some(2)));
		assert_eq!(b.items, (0..5).map(|r| Some((ADDR, r))).collect::<Vec<_>>());
	}
	assert!(built(&view_of(Page::Save, &rows_of(&[1, 2, 3, 4, 5]), 0), &Extra::default()).is_none(), "no slot facts: classic");
	assert!(built(&view_of(Page::Save, &rows_of(&[1, 2, 3]), 0), &slots()).is_none(), "not a five-row list");
}

#[test]
fn overwrite_is_a_question_over_the_cards_and_nothing_else_is_selectable() {
	let b = built(&view_of(Page::Overwrite, &rows_of(&[1, 2]), 0), &slots()).unwrap();
	let d = b.screen.dialog.as_ref().unwrap();
	assert_eq!(d.lines, ["T4fb3"]);
	assert_eq!(d.buttons.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), ["T524f", "T524b"]);
	assert_eq!(b.buttons, [CIRCLE, TRIANGLE]);
	assert!(b.items.iter().all(|i| i.is_none()) && b.menu.iter().all(|i| i.is_none()));
	let Panel::Cards { cards, .. } = &b.screen.panel else { panic!("cards") };
	assert!(cards.iter().all(|c| !c.selected));
	assert!(b.screen.menu.iter().all(|m| !m.live));
}

#[test]
fn quit_game_is_a_question_over_the_options_tab() {
	let b = built(&view_of(Page::QuitGame, &rows_of(&[0x524f, 0x524b]), 0), &Extra::default()).unwrap();
	let d = b.screen.dialog.as_ref().unwrap();
	assert_eq!(d.lines, ["T4f6d"]);
	assert_eq!(b.buttons, [CIRCLE, TRIANGLE], "Circle = Yes, Triangle = No");
	assert!(matches!(b.screen.panel, Panel::None));
	assert_eq!(b.screen.menu.iter().position(|m| m.selected), Some(5));
	assert_eq!(b.rows(), 0);
}

#[test]
fn the_shell_shows_the_level_and_the_bolts() {
	let v = view_of(Page::Options, &rows_of(&[1, 2]), 0);
	let ex = Extra::default();
	let mut f = facts(&v, &ex, &text_of);
	(f.level, f.bolts, f.lang) = (3, 98_765, 1);
	let b = build(&f).unwrap();
	let sh = b.screen.shell.unwrap();
	assert_eq!((sh.site.as_str(), sh.bolts.as_str(), sh.label.as_str(), sh.site_label.as_str()), (text_of(0x4ecc + 3).as_str(), "98765", "Pause", "Site actuel"));
	f.level = 9999;
	assert_eq!(build(&f).unwrap().screen.shell.unwrap().site, "?");
}

/// The site name is the game's own planet text in the player's language (it was the English table, with PC-made names such as
/// "Veldin (return)"): the page asks the game for 0x4ecc + planet, 0x4ede for Veldin's two levels, whatever the language.
#[test]
fn the_site_name_is_the_games_own_planet_text_in_every_language() {
	let v = view_of(Page::Options, &rows_of(&[1, 2]), 0);
	let ex = Extra::default();
	let asked = std::cell::RefCell::new(Vec::new());
	let text = |id: u32| {
		asked.borrow_mut().push(id);
		format!("L{id:x}")
	};
	for lang in 0..5 {
		for (level, id) in [(0, 0x4ede), (1, 0x4ecd), (3, 0x4ecf), (17, 0x4edd), (18, 0x4ede)] {
			let mut f = facts(&v, &ex, &text);
			(f.level, f.lang) = (level, lang);
			assert_eq!(build(&f).unwrap().screen.shell.unwrap().site, format!("L{id:x}"), "level {level}, language {lang}");
		}
	}
	assert!(asked.borrow().contains(&0x4ede), "the name came from the game's text");
	// A text the game cannot give: the English table, never an empty header.
	let none = |_: u32| String::new();
	let mut f = facts(&v, &ex, &none);
	f.level = 18;
	assert_eq!(build(&f).unwrap().screen.shell.unwrap().site, crate::toc::LEVEL_NAMES[18]);
}

#[test]
fn the_header_words_and_the_longest_tabs_fit_in_all_five_languages() {
	let m = FontMeasure;
	let tabs: [[&str; 7]; 5] = [
		["Weapons", "Gadgets", "Quick Select", "Items", "Help", "Options", "Goodies"],
		["Armes", "Gadgets", "S\u{e9}lection rapide", "Objets", "Aide", "Options", "Extras"],
		["Waffen", "Gadgets", "Schnellauswahl", "Gegenst\u{e4}nde", "Hilfe", "Optionen", "Extras"],
		["Armas", "Gadgets", "Selecci\u{f3}n r\u{e1}pida", "Objetos", "Ayuda", "Opciones", "Extras"],
		["Armi", "Gadget", "Selezione rapida", "Oggetti", "Aiuto", "Opzioni", "Extra"],
	];
	for lang in 0..5 {
		assert!(run_width(&m, FontRole::Condensed, 13.0, PAUSED[lang], 0.1) <= 200.0, "PAUSED {} (lang {lang})", PAUSED[lang]);
		assert!(run_width(&m, FontRole::Condensed, 13.0, SITE[lang], 0.1) <= 200.0, "SITE {} (lang {lang})", SITE[lang]);
		let v = view_of(Page::Root, &rows_of(&ROOT_IDS), 0);
		let text = |id: u32| tabs[lang][ROOT_IDS.iter().position(|&r| r as u32 == id).unwrap_or(0)].to_string();
		let ex = Extra::default();
		let mut f = facts(&v, &ex, &text);
		(f.lang, f.level) = (lang, 4);
		let b = build(&f).unwrap();
		let (layout, _) = page_draw::frame(&b.screen, &m);
		let sh = layout.shell.as_ref().unwrap();
		assert!(sh.tab_px >= 15.0, "tabs shrink to at least 15 px (lang {lang}: {})", sh.tab_px);
		let last = layout.menu.last().unwrap().rect;
		assert!(last.x + last.w <= sh.chip.x, "the tab bar ends before the bolts chip (lang {lang}): {} vs {}", last.x + last.w, sh.chip.x);
	}
}

/// The Options list as the game holds it with Mods on (retail six, Graphics, Controls, Mods, Achievements, Quit to Desktop, Quit Game).
const OPTIONS_12: [u16; 12] = [0x4f1e, 0x4f1f, 0x4f20, 0x4f22, 0x4f23, 0x4f24, 0x7f06, 0x7f13, 0x7f70, 0x7f2b, 0x7f30, 0x4f26];

#[test]
fn quit_to_desktop_is_a_row_of_the_options_list_and_its_question_swaps_the_body() {
	use crate::retail::pcext::quitrow::{ID_QUITD, ID_QUITD_BODY};
	assert_eq!((ID_QUITD, ID_QUITD_BODY), (0x7f30, 0x7f33));
	let b = built(&view_of(Page::Options, &rows_of(&OPTIONS_12), 10), &Extra::default()).unwrap();
	let Panel::Rows { rows, .. } = &b.screen.panel else { panic!("rows") };
	assert_eq!(rows.len(), 12);
	assert_eq!((rows[10].label.as_str(), rows[11].label.as_str()), ("T7f30", "T4f26"), "between Achievements and Quit Game");
	assert!(rows[10].selected && !rows[11].selected);
	assert_eq!(b.rows(), 12);
	assert!(b.items.iter().all(|i| i.is_some()), "every row can be chosen: the click presses Cross on the row, the worker point does the rest");
	// The Quit Game page: retail's question, or Quit to Desktop's while that row opened it.
	let ex = Extra::default();
	let v = view_of(Page::QuitGame, &rows_of(&[0x524f, 0x524b]), 0);
	let mut f = facts(&v, &ex, &text_of);
	let b = build(&f).unwrap();
	assert_eq!(b.screen.dialog.as_ref().unwrap().lines, ["T4f6d"], "retail's own Quit Game row keeps its question");
	f.quit_desktop = true;
	let b = build(&f).unwrap();
	let d = b.screen.dialog.as_ref().unwrap();
	assert_eq!(d.lines, ["T7f33"]);
	assert_eq!(b.buttons, [CIRCLE, TRIANGLE], "Circle = Yes, Triangle = No, as on retail's page");
	assert_eq!(d.buttons.len(), 2);
	assert_eq!(b.screen.menu.iter().position(|m| m.selected), Some(5), "still over the Options tab");
	// The flag changes nothing on the other pages.
	let o = view_of(Page::Options, &rows_of(&OPTIONS_12), 0);
	assert_eq!(build(&facts(&o, &ex, &text_of)).unwrap(), build(&Facts { quit_desktop: true, ..facts(&o, &ex, &text_of) }).unwrap());
}

#[test]
fn the_quit_to_desktop_label_fits_the_twelve_modern_options_rows_in_all_five_languages() {
	use crate::retail::menu_model::view::Body;
	use crate::retail::pcext::{self, quitrow::ID_QUITD};
	use rac_engine::ui::kit::page::ROW_LABEL_PX;
	let m = FontMeasure;
	for lang in 0..5 {
		let label = pcext::texts(lang).find(|(id, _)| *id == ID_QUITD).map(|(_, s)| decode(&s).plain()).expect("the label has a text");
		let mut v = view_of(Page::Options, &rows_of(&OPTIONS_12), 10);
		let Body::Rows { rows, .. } = &mut v.items[0].body else { unreachable!() };
		rows[10].text = decode(&label);
		let b = built(&v, &Extra::default()).unwrap();
		let (layout, _) = page_draw::frame(&b.screen, &m);
		let panel = layout.panel.as_ref().unwrap();
		assert_eq!(panel.items.len(), 12);
		let it = &panel.items[10];
		assert_eq!(it.label.text, label);
		let w = run_width(&m, FontRole::Medium, ROW_LABEL_PX, &label, 0.0);
		assert!(w <= it.label_w, "language {lang}: \"{label}\" is {w} px wide, the row allows {}", it.label_w);
		assert!(panel.rect.y + panel.rect.h <= 660.0, "language {lang}: the 12 rows end above the hint bar ({})", panel.rect.y + panel.rect.h);
		assert!(panel.items.iter().all(|i| i.rect.h >= 30.0), "rows keep a usable height");
	}
}

/// R3-7: every tab's main page belongs to that tab (Quick Select too), so a click on the tab of the page shown stays put.
#[test]
fn every_tab_has_its_own_main_page() {
	assert_eq!(main_page(TAB_QUICK), Some(Page::QuickSelect));
	for tab in 0..7 {
		let p = main_page(tab).expect("a main page");
		assert_eq!(branch(p), Some(tab), "{p:?}");
	}
	assert_eq!(main_page(7), None);
}

/// Review R14 F7: the dev tools' F1 planet list (Debug mode) is drawn in the HUD pass and takes every key: the modern page steps aside
/// while it is open and comes back when it closes (the achievements list no longer does: it is a page of its own, `pause_ach.rs`).
#[test]
fn the_dev_planet_list_steps_the_page_aside() {
	use crate::retail::devtools;
	use winit::keyboard::KeyCode as K;
	/// Debug mode as it was, the list closed, when the test ends (also when an assert fails): Off puts every tool away.
	struct Restore(bool);
	impl Drop for Restore {
		fn drop(&mut self) {
			devtools::set_menu(false, false);
			devtools::set_menu(self.0, false);
		}
	}
	let _g = crate::retail::pcopts::globals_guard();
	let _restore = Restore(devtools::menu());
	devtools::set_menu(true, false);
	let shown = || wanted(&Gate { list_open: list_open(), ..gate() });
	assert!(!devtools::pick_open() && shown(), "no list: the page");
	assert!(devtools::ui_key(K::F1, true, false), "F1 opens the list");
	assert!(devtools::pick_open() && !shown(), "the list is open: the page steps aside");
	assert!(devtools::ui_key(K::Escape, true, false));
	assert!(!devtools::pick_open() && shown(), "closed: the page is back");
}
