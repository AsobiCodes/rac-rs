//! The Help tab of the modern pause menu (v0.5.1, lane kn): the game's own Help rows (Help Log, Controls, Moves, Weapons, Gadgets in the
//! disc's language, locked ones dimmed) as a list panel, and a detail panel like the grid pages' (mockup `MOCKUP_pause.dc.html`, HELP):
//! the row's name, the game's own one-line description (`retail::help_seen`, read by the worker into `Extra::help_desc`) and the picture
//! retail draws for the row (the Help page's picture panel, cropped out of the overlay texture exactly as the grid pages' previews are
//! (`crop.rs`); the host places it, `pause_host/grid.rs::place_preview`). Choosing a row opens retail's own sub page (classic look
//! inside, v0.5.1); Back returns here. The page is a one-column `GridBuilt`, so keys, hover, click and the action button are the grid
//! pages' (`pause_host/grid.rs`). Pure: no statics, no clock.
use super::crop;
use super::pause::{list, Facts, TAB_IDS};
use super::pause_grid::{cross_hint, draw_button, fill_b, holed_panel, text, uv_of, wrap, GridBuilt, GridGroup, BUTTON, DETAIL_PANEL, GRID_PANEL, PAD};
use super::title::shown;
use rac_engine::ui::kit::text::{upper, TextMeasure};
use rac_engine::ui::prim::{Align, Border, FontRole, Rect};
use rac_engine::ui::theme::*;

/// The tab (index in `TAB_IDS`) of the Help page.
const TAB_HELP: usize = 4;
/// The Help page's picture panel in the 512 x 448 overlay picture (x, y, w, h; `frontend.rs` layout of `Page::Help`), cut down by
/// `FRAME` pixels per side so retail's own panel frame stays out.
const PICTURE: [f32; 4] = [336.0, 134.0, 155.0, 148.0];
const FRAME: f32 = 3.0;
/// Where the picture shows inside the detail panel (design space), under the description.
const VIEW: Rect = Rect::new(672.0, 318.0, 536.0, 258.0);
const ROW_H: f32 = 58.0;
const DESC_LINES: usize = 4;

/// The crop of the overlay picture the detail panel shows.
pub(super) fn picture_src() -> [f32; 4] {
	crop::inset(PICTURE, FRAME)
}

/// The modern Help page of `f.view`; None when the page has no list the facts fit (it then stays the plain list).
pub(super) fn build(f: &Facts, m: &dyn TextMeasure) -> Option<GridBuilt> {
	let l = list(f.view).filter(|l| !l.rows.is_empty() && l.rows.len() == l.n)?;
	let labels: Vec<String> = l.rows.iter().map(|r| r.text.plain().trim().to_string()).collect();
	if labels.iter().any(|s| s.is_empty()) {
		return None;
	}
	let sel = shown(f.local, l.cursor, f.now).min(labels.len() - 1);
	let mut out = Vec::new();
	fill_b(&mut out, GRID_PANEL, 16.0, PANEL, 1.0, HAIRLINE);
	let inner = GRID_PANEL.inset(PAD);
	text(&mut out, (inner.x, inner.y + 10.0), FontRole::Condensed, 15.0, TEXT_SECTION, Align::Left, 0.0, &upper(&(f.text)(TAB_IDS[TAB_HELP])));
	let top = inner.y + 34.0;
	let mut cells = Vec::new();
	let mut targets = Vec::new();
	for (i, label) in labels.iter().enumerate() {
		let r = Rect::new(inner.x, top + i as f32 * (ROW_H + 4.0), inner.w, ROW_H);
		let enabled = l.rows[i].enabled;
		if i == sel {
			fill_b(&mut out, r, 12.0, ACCENT.tint(0.13), 1.5, ACCENT.tint(0.6));
		}
		let colour = if !enabled { TEXT_DISABLED } else if i == sel { WHITE } else { TEXT_BODY };
		text(&mut out, (r.x + 20.0, r.cy()), FontRole::Medium, 20.0, colour, Align::Left, r.w - 40.0, label);
		cells.push(r);
		targets.push(Some((l.addr, i as u32)));
	}
	let src = picture_src();
	let hole = crop::display_rect(VIEW, src);
	out.extend(holed_panel(DETAIL_PANEL, hole, 16.0, PANEL, Border { width: 1.0, colour: HAIRLINE }));
	let (x, w) = (VIEW.x, VIEW.w);
	text(&mut out, (x, DETAIL_PANEL.y + 53.0), FontRole::Condensed, 38.0, if l.rows[sel].enabled { WHITE } else { TEXT_DISABLED }, Align::Left, w, &labels[sel]);
	if let Some(desc) = f.extra.help_desc.map(|id| (f.text)(id)).filter(|s| !s.is_empty()) {
		for (i, line) in wrap(m, FontRole::Regular, 17.0, &desc, w, DESC_LINES).iter().enumerate() {
			text(&mut out, (x, DETAIL_PANEL.y + 98.0 + i as f32 * 24.0), FontRole::Regular, 17.0, TEXT_BODY, Align::Left, w, line);
		}
	}
	// A thin frame round the picture (the panel's fill is cut out around it).
	fill_b(&mut out, hole, 0.0, WHITE.tint(0.0), 1.0, HAIRLINE);
	let button = cross_hint(f).map(|label| {
		draw_button(&mut out, f, m, &label);
		BUTTON
	});
	let groups = vec![GridGroup { addr: l.addr, rows: labels.len() as u32, cols: 1, first: 0 }];
	Some(GridBuilt { prims: out, cells, targets, groups, hole, uv: uv_of(src), flat_sel: l.cursor, button, ring: None })
}
