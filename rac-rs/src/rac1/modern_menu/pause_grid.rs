//! The icon-grid pages of the modern pause menu (v0.5 screens stage D2): Weapons, Gadgets and Items. Pure: no statics, no clock. `build_grid`
//! turns the page's grids (`menu_model::extra::GridFacts`, read by the worker) into the grid panel (a circle with a generic icon per
//! cell, the item's name under it, "?" and dimmed when the player does not have it) and the detail panel (the item's name, the weapon's
//! ammo bar or "No ammo", the Items page's description (its gold bolts are in retail's gold panel, the preview), an accent button with the game's own Cross hint). The 3D preview
//! (Ratchet holding the selected weapon, the worn gadgets, the gold bolt) is the game's own picture: retail draws it into the overlay
//! texture, the host crops it (`preview_src` / `uv_of`) into the detail panel's viewport and the draw list leaves exactly that hole
//! uncovered (`holed_scrim`, `holed_panel`). Every word is a retail text id; the only PC numbers are the three RAC_TUNE knobs of `Knobs`.
use super::pause::{Facts, TAB_IDS};
use super::title::shown;
use super::crop;
use super::icons;
use crate::retail::menu_model::extra::{GridCell, GridFacts};
use crate::retail::menu_model::pages::Page;
use crate::retail::menu_model::text::Btn;
use crate::retail::menu_model::view::Body;
use rac_engine::ui::input::Device;
use rac_engine::ui::keycaps::{self, Cap};
use rac_engine::ui::kit::shell::SHELL_SCRIM;
use rac_engine::ui::kit::text::{run_width, upper, TextMeasure};
use rac_engine::ui::prim::{Align, Border, FontRole, Prim, Rect, Rgba, Symbol, TextRun};
use rac_engine::ui::theme::*;

/// The panel previews in that space: s1 (Ratchet with the weapon / the worn gadgets) and the Items page's gold panel s2.
const HERO: [f32; 4] = [263.0, 25.0, 224.0, 236.0];
const GOLD_PANEL: [f32; 4] = [238.0, 258.0, 253.0, 103.0];
/// The grid panel, the detail panel and the preview viewport inside it (design space).
pub(super) const GRID_PANEL: Rect = Rect::new(48.0, 116.0, 576.0, 540.0);
pub(super) const DETAIL_PANEL: Rect = Rect::new(648.0, 116.0, 584.0, 540.0);
const VIEWPORT: Rect = Rect::new(672.0, 136.0, 536.0, 220.0);
pub(super) const PAD: f32 = 20.0;
const CELL_H: f32 = 118.0;
const GADGET_CELL_H: f32 = 78.0;
/// The columns a Gadgets cell is sized for (a wider group shrinks its cells).
const GADGET_COLS: u32 = 3;
const GROUP_GAP: f32 = 12.0;
const LABEL_W: f32 = 108.0;
pub(super) const BUTTON: Rect = Rect::new(672.0, 592.0, 260.0, 44.0);
/// Retail text ids: the Gadgets groups (Hand, Back, Head, Foot) and "No ammo".
const T_GROUPS: [u32; 4] = [0x4ee8, 0x4ee9, 0x4eea, 0x4eeb];
const T_NO_AMMO: u32 = 0x4f4d;
/// The tab (index in `TAB_IDS`) of each grid page.
const TAB_WEAPONS: usize = 0;
const TAB_GADGETS: usize = 1;
const TAB_ITEMS: usize = 3;

/// The RAC_TUNE knobs of the preview crop (ENV.md): `pgrid_inset` overlay pixels cut off each side of the crop (the panel's frame),
/// `pgrid_items_x0` / `pgrid_items_x1` the fraction of the Items page's gold panel kept (its width).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Knobs {
	pub inset: f32,
	pub items_x0: f32,
	pub items_x1: f32,
}

impl Knobs {
	pub const DEFAULT: Knobs = Knobs { inset: 3.0, items_x0: 0.0, items_x1: 1.0 };

	/// The knobs from RAC_TUNE (defaults when unset).
	pub fn now() -> Knobs {
		let d = Knobs::DEFAULT;
		Knobs { inset: crate::devtune::f32("pgrid_inset", d.inset), items_x0: crate::devtune::f32("pgrid_items_x0", d.items_x0), items_x1: crate::devtune::f32("pgrid_items_x1", d.items_x1) }
	}
}

