//! Tests of the pure icon-grid pages (`pause_grid.rs`, v0.5 screens stage D2): the Weapons, Gadgets and Items builders on synthetic
//! grids, the preview crop (rect, texture coordinates, aspect rule, knobs), the holes in the scrim and the detail panel (rect algebra),
//! the action button, the hit areas and the in-grid steps. The fixtures are shared with the host tests (`pause_host/tests.rs`).
use super::icons;
use super::pause::*;
use super::pause_grid::*;
use super::pause_tests::{facts, text_of};
use crate::modern_menu::hide::Screen;
use crate::retail::menu_keys;
use crate::retail::menu_model::extra::{Extra, GridCell, GridFacts};
use crate::retail::menu_model::pages::Page;
use crate::retail::menu_model::text::{decode, Btn};
use crate::retail::menu_model::view::{Body, ItemView, PageView};
use rac_engine::ui::font::FontMeasure;
use rac_engine::ui::kit::shell::SHELL_SCRIM;
use rac_engine::ui::kit::page::Panel;
use rac_engine::ui::input::{Device, GlyphSet};
use rac_engine::ui::prim::{Border, Prim, Rect, Symbol};
use rac_engine::ui::prim::GlyphKind;
use rac_engine::ui::theme::*;

pub(crate) const GRID: u32 = 0x1b2e00;
const HINT: u32 = 0x1b9000;

fn cell(id: u8, kind: u8, owned: bool, ammo: Option<(u32, u32)>) -> GridCell {
	GridCell { id, kind, owned, ammo }
}

pub(crate) fn grid(addr: u32, rows: u32, cols: u32, cells: Vec<GridCell>) -> GridFacts {
	GridFacts { addr, rows, cols, cells }
}

/// The 15 weapon cells (3 x 5): ids 10.., cells 0, 1, 3, 4, 5 owned, 2 and the rest not; 0 and 3 hold ammo, 1 has no ammo.
pub(crate) fn weapons() -> GridFacts {
	let cells = (0..15u32)
		.map(|k| {
			let ammo = match k {
				0 => Some((17, 40)),
				3 => Some((0, 60)),
				_ => None,
			};
			cell(10 + k as u8, 0, k < 6 && k != 2, ammo)
		})
		.collect();
	grid(GRID, 3, 5, cells)
}

/// The Gadgets page's four groups (Hand 2 x 3, Back 1 x 3, Head 1 x 3, Foot 1 x 2) at GRID + 0x100 * group.
pub(crate) fn gadgets() -> Vec<GridFacts> {
	let shape = [(2u32, 3u32), (1, 3), (1, 3), (1, 2)];
	shape.iter().enumerate().map(|(g, &(r, c))| grid(GRID + 0x100 * g as u32, r, c, (0..r * c).map(|k| cell(20 + 6 * g as u8 + k as u8, 0, k % 2 == 0, None)).collect())).collect()
}

/// The Items page's 3 x 3 grid in `ITEM_GRID` order; the first two are owned.
pub(crate) fn items() -> GridFacts {
	grid(GRID, 3, 3, crate::frontend::ITEM_GRID.iter().enumerate().map(|(k, e)| cell(e.3, e.2, k < 2, None)).collect())
}

fn item(addr: u32, slot: usize, body: Body) -> ItemView {
	ItemView { addr, slot, rect: [0, 0, 100, 100], body }
}

/// A grid page: one Grid item per fact set (cursor `cursors[i]`), the current item `cur`, and a Cross hint line when `hint` is set.
pub(crate) fn view(page: Page, grids: &[GridFacts], cursors: &[u32], cur: usize, hint: Option<&str>) -> PageView {
	let mut items: Vec<ItemView> = grids.iter().zip(cursors).enumerate().map(|(i, (g, &c))| item(g.addr, i + 2, Body::Grid { rows: g.rows, cols: g.cols, cursor: c })).collect();
	if let Some(h) = hint {
		items.push(item(HINT, 7, Body::Hint { button: Btn::Cross, text: decode(h) }));
	}
	PageView { screen: Screen::Pause, page, id: 0, ready: true, front: false, keys: menu_keys::DEFAULT, cur: Some(cur), items }
}

pub(crate) fn extra(grids: Vec<GridFacts>) -> Extra {
	Extra { grids, ..Extra::default() }
}

