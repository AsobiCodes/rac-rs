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

/// Waves and Underwater look dim at Water Classic (their box offers Water High, no banner) and with Lighting Classic (the Lighting
/// rule, the root of the chain); the Water row itself only with Lighting. UI only: the model's values, nothing else.
#[test]
fn water_rows_dim_with_water_classic_and_with_lighting_classic() {
	use rac_engine::ui::model::Model;
	let mut m = Model::new(menu(), 0);
	let [light, water, waves, under] = ["lighting", "water", "waves", "underwater"].map(|id| m.row_index(id).unwrap());
	m.set_tab(m.menu.tabs.iter().position(|t| t.id == "graphics").unwrap());
	m.set_value(light, 1);
	m.set_value(water, 3);
	assert!(![water, waves, under].iter().any(|&i| m.blocked(i)));
	m.set_value(water, 0);
	assert!(m.blocked(waves) && m.blocked(under) && !m.blocked(water));
	assert_eq!(m.rule(under).map(|r| r.row), Some("water"));
	assert!(m.tab_rule().is_none(), "Water Classic dims its rows without a banner");
	m.set_value(light, 0);
	assert!(m.blocked(waves) && m.blocked(under) && m.blocked(water));
	assert_eq!(m.rule(waves).map(|r| r.row), Some("lighting"), "the root of the chain first");
	m.set_value(light, 1);
	let rule = m.rule(under).unwrap();
	m.enhance(rule);
	assert_eq!(m.value(water), 3);
	assert!(!m.blocked(under) && !m.blocked(waves));
}
