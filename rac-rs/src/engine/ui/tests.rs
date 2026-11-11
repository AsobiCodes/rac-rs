//! Tests of the options model: navigation, editing, presets, dependencies, values round trip, texts, scrolling, devices.
use super::input::{Action, Device, GlyphSet, PadSample, UiKey, P_CROSS};
use super::model::{Change, Model};
use super::prim::{GlyphKind, Symbol};
use super::rows::{pick, UiSound, LANGS};
use super::screen;
use super::testkit::{menu, new_ui};
use super::Ui;

// Global row numbers of the fake menu: gfx 0 preset, 1 lighting, 2 shadows, 3 bloom, 4 fov; cam 5 sens, 6 invert; audio 7 music,
// 8 sfx; sys 9 menustyle, 10..20 o1..o11.
const PRESET: usize = 0;
const LIGHTING: usize = 1;
const SHADOWS: usize = 2;
const BLOOM: usize = 3;
const FOV: usize = 4;

fn model() -> Model {
	Model::new(menu(), 0)
}

/// Navigation walks the shown rows: with Lighting Classic, Shadows and Bloom are hidden and skipped (user 2026-11-08: hidden, not
/// dimmed); with Lighting Enhanced every row is visited.
#[test]
fn model_nav_wraps_and_skips_hidden_rows() {
	let mut m = model();
	assert_eq!((m.tab, m.focus), (0, 0));
	m.focus_step(-1);
	assert_eq!(m.focus, FOV, "up from the first row wraps to the last row of the tab");
	m.focus_step(1);
	assert_eq!(m.focus, PRESET);
	let mut seen = vec![m.focus];
	for _ in 0..2 {
		m.focus_step(1);
		seen.push(m.focus);
	}
	assert_eq!(seen, vec![PRESET, LIGHTING, FOV], "hidden rows (shadows, bloom) are skipped");
	m.focus_step(1);
	assert_eq!(m.focus, 0);
	m.focus_page(1, 1);
	assert_eq!(m.focus, LIGHTING);
	m.focus_page(1, 9);
	assert_eq!(m.focus, FOV, "page down clamps to the tab");
	m.focus_edge(false);
	assert_eq!(m.focus, 0);
	assert!(m.is_first() && !m.is_last());
	m.focus_edge(true);
	assert!(m.is_last());
	m.set_value(LIGHTING, 1);
	m.focus_edge(false);
	let mut seen = vec![m.focus];
	for _ in 0..4 {
		m.focus_step(1);
		seen.push(m.focus);
	}
	assert_eq!(seen, vec![0, 1, 2, 3, 4], "every row once Lighting is Enhanced");
	m.focus_page(-1, 3);
	assert_eq!(m.focus, LIGHTING);
	m.focus = BLOOM;
	m.set_value(LIGHTING, 0);
	m.settle_focus();
	assert_eq!(m.focus, LIGHTING, "a focus on a row that became hidden moves to the shown row above it");
}

#[test]
fn tab_switch_focuses_first_row() {
	let mut m = model();
	m.focus_to(3);
	m.scroll = 77.0;
	m.set_tab(2);
	assert_eq!((m.tab, m.focus, m.scroll), (2, 7, 0.0));
	m.tab_step(1);
	assert_eq!((m.tab, m.focus), (3, 9));
	m.tab_step(1);
	assert_eq!((m.tab, m.focus), (0, 0), "tabs wrap");
	m.tab_step(-1);
	assert_eq!(m.tab, 3);
	assert!(m.open_tab("cam"));
	assert_eq!((m.tab, m.focus), (1, 5));
	assert!(!m.open_tab("nope"));
	assert_eq!(m.tab, 1);
	m.focus_to(0);
	assert_eq!(m.tab, 0, "focusing a row of another tab opens that tab");
	assert!(m.take_sounds().iter().all(|s| *s == UiSound::Move));
}