fn build_of(v: &PageView, x: &Extra) -> Option<GridBuilt> {
	build_with(&facts(v, x, &text_of), &FontMeasure, &Knobs::DEFAULT)
}

fn weapons_built(cursor: u32) -> GridBuilt {
	let w = weapons();
	build_of(&view(Page::Weapons, &[w.clone()], &[cursor], 0, Some("Equip")), &extra(vec![w])).unwrap()
}

fn texts(g: &GridBuilt) -> Vec<String> {
	g.prims
		.iter()
		.filter_map(|p| match p {
			Prim::Text(t) => Some(t.text.clone()),
			_ => None,
		})
		.collect()
}

fn count(g: &GridBuilt, s: &str) -> usize {
	texts(g).iter().filter(|t| *t == s).count()
}

/// The fills (no border) of a draw list as rects.
fn strips(prims: &[Prim]) -> Vec<Rect> {
	prims
		.iter()
		.filter_map(|p| match p {
			Prim::RRect { rect, border: None, .. } => Some(*rect),
			_ => None,
		})
		.collect()
}

fn area(rs: &[Rect]) -> f64 {
	rs.iter().map(|r| r.w as f64 * r.h as f64).sum()
}

fn covering(rs: &[Rect], x: f32, y: f32) -> usize {
	rs.iter().filter(|r| r.contains(x, y)).count()
}

/// The cells drawn as selected (accent border, 0.6 alpha).
fn selected_cells(g: &GridBuilt) -> Vec<Rect> {
	let want = Border { width: 1.5, colour: ACCENT.tint(0.6) };
	g.prims
		.iter()
		.filter_map(|p| match p {
			Prim::RRect { rect, border: Some(b), radius, .. } if *b == want && *radius == 12.0 => Some(*rect),
			_ => None,
		})
		.collect()
}

#[test]
fn the_weapons_page_has_one_cell_per_weapon_and_the_selected_one_is_marked() {
	for cursor in [0u32, 7, 14] {
		let g = weapons_built(cursor);
		assert_eq!(g.cells.len(), 15);
		assert_eq!(g.targets, (0..15).map(|k| Some((GRID, k))).collect::<Vec<_>>());
		assert_eq!(g.groups, [GridGroup { addr: GRID, rows: 3, cols: 5, first: 0 }]);
		assert_eq!(g.flat_sel, cursor as usize);
		assert_eq!(selected_cells(&g), [g.cells[cursor as usize]], "exactly the cursor cell is selected");
	}
	let g = weapons_built(0);
	assert_eq!(count(&g, "?"), 10, "10 unowned cells show a ?; the detail name of an owned weapon is its name");
	assert!(g.cells.iter().all(|c| c.w > 60.0 && c.h > 60.0 && GRID_BOUNDS.contains(c.cx(), c.cy())));
	assert!(g.cells[5].y > g.cells[0].y && g.cells[1].x > g.cells[0].x, "row by row, left to right");
}

const GRID_BOUNDS: Rect = Rect::new(48.0, 116.0, 576.0, 540.0);

#[test]
fn only_owned_weapons_show_a_name_and_the_detail_has_the_ammo_bar_or_the_no_ammo_text() {
	let name = |id: u8| text_of(crate::inventory::name_text(id));
	let g = weapons_built(0);
	assert_eq!(count(&g, &name(10)), 2, "owned: its name under the icon and in the detail panel");
	assert_eq!(count(&g, &name(12)), 0, "not owned (cell 2): no name anywhere");
	assert_eq!(count(&g, "17 / 40"), 1);
	assert_eq!(count(&g, &text_of(0x4f4d)), 0, "a weapon with ammo has no 'No ammo' line");
	let bar = strips(&g.prims).into_iter().filter(|r| r.h == 8.0).collect::<Vec<_>>();
	assert_eq!(bar.len(), 2, "track and fill");
	assert!((bar[1].w / bar[0].w - 17.0 / 40.0).abs() < 1e-3, "the fill is n / max of the track");
	let g = weapons_built(1);
	assert_eq!((count(&g, &text_of(0x4f4d)), count(&g, "17 / 40")), (1, 0), "an owned weapon without ammo: the game's 'No ammo'");
	let g = weapons_built(2);
	assert_eq!(count(&g, &text_of(0x4f4d)), 0, "a weapon the player lacks shows no ammo line");
	assert_eq!(count(&g, "?"), 10 + 1, "its detail name is a ?");
	let g = weapons_built(3);
	assert_eq!(count(&g, "0 / 60"), 1, "an empty magazine still shows the bar");
}

