//! The modern pause menu (v0.5 screens stage C): `build` turns the pause menu on screen (`menu_model::view::PageView` plus the worker's
//! `Extra` facts) into the engine's `PageScreen` with the pause shell (tab bar, site name, bolts chip) and says what each tab, row and
//! dialog button does when chosen. Pure: no statics, no clock; the main-thread host is `pause_host.rs`, the facts are gathered by
//! `host/handler/frame_pause.rs`. Only the pages of `in_set` are drawn here; Camera, the help and goodies sub-pages, the maps and every
//! message box stay the game's own classic picture. The icon-grid pages (Weapons, Gadgets, Items) are `pause_grid.rs`, Quick Select (ring and
//! two grids) is `pause_quick.rs`. Choosing a row sends `Select` (or a pad
//! press) to the menu, so retail's code does the work; this file never changes game state.
use super::pause_grid::{build_grid, GridBuilt};
use super::pause_pc::{footer, pc_parts};
use super::title::{dialog as dialog_of, shown, word, DialogFacts, BACK, BOLTS, PLAY_TIME, SAVED, SELECT};
use crate::retail::menu_model::extra::{Extra, TEXT_MONO, TEXT_STEREO, VOLUME_MAX};
use crate::retail::menu_model::pages::Page;
use crate::retail::menu_model::view::{slot_views_of, Body, ItemView, PageView, RowView};
use crate::retail::mouse_menu::{CIRCLE, TRIANGLE};
use rac_engine::ui::font::FontMeasure;
use rac_engine::ui::input::{Device, GlyphSet};
use rac_engine::ui::kit::data::{Hint, HintKind};
use rac_engine::ui::kit::page::{MenuButton, PageRow, PageScreen, Panel, Stat};
use rac_engine::ui::kit::shell::Shell;
use rac_engine::ui::T5;

// Not retail text: the header words of the pause shell (the mockup's "PAUSED"; the label over the planet name, which the mockup's
// sample planet called "LOGGING SITE": every planet's is its current site, R3-2). Native-speaker review wanted.
pub const PAUSED: T5 = ["Paused", "Pause", "Pause", "Pausa", "Pausa"];
pub const SITE: T5 = ["Current site", "Site actuel", "Aktueller Ort", "Sitio actual", "Sito attuale"];
/// [PC] The Map tab, the last tab (v0.5 build 12; user 2026-11-04 "also we dont have a map tab"): retail's root has no Map entry (the planet
/// map opens with Select in gameplay), so choosing it closes the pause menu and opens the map (`pause_host::open_map`, `retail::map_open`).
/// Not retail text. Native-speaker review wanted.
pub const MAP_TAB: T5 = ["Map", "Carte", "Karte", "Mapa", "Mappa"];

/// The root's entries as tabs, left to right: Weapons, Gadgets, Quick Select, Items, Help, Options; then Goodies once the game lists it.
pub const TAB_IDS: [u32; 6] = [0x4ee2, 0x4ee3, 0x4ee4, 0x4ee5, 0x4ee6, 0x4ee7];
pub const TAB_GOODIES: u32 = 0x4fc2;
const TAB_WEAPONS: usize = 0;
const TAB_GADGETS: usize = 1;
pub const TAB_QUICK: usize = 2;
const TAB_ITEMS: usize = 3;
const TAB_HELP: usize = 4;
const TAB_OPTIONS: usize = 5;
const TAB_GOODIES_AT: usize = 6;
/// Texts of the pages [C: decomp/notes/ingame_pause_menu.md]: page titles, the Sound rows, the two message boxes, empty slot, Yes / No.
const T_HELPDESK: u32 = 0x4f40;
const T_SUBTITLES: u32 = 0x4f6f;
const T_SOUND: u32 = 0x4f43;
const T_SAVE: u32 = 0x4f29;
const T_LOAD: u32 = 0x4f28;
const T_SOUND_ROWS: [u32; 3] = [0x5212, 0x5213, 0x5214];
const T_QUIT: u32 = 0x4f6d;
/// The question when the Quit Game page was opened by the Options row "Quit to Desktop" (`Facts::quit_desktop`).
const T_QUIT_DESKTOP: u32 = crate::retail::pcext::quitrow::ID_QUITD_BODY;
const T_OVERWRITE: u32 = 0x4fb3;
const T_EMPTY: u32 = 0x5217;
const T_YES: u32 = 0x524f;
const T_NO: u32 = 0x524b;

