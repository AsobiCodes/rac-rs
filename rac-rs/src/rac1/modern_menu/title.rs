//! The modern title screens (v0.5 screens stage B): `build` turns what the front end shows (its page, row strings, save slots,
//! selected row) into the engine's `PageScreen` plus what each button, card and row does when chosen. Pure: no statics, no clock;
//! the main-thread host is `title_host.rs`, the facts are gathered by `host/front_modern.rs`. The retail pages (New Game, Load,
//! Options, ...) stay the game's own: choosing a row sends `Select` (or a pad press) to the menu, so retail's code does the work.
use crate::frontend::Page;
use crate::retail::menu_model::view::SlotView;
use crate::retail::mouse_menu::{Item, Kind, MenuSnap, CIRCLE, CROSS, TRIANGLE};
use crate::retail::title_quit::NATIVE_ITEM;
use rac_engine::ui::input::{Device, GlyphSet};
use rac_engine::ui::keycaps::{self, Cap};
use rac_engine::ui::kit::data::{Hint, HintKind};
use rac_engine::ui::kit::page::{Dialog, DialogButton, MenuButton, PageRow, PageScreen, Panel, Stat};
use rac_engine::ui::prim::Symbol;
use rac_engine::ui::T5;

pub const BOLTS: T5 = ["Bolts", "Boulons", "Bolzen", "Tuercas", "Bulloni"];
pub const PLAY_TIME: T5 = ["Play time", "Temps de jeu", "Spielzeit", "Tiempo de juego", "Tempo di gioco"];
pub const SAVED: T5 = ["Saved", "Sauvegard\u{e9}", "Gespeichert", "Guardado", "Salvato"];
pub const SELECT: T5 = ["Select", "Choisir", "W\u{e4}hlen", "Elegir", "Scegli"];
pub const BACK: T5 = ["Back", "Retour", "Zur\u{fc}ck", "Volver", "Indietro"];

/// A message box over the page: its lines (an empty one is a blank line) and its buttons (pad bit, word).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct DialogFacts {
	pub lines: Vec<String>,
	pub buttons: Vec<(u16, String)>,
}

/// Everything `build` reads.
pub struct Facts<'a> {
	pub page: Page,
	/// The page's rows as `Menu::items` lists them (label and value joined by two spaces).
	pub rows: &'a [(String, bool)],
	pub header: String,
	/// New Game, Load Game, Options, and the host's own "Quit to Desktop" when its label is known.
	pub root: Vec<String>,
	pub slots: &'a [Option<SlotView>],
	pub empty: String,
	/// "Continue without saving" (New Game's last row; the Overwrite page lists no rows of its own).
	pub no_save: String,
	/// The selected row in the page's flat list (`front_mirror::sel_of`).
	pub sel: usize,
	/// The (item address, row) behind each flat row (`flatten`).
	pub targets: Vec<(u32, u32)>,
	pub dialog: Option<DialogFacts>,
	/// Sound page: effects and music volume, 0..1024.
	pub sound: Option<[u16; 2]>,
	pub lang: usize,
	pub device: Device,
	pub glyphs: GlyphSet,
}

/// The screen and what its parts do.
#[derive(Clone, Debug, PartialEq)]
pub struct Built {
	pub screen: PageScreen,
	/// Per menu button / panel item: the (item, row) a hover or click selects; None = inert.
	pub menu: Vec<Option<(u32, u32)>>,
	pub items: Vec<Option<(u32, u32)>>,
	/// Per dialog button: the pad bit it presses.
	pub buttons: Vec<u16>,
	/// The root page is a bare menu column (the arrows move it); on every other page the panel's rows are the list.
	pub root: bool,
	pub page: Page,
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

pub fn word(t: &T5, lang: usize) -> &'static str {
	t[lang.min(4)]
}

/// The flat row list of a retail page: the (address, row) of every row of its on-screen items in slot order (the root's three
/// items, New / Load's slot list and the "continue without saving" panel, a list item's rows, a value page's rows).
pub fn flatten(m: &MenuSnap) -> Vec<(u32, u32)> {
	let mut its: Vec<&Item> = m.items.iter().filter(|i| i.rect[2] > 0).collect();
	its.sort_by_key(|i| i.slot);
	let mut out = Vec::new();
	for it in its {
		let n = match &it.kind {
			Kind::Label { rows, .. } => rows.len() as u32,
			Kind::Value { n, .. } => *n,
			_ => 0,
		};
		out.extend((0..n).map(|r| (it.addr, r)));
	}
	out
}

fn symbol(bit: u16) -> Symbol {
	match bit {
		CROSS => Symbol::Cross,
		CIRCLE => Symbol::Circle,
		TRIANGLE => Symbol::Triangle,
		_ => Symbol::Square,
	}
}