#[test]
fn choice_step_wraps_and_toggle_flips() {
	let mut m = model();
	m.step_row(LIGHTING, 1);
	assert_eq!(m.value(LIGHTING), 1);
	m.step_row(LIGHTING, 1);
	assert_eq!(m.value(LIGHTING), 0, "a choice wraps forward");
	m.step_row(LIGHTING, -1);
	assert_eq!(m.value(LIGHTING), 1, "and backward");
	assert_eq!(m.value(SHADOWS), 3);
	m.step_row(SHADOWS, 1);
	assert_eq!(m.value(SHADOWS), 0);
	assert_eq!(m.value(BLOOM), 1);
	m.focus_to(BLOOM);
	m.confirm();
	assert_eq!(m.value(BLOOM), 0);
	m.confirm();
	assert_eq!(m.value(BLOOM), 1);
	let ch = m.take_changes();
	assert_eq!(ch.first(), Some(&Change { id: "lighting", value: 1 }));
	assert_eq!(ch.len(), 6);
	assert!(m.take_changes().is_empty());
	assert!(m.take_sounds().contains(&UiSound::Select));
}

#[test]
fn slider_clamps_to_step() {
	let mut m = model();
	assert_eq!(m.value(FOV), 90);
	for _ in 0..10 {
		m.step_row(FOV, 1);
	}
	assert_eq!(m.value(FOV), 110, "stops at the end, no wrap");
	m.step_row(FOV, -1);
	assert_eq!(m.value(FOV), 105);
	m.slider_to(FOV, 0.0);
	assert_eq!(m.value(FOV), 60);
	m.slider_to(FOV, 1.0);
	assert_eq!(m.value(FOV), 110);
	m.slider_to(FOV, 0.5);
	assert_eq!(m.value(FOV), 85);
	m.slider_to(FOV, 0.52);
	assert_eq!(m.value(FOV), 85, "snaps to the step");
	m.slider_to(FOV, 0.58);
	assert_eq!(m.value(FOV), 90);
	m.take_changes();
	m.slider_to(FOV, 0.58);
	m.set_value(FOV, 200);
	assert!(m.take_changes().is_empty(), "an unchanged or illegal value is not reported");
	m.focus_to(FOV);
	m.confirm();
	assert_eq!(m.value(FOV), 90, "confirm does nothing on a slider");
	assert_eq!(m.value_text(FOV), "90 deg");
	assert!((m.slider_pos(FOV) - 0.6).abs() < 1e-6);
}

#[test]
fn preset_applies_set_then_reports_name_and_custom_on_any_change() {
	let mut m = model();
	assert_eq!(m.preset_index(), None, "lighting classic + shadows high + bloom on matches no preset");
	assert_eq!(m.value_text(PRESET), "Custom");
	m.apply_preset(2);
	assert_eq!(m.preset_index(), Some(2));
	assert_eq!(m.value_text(PRESET), "High");
	assert_eq!(
		m.take_changes(),
		vec![Change { id: "lighting", value: 1 }, Change { id: "filter", value: 1 }],
		"only rows that differ are reported, then the preset's settings without a row (always)"
	);
	m.set_value(SHADOWS, 2);
	assert_eq!(m.preset_index(), Some(1), "Medium matches now");
	m.set_value(SHADOWS, 1);
	assert_eq!(m.preset_index(), None);
	assert_eq!(m.value_text(PRESET), "Custom");
	m.apply_preset(0);
	assert_eq!((m.value(LIGHTING), m.value(SHADOWS), m.value(BLOOM)), (0, 0, 0));
	assert_eq!(m.value_text(PRESET), "Low");
	m.step_row(PRESET, 1);
	assert_eq!(m.preset_index(), Some(1));
	m.step_row(PRESET, -1);
	assert_eq!(m.preset_index(), Some(0));
	m.step_row(PRESET, -1);
	assert_eq!(m.preset_index(), Some(2), "wraps");
	m.set_value(BLOOM, 0);
	assert_eq!(m.preset_index(), None);
	m.step_row(PRESET, 1);
	assert_eq!(m.preset_index(), Some(2), "from Custom the next press picks the last preset");
}

#[test]
fn reset_row_default_and_preset_high() {
	let mut m = model();
	m.set_value(SHADOWS, 0);
	m.focus_to(SHADOWS);
	m.reset();
	assert_eq!(m.value(SHADOWS), 3);
	m.set_value(FOV, 60);
	m.focus_to(FOV);
	m.reset();
	assert_eq!(m.value(FOV), 90);
	m.apply_preset(0);
	m.focus_to(PRESET);
	m.reset();
	assert_eq!(m.preset_index(), Some(2), "the Preset row resets to High");
	assert_eq!(m.default_text(PRESET), "High");
	assert_eq!(m.default_text(SHADOWS), "High");
}