#[test]
fn every_word_on_the_grid_pages_is_a_retail_text_a_number_or_a_question_mark() {
	let g = weapons_built(0);
	for t in texts(&g) {
		let ok = t.starts_with('T') || t == "?" || t.contains(" / ") || t == "Equip" || t.chars().all(|c| c.is_ascii_digit());
		assert!(ok, "{t:?} is a PC word: the five-language fit would need a T5");
	}
}

#[test]
fn the_gadgets_page_has_four_labelled_groups_in_one_flat_list() {
	let gs = gadgets();
	let v = view(Page::Gadgets, &gs, &[0, 1, 2, 0], 2, None);
	let g = build_of(&v, &extra(gs.clone())).unwrap();
	assert_eq!(g.cells.len(), 6 + 3 + 3 + 2);
	assert_eq!(g.groups.iter().map(|g| (g.addr, g.first)).collect::<Vec<_>>(), [(GRID, 0), (GRID + 0x100, 6), (GRID + 0x200, 9), (GRID + 0x300, 12)]);
	for id in [0x4ee8, 0x4ee9, 0x4eea, 0x4eeb] {
		assert_eq!(count(&g, &text_of(id)), 1, "group label {id:#x}");
	}
	assert_eq!(g.flat_sel, 9 + 2, "the current item is the third group, its cursor 2");
	assert_eq!(g.targets[7], Some((GRID + 0x100, 1)));
	assert_eq!(g.targets[13], Some((GRID + 0x300, 1)));
	assert!(g.cells[3].y > g.cells[0].y && g.cells[6].y > g.cells[3].y && g.cells[9].y > g.cells[6].y && g.cells[12].y > g.cells[9].y, "groups top to bottom");
	assert!(g.cells.iter().all(|c| c.bottom() < GRID_BOUNDS.bottom() && c.right() < GRID_BOUNDS.right()));
	assert_eq!(selected_cells(&g), [g.cells[11]]);
	assert_eq!(g.button, None, "no Cross hint line: no button");
}

/// A Gadgets group with more than three columns (or more rows than the panel holds at the normal height) stays inside the grid panel:
/// it used to be laid out for three columns and 78 px rows, so its cells ran into the detail panel and below the panel.
#[test]
fn a_wide_or_tall_gadgets_group_stays_inside_the_grid_panel() {
	let shape = [(2u32, 5u32), (3, 5), (3, 4), (2, 3)];
	let gs: Vec<GridFacts> = shape.iter().enumerate().map(|(g, &(r, c))| grid(GRID + 0x100 * g as u32, r, c, (0..r * c).map(|k| cell(20 + k as u8, 0, true, None)).collect())).collect();
	let g = build_of(&view(Page::Gadgets, &gs, &[0, 0, 0, 0], 0, None), &extra(gs.clone())).unwrap();
	assert_eq!(g.cells.len(), 10 + 15 + 12 + 6);
	assert!(g.cells.iter().all(|c| c.x >= GRID_BOUNDS.x && c.right() <= GRID_BOUNDS.right() && c.y >= GRID_BOUNDS.y && c.bottom() <= GRID_BOUNDS.bottom()), "every cell is inside the grid panel");
	// The cells of one group are a real grid: the last column's cell is right of the first and below the row above, none overlap.
	for (i, a) in g.cells.iter().enumerate() {
		for b in &g.cells[i + 1..] {
			assert!(a.right() <= b.x + 0.01 || b.right() <= a.x + 0.01 || a.bottom() <= b.y + 0.01 || b.bottom() <= a.y + 0.01, "{a:?} overlaps {b:?}");
		}
	}
	// And a click on the last cell of the five-column group hits that cell, not the one after a wrap.
	let last = &g.cells[9];
	assert_eq!(g.cell_at(last.cx(), last.cy()), Some(9));
	assert_eq!(g.targets[9], Some((GRID, 9)));
}