/// The region of the overlay picture (x, y, w, h in 512 x 448 pixels) cropped into the detail panel: the hero panel on Weapons and
/// Gadgets, the gold panel on Items, `inset` smaller on each side.
pub fn preview_src(page: Page, k: &Knobs) -> [f32; 4] {
	let [x, y, w, h] = if page == Page::Items {
		let x0 = k.items_x0.clamp(0.0, 0.9);
		let x1 = k.items_x1.clamp(x0 + 0.05, 1.0);
		[GOLD_PANEL[0] + GOLD_PANEL[2] * x0, GOLD_PANEL[1], GOLD_PANEL[2] * (x1 - x0), GOLD_PANEL[3]]
	} else {
		HERO
	};
	crop::inset([x, y, w, h], k.inset)
}

pub use super::crop::uv_of;

/// Where the crop shows in the design space: the largest rect of the crop's DISPLAY aspect inside the viewport, centred (`crop::display_rect`).
pub fn viewport_of(src: [f32; 4]) -> Rect {
	crop::display_rect(VIEWPORT, src)
}

/// The scrim of the shell with a hole: four strips that together cover what the giant scrim covered (-5000..5000) minus `hole`.
pub fn holed_scrim(hole: Rect) -> Vec<Prim> {
	const B: f32 = 5000.0;
	let strips = [
		Rect::new(-B, -B, 2.0 * B, hole.y + B),
		Rect::new(-B, hole.bottom(), 2.0 * B, B - hole.bottom()),
		Rect::new(-B, hole.y, hole.x + B, hole.h),
		Rect::new(hole.right(), hole.y, B - hole.right(), hole.h),
	];
	strips.into_iter().filter(|r| r.w > 0.0 && r.h > 0.0).map(|rect| Prim::RRect { rect, radius: 0.0, fill: SHELL_SCRIM, border: None }).collect()
}

/// A rounded panel with a rectangular hole: the fill as four square strips around the hole inside a rounded clip, then the border alone.
pub fn holed_panel(rect: Rect, hole: Rect, radius: f32, fill: Rgba, border: Border) -> Vec<Prim> {
	let strips = [
		Rect::new(rect.x, rect.y, rect.w, hole.y - rect.y),
		Rect::new(rect.x, hole.bottom(), rect.w, rect.bottom() - hole.bottom()),
		Rect::new(rect.x, hole.y, hole.x - rect.x, hole.h),
		Rect::new(hole.right(), hole.y, rect.right() - hole.right(), hole.h),
	];
	let mut out = vec![Prim::ClipPush { rect, radius }];
	out.extend(strips.into_iter().filter(|r| r.w > 0.0 && r.h > 0.0).map(|rect| Prim::RRect { rect, radius: 0.0, fill, border: None }));
	out.push(Prim::ClipPop);
	out.push(Prim::RRect { rect, radius, fill: fill.tint(0.0), border: Some(border) });
	out
}

/// One grid of the page in the flat cell list: `first` = the flat index of its first cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridGroup {
	pub addr: u32,
	pub rows: u32,
	pub cols: u32,
	pub first: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
	Up,
	Down,
	Left,
	Right,
}

/// The modern grid page: its draw list (after the shell's header), where every cell is, and the preview crop.
#[derive(Clone, Debug, PartialEq)]
pub struct GridBuilt {
	/// Panels, cells and the detail text (drawn after the shell's header and hints; the scrim is replaced by `holed_scrim(hole)`).
	pub prims: Vec<Prim>,
	/// Per flat cell: its design-space rect (the hit area).
	pub cells: Vec<Rect>,
	/// Per flat cell: the (grid item, cell) a hover or click selects.
	pub targets: Vec<Option<(u32, u32)>>,
	pub groups: Vec<GridGroup>,
	/// Where the preview shows (design space) and the overlay region it shows.
	pub hole: Rect,
	pub uv: [f32; 4],
	/// Retail's selected flat cell (before the player's own focus).
	pub flat_sel: usize,
	/// The accent action button (the page lists a Cross hint), a click confirms the selected cell.
	pub button: Option<Rect>,
	/// Quick Select: the ring's hit areas (`pause_quick.rs`); None on the other pages.
	pub ring: Option<RingBuilt>,
}

/// The Quick Select ring's hit areas (design space): the eight slot discs (slot 0 at the top, clockwise), the two turn arrows and the
/// game's cursor slot.
#[derive(Clone, Debug, PartialEq)]
pub struct RingBuilt {
	pub slots: [Rect; 8],
	pub prev: Rect,
	pub next: Rect,
	pub cursor: u32,
}

