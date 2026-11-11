//! Tests of the real R&C1 rows table (not the engine's demo table): the table is consistent, every character of every text is in the
//! fonts, every drawn text fits its box in all five languages, and every row starts at the option default.
use super::menu;
use rac_engine::ui::{fit, font::FontMeasure, RowKind};

/// `UiMenu::validate`, the font coverage and the box fit of the drawn texts, and the rows' `def` against what a fresh RAM publishes
/// from the default options (`ui_bridge::defaults_for_test`).
#[test]
fn rows_validate_fit_and_defaults() {
	let m = menu();
	assert_eq!(m.validate(), Vec::<String>::new(), "the table is consistent");
	assert_eq!(fit::missing_chars(m), Vec::<(String, char)>::new(), "every character is in the fonts");
	assert_eq!(fit::overflows(m, &|| Box::new(FontMeasure)), Vec::<String>::new(), "every text fits its box");
	assert_eq!(fit::detail_cuts(m, &|| Box::new(FontMeasure)), Vec::<String>::new(), "every detail text fits above its meter / footer");
	for id in ["graphics", "camera", "display", "game"] {
		assert!(m.tabs.iter().any(|t| t.id == id), "tab {id}");
	}
	let _g = crate::retail::pcopts::globals_guard();
	let published = crate::retail::pcext::ui_bridge::defaults_for_test();
	let mut bad = Vec::new();
	for (_, _, r) in m.rows().filter(|(_, _, r)| !matches!(r.kind, RowKind::Preset)) {
		match published.iter().find(|(k, _)| *k == r.id) {
			None => bad.push(format!("{}: the bridge does not publish it", r.id)),
			Some((_, v)) if *v != r.def => bad.push(format!("{}: table default {}, option default {v}", r.id, r.def)),
			_ => {}
		}
	}
	assert!(bad.is_empty(), "{bad:#?}");
}

/// 2026-11-08 (user: "too many options"): no MSAA, DLSS preset, Waves or Underwater look row; Supporter drawings in the Game tab's
/// Extras; a row whose dependency is unmet is hidden (engine `Model::cur_shown`), and the Lighting banner still says why.
#[test]
fn trimmed_rows_and_hidden_dependents() {
	use rac_engine::ui::model::Model;
	let mut m = Model::new(menu(), 0);
	for id in ["scene_msaa", "dlss_preset", "waves", "underwater"] {
		assert!(m.row_index(id).is_none(), "{id} has no row");
	}
	let sup = m.row_index("supporters").unwrap();
	assert_eq!(m.menu.tabs[m.rows[sup].tab].id, "game");
	let [light, water, up, q, fg] = ["lighting", "water", "upscaler", "upscale_q", "framegen"].map(|id| m.row_index(id).unwrap());
	m.set_tab(m.menu.tabs.iter().position(|t| t.id == "graphics").unwrap());
	m.set_value(light, 1);
	m.set_value(up, 0);
	let shown = m.cur_shown();
	assert!(shown.contains(&up) && !shown.contains(&q) && !shown.contains(&fg), "Upscaler Off hides its rows");
	m.set_value(up, rac_engine::viewer::upscale_choice::FFX_VALUE);
	assert!(m.cur_shown().contains(&q));
	m.set_value(light, 0);
	assert!(!m.cur_shown().contains(&water) && m.cur_shown().contains(&light), "Lighting Classic hides the effect rows");
	assert_eq!(m.tab_rule().map(|r| r.row), Some("lighting"), "the banner names Lighting");
	m.focus = water;
	m.settle_focus();
	assert!(m.cur_shown().contains(&m.focus) && m.focus < water, "a hidden focus moves to the shown row above it");
}

