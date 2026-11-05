//! The modern planet map and its mission list (v0.5 build 12): `build` turns what the game's pages 0xb (planet map) and 0xc (mission list)
//! show into a draw list in the 1280x720 design space inside the pause shell (tab bar with the Map tab selected), plus what a click on
//! a key row or a hint-bar entry presses. Pure: no statics, no clock. The game stays in charge: it draws its own pictures (the spinning
//! planet, the map, the mission list, the portrait and the description) only into the 512 x 448 HUD overlay texture; the page leaves a
//! HOLE per picture (`Built::holes`, the design-space rect with the overlay uv of the crop, cut by `crop::inset`) where the host puts
//! one overlay quad (`crop::place`). Around the holes the page draws modern panels, the planet's location and name from the texts the
//! game looked up (`retail::map_seen::Seen`), the gold bolts found, and the key list (map stage) or the hint bar (missions stage).
//! The host is written against this file's API; `compose` merges the shell's draw list with the page (the shell's scrim gets holes).
use super::crop::{display_rect, inset, uv_of};
use super::pause_grid::holed_panel;
use super::ship_marks::{name_text, LOCATION_BASE};
use super::ship_pic::CARD_FILL;
use super::title::word;
use super::vendor::{rrect, text};
use crate::retail::map_seen::Seen;
use crate::retail::menu_keys::{MAP, MISSIONS};
use rac_engine::ui::inline;
use rac_engine::ui::input::{Device, GlyphSet};
use rac_engine::ui::kit::page::{MenuButton, PageScreen, Panel};
use rac_engine::ui::kit::shell::{Shell, SHELL_SCRIM};
use rac_engine::ui::kit::text::upper;
use rac_engine::ui::kit::TextMeasure;
use rac_engine::ui::prim::{Align, Border, FontRole, Prim, Rect};
use rac_engine::ui::theme::*;
use rac_engine::ui::T5;

/// Crop sources in the overlay's 512 x 448 pixels (x, y, w, h) [C: menu log, PAL, 2026-11-04].
pub const MAP_SRC: [f32; 4] = [214.0, 80.0, 276.0, 341.0];
pub const GLOBE_SRC: [f32; 4] = [23.0, 81.0, 172.0, 170.0];
pub const LIST_SRC: [f32; 4] = [22.0, 81.0, 223.0, 252.0];
pub const PORTRAIT_SRC: [f32; 4] = [268.0, 81.0, 223.0, 197.0];
pub const DESC_SRC: [f32; 4] = [268.0, 298.0, 222.0, 125.0];
/// Overlay pixels cut off each side of a crop (the retail panel frames stay out).
pub const CROP_INSET: f32 = 3.0;

/// The panels and the boxes the pictures show inside (the hole is the crop's display rect inside its box).
pub const GLOBE_PANEL: Rect = Rect::new(48.0, 116.0, 560.0, 300.0);
pub const GLOBE_BOX: Rect = Rect::new(64.0, 132.0, 240.0, 268.0);
pub const KEYS_PANEL: Rect = Rect::new(48.0, 432.0, 560.0, 224.0);
pub const MAP_PANEL: Rect = Rect::new(632.0, 116.0, 600.0, 540.0);
pub const MAP_BOX: Rect = Rect::new(648.0, 132.0, 568.0, 508.0);
pub const LIST_PANEL: Rect = Rect::new(48.0, 116.0, 560.0, 540.0);
pub const LIST_BOX: Rect = Rect::new(64.0, 160.0, 528.0, 480.0);
pub const PORTRAIT_PANEL: Rect = Rect::new(632.0, 116.0, 600.0, 330.0);
pub const PORTRAIT_BOX: Rect = Rect::new(648.0, 132.0, 568.0, 298.0);
pub const DESC_PANEL: Rect = Rect::new(632.0, 462.0, 600.0, 194.0);
pub const DESC_BOX: Rect = Rect::new(648.0, 478.0, 568.0, 162.0);

/// The text column beside the globe.
const COL_X: f32 = 324.0;
const COL_W: f32 = 264.0;
/// The key list: cells of 264 x 56 in two columns of three, from (64, 448).
const CELL_W: f32 = 264.0;
const CELL_H: f32 = 56.0;
const CELL_PITCH: f32 = 64.0;
const CELL_X: f32 = 64.0;
const CELL_Y: f32 = 448.0;
const KEY_ROWS: usize = 6;
/// The hint bar's centre line and the gap between its entries.
const HINT_CY: f32 = 688.0;
const HINT_GAP: f32 = 26.0;
/// The shell scrim's extent (`pause_grid::holed_scrim`).
const BIG: f32 = 5000.0;
/// Retail's '????' title text id.
const T_UNKNOWN: u32 = 0x4f54;