/// The pages drawn modern. Everything else (Camera, help and goodies sub-pages, maps, mode-4 boxes, busy pages) keeps the classic
/// picture.
pub fn in_set(p: Page) -> bool {
	matches!(p, Page::Root | Page::Weapons | Page::Gadgets | Page::QuickSelect | Page::Items | Page::Help | Page::Options | Page::Goodies | Page::HelpDesk | Page::Subtitles | Page::Sound | Page::Save | Page::Load | Page::Overwrite | Page::QuitGame | Page::PcControls | Page::PcMods)
}

/// The tab a page belongs to (index into the tab list), None for the root and the pages outside the set.
pub(super) fn branch(p: Page) -> Option<usize> {
	Some(match p {
		Page::Weapons => TAB_WEAPONS,
		Page::Gadgets => TAB_GADGETS,
		Page::QuickSelect => TAB_QUICK,
		Page::Items => TAB_ITEMS,
		Page::Help => TAB_HELP,
		Page::Options | Page::HelpDesk | Page::Subtitles | Page::Sound | Page::Save | Page::Load | Page::Overwrite | Page::QuitGame | Page::PcControls | Page::PcMods => TAB_OPTIONS,
		Page::Goodies => TAB_GOODIES_AT,
		_ => return None,
	})
}

/// The main page of tab `tab` (where the root's row of that tab leads), None for a tab past the list.
pub(super) fn main_page(tab: usize) -> Option<Page> {
	Some(match tab {
		TAB_WEAPONS => Page::Weapons,
		TAB_GADGETS => Page::Gadgets,
		TAB_QUICK => Page::QuickSelect, // its own tab clicked on it stays put (R3-7)
		TAB_ITEMS => Page::Items,
		TAB_HELP => Page::Help,
		TAB_OPTIONS => Page::Options,
		TAB_GOODIES_AT => Page::Goodies,
		_ => return None,
	})
}

/// What decides whether the modern pause menu is the picture this display frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gate {
	/// The Modern menu style is chosen.
	pub modern: bool,
	/// The modern Options page (or another engine page) is up over the game.
	pub ui_open: bool,
	/// The dev tools' F1 planet list (Debug mode, `devtools::pick_open`) is open: it is drawn in the HUD pass and owns the input, so the
	/// page steps aside while it is up. The achievements list is NOT such a list in the Modern style: it is a page of the modern menu
	/// (`pause_ach`, Facts::ach) and the HUD list only draws while the page is not hosting it (`achievements::hosted`).
	pub list_open: bool,
	/// The UI pipes were refused (the classic menu is the fallback).
	pub refused: bool,
	/// The front end (title) runs.
	pub front: bool,
	pub paused: bool,
	/// The in-level page on screen, or the one flying in (`Retail::level_menu`; None = no page menu).
	pub page: Option<Page>,
}

/// The one rule: is the modern pause menu the picture now? The pages outside `in_set` are not drawn. A page flying in is drawn at once
/// (the root coming up from the game too), so the retail picture never shows for the 12 fields of a page change; a closing menu has
/// no page and is kept hidden by the host (`frame_pause`).
pub fn wanted(g: &Gate) -> bool {
	g.modern && !g.ui_open && !g.list_open && !g.refused && !g.front && g.paused && g.page.is_some_and(in_set)
}

/// `Gate::list_open`: a list drawn in the HUD pass that owns the input is open (the dev tools' planet list).
pub fn list_open() -> bool {
	crate::retail::devtools::pick_open()
}

/// Everything `build` reads.
pub struct Facts<'a> {
	pub view: &'a PageView,
	pub extra: &'a Extra,
	/// A retail text id as page text in the player's language (buttons as inline chips, `btn_text`; trimmed; empty = unknown).
	pub text: &'a dyn Fn(u32) -> String,
	/// The root listed the Goodies entry in this pause (the host remembers it for the child pages).
	pub goodies: bool,
	pub level: u32,
	pub bolts: u32,
	/// The player's own focus: (flat row, ms) and the clock, so the page reacts in the very frame of the key (`title::shown`).
	pub local: Option<(usize, u64)>,
	pub now: u64,
	pub lang: usize,
	pub device: Device,
	pub glyphs: GlyphSet,
	/// The Quit Game page belongs to the Quit to Desktop row (`pcext::quitrow::pending`): its question is "Quit to the desktop?".
	pub quit_desktop: bool,
	/// The achievements list is open (F6, or the Options list's Achievements row): the page shows it (`pause_ach`).
	pub ach: Option<&'a super::pause_ach::AchFacts>,
}