impl RingBuilt {
	/// The slot disc under design point (x, y).
	pub fn slot_at(&self, x: f32, y: f32) -> Option<u32> {
		self.slots.iter().position(|r| r.contains(x, y)).map(|i| i as u32)
	}
}

impl GridBuilt {
	/// The flat cell under design point (x, y).
	pub fn cell_at(&self, x: f32, y: f32) -> Option<usize> {
		self.cells.iter().position(|r| r.contains(x, y))
	}

	pub fn on_button(&self, x: f32, y: f32) -> bool {
		self.button.is_some_and(|b| b.contains(x, y))
	}

	/// The flat cell one step from `flat` inside its own grid; None at the grid's edge (retail then wraps or jumps to the next group).
	pub fn step(&self, flat: usize, d: Dir) -> Option<usize> {
		let g = self.groups.iter().rev().find(|g| g.first <= flat)?;
		let (cols, rows) = (g.cols as usize, g.rows as usize);
		let k = flat - g.first;
		if k >= cols * rows {
			return None;
		}
		let (r, c) = (k / cols, k % cols);
		match d {
			Dir::Left if c > 0 => Some(flat - 1),
			Dir::Right if c + 1 < cols => Some(flat + 1),
			Dir::Up if r > 0 => Some(flat - cols),
			Dir::Down if r + 1 < rows => Some(flat + cols),
			_ => None,
		}
	}
}

#[allow(clippy::too_many_arguments)]
pub(super) fn text(out: &mut Vec<Prim>, (x, y): (f32, f32), role: FontRole, px: f32, colour: Rgba, align: Align, max_w: f32, s: &str) {
	out.push(Prim::Text(TextRun { x, y, role, px, colour, align, max_w, track: 0.0, text: s.to_string() }));
}

pub(super) fn fill(out: &mut Vec<Prim>, rect: Rect, radius: f32, fill: Rgba) {
	out.push(Prim::RRect { rect, radius, fill, border: None });
}

pub(super) fn fill_b(out: &mut Vec<Prim>, rect: Rect, radius: f32, fill: Rgba, width: f32, colour: Rgba) {
	out.push(Prim::RRect { rect, radius, fill, border: Some(Border { width, colour }) });
}

/// `s` broken into at most `max_lines` lines of at most `max_w` design pixels (greedy, at spaces; the last line is cut by the renderer).
pub fn wrap(m: &dyn TextMeasure, role: FontRole, px: f32, s: &str, max_w: f32, max_lines: usize) -> Vec<String> {
	let mut lines: Vec<String> = Vec::new();
	for word in s.split_whitespace() {
		let joined = lines.last().map(|l| format!("{l} {word}"));
		match joined {
			Some(j) if run_width(m, role, px, &j, 0.0) <= max_w => *lines.last_mut().unwrap() = j,
			_ => lines.push(word.to_string()),
		}
	}
	lines.truncate(max_lines);
	lines
}

/// A cell placed: its grid's index in the page, its index in that grid, its design rect and the circle's size.
pub(super) struct Place {
	pub(super) group: usize,
	pub(super) cell: usize,
	pub(super) rect: Rect,
	pub(super) circle: f32,
}

/// The Gadgets cell height: `GADGET_CELL_H`, smaller when the groups' rows (and the gaps) would not fit the `avail` height.
fn gadget_cell_h(grids: &[&GridFacts], avail: f32) -> f32 {
	let rows: u32 = grids.iter().map(|g| g.rows).sum::<u32>().max(1);
	let gaps = grids.len().saturating_sub(1) as f32 * GROUP_GAP;
	GADGET_CELL_H.min(((avail - gaps) / rows as f32).max(1.0))
}