#[test]
fn the_items_page_shows_the_description_of_an_owned_item_and_leaves_the_gold_bolts_to_the_preview() {
	let it = items();
	let x = extra(vec![it.clone()]);
	let g = build_of(&view(Page::Items, &[it.clone()], &[1], 0, None), &x).unwrap();
	assert_eq!(g.cells.len(), 9);
	let e = crate::frontend::ITEM_GRID[1];
	assert_eq!(count(&g, &text_of(e.5)), 1, "the description of cell 1");
	assert_eq!(count(&g, &text_of(e.4)), 2, "its name under the icon and in the detail panel");
	for id in [0x4f4e, 0x4f4f, 0x4f50, 0x4f51] {
		assert_eq!(count(&g, &text_of(id)), 0, "gold line {id:#x}: retail's gold panel in the preview shows it, the text does not repeat it");
	}
	let g = build_of(&view(Page::Items, &[it], &[5], 0, None), &x).unwrap();
	assert_eq!(count(&g, &text_of(crate::frontend::ITEM_GRID[5].5)), 0, "an item the player lacks has no description");
	assert_eq!(count(&g, "?"), 7 + 1, "7 unowned icons and the detail name");
}

#[test]
fn the_grid_pages_select_their_tab_and_walk_the_flat_cell_list() {
	for (page, grids, tab) in [(Page::Weapons, vec![weapons()], 0), (Page::Gadgets, gadgets(), 1), (Page::Items, vec![items()], 3)] {
		let cursors = vec![0; grids.len()];
		let v = view(page, &grids, &cursors, 0, Some("Equip"));
		let x = extra(grids.clone());
		let b = build(&facts(&v, &x, &text_of)).unwrap_or_else(|| panic!("{page:?} builds"));
		let n: usize = grids.iter().map(|g| g.cells.len()).sum();
		assert!(!b.root);
		assert_eq!((b.rows(), b.items.len()), (n, n));
		assert_eq!(b.target(1), Some((GRID, 1)));
		assert_eq!(b.screen.menu.iter().position(|m| m.selected), Some(tab), "{page:?}");
		assert_eq!(b.screen.menu.len(), 7, "six tabs and the Map tab");
		assert!(b.screen.menu.iter().all(|m| !m.live), "the tabs are inert off the root");
		assert!(matches!(b.screen.panel, Panel::None));
		assert!(b.screen.shell.is_some() && b.screen.dialog.is_none() && b.buttons.is_empty());
		let g = b.grid.as_ref().expect("the grid is kept");
		assert_eq!((g.cells.len(), g.flat_sel, b.sel), (n, 0, 0));
		assert_eq!(count(g, &text_of(TAB_IDS[tab])).max(count(g, &text_of(TAB_IDS[tab]).to_uppercase())), 1, "the caption is the tab's name");
	}
}

#[test]
fn the_players_own_focus_wins_for_a_moment_over_retails_cursor() {
	let w = weapons();
	let v = view(Page::Weapons, &[w.clone()], &[1], 0, None);
	let x = extra(vec![w]);
	let mut f = facts(&v, &x, &text_of);
	f.local = Some((4, 1000));
	f.now = 1100;
	let g = build_with(&f, &FontMeasure, &Knobs::DEFAULT).unwrap();
	assert_eq!((g.flat_sel, selected_cells(&g)), (1, vec![g.cells[4]]), "retail's cursor is cell 1, the player's focus cell 4");
	f.now = 1000 + super::title::GRACE_MS;
	let g = build_with(&f, &FontMeasure, &Knobs::DEFAULT).unwrap();
	assert_eq!(selected_cells(&g), [g.cells[1]], "after the grace retail's cursor shows again");
}

#[test]
fn a_grid_that_does_not_match_the_view_leaves_the_page_classic() {
	let w = weapons();
	let good = view(Page::Weapons, &[w.clone()], &[0], 0, None);
	assert!(build_of(&good, &extra(vec![w.clone()])).is_some());
	assert!(build_of(&good, &Extra::default()).is_none(), "no grid facts (a page flying in)");
	let other = GridFacts { addr: GRID + 0x40, ..w.clone() };
	assert!(build_of(&good, &extra(vec![other])).is_none(), "facts of another item");
	let short = GridFacts { cells: w.cells[..14].to_vec(), ..w.clone() };
	assert!(build_of(&good, &extra(vec![short])).is_none(), "a cell table that is not rows x cols");
	let gs = gadgets();
	let v = view(Page::Gadgets, &gs[..3], &[0, 0, 0], 0, None);
	assert!(build_of(&v, &extra(gs[..3].to_vec())).is_none(), "the Gadgets page needs all four groups");
	assert!(build_of(&view(Page::Camera, &[w.clone()], &[0], 0, None), &extra(vec![w])).is_none(), "not a grid page");
}

