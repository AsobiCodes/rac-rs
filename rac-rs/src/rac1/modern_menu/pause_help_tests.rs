//! Tests of the modern Help page (`pause_help.rs`): rows and targets, the picture hole and crop, description, hit areas, fallback.
use super::pause::*;
use super::pause_tests::{facts, text_of, view_of};
use crate::retail::menu_model::extra::Extra;
use crate::retail::menu_model::pages::Page;
use rac_engine::ui::kit::page::Panel;
use rac_engine::ui::prim::Prim;

const IDS: [u16; 5] = [0x4fd5, 0x4ef1, 0x4ef2, 0x4ee2, 0x4ee3];

fn rows(n: usize) -> Vec<(u16, bool)> {
	IDS.iter().take(n).map(|&i| (i, true)).collect()
}

fn texts(b: &Built) -> Vec<String> {
	let g = b.grid.as_ref().unwrap();
	g.prims.iter().filter_map(|p| if let Prim::Text(t) = p { Some(t.text.clone()) } else { None }).collect()
}

#[test]
fn help_is_a_one_column_list_with_a_picture_hole() {
	let v = view_of(Page::Help, &rows(5), 2);
	let ex = Extra::default();
	let b = build(&facts(&v, &ex, &text_of)).unwrap();
	let g = b.grid.as_ref().expect("a grid-style page");
	assert!(matches!(b.screen.panel, Panel::None));
	assert_eq!(g.cells.len(), 5);
	assert_eq!(g.flat_sel, 2);
	assert_eq!(b.items[3], Some((super::pause_tests::ADDR, 3)));
	assert_eq!(g.step(2, super::pause_grid::Dir::Down), Some(3));
	assert_eq!(g.step(4, super::pause_grid::Dir::Down), None, "the edge goes to the game, which wraps");
	assert!(g.hole.w > 0.0 && g.hole.h > 0.0);
	assert_eq!(g.uv, super::pause_grid::uv_of(super::pause_help::picture_src()));
	assert_eq!(g.cell_at(g.cells[1].cx(), g.cells[1].cy()), Some(1));
	assert_eq!(b.screen.menu.iter().position(|m| m.selected), Some(4), "the Help tab");
}

#[test]
fn the_detail_panel_shows_the_selected_name_and_the_games_description() {
	let v = view_of(Page::Help, &rows(5), 1);
	let mut ex = Extra::default();
	ex.help_desc = Some(0x4fd6);
	let b = build(&facts(&v, &ex, &text_of)).unwrap();
	let t = texts(&b);
	assert!(t.contains(&text_of(0x4ef1)), "name of the selected row in the detail panel");
	assert!(t.contains(&text_of(0x4fd6)) || t.iter().any(|s| text_of(0x4fd6).starts_with(s.as_str())), "description {t:?}");
	assert!(b.screen.footer.is_empty(), "the note moved into the panel");
}

#[test]
fn a_page_without_rows_falls_back_to_the_plain_page() {
	let v = view_of(Page::Help, &[], 0);
	let ex = Extra::default();
	assert!(super::pause_help::build(&facts(&v, &ex, &text_of), &rac_engine::ui::font::FontMeasure).is_none());
}