// Not retail text: the fallback of the unknown planet's title when the game's text is not at hand.
pub const UNKNOWN: T5 = ["????", "????", "????", "????", "????"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
	/// Page 0xb: the planet map.
	Map,
	/// Page 0xc: the mission list.
	Missions,
}

/// The stage of a page id; None = not a map page.
pub fn stage_of(page: u32) -> Option<Stage> {
	match page {
		MAP => Some(Stage::Map),
		MISSIONS => Some(Stage::Missions),
		_ => None,
	}
}

/// What decides whether the modern page is the picture this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gate {
	pub modern: bool,
	/// The modern Options page is up.
	pub ui_open: bool,
	/// The UI pipes were refused (no GPU for them).
	pub refused: bool,
	/// The in-level page on screen, or the one flying in (None = no page menu).
	pub page: Option<u32>,
}

/// The one rule: the modern page is the picture on page 0xb / 0xc, also while it flies in.
pub fn wanted(g: &Gate) -> bool {
	g.modern && !g.ui_open && !g.refused && g.page.and_then(stage_of).is_some()
}

/// One row of the key list: `text` = the button(s) as inline chips followed by the words (built by the glue); `bit` = the pad press a
/// click on the row makes (0 = the row is not clickable, e.g. the sticks row).
#[derive(Clone, Debug, PartialEq)]
pub struct KeyRow {
	pub text: String,
	pub bit: u16,
}

/// Everything `build` reads.
pub struct Facts<'a> {
	pub stage: Stage,
	/// The planet the map's titles name (None = not known yet: the column stays empty).
	pub seen: Option<Seen>,
	/// (found, total) gold bolts of the shown planet; None = not shown.
	pub gold: Option<(u32, u32)>,
	/// A retail text id as plain text in the player's language ("" = unknown).
	pub text: &'a dyn Fn(u32) -> String,
	/// Map stage: the key rows in order (normally 6); Missions stage: the Exit row (1). May be empty.
	pub keys: &'a [KeyRow],
	pub shell: Shell,
	/// The tab bar's words, the Map tab last.
	pub tabs: &'a [String],
	pub lang: usize,
	pub device: Device,
	pub glyphs: GlyphSet,
	/// The retail page has landed (not flying in / closing): the holes for the crops are made.
	pub settled: bool,
}

/// The page and what its parts do.
#[derive(Clone, Debug, PartialEq)]
pub struct Built {
	/// The pause shell with the tabs (the tabs are hit-tested by the host, so none is live).
	pub screen: PageScreen,
	/// The page's own panels and texts, drawn after the shell.
	pub prims: Vec<Prim>,
	/// The crops: (design-space hole, overlay uv [u0,v0,u1,v1]); empty unless `settled`.
	pub holes: Vec<(Rect, [f32; 4])>,
	/// Clickable rects with the pad bit they press.
	pub acts: Vec<(u16, Rect)>,
	pub stage: Stage,
	/// Index of the Map tab in `screen.menu` (tabs.len() - 1, 0 when empty).
	pub map_tab: usize,
}

impl Built {
	/// The pad bit a click at design point (x, y) presses: the first act whose rect contains it.
	pub fn act_at(&self, x: f32, y: f32) -> Option<u16> {
		self.acts.iter().find(|a| a.1.contains(x, y)).map(|a| a.0)
	}
}

/// The shell of the page: the header texts and the tab bar, the last tab (Map) selected.
fn screen_of(f: &Facts) -> PageScreen {
	let last = f.tabs.len().saturating_sub(1);
	let menu = f.tabs.iter().enumerate().map(|(i, t)| MenuButton { text: t.clone(), selected: i == last, live: false }).collect();
	PageScreen { menu, panel: Panel::None, shell: Some(f.shell.clone()), device: f.device, glyphs: f.glyphs, ..PageScreen::default() }
}