pub fn dialog(d: &DialogFacts) -> Dialog {
	let buttons = d.buttons.iter().map(|(bit, text)| DialogButton { symbol: symbol(*bit), key: keycaps::name(if *bit == TRIANGLE { Cap::Esc } else { Cap::Enter }).to_string(), text: text.clone() }).collect();
	Dialog { lines: d.lines.clone(), buttons }
}

/// The root entry a child page belongs to.
fn root_entry(p: Page) -> usize {
	match p {
		Page::NewGame | Page::Overwrite => 0,
		Page::LoadGame => 1,
		_ => 2,
	}
}

fn cards(f: &Facts, new: bool, focus: bool) -> Panel {
	let mut out = Vec::new();
	for (i, s) in f.slots.iter().enumerate() {
		let selected = focus && i == f.sel;
		out.push(match s {
			Some(s) => {
				let st = |l: &T5, v: String| Stat { label: word(l, f.lang).to_string(), value: v };
				PageRow {
					label: format!("{}  {}", i + 1, s.name),
					stats: vec![st(&BOLTS, s.bolts.to_string()), st(&PLAY_TIME, format!("{}:{:02}", s.hours, s.minutes)), st(&SAVED, s.date.clone())],
					selected,
					..Default::default()
				}
			}
			None => PageRow { label: format!("{}  {}", i + 1, f.empty), selected, dim: true, ..Default::default() },
		});
	}
	let extra = new.then(|| PageRow { label: f.no_save.clone(), selected: focus && f.sel == f.slots.len(), ..Default::default() });
	Panel::Cards { caption: f.header.clone(), side: String::new(), cards: out, extra }
}

/// A label and its value from a row string: the value is what follows the last double space.
pub fn split_row(s: &str) -> (String, String) {
	match s.rfind("  ") {
		Some(i) => (s[..i].trim_end().to_string(), s[i + 2..].trim_start().to_string()),
		None => (s.to_string(), String::new()),
	}
}

fn rows_panel(f: &Facts) -> Panel {
	let mut rows: Vec<PageRow> = f
		.rows
		.iter()
		.enumerate()
		.map(|(i, (text, enabled))| {
			let (label, value) = split_row(text);
			PageRow { label, value, selected: i == f.sel, dim: !enabled, ..Default::default() }
		})
		.collect();
	if let (Page::Sound, Some(v)) = (f.page, f.sound) {
		for (r, raw) in rows.iter_mut().zip(v) {
			r.value = format!("{}%", (raw as u32 * 100 + 512) / 1024);
		}
	}
	Panel::Rows { caption: f.header.clone(), rows }
}

fn targets(f: &Facts, n: usize) -> Vec<Option<(u32, u32)>> {
	(0..n).map(|i| f.targets.get(i).copied()).collect()
}

/// The (item, row) behind each root button: the game's three items, then the host's own entry as the sentinel target
/// `(NATIVE_ITEM, 0)` (it is no retail item: the host answers it itself). Inert (all None) off the root page or under a dialog.
fn root_targets(f: &Facts, live: bool) -> Vec<Option<(u32, u32)>> {
	if !live {
		return vec![None; f.root.len()];
	}
	let mut out = targets(f, 3);
	if f.root.len() > 3 {
		out.push(Some((NATIVE_ITEM, 0)));
	}
	out
}

pub fn build(f: &Facts) -> Built {
	let root = f.page == Page::Root;
	let entry = root_entry(f.page);
	let menu: Vec<MenuButton> = f.root.iter().enumerate().map(|(i, t)| MenuButton { text: t.clone(), selected: if root { i == f.sel } else { i == entry }, live: root }).collect();
	let panel = match f.page {
		Page::Root => Panel::None,
		Page::NewGame => cards(f, true, true),
		Page::Overwrite => cards(f, true, false),
		Page::LoadGame => cards(f, false, true),
		_ => rows_panel(f),
	};
	let n_items = match &panel {
		Panel::Cards { cards, extra, .. } => cards.len() + extra.iter().count(),
		Panel::Rows { rows, .. } => rows.len(),
		Panel::None => 0,
	};
	let hints = vec![Hint { kind: HintKind::Change, text: word(&SELECT, f.lang).to_string() }, Hint { kind: HintKind::Back, text: word(&BACK, f.lang).to_string() }];
	let dlg = f.dialog.as_ref();
	let screen = PageScreen { menu, panel, dialog: dlg.map(dialog), hints, device: f.device, glyphs: f.glyphs, ..Default::default() };
	let live = dlg.is_none();
	Built {
		menu: root_targets(f, root && live),
		items: if live { targets(f, n_items) } else { vec![None; n_items] },
		buttons: dlg.map(|d| d.buttons.iter().map(|b| b.0).collect()).unwrap_or_default(),
		screen,
		root,
		page: f.page,
	}
}