/// Where every cell goes, flat order, and where each Gadgets group's label goes (empty on the other pages).
fn places(page: Page, grids: &[&GridFacts]) -> (Vec<Place>, Vec<Rect>) {
	let inner = GRID_PANEL.inset(PAD);
	let top = inner.y + 34.0;
	let mut out = Vec::new();
	let mut labels = Vec::new();
	if page == Page::Gadgets {
		let ch = gadget_cell_h(grids, inner.bottom() - top);
		let mut y = top;
		for (gi, g) in grids.iter().enumerate() {
			// A group keeps the old three-column width up to three columns, and fits when it has more.
			let cols = g.cols.max(1);
			let cw = (inner.w - LABEL_W) / cols.max(GADGET_COLS) as f32;
			labels.push(Rect::new(inner.x, y, LABEL_W - 8.0, ch));
			for k in 0..g.cells.len() {
				let (r, c) = ((k as u32 / cols) as f32, (k as u32 % cols) as f32);
				out.push(Place { group: gi, cell: k, rect: Rect::new(inner.x + LABEL_W + c * cw, y + r * ch, cw, ch).inset(3.0), circle: 40.0 });
			}
			y += g.rows as f32 * ch + GROUP_GAP;
		}
		return (out, labels);
	}
	for (gi, g) in grids.iter().enumerate() {
		let ch = CELL_H.min((inner.bottom() - top) / g.rows.max(1) as f32);
		let cw = inner.w / g.cols.max(1) as f32;
		for k in 0..g.cells.len() {
			let (r, c) = ((k as u32 / g.cols) as f32, (k as u32 % g.cols) as f32);
			out.push(Place { group: gi, cell: k, rect: Rect::new(inner.x + c * cw, top + r * ch, cw, ch).inset(3.0), circle: 56.0 });
		}
	}
	(out, labels)
}

/// The item's name under its icon: only for a cell the player has.
fn cell_name(f: &Facts, page: Page, c: &GridCell) -> String {
	if !c.owned {
		return String::new();
	}
	let id = if page == Page::Items { item_entry(c).map_or(0, |e| e.4) } else { crate::inventory::name_text(c.id) };
	(f.text)(id)
}

/// The Items page's `ITEM_GRID` row of a cell (matched by kind and id).
fn item_entry(c: &GridCell) -> Option<&'static (u16, u8, u8, u8, u32, u32)> {
	item_index(c).map(|j| &crate::frontend::ITEM_GRID[j])
}

/// The `ITEM_GRID` index of a cell (also the id of its picture, `icons::grid`).
fn item_index(c: &GridCell) -> Option<usize> {
	crate::frontend::ITEM_GRID.iter().position(|e| e.2 == c.kind && e.3 == c.id)
}

/// A generic icon in a circle (the game's own icon art is not available as vectors): an accent disc with a two-step gem when the
/// player has the item, a dim disc with "?" when not.
pub(super) fn icon(out: &mut Vec<Prim>, c: Rect, owned: bool) {
	if owned {
		fill_b(out, c, c.w / 2.0, ACCENT.tint(0.16), 1.5, ACCENT.tint(0.55));
		let s = c.w * 0.42;
		fill(out, Rect::new(c.cx() - s / 2.0, c.cy() - s / 2.0, s, s), s * 0.28, ACCENT.tint(0.9));
		let t = s * 0.45;
		fill(out, Rect::new(c.cx() - t / 2.0, c.cy() - t / 2.0, t, t), t * 0.3, Rgba::rgba(10, 15, 27, 0.9));
	} else {
		fill_b(out, c, c.w / 2.0, WHITE.tint(0.04), 1.0, HAIRLINE);
		text(out, (c.cx(), c.cy()), FontRole::Condensed, 24.0, TEXT_DISABLED, Align::Center, 0.0, "?");
	}
}

/// The item's own picture (v5i): the HUD sheet frame 0 of a Weapons / Gadgets item, the Items page's own sprite; false when the player
/// does not have the item or the level holds no picture (the caller draws the generic icon).
fn picture(out: &mut Vec<Prim>, page: Page, c: &GridCell, bx: Rect) -> bool {
	if !c.owned {
		return false;
	}
	if page == Page::Items {
		return item_index(c).is_some_and(|j| icons::draw(out, icons::grid(j), bx, WHITE));
	}
	icons::draw_item(out, c.id, bx, WHITE)
}

pub(super) fn draw_cell(out: &mut Vec<Prim>, f: &Facts, page: Page, p: &Place, c: &GridCell, selected: bool) {
	if selected {
		fill_b(out, p.rect, 12.0, ACCENT.tint(0.13), 1.5, ACCENT.tint(0.6));
	} else {
		fill(out, p.rect, 12.0, WHITE.tint(0.03));
	}
	let content = p.circle + 6.0 + 16.0;
	let top = p.rect.y + (p.rect.h - content) * 0.5;
	let bx = Rect::new(p.rect.cx() - p.circle / 2.0, top, p.circle, p.circle);
	if !picture(out, page, c, bx) {
		icon(out, bx, c.owned);
	}
	let name = cell_name(f, page, c);
	if !name.is_empty() {
		text(out, (p.rect.cx(), top + p.circle + 14.0), FontRole::Medium, 13.0, if selected { WHITE } else { TEXT_BODY }, Align::Center, p.rect.w - 8.0, &name);
	}
}

