//! The pause shell of the page screen (v0.5 screens stage C): the approved pause mockup's header. `PageScreen::shell = Some(..)` turns
//! the left menu column into a tab bar across the top (label above it, site name and bolts chip at the right), moves the panel to the
//! mockup's list position (48, 116) and darkens the whole scene evenly instead of the title's left scrim. Plain data in, rectangles out
//! (`tab_boxes`, `shell_box`, called by `page::layout`), prims in `draw`. Nothing here knows a game.
use super::draw::{rrect, rrect_b, text};
use super::layout::Line;
use super::page::{MenuBox, PageLayout, PageScreen};
use super::text::{run_width, upper, TextMeasure};
use crate::ui::prim::{Align, FontRole, Prim, Rect, Rgba};
use crate::ui::theme::*;

pub const TAB_PX: f32 = 25.0;
pub const TAB_MIN_PX: f32 = 15.0;
pub const TAB_TRACK: f32 = 0.05;
pub const TAB_PAD: f32 = 11.0;
pub const TAB_Y: f32 = 61.0;
pub const TAB_H: f32 = 41.0;
/// The list panel's width and the header's right edge.
pub const SHELL_ROWS_W: f32 = 520.0;
pub const SHELL_RIGHT: f32 = 1232.0;
pub const SHELL_SCRIM: Rgba = Rgba::rgba(5, 8, 15, 0.45);
const LABEL_PX: f32 = 13.0;
const SITE_PX: f32 = 22.0;
const CHIP_H: f32 = 40.0;
const CHIP_PAD: f32 = 14.0;

/// The header texts: the label over the tabs ("PAUSED"), the site name with its small label, and the bolts figure.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Shell {
	pub label: String,
	pub site_label: String,
	pub site: String,
	pub bolts: String,
}

/// The header placed.
#[derive(Clone, Debug, PartialEq)]
pub struct ShellBox {
	pub label: Line,
	pub site_label: Line,
	pub site: Line,
	pub bolts: Line,
	pub chip: Rect,
	/// The font size the tabs got (shrunk when the tabs would not fit beside the site block).
	pub tab_px: f32,
}

fn line(text: &str, x: f32, y: f32) -> Line {
	Line { text: text.to_string(), x, y }
}

/// The right-hand block: (chip rect, site block's left edge).
fn right_block(sh: &Shell, m: &dyn TextMeasure) -> (Rect, f32) {
	let chip_w = run_width(m, FontRole::Condensed, SITE_PX, &sh.bolts, 0.0) + 2.0 * CHIP_PAD;
	let chip = Rect::new(SHELL_RIGHT - chip_w, 50.0, chip_w, CHIP_H);
	let site_w = run_width(m, FontRole::Condensed, SITE_PX, &sh.site, 0.0).max(run_width(m, FontRole::SemiBold, LABEL_PX, &upper(&sh.site_label), 0.14));
	(chip, chip.x - 22.0 - site_w)
}

fn tabs_width(s: &PageScreen, m: &dyn TextMeasure, px: f32) -> f32 {
	s.menu.iter().map(|b| run_width(m, FontRole::Condensed, px, &upper(&b.text), TAB_TRACK) + 2.0 * TAB_PAD).sum()
}

/// The tabs left to right from x 48, at `TAB_PX` or smaller when they would run into the site block; and the size used.
pub fn tab_boxes(s: &PageScreen, sh: &Shell, m: &dyn TextMeasure) -> (Vec<MenuBox>, f32) {
	let avail = right_block(sh, m).1 - 24.0 - 48.0;
	let full = tabs_width(s, m, TAB_PX);
	let px = if full <= avail { TAB_PX } else { (TAB_PX * avail / full).max(TAB_MIN_PX) };
	let mut x = 48.0;
	let boxes = s
		.menu
		.iter()
		.map(|b| {
			let t = upper(&b.text);
			let w = run_width(m, FontRole::Condensed, px, &t, TAB_TRACK) + 2.0 * TAB_PAD;
			let rect = Rect::new(x, TAB_Y, w, TAB_H);
			x += w;
			MenuBox { rect, bar: Rect::new(rect.x + TAB_PAD, rect.bottom() - 3.0, w - 2.0 * TAB_PAD, 3.0), text: line(&t, rect.x + TAB_PAD, rect.y + 4.0 + px * 0.6) }
		})
		.collect();
	(boxes, px)
}

pub fn shell_box(sh: &Shell, m: &dyn TextMeasure, tab_px: f32) -> ShellBox {
	let chip = right_block(sh, m).0;
	let right = chip.x - 22.0;
	ShellBox {
		label: line(&upper(&sh.label), 48.0, 40.0),
		site_label: line(&upper(&sh.site_label), right, 58.0),
		site: line(&sh.site, right, 80.0),
		bolts: line(&sh.bolts, chip.x + CHIP_PAD, chip.cy()),
		chip,
		tab_px,
	}
}

/// The tab bar and header as prims (before the panel).
pub fn draw(out: &mut Vec<Prim>, s: &PageScreen, l: &PageLayout, sb: &ShellBox) {
	rrect(out, Rect::new(-5000.0, -5000.0, 10000.0, 10000.0), 0.0, SHELL_SCRIM);
	text(out, sb.label.x, sb.label.y, FontRole::SemiBold, LABEL_PX, TEXT_MUTED, Align::Left, 0.0, 0.18, &sb.label.text);
	for (b, bx) in s.menu.iter().zip(&l.menu) {
		let colour = if b.selected { WHITE } else { TEXT_MUTED };
		text(out, bx.text.x, bx.text.y, FontRole::Condensed, sb.tab_px, colour, Align::Left, bx.rect.w - 2.0 * TAB_PAD + 2.0, TAB_TRACK, &bx.text.text);
		if b.selected {
			rrect(out, bx.bar, 2.0, s.accent);
		}
	}
	text(out, sb.site_label.x, sb.site_label.y, FontRole::SemiBold, LABEL_PX, TEXT_MUTED, Align::Right, 0.0, 0.14, &sb.site_label.text);
	text(out, sb.site.x, sb.site.y, FontRole::Condensed, SITE_PX, WHITE, Align::Right, 0.0, 0.0, &sb.site.text);
	rrect_b(out, sb.chip, 10.0, PANEL, 1.0, HAIRLINE);
	text(out, sb.bolts.x, sb.bolts.y, FontRole::Condensed, SITE_PX, WHITE, Align::Left, 0.0, 0.0, &sb.bolts.text);
}