/// A panel around the picture of `src` shown inside `bx`: with a settled page a panel with the hole (and the hole is recorded),
/// else a plain panel with the empty card in the box.
fn picture_panel(b: &mut Built, settled: bool, panel: Rect, bx: Rect, src: [f32; 4]) {
	let cut = inset(src, CROP_INSET);
	let hole = display_rect(bx, cut);
	if settled {
		b.prims.extend(holed_panel(panel, hole, 16.0, PANEL, Border { width: 1.0, colour: HAIRLINE }));
		b.holes.push((hole, uv_of(cut)));
	} else {
		rrect(&mut b.prims, panel, 16.0, PANEL, Some((1.0, HAIRLINE)));
		rrect(&mut b.prims, bx, 12.0, CARD_FILL, None);
	}
}

/// The location text of the shown planet, upper-cased ("" = none).
fn location_of(f: &Facts, id: u32) -> String {
	upper(&(f.text)(LOCATION_BASE + id))
}

/// The planet's name text ("" = none).
fn name_of(f: &Facts, id: u32) -> String {
	name_text(id).map(|t| (f.text)(t)).unwrap_or_default()
}

/// The '????' title of a planet the game hides.
fn unknown_name(f: &Facts) -> String {
	Some((f.text)(T_UNKNOWN)).filter(|s| !s.is_empty()).unwrap_or_else(|| word(&UNKNOWN, f.lang).to_string())
}

/// The map stage's text column beside the globe: location, name and the gold bolts found.
fn column(b: &mut Built, f: &Facts) {
	let Some(seen) = f.seen else { return };
	let out = &mut b.prims;
	if let Some(id) = seen.planet {
		let loc = location_of(f, id);
		if !loc.is_empty() {
			text(out, COL_X, 164.0, FontRole::SemiBold, 13.0, TEXT_MUTED, Align::Left, COL_W, 0.14, &loc);
		}
		text(out, COL_X, 204.0, FontRole::Condensed, 34.0, WHITE, Align::Left, COL_W, 0.0, &name_of(f, id));
		if let Some((found, total)) = f.gold {
			let s = word(&super::ship::OF, f.lang).replacen("{}", &found.to_string(), 1).replacen("{}", &total.to_string(), 1);
			text(out, COL_X, 252.0, FontRole::Regular, 15.0, TEXT_MUTED, Align::Left, COL_W, 0.0, word(&super::ship::GOLD, f.lang));
			text(out, COL_X, 276.0, FontRole::SemiBold, 18.0, TEXT_BODY, Align::Left, COL_W, 0.0, &s);
		}
	} else if seen.unknown {
		text(out, COL_X, 204.0, FontRole::Condensed, 34.0, WHITE, Align::Left, COL_W, 0.0, &unknown_name(f));
	}
}

/// The key list: up to six rows in two columns of three; a row with a pad bit is clickable.
fn key_list(b: &mut Built, f: &Facts) {
	rrect(&mut b.prims, KEYS_PANEL, 16.0, PANEL, Some((1.0, HAIRLINE)));
	for (i, k) in f.keys.iter().take(KEY_ROWS).enumerate() {
		let (col, row) = ((i / 3) as f32, (i % 3) as f32);
		let cell = Rect::new(CELL_X + col * CELL_W, CELL_Y + row * CELL_PITCH, CELL_W, CELL_H);
		text(&mut b.prims, cell.x + 16.0, cell.cy(), FontRole::Medium, 16.0, TEXT_BODY, Align::Left, cell.w - 24.0, 0.0, &k.text);
		if k.bit != 0 {
			b.acts.push((k.bit, cell));
		}
	}
}

fn map_stage(b: &mut Built, f: &Facts) {
	picture_panel(b, f.settled, GLOBE_PANEL, GLOBE_BOX, GLOBE_SRC);
	column(b, f);
	key_list(b, f);
	picture_panel(b, f.settled, MAP_PANEL, MAP_BOX, MAP_SRC);
}

/// The caption line of the list panel: the location at the left, the planet's name at the right.
fn caption(b: &mut Built, f: &Facts) {
	let Some(seen) = f.seen else { return };
	let out = &mut b.prims;
	if let Some(id) = seen.planet {
		let loc = location_of(f, id);
		if !loc.is_empty() {
			text(out, 64.0, 142.0, FontRole::SemiBold, 13.0, TEXT_MUTED, Align::Left, 300.0, 0.14, &loc);
		}
		text(out, 592.0, 142.0, FontRole::SemiBold, 15.0, TEXT_BODY, Align::Right, 220.0, 0.0, &name_of(f, id));
	} else if seen.unknown {
		text(out, 592.0, 142.0, FontRole::SemiBold, 15.0, TEXT_BODY, Align::Right, 220.0, 0.0, &unknown_name(f));
	}
}