/// The Cross hint line of the page (the game's "Equip" etc.): its plain text.
pub(super) fn cross_hint(f: &Facts) -> Option<String> {
	let t = f.view.items.iter().find_map(|i| match &i.body {
		Body::Hint { button: Btn::Cross, text } => Some(text.plain().trim().to_string()),
		_ => None,
	});
	t.filter(|s| !s.is_empty())
}

/// The accent button of the page: the lead (the Cross glyph in the player's glyph set with a pad, the Enter keycap with the keyboard and
/// mouse) then the label, centred together in `BUTTON`; the same look as the shop's buy button.
pub(super) fn draw_button(out: &mut Vec<Prim>, f: &Facts, m: &dyn TextMeasure, label: &str) {
	let keyboard = f.device == Device::Keyboard;
	let lw = if keyboard { (run_width(m, FontRole::SemiBold, 13.0, keycaps::name(Cap::Enter), 0.0) + 14.0).max(24.0) } else { 24.0 };
	let tw = run_width(m, FontRole::Medium, 17.0, label, 0.0);
	let x0 = BUTTON.cx() - (lw + 10.0 + tw) * 0.5;
	let rect = Rect::new(x0, BUTTON.cy() - 12.0, lw, 24.0);
	fill(out, BUTTON, 12.0, ACCENT);
	if keyboard {
		fill_b(out, rect, 6.0, CAP_BG, 1.0, CAP_BORDER);
		out.push(Prim::Text(TextRun { x: rect.cx(), y: rect.cy(), role: FontRole::SemiBold, px: 13.0, colour: TEXT, align: Align::Center, max_w: 0.0, track: 0.0, text: keycaps::name(Cap::Enter).to_string() }));
	} else {
		out.push(Prim::Glyph { rect, kind: f.glyphs.glyph(Symbol::Cross), colour: symbol_colour(Symbol::Cross) });
	}
	text(out, (x0 + lw + 10.0, BUTTON.cy()), FontRole::Medium, 17.0, INK_ON_ACCENT, Align::Left, BUTTON.w - 24.0 - 10.0 - lw, label);
}

/// The detail panel's texts under the viewport for the selected cell.
fn detail(out: &mut Vec<Prim>, f: &Facts, m: &dyn TextMeasure, page: Page, c: &GridCell) {
	let (x, w) = (VIEWPORT.x, VIEWPORT.w);
	let name = if c.owned { cell_name(f, page, c) } else { "?".to_string() };
	text(out, (x, VIEWPORT.bottom() + 33.0), FontRole::Condensed, 38.0, if c.owned { WHITE } else { TEXT_DISABLED }, Align::Left, w, &name);
	let y = VIEWPORT.bottom() + 74.0;
	if page == Page::Weapons && c.owned {
		match c.ammo {
			Some((n, max)) => {
				fill(out, Rect::new(x, y + 14.0, w, 8.0), 4.0, BAR_OFF);
				fill(out, Rect::new(x, y + 14.0, w * (n as f32 / max.max(1) as f32).clamp(0.0, 1.0), 8.0), 4.0, ACCENT);
				text(out, (x + w, y), FontRole::SemiBold, 18.0, TEXT, Align::Right, 0.0, &format!("{n} / {max}"));
			}
			None => text(out, (x, y), FontRole::Regular, 16.0, TEXT_DIM, Align::Left, w, &(f.text)(T_NO_AMMO)),
		}
	}
	if page != Page::Items {
		return;
	}
	if let Some(e) = c.owned.then(|| item_entry(c)).flatten() {
		// As many 22 px lines as the panel holds above the button (or its bottom): 4 cut retail's longer texts (de Hoverboard).
		let bottom = if cross_hint(f).is_some() { BUTTON.y - 14.0 } else { DETAIL_PANEL.bottom() - 20.0 };
		let max_lines = ((bottom - y) / 22.0).floor().max(0.0) as usize + 1;
		for (i, l) in wrap(m, FontRole::Regular, 16.0, &(f.text)(e.5), w, max_lines).iter().enumerate() {
			text(out, (x, y + i as f32 * 22.0), FontRole::Regular, 16.0, TEXT_BODY, Align::Left, w, l);
		}
	}
	// The gold bolts (Found / Used / Remain) are not repeated here: the preview above is retail's own gold panel, which shows them.
}