/// The screen and what its parts do.
#[derive(Clone, Debug, PartialEq)]
pub struct Built {
	pub screen: PageScreen,
	/// Per tab: the (item, row) a hover or click selects; None = inert.
	pub menu: Vec<Option<(u32, u32)>>,
	/// Per panel row / card: the same.
	pub items: Vec<Option<(u32, u32)>>,
	/// Per dialog button: the pad bit it presses.
	pub buttons: Vec<u16>,
	/// The tab bar is the list the arrows walk (the root page); on every other page the panel's rows are.
	pub root: bool,
	pub page: Page,
	/// Retail's selected row of the list now (before the player's own focus).
	pub sel: usize,
	/// The tab list has the Goodies entry.
	pub goodies: bool,
	/// The icon-grid pages (Weapons, Gadgets, Items): the grid panel, the preview crop and the cells; `items` is then the flat cell list.
	pub grid: Option<GridBuilt>,
	/// The achievements page (`pause_ach`): the cards' window over the list; `items` is then all inert.
	pub ach: Option<super::pause_ach::AchBuilt>,
	/// The Map tab's index in the tab bar (the last tab; `menu` has no target for it: the host opens the map itself).
	pub map_tab: usize,
	/// The modern Controls table (`pause_keys`, the PcControls page): it replaces the panel; `items` is then all inert.
	pub keys: Option<super::pause_keys::KeysBuilt>,
}

impl Built {
	/// How many rows the arrows walk.
	pub fn rows(&self) -> usize {
		if self.root { self.menu.len() } else { self.items.len() }
	}

	/// The (item, row) of flat row `i`.
	pub fn target(&self, i: usize) -> Option<(u32, u32)> {
		if self.root { self.menu.get(i).copied().flatten() } else { self.items.get(i).copied().flatten() }
	}
}

/// A list on the page: a label item's rows or a value item's rows.
pub(super) struct List<'a> {
	pub(super) addr: u32,
	pub(super) n: usize,
	pub(super) cursor: usize,
	pub(super) rows: &'a [RowView],
}

fn list_of(it: &ItemView) -> Option<List<'_>> {
	match &it.body {
		Body::Rows { rows, cursor } if !rows.is_empty() => Some(List { addr: it.addr, n: rows.len(), cursor: (*cursor).min(rows.len() - 1), rows }),
		Body::Values { n, cursor } if *n > 0 => Some(List { addr: it.addr, n: *n as usize, cursor: (*cursor as usize).min(*n as usize - 1), rows: &[] }),
		_ => None,
	}
}

/// The page's list: the current item when it is one, else the item with the most rows (the hint lines and the description panel
/// beside a list are no lists).
pub(super) fn list(v: &PageView) -> Option<List<'_>> {
	v.cur.and_then(|i| v.items.get(i)).and_then(list_of).or_else(|| v.items.iter().filter_map(list_of).max_by_key(|l| l.n))
}

/// What the page body is: the panel, what each of its rows selects, a message box over it, retail's selected row.
pub(super) struct Parts {
	pub(super) panel: Panel,
	pub(super) targets: Vec<Option<(u32, u32)>>,
	pub(super) dialog: Option<DialogFacts>,
	pub(super) sel: usize,
	/// The panel's rows take the selection (false = nothing is selected: a message box is over them).
	pub(super) focus: bool,
	/// The icon-grid pages' own panels (`panel` is then none).
	pub(super) grid: Option<GridBuilt>,
}

fn rows_parts(f: &Facts, l: &List, rows: Vec<PageRow>, caption: u32) -> Parts {
	let targets = (0..rows.len()).map(|r| Some((l.addr, r as u32))).collect();
	Parts { panel: Panel::Rows { caption: (f.text)(caption), rows }, targets, dialog: None, sel: l.cursor, focus: true, grid: None }
}