#[test]
fn the_action_button_exists_only_with_a_cross_hint_and_says_the_games_word() {
	let g = weapons_built(0);
	let b = g.button.expect("the Weapons page lists Equip");
	assert!(b.w > 100.0 && GRID_BOUNDS.right() < b.x && b.bottom() < 656.0);
	assert!(g.on_button(b.cx(), b.cy()) && !g.on_button(b.x - 1.0, b.cy()));
	assert_eq!(count(&g, "Equip"), 1);
	assert!(g.prims.iter().any(|p| matches!(p, Prim::RRect { rect, fill, .. } if *rect == b && *fill == ACCENT)), "an accent fill");
	let w = weapons();
	let none = build_of(&view(Page::Weapons, &[w.clone()], &[0], 0, None), &extra(vec![w.clone()])).unwrap();
	assert_eq!((none.button, count(&none, "Equip")), (None, 0));
	// A Circle hint line is no action button.
	let mut v = view(Page::Weapons, &[w.clone()], &[0], 0, None);
	v.items.push(item(HINT, 8, Body::Hint { button: Btn::Circle, text: decode("Exit") }));
	assert_eq!(build_of(&v, &extra(vec![w])).unwrap().button, None);
}

#[test]
fn each_cell_answers_a_hit_at_its_centre_and_nothing_between() {
	let g = weapons_built(0);
	for (k, c) in g.cells.iter().enumerate() {
		assert_eq!(g.cell_at(c.cx(), c.cy()), Some(k));
		assert_eq!(g.cell_at(c.x + 1.0, c.y + 1.0), Some(k));
	}
	assert_eq!(g.cell_at(10.0, 10.0), None);
	assert_eq!(g.cell_at(g.hole.cx(), g.hole.cy()), None, "the preview is no cell");
	for w in g.cells.windows(2) {
		assert!(!w[0].overlaps(&w[1]), "cells do not overlap");
	}
	let gs = gadgets();
	let gg = build_of(&view(Page::Gadgets, &gs, &[0, 0, 0, 0], 0, None), &extra(gs)).unwrap();
	for (k, c) in gg.cells.iter().enumerate() {
		assert_eq!(gg.cell_at(c.cx(), c.cy()), Some(k), "gadget cell {k}");
	}
}

#[test]
fn steps_inside_a_grid_stop_at_its_edge() {
	let g = weapons_built(0);
	assert_eq!([Dir::Left, Dir::Up].map(|d| g.step(0, d)), [None, None]);
	assert_eq!((g.step(0, Dir::Right), g.step(0, Dir::Down)), (Some(1), Some(5)));
	assert_eq!((g.step(4, Dir::Right), g.step(14, Dir::Right), g.step(10, Dir::Down), g.step(7, Dir::Up)), (None, None, None, Some(2)));
	assert_eq!(g.step(15, Dir::Left), None, "a cell index past the grid");
	let gs = gadgets();
	let gg = build_of(&view(Page::Gadgets, &gs, &[0, 0, 0, 0], 0, None), &extra(gs)).unwrap();
	assert_eq!((gg.step(0, Dir::Down), gg.step(3, Dir::Up), gg.step(3, Dir::Down)), (Some(3), Some(0), None), "the Hand group has two rows");
	assert_eq!((gg.step(7, Dir::Up), gg.step(7, Dir::Down), gg.step(7, Dir::Left), gg.step(8, Dir::Right)), (None, None, Some(6), None), "a one-row group leaves at every vertical step");
	assert_eq!((gg.step(12, Dir::Right), gg.step(13, Dir::Right)), (Some(13), None), "Foot has two cells");
}