/// Retail's selected cell: the cursor of the page's current grid (the first grid when the current item is no grid).
pub(super) fn retail_sel(f: &Facts, groups: &[GridGroup]) -> usize {
	let cur = f.view.cur.and_then(|i| f.view.items.get(i)).map(|i| i.addr);
	let Some(g) = groups.iter().find(|g| Some(g.addr) == cur).or(groups.first()) else { return 0 };
	let cursor = f.view.items.iter().find(|i| i.addr == g.addr).and_then(|i| if let Body::Grid { cursor, .. } = i.body { Some(cursor as usize) } else { None });
	g.first + cursor.unwrap_or(0).min((g.rows * g.cols) as usize - 1)
}

/// The grids of the page in the order the modern page lists them: the worker's facts for the grid items the view shows. None when
/// they do not match (a page flying in, a layout that does not look right): the page then stays classic.
pub(super) fn grids_of<'a>(f: &'a Facts, page: Page) -> Option<Vec<&'a GridFacts>> {
	let shown_here = |g: &&GridFacts| f.view.items.iter().any(|i| i.addr == g.addr && matches!(i.body, Body::Grid { .. }));
	let grids: Vec<&GridFacts> = f.extra.grids.iter().filter(shown_here).collect();
	let want = match page {
		Page::Gadgets => 4,
		Page::QuickSelect => 2,
		_ => 1,
	};
	let sound = grids.len() == want && grids.iter().all(|g| g.rows > 0 && g.cols > 0 && g.cells.len() == (g.rows * g.cols) as usize);
	sound.then_some(grids)
}

/// The modern Weapons / Gadgets / Items page of `f.view`; None = not a grid page or the facts do not look right.
pub fn build_grid(f: &Facts, m: &dyn TextMeasure) -> Option<GridBuilt> {
	build_with(f, m, &Knobs::now())
}

/// `build_grid` with explicit knobs (tests).
pub fn build_with(f: &Facts, m: &dyn TextMeasure, k: &Knobs) -> Option<GridBuilt> {
	let page = f.view.page;
	if page == Page::QuickSelect {
		return super::pause_quick::build(f);
	}
	let tab = match page {
		Page::Weapons => TAB_WEAPONS,
		Page::Gadgets => TAB_GADGETS,
		Page::Items => TAB_ITEMS,
		_ => return None,
	};
	let grids = grids_of(f, page)?;
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
	let (placed, labels) = places(page, &grids);
	let mut out = Vec::new();
	fill_b(&mut out, GRID_PANEL, 16.0, PANEL, 1.0, HAIRLINE);
	let inner = GRID_PANEL.inset(PAD);
	text(&mut out, (inner.x, inner.y + 10.0), FontRole::Condensed, 15.0, TEXT_SECTION, Align::Left, 0.0, &upper(&(f.text)(TAB_IDS[tab])));
	for (i, r) in labels.iter().enumerate() {
		text(&mut out, (r.x, r.cy()), FontRole::Condensed, 16.0, TEXT_MUTED, Align::Left, r.w, &(f.text)(T_GROUPS[i]));
	}
	let mut cells = Vec::new();
	let mut targets = Vec::new();
	let mut chosen = None;
	for (flat, p) in placed.iter().enumerate() {
		let (g, c) = (grids[p.group], &grids[p.group].cells[p.cell]);
		draw_cell(&mut out, f, page, p, c, flat == sel);
		cells.push(p.rect);
		targets.push(Some((g.addr, p.cell as u32)));
		if flat == sel {
			chosen = Some(*c);
		}
	}
	let src = preview_src(page, k);
	let hole = viewport_of(src);
	out.extend(holed_panel(DETAIL_PANEL, hole, 16.0, PANEL, Border { width: 1.0, colour: HAIRLINE }));
	if let Some(c) = chosen {
		detail(&mut out, f, m, page, &c);
	}
	let button = cross_hint(f).map(|label| {
		draw_button(&mut out, f, m, &label);
		BUTTON
	});
	Some(GridBuilt { prims: out, cells, targets, groups, hole, uv: uv_of(src), flat_sel, button, ring: None })
}