/// Help, Options and Goodies: the game's own list rows.
fn list_parts(f: &Facts, tab: usize) -> Option<Parts> {
	let l = list(f.view).filter(|l| !l.rows.is_empty())?;
	let rows: Vec<PageRow> = l.rows.iter().map(|r| PageRow { label: r.text.plain().trim().to_string(), dim: !r.enabled, ..Default::default() }).collect();
	if rows.iter().any(|r| r.label.is_empty()) {
		return None;
	}
	Some(rows_parts(f, &l, rows, tab_id(tab)))
}

/// HelpDesk Options and Subtitles: one row per toggle, its value the on / off word.
fn toggle_parts(f: &Facts, caption: u32) -> Option<Parts> {
	let l = list(f.view)?;
	let tg = &f.extra.toggles;
	if tg.is_empty() || tg.len() != l.n {
		return None;
	}
	let rows = tg.iter().map(|t| PageRow { label: (f.text)(t.text), value: (f.text)(if t.on { t.on_text } else { t.off_text }), ..Default::default() }).collect();
	Some(rows_parts(f, &l, rows, caption))
}

fn percent(v: u32) -> String {
	format!("{}%", (v * 100 + 512) / 1024)
}

/// Sound Options: two volume bars and the stereo / mono row.
fn sound_parts(f: &Facts) -> Option<Parts> {
	let s = f.extra.sound?;
	let l = list(f.view).filter(|l| l.n == 3)?;
	let bar = |v: u32| Some(v.min(VOLUME_MAX) as f32 / VOLUME_MAX as f32);
	let row = |i: usize, value: String, bar: Option<f32>| PageRow { label: (f.text)(T_SOUND_ROWS[i]), value, bar, ..Default::default() };
	let rows = vec![row(0, percent(s.sfx), bar(s.sfx)), row(1, percent(s.music), bar(s.music)), row(2, (f.text)(if s.stereo { TEXT_STEREO } else { TEXT_MONO }), None)];
	Some(rows_parts(f, &l, rows, T_SOUND))
}

/// Save, Load and (behind its message box) Overwrite: the five slots as cards.
fn card_parts(f: &Facts, focus: bool, caption: u32) -> Option<Parts> {
	if f.extra.slots.len() != 5 {
		return None;
	}
	let (targets, sel) = if focus {
		let l = list(f.view).filter(|l| l.n >= 5)?;
		((0..5).map(|r| Some((l.addr, r))).collect(), l.cursor.min(4))
	} else {
		(vec![None; 5], 0)
	};
	let lang = f.lang;
	let cards = slot_views_of(&f.extra.slots)
		.into_iter()
		.enumerate()
		.map(|(i, s)| match s {
			Some(s) => {
				let st = |l: &T5, v: String| Stat { label: word(l, lang).to_string(), value: v };
				PageRow { label: format!("{}  {}", i + 1, s.name), stats: vec![st(&BOLTS, s.bolts.to_string()), st(&PLAY_TIME, format!("{}:{:02}", s.hours, s.minutes)), st(&SAVED, s.date.clone())], ..Default::default() }
			}
			None => PageRow { label: format!("{}  {}", i + 1, (f.text)(T_EMPTY)), dim: true, ..Default::default() },
		})
		.collect();
	let panel = Panel::Cards { caption: (f.text)(caption), side: String::new(), cards, extra: None };
	Some(Parts { panel, targets, dialog: None, sel, focus, grid: None })
}

fn question(f: &Facts, text: u32) -> DialogFacts {
	DialogFacts { lines: vec![(f.text)(text)], buttons: vec![(CIRCLE, (f.text)(T_YES)), (TRIANGLE, (f.text)(T_NO))] }
}

/// Weapons, Gadgets, Quick Select and Items: the icon grid and its detail panel (Quick Select: the ring and its two grids); the flat cell
/// list is what the arrows walk.
fn grid_parts(f: &Facts) -> Option<Parts> {
	let g = build_grid(f, &FontMeasure)?;
	Some(Parts { panel: Panel::None, targets: g.targets.clone(), dialog: None, sel: g.flat_sel, focus: true, grid: Some(g) })
}