#[test]
fn the_preview_crop_keeps_the_aspect_and_follows_the_knobs() {
	let d = Knobs::DEFAULT;
	assert_eq!(preview_src(Page::Weapons, &d), [266.0, 28.0, 218.0, 230.0], "the hero panel minus 3 px each side");
	assert_eq!(preview_src(Page::Gadgets, &d), preview_src(Page::Weapons, &d));
	assert_eq!(preview_src(Page::Items, &d), [241.0, 261.0, 247.0, 97.0], "the gold panel minus 3 px each side");
	assert_eq!(preview_src(Page::Weapons, &Knobs { inset: 0.0, ..d }), [263.0, 25.0, 224.0, 236.0]);
	assert_eq!(preview_src(Page::Weapons, &Knobs { inset: 10.0, ..d }), [273.0, 35.0, 204.0, 216.0]);
	let part = preview_src(Page::Items, &Knobs { inset: 0.0, items_x0: 0.2, items_x1: 0.8, ..d });
	assert!((part[0] - (238.0 + 253.0 * 0.2)).abs() < 1e-3 && (part[2] - 253.0 * 0.6).abs() < 1e-3 && part[1] == 258.0 && part[3] == 103.0);
	let silly = preview_src(Page::Items, &Knobs { inset: 0.0, items_x0: 0.9, items_x1: 0.1, ..d });
	assert!(silly[2] >= 253.0 * 0.05 - 1e-3 && silly[0] + silly[2] <= 238.0 + 253.0 + 1e-3, "x1 never goes below x0");
	let uv = uv_of(preview_src(Page::Weapons, &Knobs { inset: 0.0, ..d }));
	assert_eq!(uv, [263.0 / 512.0, 25.0 / 448.0, 487.0 / 512.0, 261.0 / 448.0]);
	assert_eq!(uv_of([0.0, 0.0, 512.0, 448.0]), [0.0, 0.0, 1.0, 1.0]);
}

#[test]
fn the_viewport_is_the_crop_shown_at_its_picture_aspect_inside_the_panels_viewport() {
	let vp = Rect::new(672.0, 136.0, 536.0, 220.0);
	for page in [Page::Weapons, Page::Items] {
		let src = preview_src(page, &Knobs::DEFAULT);
		let r = viewport_of(src);
		let want = src[2] / src[3] * 7.0 / 6.0;
		assert!(((r.w / r.h) - want).abs() < 1e-3, "{page:?}: aspect {} vs {want}", r.w / r.h);
		assert!(r.x >= vp.x - 1e-3 && r.y >= vp.y - 1e-3 && r.right() <= vp.right() + 1e-3 && r.bottom() <= vp.bottom() + 1e-3);
		assert!((r.cx() - vp.cx()).abs() < 1e-3 && (r.cy() - vp.cy()).abs() < 1e-3, "centred");
		assert!((r.w - vp.w).abs() < 1e-3 || (r.h - vp.h).abs() < 1e-3, "it fills one dimension");
	}
	assert!(viewport_of(preview_src(Page::Weapons, &Knobs::DEFAULT)).w < 300.0, "the hero crop is taller than wide: pillar boxes");
	assert!((viewport_of(preview_src(Page::Items, &Knobs::DEFAULT)).w - 536.0).abs() < 1e-3, "the gold panel crop is wide: the full width");
	let g = weapons_built(0);
	assert_eq!(g.hole, viewport_of(preview_src(Page::Weapons, &Knobs::DEFAULT)));
	assert_eq!(g.uv, uv_of(preview_src(Page::Weapons, &Knobs::DEFAULT)));
}

#[test]
fn the_holed_scrim_covers_what_the_giant_scrim_covered_except_the_hole() {
	let hole = Rect::new(700.0, 140.0, 243.0, 220.0);
	let ps = holed_scrim(hole);
	assert!(ps.iter().all(|p| matches!(p, Prim::RRect { fill, border: None, radius, .. } if *fill == SHELL_SCRIM && *radius == 0.0)));
	let rs = strips(&ps);
	assert_eq!(rs.len(), 4);
	assert!((area(&rs) - (10_000.0 * 10_000.0 - 243.0 * 220.0)).abs() < 1.0);
	for i in 0..100 {
		for j in 0..100 {
			let (x, y) = (-4995.0 + i as f32 * 100.0, -4995.0 + j as f32 * 100.0);
			assert_eq!(covering(&rs, x, y), usize::from(!hole.contains(x, y)), "({x}, {y}) is covered once outside the hole");
		}
	}
	for (x, y) in [(700.0, 140.0), (942.9, 359.9), (800.0, 200.0), (700.0, 250.0)] {
		assert_eq!(covering(&rs, x, y), 0, "({x}, {y}) is in the hole");
	}
	for (x, y) in [(699.9, 200.0), (943.1, 200.0), (800.0, 139.9), (800.0, 360.1), (-5000.0, -5000.0), (4999.9, 4999.9)] {
		assert_eq!(covering(&rs, x, y), 1, "({x}, {y}) is just outside the hole");
	}
	// A hole at the panel's corner leaves three strips (nothing of zero size).
	assert!(holed_scrim(Rect::new(-5000.0, -5000.0, 100.0, 100.0)).len() <= 3);
}

