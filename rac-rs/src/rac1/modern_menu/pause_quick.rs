//! The modern Quick Select page (v5i, 2026-11-02): the pause menu's page 5 as a ring board. Pure: no statics, no clock. On the right the
//! ring: eight slot discs on a circle (slot 0 at the top, clockwise, the order of the game's own ring), each with the game's own picture
//! of the item it holds (`icons::draw_item`), the game's cursor slot marked and its item's name in the middle; on the left the two
//! grids the game keeps on this page (the weapons 3 x 5 and the gadgets 2 x 3), a dot on the items that already sit in the ring.
//! Retail's rule stays the game's: L1 / R1 turn the ring cursor, the D-pad walks the grids, Cross puts the cell's item into the cursor
//! slot. This file only draws and says where things are (`GridBuilt`, `RingBuilt`); the host sends the pad pulses (`pause_host/grid.rs`).
//! Every word is a retail text id (the tab name, the item names, the page's own Equip text); the numbers 1..8 are no words.
use super::icons;
use super::pause::{Facts, TAB_IDS, TAB_QUICK};
use super::pause_grid::{draw_cell, fill, fill_b, grids_of, icon, retail_sel, text, GridBuilt, GridGroup, Place, RingBuilt, BUTTON, DETAIL_PANEL, GRID_PANEL, PAD};
use super::title::shown;
use crate::retail::menu_model::extra::RingFacts;
use crate::retail::menu_model::pages::Page;
use rac_engine::ui::kit::text::upper;
use rac_engine::ui::prim::{Align, FontRole, Rect};
use rac_engine::ui::theme::*;

/// The ring panel is the place of the other grid pages' detail panel.
const RING_PANEL: Rect = DETAIL_PANEL;
/// The page's own "Equip" text (the Cross hint of the Weapons page says the same): here Cross puts the item into the ring slot.
const T_EQUIP: u32 = 0x4eec;
const CELL_H: f32 = 84.0;
const CELL_COLS: f32 = 5.0;
const GROUP_GAP: f32 = 14.0;
const CIRCLE: f32 = 44.0;
/// The ring: centre, radius of the slot circle, slot disc size, the picture inside it.
const RING_AT: (f32, f32) = (940.0, 350.0);
const RING_R: f32 = 150.0;
const SLOT: f32 = 64.0;
const SLOT_PIC: f32 = 40.0;
/// The two arrows that turn the ring (L1 / R1), left and right of it.
const ARROW: f32 = 48.0;

/// Where slot `i` sits: 0 at the top, clockwise (the game's `mouse_menu::ring_slot` order).
pub fn slot_centre(i: u32) -> (f32, f32) {
	let a = i as f32 * std::f32::consts::FRAC_PI_4 - std::f32::consts::FRAC_PI_2;
	(RING_AT.0 + a.cos() * RING_R, RING_AT.1 + a.sin() * RING_R)
}

/// How to turn the ring cursor from `cursor` to `slot`: (true = R1 / false = L1, how many presses), the short way round (a tie goes to R1,
/// as `mouse_menu::toward`).
pub fn ring_steps(cursor: u32, slot: u32) -> (bool, u32) {
	let d = (slot % 8 + 8 - cursor % 8) % 8;
	if d <= 4 { (true, d) } else { (false, 8 - d) }
}

fn arrows() -> (Rect, Rect) {
	let y = RING_AT.1 - ARROW / 2.0;
	(Rect::new(RING_PANEL.x + 28.0, y, ARROW, ARROW), Rect::new(RING_PANEL.right() - 28.0 - ARROW, y, ARROW, ARROW))
}