#[test]
fn needs_row_marks_blocked_exactly_when_lighting_classic() {
	let mut m = model();
	let blocked: Vec<usize> = (0..m.rows.len()).filter(|&i| m.blocked(i)).collect();
	assert_eq!(blocked, vec![SHADOWS, BLOOM]);
	assert!(m.tab_rule().is_some_and(|r| r.row == "lighting"));
	m.set_tab(1);
	assert!(m.tab_rule().is_none(), "no banner on a tab without blocked rows");
	m.set_tab(0);
	m.focus_to(SHADOWS);
	let rule = m.rule(SHADOWS).unwrap();
	m.enhance(rule);
	assert_eq!(m.value(LIGHTING), 1);
	assert_eq!(m.focus, SHADOWS, "the focus stays");
	assert!((0..m.rows.len()).all(|i| !m.blocked(i)));
	assert!(m.tab_rule().is_none());
	m.set_value(LIGHTING, 0);
	assert!(m.blocked(SHADOWS) && m.blocked(BLOOM));
}

#[test]
fn screen_builds_banner_hidden_rows_and_detail() {
	let mut m = model();
	m.focus_to(LIGHTING);
	let (s, map) = screen::build(&m);
	assert!(matches!(s.items[0], super::kit::Item::Banner(_)));
	assert_eq!(map.item_row.len(), s.items.len());
	assert_eq!(map.row(0), None);
	assert_eq!(map.row_item(LIGHTING), s.focus);
	let rows: Vec<Option<usize>> = map.item_row.iter().copied().filter(Option::is_some).collect();
	assert_eq!(rows, vec![Some(PRESET), Some(LIGHTING), Some(FOV)], "Shadows and Bloom are hidden with Lighting Classic");
	assert!(s.items.iter().all(|i| !matches!(i, super::kit::Item::Row(r) if r.dim)), "a shown row is never dimmed");
	assert_eq!(s.detail.as_ref().unwrap().path, "Graphics \u{b7} Look");
	assert!(s.detail.unwrap().boxed.is_none());
	// A focus written straight onto a hidden row (no `settle_focus`): no focused item, the detail panel keeps its box.
	m.focus = SHADOWS;
	let (s, map) = screen::build(&m);
	assert_eq!((map.row_item(SHADOWS), s.focus), (None, None));
	let d = s.detail.unwrap();
	assert_eq!(d.boxed.unwrap().text, "This only shows with Lighting set to Enhanced. Lighting is Classic now.");
	assert_eq!(d.meter.unwrap().level, 2);
	assert_eq!(s.hints.len(), 5);
	assert_eq!(s.notice, None);
	m.set_value(LIGHTING, 1);
	let (s, map) = screen::build(&m);
	assert!(!matches!(s.items[0], super::kit::Item::Banner(_)), "the banner goes with the dependency");
	assert_eq!(map.row_item(SHADOWS), s.focus, "Shadows is shown again");
	assert!(s.detail.unwrap().boxed.is_none());
	m.set_tab(3);
	m.set_value(9, 1);
	assert_eq!(screen::build(&m).0.notice.as_deref(), Some("Classic menus from the next time"));
	m.front = true;
	assert_eq!(screen::build(&m).0.title, "Options");
}

#[test]
fn row_get_set_round_trip() {
	use std::cell::RefCell;
	use std::collections::HashMap;
	let store: RefCell<HashMap<String, i32>> = RefCell::new(HashMap::new());
	store.borrow_mut().insert("lighting".into(), 1);
	store.borrow_mut().insert("fov".into(), 75);
	store.borrow_mut().insert("shadows".into(), 99);
	let get = |id: &str| store.borrow().get(id).copied();
	let mut m = model();
	m.refresh(&get);
	assert_eq!((m.value(LIGHTING), m.value(FOV), m.value(SHADOWS)), (1, 75, 3), "an illegal value from the game is ignored");
	m.now = 1000;
	m.set_value(FOV, 100);
	for c in m.take_changes() {
		store.borrow_mut().insert(c.id.to_string(), c.value);
	}
	assert_eq!(store.borrow()["fov"], 100);
	store.borrow_mut().insert("fov".into(), 80);
	m.now = 1100;
	m.refresh(&get);
	assert_eq!(m.value(FOV), 100, "a row just changed keeps its value for 300 ms");
	m.now = 1400;
	m.refresh(&get);
	assert_eq!(m.value(FOV), 80);
}