/// Help: the list and its detail panel with retail's picture (`pause_help.rs`).
fn help_parts(f: &Facts) -> Option<Parts> {
	let g = super::pause_help::build(f, &FontMeasure)?;
	Some(Parts { panel: Panel::None, targets: g.targets.clone(), dialog: None, sel: g.flat_sel, focus: true, grid: Some(g) })
}

fn parts(f: &Facts) -> Option<Parts> {
	match f.view.page {
		Page::Root => Some(Parts { panel: Panel::None, targets: Vec::new(), dialog: None, sel: 0, focus: true, grid: None }),
		Page::Weapons | Page::Gadgets | Page::QuickSelect | Page::Items => grid_parts(f),
		Page::Help => help_parts(f).or_else(|| list_parts(f, TAB_HELP)),
		Page::Options => list_parts(f, TAB_OPTIONS),
		Page::Goodies => list_parts(f, TAB_GOODIES_AT),
		Page::HelpDesk => toggle_parts(f, T_HELPDESK),
		Page::Subtitles => toggle_parts(f, T_SUBTITLES),
		Page::Sound => sound_parts(f),
		Page::PcControls | Page::PcMods => pc_parts(f),
		Page::Save => card_parts(f, true, T_SAVE),
		Page::Load => card_parts(f, true, T_LOAD),
		Page::Overwrite => card_parts(f, false, T_SAVE).map(|p| Parts { dialog: Some(question(f, T_OVERWRITE)), ..p }),
		Page::QuitGame => Some(Parts { panel: Panel::None, targets: Vec::new(), dialog: Some(question(f, if f.quit_desktop { T_QUIT_DESKTOP } else { T_QUIT })), sel: 0, focus: false, grid: None }),
		_ => None,
	}
}

pub(super) fn tab_id(tab: usize) -> u32 {
	if tab < TAB_IDS.len() { TAB_IDS[tab] } else { TAB_GOODIES }
}

pub(super) struct Tab {
	pub(super) text: String,
	target: Option<(u32, u32)>,
	live: bool,
}

/// The Map tab (`MAP_TAB`): no root row, live on the root page.
fn map_tab(f: &Facts, live: bool) -> Tab {
	Tab { text: word(&MAP_TAB, f.lang).to_string(), target: None, live }
}

/// The tab bar: on the root page each tab is the root's row of that text (its target; live while the game lets it be chosen) and
/// the cursor's row is the selected one; on a child page the tabs are inert and the page's branch is selected. The Map tab is last.
pub(super) fn tabs(f: &Facts) -> Option<(Vec<Tab>, usize, bool)> {
	let text = |id: u32| (f.text)(id);
	if f.view.page != Page::Root {
		let goodies = f.goodies || branch(f.view.page) == Some(TAB_GOODIES_AT);
		let n = TAB_IDS.len() + usize::from(goodies);
		let mut out: Vec<Tab> = (0..n).map(|i| Tab { text: text(tab_id(i)), target: None, live: false }).collect();
		out.push(map_tab(f, false));
		return Some((out, 0, goodies));
	}
	let all = entries(f.view);
	let goodies = f.goodies || all.iter().any(|e| e.1.text_id as u32 == TAB_GOODIES);
	let mut sel = 0;
	let mut out = Vec::new();
	for i in 0..TAB_IDS.len() + usize::from(goodies) {
		let id = tab_id(i);
		let e = all.iter().find(|e| e.1.text_id as u32 == id);
		if e.is_some_and(|e| e.2) {
			sel = i;
		}
		out.push(Tab { text: text(id), target: e.map(|e| e.0), live: e.is_some_and(|e| e.1.enabled) });
	}
	let found = out.iter().any(|t| t.target.is_some());
	out.push(map_tab(f, true));
	found.then_some((out, sel, goodies))
}

/// Every row of every list item on the page: its target, the row, and whether it is the game's cursor. Retail builds the pause root
/// as one one-row label item per entry [C: RAC_MENU_LOG of page 0x2, 2026-11-02], so the current item's row is the selected one; a
/// page with one list reads the same way.
fn entries(v: &PageView) -> Vec<((u32, u32), &RowView, bool)> {
	let mut out = Vec::new();
	for (k, it) in v.items.iter().enumerate() {
		if let Body::Rows { rows, cursor } = &it.body {
			for (r, row) in rows.iter().enumerate() {
				out.push(((it.addr, r as u32), row, v.cur.is_none_or(|c| c == k) && r == *cursor));
			}
		}
	}
	out
}