/// The hint bar along the bottom, right aligned to 1232: one entry per key row.
fn hint_bar(b: &mut Built, f: &Facts, m: &dyn TextMeasure) {
	if f.keys.is_empty() {
		return;
	}
	let widths: Vec<f32> = f.keys.iter().map(|k| inline::width(m, FontRole::Medium, 15.0, &k.text)).collect();
	let mut x = 1232.0 - widths.iter().sum::<f32>() - HINT_GAP * (f.keys.len() - 1) as f32;
	for (k, w) in f.keys.iter().zip(&widths) {
		text(&mut b.prims, x, HINT_CY, FontRole::Medium, 15.0, TEXT_BODY, Align::Left, 0.0, 0.0, &k.text);
		if k.bit != 0 {
			b.acts.push((k.bit, Rect::new(x - 6.0, HINT_CY - 16.0, w + 12.0, 32.0)));
		}
		x += w + HINT_GAP;
	}
}

fn missions_stage(b: &mut Built, f: &Facts, m: &dyn TextMeasure) {
	picture_panel(b, f.settled, LIST_PANEL, LIST_BOX, LIST_SRC);
	caption(b, f);
	picture_panel(b, f.settled, PORTRAIT_PANEL, PORTRAIT_BOX, PORTRAIT_SRC);
	picture_panel(b, f.settled, DESC_PANEL, DESC_BOX, DESC_SRC);
	hint_bar(b, f, m);
}

/// The page of `f`.
pub fn build(f: &Facts, m: &dyn TextMeasure) -> Built {
	let mut b = Built { screen: screen_of(f), prims: Vec::new(), holes: Vec::new(), acts: Vec::new(), stage: f.stage, map_tab: f.tabs.len().saturating_sub(1) };
	match f.stage {
		Stage::Map => map_stage(&mut b, f),
		Stage::Missions => missions_stage(&mut b, f, m),
	}
	b
}

/// The shell's scrim (`SHELL_SCRIM`, radius 0 RRects) over the whole design plane (-5000..5000 both ways) minus `holes`
/// (non-overlapping rects): horizontal bands between all the holes' top and bottom edges, each band cut around the holes that cross it.
/// Empty input = one rect.
pub fn scrim_holes(holes: &[Rect]) -> Vec<Prim> {
	let mut ys = vec![-BIG, BIG];
	for h in holes {
		ys.extend([h.y.clamp(-BIG, BIG), h.bottom().clamp(-BIG, BIG)]);
	}
	ys.sort_by(|a, b| a.total_cmp(b));
	ys.dedup();
	let mut out = Vec::new();
	let mut cut = |x: f32, y: f32, w: f32, h: f32| {
		if w > 0.0 && h > 0.0 {
			out.push(Prim::RRect { rect: Rect::new(x, y, w, h), radius: 0.0, fill: SHELL_SCRIM, border: None });
		}
	};
	for band in ys.windows(2) {
		let (y0, y1) = (band[0], band[1]);
		let mut cross: Vec<&Rect> = holes.iter().filter(|h| h.y < y1 && h.bottom() > y0).collect();
		cross.sort_by(|a, b| a.x.total_cmp(&b.x));
		let mut x = -BIG;
		for h in cross {
			cut(x, y0, h.x - x, y1 - y0);
			x = x.max(h.right());
		}
		cut(x, y0, BIG - x, y1 - y0);
	}
	out
}

/// The final draw list: `shell` = what `page_draw::frame(&b.screen, m)` gave; its first RRect whose fill is `SHELL_SCRIM` is replaced by
/// `scrim_holes` of the holes' rects (only when there are holes), then the page's own prims are appended.
pub fn compose(shell: Vec<Prim>, b: &Built) -> Vec<Prim> {
	let mut out = shell;
	if !b.holes.is_empty() {
		if let Some(at) = out.iter().position(|p| matches!(p, Prim::RRect { fill, .. } if *fill == SHELL_SCRIM)) {
			let rects: Vec<Rect> = b.holes.iter().map(|h| h.0).collect();
			out.splice(at..=at, scrim_holes(&rects));
		}
	}
	out.extend(b.prims.iter().cloned());
	out
}

/// Every table, for the font-coverage test.
pub fn all() -> [&'static T5; 1] {
	[&UNKNOWN]
}