#[test]
fn all_five_languages_present_and_nonempty() {
	assert_eq!(menu().validate(), Vec::<String>::new());
	let mut m = model();
	assert_eq!(pick(&menu().tabs[0].sections[0].rows[1].label, 1), "Lumiere");
	assert_eq!(pick(&menu().tabs[0].sections[0].rows[1].label, 99), "Luce", "an unknown language falls back to the last");
	for lang in 0..LANGS {
		m.lang = lang;
		for tab in 0..4 {
			m.set_tab(tab);
			for i in m.cur_rows() {
				m.focus_to(i);
				let (s, _) = screen::build(&m);
				assert!(s.tabs.iter().all(|t| !t.is_empty()) && !s.title.is_empty() && !s.status.is_empty());
				let d = s.detail.unwrap();
				assert!(!d.title.is_empty() && !d.body.is_empty() && !d.path.is_empty());
			}
		}
	}
}

#[test]
fn scroll_keeps_focus_visible() {
	let mut ui = new_ui();
	ui.begin(false, Some("sys"));
	assert_eq!(ui.model.focus, 9);
	for _ in 0..11 {
		ui.act(Action::Down);
		let (l, _) = ui.frame();
		let (_, map) = ui.screen();
		let it = map.row_item(ui.model.focus).unwrap();
		let r = l.items[it].rect();
		assert!(r.y >= l.viewport.y - 0.01 && r.bottom() <= l.viewport.bottom() + 0.01, "row {} at {:?} outside {:?}", ui.model.focus, r, l.viewport);
	}
	let (l, _) = ui.frame();
	assert!(l.max_scroll > 0.0);
	assert_eq!(ui.model.scroll, l.max_scroll, "the last row goes to the very bottom");
	ui.act(Action::Down);
	assert_eq!((ui.model.focus, ui.model.scroll), (9, 0.0), "wrap to the first row goes to the very top");
	ui.act(Action::End);
	assert_eq!(ui.model.scroll, l.max_scroll);
	ui.act(Action::Home);
	assert_eq!(ui.model.scroll, 0.0);
}

#[test]
fn glyph_sets_map_letters() {
	use Symbol::*;
	let g = |s: GlyphSet, y| s.glyph(y);
	assert_eq!(g(GlyphSet::PlayStation, Cross), GlyphKind::Symbol(Cross));
	assert_eq!(g(GlyphSet::Letters, Cross), GlyphKind::Letter('A'));
	assert_eq!(g(GlyphSet::Letters, Circle), GlyphKind::Letter('B'));
	assert_eq!(g(GlyphSet::Letters, Square), GlyphKind::Letter('X'));
	assert_eq!(g(GlyphSet::Letters, Triangle), GlyphKind::Letter('Y'));
	assert_eq!(g(GlyphSet::Switch, Cross), GlyphKind::Letter('B'));
	assert_eq!(g(GlyphSet::Switch, Circle), GlyphKind::Letter('A'));
	assert_eq!(g(GlyphSet::Switch, Square), GlyphKind::Letter('Y'));
	assert_eq!(g(GlyphSet::Switch, Triangle), GlyphKind::Letter('X'));
	assert_eq!([0, 1, 2, 7].map(GlyphSet::from_index), [GlyphSet::PlayStation, GlyphSet::Letters, GlyphSet::Switch, GlyphSet::PlayStation]);
	assert_eq!(GlyphSet::Letters.shoulders(), ("LB", "RB"));
	assert_eq!(GlyphSet::PlayStation.shoulders(), ("L1", "R1"));
}

#[test]
fn device_follows_last_input() {
	let mut ui: Ui = new_ui();
	assert_eq!(ui.screen().0.device, Device::Keyboard);
	ui.pad(PadSample { buttons: P_CROSS, ..PadSample::IDLE }, 0);
	assert_eq!(ui.screen().0.device, Device::Pad);
	ui.pad(PadSample::IDLE, 10);
	ui.key(UiKey::Down, true, 20);
	assert_eq!(ui.screen().0.device, Device::Keyboard);
	ui.pad(PadSample { lx: 0, ..PadSample::IDLE }, 30);
	assert_eq!(ui.screen().0.device, Device::Pad, "the left stick counts as the pad");
	ui.mouse(super::MouseEvent::Move { x: 10.0, y: 10.0 }, 40);
	assert_eq!(ui.screen().0.device, Device::Keyboard, "the device follows the move at once, only the hover waits for the frame");
}