/// Marks the selected row of the panel.
fn mark(panel: &mut Panel, sel: usize, focus: bool) {
	let rows = match panel {
		Panel::Rows { rows, .. } => rows,
		Panel::Cards { cards, .. } => cards,
		Panel::None => return,
	};
	for (i, r) in rows.iter_mut().enumerate() {
		r.selected = focus && i == sel;
	}
}

/// The place the player is at, in the player's language: the game's own planet name text. Only a text the game cannot give (empty)
/// falls back to the English table, and a level the table does not know shows "?".
fn site_name(level: u32, text: &dyn Fn(u32) -> String) -> String {
	let own = super::ship_marks::name_text(level).map(text).filter(|s| !s.is_empty());
	own.unwrap_or_else(|| crate::toc::LEVEL_NAMES.get(level as usize).copied().unwrap_or("?").to_string())
}

/// The pause shell's header on `level` with `bolts` (the modern map page wears it too, `map.rs`).
pub fn shell_for(level: u32, bolts: u32, lang: usize, text: &dyn Fn(u32) -> String) -> Shell {
	Shell {
		label: word(&PAUSED, lang).to_string(),
		site_label: word(&SITE, lang).to_string(),
		site: site_name(level, text),
		bolts: bolts.to_string(),
	}
}

pub(super) fn header(f: &Facts) -> Shell {
	shell_for(f.level, f.bolts, f.lang, f.text)
}

/// The tab bar's words as a child page shows them (the Goodies tab once the root listed it), the Map tab last: the modern map page's
/// tab bar.
pub fn tab_words(text: &dyn Fn(u32) -> String, lang: usize, goodies: bool) -> Vec<String> {
	let n = TAB_IDS.len() + usize::from(goodies);
	let mut out: Vec<String> = (0..n).map(|i| text(tab_id(i))).collect();
	out.push(word(&MAP_TAB, lang).to_string());
	out
}

/// The modern pause menu of the page in `f.view`; None when the data does not look right (the page then stays classic).
pub fn build(f: &Facts) -> Option<Built> {
	let page = f.view.page;
	if !in_set(page) {
		return None;
	}
	if let Some(a) = f.ach {
		return Some(super::pause_ach::build(f, a));
	}
	let (tabs, root_sel, goodies) = tabs(f)?;
	let mut parts = parts(f)?;
	let keys = (page == Page::PcControls).then(|| super::pause_keys::build(f));
	if keys.is_some() {
		(parts.panel, parts.targets) = (Panel::None, Vec::new());
	}
	let root = page == Page::Root;
	let retail = if root { root_sel } else { parts.sel };
	let sel = shown(f.local, retail, f.now);
	let live = parts.dialog.is_none();
	mark(&mut parts.panel, sel, parts.focus && live);
	let branch = branch(page);
	let menu = tabs.iter().enumerate().map(|(i, t)| MenuButton { text: t.text.clone(), selected: if root { i == sel } else { Some(i) == branch }, live: root && live && t.live }).collect();
	let hints = vec![Hint { kind: HintKind::Change, text: word(&SELECT, f.lang).to_string() }, Hint { kind: HintKind::Back, text: word(&BACK, f.lang).to_string() }];
	let screen = PageScreen { menu, panel: parts.panel, dialog: parts.dialog.as_ref().map(dialog_of), hints, device: f.device, glyphs: f.glyphs, shell: Some(header(f)), footer: footer(f), ..Default::default() };
	let n_items = parts.targets.len();
	Some(Built {
		menu: tabs.iter().map(|t| t.target.filter(|_| root && live && t.live)).collect(),
		items: if live { parts.targets } else { vec![None; n_items] },
		buttons: parts.dialog.as_ref().map(|d| d.buttons.iter().map(|b| b.0).collect()).unwrap_or_default(),
		screen,
		root,
		page,
		sel: retail,
		goodies,
		grid: parts.grid,
		ach: None,
		map_tab: tabs.len() - 1,
		keys,
	})
}
