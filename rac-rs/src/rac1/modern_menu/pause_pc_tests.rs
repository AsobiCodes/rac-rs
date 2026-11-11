//! Tests of the modern Controls and Mods pages (`pause_pc.rs`) on synthetic pages.
use super::pause::*;
use super::pause_tests::{facts, text_of, values_of};
use crate::retail::menu_model::extra::Extra;
use crate::retail::menu_model::pages::Page;
use crate::retail::menu_model::pcpage::{PcKind, PcPage, PcRow};
use rac_engine::ui::kit::page::Panel;

fn row(label: &str, value: &str, stepper: bool) -> PcRow {
	PcRow { label: label.into(), value: value.into(), stepper, ..Default::default() }
}

fn controls() -> PcPage {
	PcPage { kind: PcKind::Controls, title: "Controls".into(), rows: vec![row("Page", "Keyboard buttons 1/2", true), row("Cross", "Space", false), row("Square", "F", false), row("", "", false)] }
}

fn built(page: Page, pc: PcPage, cursor: u32) -> Option<Built> {
	let n = pc.rows.len() as u32;
	let (view, extra) = (values_of(page, n, cursor), Extra { pc: Some(pc), ..Default::default() });
	build(&facts(&view, &extra, &text_of))
}

#[test]
fn the_pc_pages_are_modern_and_belong_to_the_options_tab() {
	assert!(in_set(Page::PcControls) && in_set(Page::PcMods));
	assert!(!in_set(Page::Camera), "the retail Camera page stays with the Camera tab of the engine page");
	let b = built(Page::PcControls, controls(), 1).unwrap();
	assert!(!b.root);
	assert_eq!(b.screen.menu.iter().position(|m| m.selected), Some(5), "the Options tab");
}

#[test]
fn controls_rows_show_the_bindings_and_the_game_cursor() {
	let b = built(Page::PcControls, controls(), 1).unwrap();
	assert!(matches!(b.screen.panel, Panel::None), "the Controls page is the keys table, not a rows panel");
	let k = b.keys.as_ref().expect("the keys table");
	assert!(k.rows.iter().any(|r| matches!(r, super::pause_keys::Row::Action { .. })));
	assert!(b.items.is_empty() && b.screen.footer.is_empty());
}

#[test]
fn a_row_without_a_value_cannot_be_chosen() {
	let pc = PcPage { kind: PcKind::Mods, ..controls() };
	let b = built(Page::PcMods, pc, 0).unwrap();
	assert_eq!(b.items.len(), 4);
	assert!(b.items[..3].iter().all(|t| t.is_some()), "stepper and bindings are chosen by (item, row)");
	assert_eq!(b.items[2], Some((super::pause_tests::ADDR, 2)));
	assert_eq!(b.items[3], None, "the blank row is inert");
}

#[test]
fn mods_page_builds_without_a_footer() {
	let pc = PcPage { kind: PcKind::Mods, title: "Mods".into(), rows: vec![row("Texture packs", "Off", false), row("SCUS-97199", "Off", false), row("Packs change at the next planet", "", false)] };
	let b = built(Page::PcMods, pc, 0).unwrap();
	assert!(b.screen.footer.is_empty());
	let Panel::Rows { rows, .. } = &b.screen.panel else { panic!("rows panel") };
	assert!(rows[2].dim && b.items[2].is_none(), "the note row is dimmed and inert");
}

#[test]
fn a_page_whose_rows_do_not_match_the_game_list_stays_classic() {
	let view = values_of(Page::PcControls, 9, 0);
	let extra = Extra { pc: Some(controls()), ..Default::default() };
	assert!(build(&facts(&view, &extra, &text_of)).is_none(), "4 rows read, 9 in the list");
	let view = values_of(Page::PcMods, 4, 0);
	assert!(build(&facts(&view, &extra, &text_of)).is_none(), "the rows are the Controls page's");
	assert!(build(&facts(&view, &Extra::default(), &text_of)).is_none(), "no rows read at all");
}

#[test]
fn a_retail_text_id_the_page_module_cannot_answer_is_the_games_text() {
	// The mod rows' On / Off are retail ids (0x4f5a / 0x4f5b): pcmods answers only its own, so the row came back valueless (dimmed, inert).
	let on = PcRow { label: "SCUS-97199".into(), value_id: 0x4f5a, ..Default::default() };
	let own = PcRow { label: "Texture packs".into(), value_id: 0x7f50, ..Default::default() };
	let pc = PcPage { kind: PcKind::Mods, title: "Mods".into(), rows: vec![on, own] };
	let b = built(Page::PcMods, pc, 0).unwrap();
	let Panel::Rows { rows, .. } = &b.screen.panel else { panic!("rows panel") };
	assert_eq!(rows[0].value, "T4f5a");
	assert!(!rows[0].dim && b.items[0].is_some(), "the row is live");
	assert!(rows[1].value.is_empty() && rows[1].dim, "an own id the module left blank stays a blank row");
}
