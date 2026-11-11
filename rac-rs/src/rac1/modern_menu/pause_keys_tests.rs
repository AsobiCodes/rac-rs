//! Tests of the modern Controls table (`pause_keys.rs`): pure, on the default bindings (no live state).
use super::pause_keys::*;
use crate::kbm_actions::TABLE;
use rac_engine::input::kbm::{Binds, Cap, Input, Mouse};

fn rows() -> Vec<Row> {
	rows_of(&TABLE, &Binds::defaults(&TABLE), 0, false, false)
}

fn action(rows: &[Row], id: &str) -> usize {
	let a = TABLE.actions.iter().position(|d| d.id == id).unwrap();
	rows.iter().position(|r| matches!(r, Row::Action { a: x, .. } if *x == a)).unwrap()
}

#[test]
fn the_table_has_sections_actions_pad_rows_and_two_resets() {
	let r = rows();
	assert!(matches!(r[0], Row::Device { pad: false, .. }) && matches!(r[1], Row::Head(_)));
	assert_eq!(r.iter().filter(|x| matches!(x, Row::Action { .. })).count(), TABLE.actions.len());
	assert!(matches!(r[r.len() - 1], Row::Reset { pad: false, .. }));
	// The controller view: the bindable actions only (17), the sticks line, Reset controller.
	let p = rows_of(&TABLE, &Binds::defaults(&TABLE), 0, false, true);
	assert!(matches!(p[0], Row::Device { pad: true, .. }));
	assert_eq!(p.iter().filter(|x| matches!(x, Row::Action { pad: true, .. })).count(), TABLE.actions.iter().filter(|a| a.pad_ok).count());
	assert!(matches!(p[p.len() - 1], Row::Reset { pad: true, .. }));
	let jump = TABLE.actions.iter().position(|d| d.id == "jump").unwrap();
	let Row::Action { slots, .. } = &p[p.iter().position(|x| matches!(x, Row::Action { a, .. } if *a == jump)).unwrap()] else { panic!() };
	assert_eq!(slots, &["A".to_string(), "\u{2014}".to_string()]);
	let ps = rows_of(&TABLE, &Binds::defaults(&TABLE), 0, true, true);
	assert!(ps.iter().any(|x| matches!(x, Row::Action { slots, .. } if slots[0] == "Cross")));
	assert_eq!(cell_effect(&p[p.iter().position(|x| matches!(x, Row::Action { a, .. } if *a == jump)).unwrap()], 1), Effect::Capture(Cap::Pad(jump, 1)));
	assert_eq!(cell_effect(&p[0], 0), Effect::View(false));
	assert_eq!(view_pad(rac_engine::ui::input::Device::Pad), true);
	let pause = TABLE.actions.iter().position(|d| d.id == "pause").unwrap();
	let mut b = Binds::defaults(&TABLE);
	let row = Row::Action { a: pause, pad: true, label: String::new(), slots: Default::default(), shared: [false; 2] };
	assert_eq!(clear_effect(&TABLE, &b, &row, 0), Clear::Refused, "Pause keeps its last controller button");
	b.assign_pad(&TABLE, pause, 1, 1);
	assert_eq!(clear_effect(&TABLE, &b, &row, 0), Clear::Cleared);
}

#[test]
fn defaults_show_both_slots_and_an_empty_one_as_a_dash() {
	let r = rows();
	let Row::Action { slots, shared, .. } = &r[action(&r, "fire")] else { panic!() };
	assert_eq!(slots[0], "LMB");
	assert!(!shared[0], "ship_fire is another layer: no conflict");
	let Row::Action { slots, .. } = &r[action(&r, "jump")] else { panic!() };
	assert_eq!(slots[1], "\u{2014}");
	let mut b = Binds::defaults(&TABLE);
	let (j, w) = (TABLE.actions.iter().position(|d| d.id == "jump").unwrap(), TABLE.actions.iter().position(|d| d.id == "wrench").unwrap());
	b.assign(j, 1, b.0[w][0].unwrap());
	let r = rows_of(&TABLE, &b, 0, false, false);
	let Row::Action { shared, .. } = &r[action(&r, "jump")] else { panic!() };
	assert_eq!(*shared, [false, true], "a key two actions share is marked");
}

#[test]
fn choosing_a_cell_captures_that_slot() {
	let r = rows();
	let i = action(&r, "jump");
	let Row::Action { a, .. } = &r[i] else { panic!() };
	assert_eq!(cell_effect(&r[i], 1), Effect::Capture(Cap::Kbm(*a, 1)));
	assert_eq!(cell_effect(&r[0], 0), Effect::View(true));
	let reset = r.len() - 1;
	assert_eq!(cell_effect(&r[reset], 0), Effect::Reset(false));
}