/// The row to show as selected: the player's own focus while it is fresh (retail's cursor follows a frame or two later), else
/// retail's.
pub fn shown(local: Option<(usize, u64)>, retail: usize, now_ms: u64) -> usize {
	match local {
		Some((i, at)) if now_ms.saturating_sub(at) < GRACE_MS => i,
		_ => retail,
	}
}

/// How long a local focus wins over retail's cursor (like `Model::refresh`'s touch grace).
pub const GRACE_MS: u64 = 300;

#[cfg(test)]
mod tests {
	use super::*;
	use crate::retail::menu_model::text::decode;
	use crate::retail::pcext::{self, quitrow::ID_QUITD};
	use rac_engine::ui::font::FontMeasure;
	use rac_engine::ui::kit::page::{MENU_PX, MENU_TRACK, MENU_W};
	use rac_engine::ui::kit::page_draw;
	use rac_engine::ui::kit::text::{run_width, upper};
	use rac_engine::ui::prim::FontRole;

	fn slot(name: &'static str) -> Option<SlotView> {
		Some(SlotView { level: 1, name, bolts: 1234, hours: 1, minutes: 5, date: "01/02/03".to_string() })
	}

	fn facts<'a>(page: Page, rows: &'a [(String, bool)], slots: &'a [Option<SlotView>], sel: usize) -> Facts<'a> {
		Facts {
			page,
			rows,
			header: "Select file".into(),
			root: vec!["New Game".into(), "Load Game".into(), "Options".into()],
			slots,
			empty: "Empty".into(),
			no_save: "Continue without saving".into(),
			sel,
			targets: (0..6u32).map(|i| (0x100 + i, 0)).collect(),
			dialog: None,
			sound: None,
			lang: 0,
			device: Device::Keyboard,
			glyphs: GlyphSet::PlayStation,
		}
	}

	#[test]
	fn root_is_a_live_column_without_a_panel() {
		let b = build(&facts(Page::Root, &[], &[], 1));
		assert!(b.root && b.screen.panel == Panel::None);
		assert_eq!(b.screen.menu.len(), 3);
		assert!(b.screen.menu.iter().all(|m| m.live) && b.screen.menu[1].selected && !b.screen.menu[0].selected);
		assert_eq!((b.rows(), b.target(1)), (3, Some((0x101, 0))));
	}

	/// The facts with the host's own fourth root entry.
	fn with_quit(mut f: Facts) -> Facts {
		f.root.push("Quit to Desktop".into());
		f
	}

	#[test]
	fn the_root_gets_the_hosts_quit_entry_as_a_fourth_button_with_a_sentinel_target() {
		let b = build(&with_quit(facts(Page::Root, &[], &[], 3)));
		assert_eq!(b.screen.menu.len(), 4);
		assert!(b.screen.menu.iter().all(|m| m.live));
		assert!(b.screen.menu[3].selected && !b.screen.menu[2].selected, "the entry is the focused row");
		assert_eq!(b.screen.menu[3].text, "Quit to Desktop");
		assert_eq!((b.rows(), b.target(2), b.target(3)), (4, Some((0x102, 0)), Some((NATIVE_ITEM, 0))), "the retail rows keep their targets, the entry has the sentinel");
		assert_eq!(b.target(4), None);
		let three = build(&facts(Page::Root, &[], &[], 0));
		assert_eq!((three.screen.menu.len(), three.rows(), three.target(3)), (3, 3, None), "no label: the three retail buttons as before");
	}

	#[test]
	fn the_quit_entry_is_inert_off_the_root_page_and_under_a_dialog() {
		let slots = [slot("Veldin"), None, None, None, None];
		let n = build(&with_quit(facts(Page::NewGame, &[], &slots, 0)));
		assert_eq!(n.screen.menu.len(), 4);
		assert!(n.screen.menu.iter().all(|m| !m.live) && !n.screen.menu[3].selected, "a child page keeps its own entry selected, the others inert");
		assert_eq!(n.menu, vec![None; 4]);
		let mut f = with_quit(facts(Page::Root, &[], &[], 3));
		f.dialog = Some(DialogFacts { lines: vec!["Quit?".into()], buttons: vec![(CIRCLE, "Yes".into()), (TRIANGLE, "No".into())] });
		let b = build(&f);
		assert_eq!((b.menu.clone(), b.rows(), b.buttons.clone()), (vec![None; 4], 4, vec![CIRCLE, TRIANGLE]), "under the question no row can be chosen");
		assert!(b.screen.dialog.is_some());
	}