#[test]
fn the_holed_panel_fills_its_rounded_rect_except_the_hole_and_draws_its_border_on_top() {
	let (rect, hole) = (Rect::new(648.0, 116.0, 584.0, 540.0), Rect::new(700.0, 136.0, 243.0, 220.0));
	let border = Border { width: 1.0, colour: HAIRLINE };
	let ps = holed_panel(rect, hole, 16.0, PANEL, border);
	assert!(matches!(ps.first(), Some(Prim::ClipPush { rect: r, radius }) if *r == rect && *radius == 16.0));
	let pop = ps.iter().position(|p| matches!(p, Prim::ClipPop)).expect("the clip is closed");
	assert_eq!(pop, ps.len() - 2, "the border comes after the clip");
	let fills = strips(&ps[1..pop]);
	assert_eq!(fills.len(), 4);
	assert!((area(&fills) - (584.0 * 540.0 - 243.0 * 220.0)).abs() < 0.5);
	for i in 0..58 {
		for j in 0..54 {
			let (x, y) = (648.5 + i as f32 * 10.0, 116.5 + j as f32 * 10.0);
			assert_eq!(covering(&fills, x, y), usize::from(!hole.contains(x, y)), "({x}, {y})");
		}
	}
	match ps.last() {
		Some(Prim::RRect { rect: r, radius, fill, border: Some(b) }) => {
			assert_eq!((*r, *radius, *b), (rect, 16.0, border));
			assert_eq!(fill.a, 0, "the border alone: its fill is clear, the hole stays open");
		}
		p => panic!("the last prim is the border: {p:?}"),
	}
	assert!(ps[1..pop].iter().all(|p| matches!(p, Prim::RRect { fill, .. } if *fill == PANEL)));
	let flush = holed_panel(rect, Rect::new(rect.x, rect.y, 100.0, 100.0), 16.0, PANEL, border);
	assert_eq!(strips(&flush[1..flush.len() - 2]).len(), 2, "a hole in the corner needs two strips");
}

#[test]
fn the_built_page_leaves_exactly_the_preview_uncovered_inside_its_detail_panel() {
	let g = weapons_built(0);
	let panel_fills: Vec<Rect> = {
		let at = g.prims.iter().position(|p| matches!(p, Prim::ClipPush { .. })).unwrap();
		let end = g.prims.iter().position(|p| matches!(p, Prim::ClipPop)).unwrap();
		strips(&g.prims[at + 1..end])
	};
	assert_eq!(covering(&panel_fills, g.hole.cx(), g.hole.cy()), 0);
	assert!(g.hole.x >= 672.0 && g.hole.right() <= 1208.0 && (g.hole.y - 136.0).abs() < 0.01, "inside the viewport");
	for r in &g.cells {
		assert!(!r.overlaps(&g.hole), "no cell reaches into the preview");
	}
	// Nothing else the page draws lies over the hole.
	for p in &g.prims {
		match p {
			Prim::RRect { rect, fill, border: None, .. } if rect.overlaps(&g.hole) => assert!(panel_fills.contains(rect) || fill.a == 0, "{rect:?} covers the preview"),
			Prim::Text(t) => assert!(!g.hole.contains(t.x, t.y), "text {:?} sits in the preview", t.text),
			_ => {}
		}
	}
}

#[test]
fn the_classic_style_and_the_other_pages_build_nothing_and_the_gate_knows_the_new_pages() {
	for id in [3u32, 4, 6] {
		assert!(in_set(Page::from_id(id)), "page {id} is modern");
	}
	for id in [0xbu32, 0xc] {
		assert!(!in_set(Page::from_id(id)), "page {id:#x} stays classic");
	}
	let gate = Gate { modern: true, ui_open: false, list_open: false, refused: false, front: false, paused: true, page: Some(Page::Weapons) };
	assert!(wanted(&gate) && wanted(&Gate { page: Some(Page::Gadgets), ..gate }) && wanted(&Gate { page: Some(Page::Items), ..gate }));
	assert!(!wanted(&Gate { modern: false, ..gate }) && wanted(&Gate { page: Some(Page::QuickSelect), ..gate }));
}

fn images_of(g: &GridBuilt) -> Vec<u32> {
	g.prims.iter().filter_map(|p| if let Prim::Image { id, .. } = p { Some(*id) } else { None }).collect()
}