#[test]
fn clearing_pause_keeps_one_binding() {
	let r = rows();
	let b = Binds::defaults(&TABLE);
	let p = action(&r, "pause");
	assert_eq!(clear_effect(&TABLE, &b, &r[p], 1), Clear::Cleared);
	let mut one = b;
	let Row::Action { a, .. } = &r[p] else { panic!() };
	one.clear(&TABLE, *a, 1);
	assert_eq!(clear_effect(&TABLE, &one, &r[p], 0), Clear::Refused);
	assert_eq!(clear_effect(&TABLE, &b, &r[action(&r, "jump")], 1), Clear::Nothing);
	let mut c = b;
	let j = TABLE.actions.iter().position(|d| d.id == "jump").unwrap();
	c.assign(j, 1, Input::Mouse(Mouse::Back));
	assert_eq!(clear_effect(&TABLE, &c, &r[action(&r, "jump")], 1), Clear::Cleared);
}

#[test]
fn the_focus_skips_headers_wraps_and_scrolls() {
	let r = rows();
	let mut ui = Ui::default();
	ui.follow(&r);
	assert!(r[ui.row].selectable() && ui.row == 0 && ui.top == 0);
	ui.step(&r, -1);
	assert_eq!(ui.row, r.len() - 1, "wraps to the last row");
	assert_eq!(ui.top, r.len() - VISIBLE);
	ui.step(&r, 1);
	assert_eq!(ui.row, 0);
	assert_eq!(ui.top, 0);
	for _ in 0..30 {
		ui.step(&r, 1);
		assert!(r[ui.row].selectable() && ui.row >= ui.top && ui.row < ui.top + VISIBLE);
	}
}

#[test]
fn hit_finds_the_cell_under_the_pointer() {
	let r = rows();
	let (x1, y) = (cell_rect(2, 0).cx(), cell_rect(2, 0).cy());
	assert_eq!(hit(&r, 0, x1, y), Some((2, 0)));
	assert_eq!(hit(&r, 0, cell_rect(2, 1).cx(), y), Some((2, 1)));
	assert_eq!(hit(&r, 0, x1, cell_rect(1, 0).cy()), None, "a header");
	assert_eq!(hit(&r, 0, x1, cell_rect(0, 0).cy()), Some((0, 0)), "the device switch");
	assert_eq!(hit(&r, 0, 5.0, y), None);
}

#[test]
fn the_draw_list_has_the_panel_and_a_notice_only_while_it_lasts() {
	let r = rows();
	let k = KeysBuilt { caption: "Controls".into(), cols: ["a".into(), "b".into(), "c".into()], rows: r, lang: 0, device: rac_engine::ui::input::Device::Keyboard, glyphs: rac_engine::ui::input::GlyphSet::Letters };
	let mut ui = Ui::default();
	ui.follow(&k.rows);
	let plain = draw(&k, &ui, Cap::Idle, 0, Vec::new()).len();
	ui.notice = Some((Clear::Refused, 100));
	assert!(draw(&k, &ui, Cap::Idle, 200, Vec::new()).len() > plain);
	assert_eq!(draw(&k, &ui, Cap::Idle, 100 + NOTICE_MS, Vec::new()).len(), plain);
}

#[test]
fn the_mouse_invert_rows_end_the_pc_section_and_cycle_through_three_values() {
	let _g = crate::retail::pcopts::globals_guard(); // the invert modes are process-wide (pcmouseinv tests set them)
	let r = rows();
	let i = r.iter().position(|x| matches!(x, Row::Mouse { axis: 0, .. })).expect("up/down row");
	assert!(matches!(r[i + 1], Row::Mouse { axis: 1, .. }) && matches!(r[i + 2], Row::Head(_)), "two rows, then the Controller header");
	assert!(matches!(r[i - 1], Row::Action { .. }), "right after the PC section's actions");
	let Row::Mouse { mode, value, label, .. } = &r[i] else { panic!() };
	assert_eq!((*mode, value.as_str(), label.as_str()), (0, "Like camera", "Mouse up/down"), "default = Like camera");
	assert_eq!(cell_effect(&r[i], 0), Effect::Mouse(0, 1));
	assert_eq!(cell_effect(&Row::Mouse { axis: 1, mode: 2, label: String::new(), value: String::new() }, 0), Effect::Mouse(1, 0), "Inverted wraps to Like camera");
	for l in 0..5 {
		assert!(MOUSE_LABEL.iter().all(|t| !t[l].is_empty()) && MOUSE_VALUE.iter().all(|t| !t[l].is_empty()), "language {l}");
	}
}