	#[test]
	fn the_quit_label_fits_the_menu_button_in_all_five_languages_and_four_buttons_fit_the_screen() {
		let m = FontMeasure;
		for lang in 0..5 {
			let label = pcext::texts(lang).find(|(id, _)| *id == ID_QUITD).map(|(_, s)| decode(&s).plain()).expect("the label has a text");
			let w = run_width(&m, FontRole::Condensed, MENU_PX, &upper(&label), MENU_TRACK);
			assert!(w <= MENU_W - 38.0 - 16.0, "language {lang}: \"{label}\" is {w} px wide, the button's text allows {}", MENU_W - 38.0 - 16.0);
		}
		let b = build(&with_quit(facts(Page::Root, &[], &[], 0)));
		let (layout, _) = page_draw::frame(&b.screen, &m);
		assert_eq!(layout.menu.len(), 4);
		let r = layout.menu[3].rect;
		assert!(r.y + r.h <= 660.0, "the fourth button ends above the hint bar ({})", r.y + r.h);
	}

	#[test]
	fn new_game_lists_five_cards_and_the_continue_row() {
		let slots = [slot("Veldin"), None, None, None, None];
		let b = build(&facts(Page::NewGame, &[], &slots, 5));
		let Panel::Cards { cards, extra, .. } = &b.screen.panel else { panic!("cards panel") };
		assert_eq!(cards.len(), 5);
		assert_eq!(cards[0].stats[1].value, "1:05");
		assert!(cards[1].dim && cards[1].stats.is_empty(), "an empty slot is a dimmed card");
		assert!(extra.as_ref().is_some_and(|e| e.selected && e.label == "Continue without saving"));
		assert!(!b.root && b.rows() == 6 && b.target(5) == Some((0x105, 0)));
		assert!(b.screen.menu[0].selected && b.screen.menu.iter().all(|m| !m.live), "the entry that led here stays, inert");
		let load = build(&facts(Page::LoadGame, &[], &slots, 0));
		assert!(matches!(&load.screen.panel, Panel::Cards { extra: None, .. }) && load.screen.menu[1].selected);
	}

	#[test]
	fn overwrite_shows_the_dialog_and_no_focus() {
		let slots = [slot("Veldin"), None, None, None, None];
		let mut f = facts(Page::Overwrite, &[], &slots, 0);
		f.dialog = Some(DialogFacts { lines: vec!["Overwrite?".into()], buttons: vec![(CIRCLE, "Yes".into()), (TRIANGLE, "No".into())] });
		let b = build(&f);
		assert_eq!(b.buttons, vec![CIRCLE, TRIANGLE]);
		assert!(b.items.iter().all(|i| i.is_none()), "the cards behind the dialog are inert");
		let Panel::Cards { cards, .. } = &b.screen.panel else { panic!("cards panel") };
		assert!(cards.iter().all(|c| !c.selected));
		assert_eq!(b.screen.dialog.as_ref().map(|d| d.buttons[0].symbol), Some(Symbol::Circle));
	}

	#[test]
	fn sound_rows_show_percent_and_rows_split_at_the_last_double_space() {
		let rows = [("Sound Effects Volume".to_string(), true), ("Music Volume".to_string(), true), ("Sound Mode  Stereo".to_string(), true)];
		let mut f = facts(Page::Sound, &rows, &[], 0);
		f.sound = Some([512, 1024]);
		let Panel::Rows { rows, .. } = build(&f).screen.panel else { panic!("rows panel") };
		assert_eq!((rows[0].value.as_str(), rows[1].value.as_str(), rows[2].value.as_str()), ("50%", "100%", "Stereo"));
		assert_eq!(split_row("Video Mode  60 Hz"), ("Video Mode".to_string(), "60 Hz".to_string()));
		assert_eq!(split_row("Language"), ("Language".to_string(), String::new()));
	}

	#[test]
	fn own_focus_wins_for_a_moment_then_retail_does() {
		assert_eq!(shown(Some((3, 1000)), 1, 1000), 3, "same frame: no lag");
		assert_eq!(shown(Some((3, 1000)), 1, 1000 + GRACE_MS - 1), 3);
		assert_eq!(shown(Some((3, 1000)), 1, 1000 + GRACE_MS), 1);
		assert_eq!(shown(None, 2, 5), 2);
	}

	#[test]
	fn the_new_words_fit_their_boxes_in_all_five_languages() {
		let m = FontMeasure;
		for lang in 0..5 {
			for t in [&BOLTS, &PLAY_TIME, &SAVED] {
				assert!(run_width(&m, FontRole::Regular, 12.0, t[lang], 0.0) <= 110.0, "stat label {} (lang {lang})", t[lang]);
			}
			for t in [&SELECT, &BACK] {
				assert!(run_width(&m, FontRole::Medium, 15.0, t[lang], 0.0) <= 120.0, "hint {} (lang {lang})", t[lang]);
			}
		}
	}
}