#[test]
fn owned_weapons_and_gadgets_draw_the_games_own_picture_and_the_rest_keep_the_generic_icon() {
	let w = weapons();
	let v = view(Page::Weapons, &[w.clone()], &[0], 0, Some("Equip"));
	let x = extra(vec![w.clone()]);
	let pics: Vec<(u32, u32, u32)> = w.cells.iter().map(|c| (icons::item(c.id, icons::FRAME_MENU), 41, 38)).collect();
	let mut got = icons::with_pictures(&pics, || images_of(&build_of(&v, &x).unwrap()));
	got.sort_unstable();
	let mut want: Vec<u32> = w.cells.iter().filter(|c| c.owned).map(|c| icons::item(c.id, icons::FRAME_MENU)).collect();
	want.sort_unstable();
	assert_eq!(got, want, "one frame-0 picture per owned weapon, none for the unowned ones");
	let g = icons::with_pictures(&pics, || build_of(&v, &x).unwrap());
	assert_eq!(count(&g, "?"), 10, "the unowned cells keep their ?");
	let plain = icons::with_pictures(&[], || build_of(&v, &x).unwrap());
	assert!(images_of(&plain).is_empty(), "no registered picture: the generic icon");
	assert_eq!(plain.prims.len() - g.prims.len(), 5 * 3 - 5, "each owned cell's three generic-icon prims became one picture");
	// Gadgets: the same frame, per group cell.
	let gs = gadgets();
	let cursors = vec![0; gs.len()];
	let gv = view(Page::Gadgets, &gs, &cursors, 0, None);
	let gx = extra(gs.clone());
	let gpics: Vec<(u32, u32, u32)> = gs.iter().flat_map(|g| g.cells.iter().map(|c| (icons::item(c.id, icons::FRAME_MENU), 30, 30))).collect();
	let n = icons::with_pictures(&gpics, || images_of(&build_of(&gv, &gx).unwrap()).len());
	assert_eq!(n, gs.iter().flat_map(|g| &g.cells).filter(|c| c.owned).count());
}

#[test]
fn the_items_page_draws_its_own_sprites_by_grid_position() {
	let it = items();
	let v = view(Page::Items, &[it.clone()], &[0], 0, Some("Equip"));
	let x = extra(vec![it]);
	let pics: Vec<(u32, u32, u32)> = (0..9).map(|j| (icons::grid(j), 40, 40)).collect();
	let got = icons::with_pictures(&pics, || images_of(&build_of(&v, &x).unwrap()));
	assert_eq!(got, [icons::grid(0), icons::grid(1)], "the two owned items, by ITEM_GRID position");
}

/// The action button's lead follows the last input device and the Button glyphs option: the Cross glyph of the chosen set with a pad,
/// the Enter keycap with the keyboard and mouse (never both), then the game's word.
#[test]
fn the_action_button_lead_follows_the_device_and_the_glyph_set() {
	let w = weapons();
	let (v, x) = (view(Page::Weapons, &[w.clone()], &[0], 0, Some("Equip")), extra(vec![w]));
	let built = |device, glyphs| {
		let mut f = facts(&v, &x, &text_of);
		(f.device, f.glyphs) = (device, glyphs);
		build_with(&f, &FontMeasure, &Knobs::DEFAULT).unwrap()
	};
	let glyphs = |g: &GridBuilt| -> Vec<GlyphKind> { g.prims.iter().filter_map(|p| if let Prim::Glyph { kind, .. } = p { Some(*kind) } else { None }).collect() };
	for set in [GlyphSet::PlayStation, GlyphSet::Letters] {
		let g = built(Device::Pad, set);
		assert_eq!(glyphs(&g), vec![set.glyph(Symbol::Cross)], "{set:?}: the Cross glyph of the set, once");
		assert_eq!((count(&g, "Enter"), count(&g, "Equip")), (0, 1));
	}
	assert_ne!(GlyphSet::PlayStation.glyph(Symbol::Cross), GlyphSet::Letters.glyph(Symbol::Cross), "the sets differ");
	let k = built(Device::Keyboard, GlyphSet::Letters);
	assert!(glyphs(&k).is_empty(), "no pad glyph with the keyboard");
	assert_eq!((count(&k, "Enter"), count(&k, "Equip")), (1, 1));
	assert!(k.button.is_some(), "the button is still there");
}