/// The ring panel's drawing: the track, the eight slots, the middle name and the two turn arrows.
fn draw_ring(out: &mut Vec<rac_engine::ui::prim::Prim>, f: &Facts, r: &RingFacts) -> RingBuilt {
	fill_b(out, RING_PANEL, 16.0, PANEL, 1.0, HAIRLINE);
	let track = Rect::new(RING_AT.0 - RING_R, RING_AT.1 - RING_R, 2.0 * RING_R, 2.0 * RING_R);
	fill_b(out, track, RING_R, WHITE.tint(0.0), 2.0, HAIRLINE);
	let mut slots = [Rect::new(0.0, 0.0, 0.0, 0.0); 8];
	for (i, hit) in slots.iter_mut().enumerate() {
		let (sx, sy) = slot_centre(i as u32);
		let disc = Rect::new(sx - SLOT / 2.0, sy - SLOT / 2.0, SLOT, SLOT);
		*hit = disc;
		if i as u32 == r.cursor {
			fill_b(out, Rect::new(disc.x - 5.0, disc.y - 5.0, SLOT + 10.0, SLOT + 10.0), (SLOT + 10.0) / 2.0, ACCENT.tint(0.16), 2.0, ACCENT);
		} else {
			fill_b(out, disc, SLOT / 2.0, WHITE.tint(0.05), 1.0, HAIRLINE);
		}
		let id = r.ids[i];
		if id != 0 {
			let pic = Rect::new(sx - SLOT_PIC / 2.0, sy - SLOT_PIC / 2.0, SLOT_PIC, SLOT_PIC);
			if !icons::draw_item(out, id, pic, WHITE) {
				icon(out, pic, true);
			}
		}
		text(out, (sx, sy + SLOT / 2.0 + 14.0), FontRole::Condensed, 12.0, if i as u32 == r.cursor { ACCENT } else { TEXT_DIM }, Align::Center, 0.0, &(i + 1).to_string());
	}
	let mid = r.ids[(r.cursor & 7) as usize];
	let name = if mid == 0 { String::new() } else { (f.text)(crate::inventory::name_text(mid)) };
	text(out, (RING_AT.0, RING_AT.1 + 8.0), FontRole::Condensed, 26.0, WHITE, Align::Center, 2.0 * (RING_R - SLOT), &name);
	let (prev, next) = arrows();
	for (rect, glyph) in [(prev, "<"), (next, ">")] {
		fill_b(out, rect, ARROW / 2.0, WHITE.tint(0.05), 1.0, HAIRLINE);
		text(out, (rect.cx(), rect.cy() + 1.0), FontRole::Condensed, 28.0, TEXT_BODY, Align::Center, 0.0, glyph);
	}
	RingBuilt { slots, prev, next, cursor: r.cursor }
}

/// The modern Quick Select page of `f.view`; None when the facts do not look right (the ring or one of the two grids is missing): the
/// page then stays classic.
pub fn build(f: &Facts) -> Option<GridBuilt> {
	let ring = f.extra.ring?;
	let grids = grids_of(f, Page::QuickSelect)?;
	let mut first = 0;
	let groups: Vec<GridGroup> = grids
		.iter()
		.map(|g| {
			let r = GridGroup { addr: g.addr, rows: g.rows, cols: g.cols, first };
			first += g.cells.len();
			r
		})
		.collect();
	let flat_sel = retail_sel(f, &groups);
	let sel = shown(f.local, flat_sel, f.now).min(first.saturating_sub(1));
	let mut out = Vec::new();
	fill_b(&mut out, GRID_PANEL, 16.0, PANEL, 1.0, HAIRLINE);
	let inner = GRID_PANEL.inset(PAD);
	let caption = upper(&(f.text)(TAB_IDS[TAB_QUICK]));
	text(&mut out, (inner.x, inner.y + 10.0), FontRole::Condensed, 15.0, TEXT_SECTION, Align::Left, 0.0, &caption);
	let cw = inner.w / CELL_COLS;
	let (mut cells, mut targets, mut flat) = (Vec::new(), Vec::new(), 0usize);
	let mut y = inner.y + 34.0;
	for (gi, g) in grids.iter().enumerate() {
		for (k, c) in g.cells.iter().enumerate() {
			let (r, col) = ((k as u32 / g.cols) as f32, (k as u32 % g.cols) as f32);
			let rect = Rect::new(inner.x + col * cw, y + r * CELL_H, cw, CELL_H).inset(3.0);
			draw_cell(&mut out, f, Page::QuickSelect, &Place { group: gi, cell: k, rect, circle: CIRCLE }, c, flat == sel);
			if c.owned && ring.ids.contains(&c.id) && c.id != 0 {
				fill(&mut out, Rect::new(rect.right() - 16.0, rect.y + 8.0, 8.0, 8.0), 4.0, ACCENT);
			}
			cells.push(rect);
			targets.push(Some((g.addr, k as u32)));
			flat += 1;
		}
		y += g.rows as f32 * CELL_H + GROUP_GAP;
	}
	let ring_built = draw_ring(&mut out, f, &ring);
	let label = (f.text)(T_EQUIP);
	let button = (!label.is_empty()).then(|| {
		fill(&mut out, BUTTON, 12.0, ACCENT);
		text(&mut out, (BUTTON.cx(), BUTTON.cy()), FontRole::Medium, 17.0, INK_ON_ACCENT, Align::Center, BUTTON.w - 24.0, &label);
		BUTTON
	});
	// No 3D preview on this page: an empty hole (the scrim stays whole, the host places no crop).
	Some(GridBuilt { prims: out, cells, targets, groups, hole: Rect::new(0.0, 0.0, 0.0, 0.0), uv: [0.0; 4], flat_sel, button, ring: Some(ring_built) })
}
