//! Draw the detail panel from its layout.
use super::data::Screen;
use super::draw::{lead, rrect, rrect_b, text};
use super::layout::{self, Line};
use super::layout_detail::{self as ld, DetailLayout, Pair};
use crate::ui::prim::{Align, FontRole, Prim, Rect};
use crate::ui::theme::*;

pub(super) fn detail(out: &mut Vec<Prim>, s: &Screen, d: &DetailLayout) {
	rrect_b(out, d.panel, 16.0, PANEL, 1.0, HAIRLINE);
	text(out, d.path.x, d.path.y, FontRole::SemiBold, ld::PATH_PX, TEXT_SECTION, Align::Left, ld::W, ld::PATH_TRACK, &d.path.text);
	for t in &d.title {
		text(out, t.x, t.y, FontRole::Condensed, ld::TITLE_PX, WHITE, Align::Left, ld::W, 0.0, &t.text);
	}
	if let Some(p) = &d.now {
		pair(out, p, s.accent);
	}
	if let Some(p) = &d.default {
		pair(out, p, TEXT);
	}
	for l in &d.body {
		text(out, l.x, l.y, FontRole::Regular, ld::BODY_PX, TEXT_BODY, Align::Left, ld::W, 0.0, &l.text);
	}
	for l in &d.note {
		text(out, l.x, l.y, FontRole::Regular, ld::SMALL_PX, TEXT_MUTED, Align::Left, ld::W, 0.0, &l.text);
	}
	if let Some(b) = &d.boxed {
		rrect(out, b.rect, 12.0, BOX_BG);
		for l in &b.lines {
			text(out, l.x, l.y, FontRole::Regular, ld::SMALL_PX, TEXT_BOX, Align::Left, ld::W - 32.0, 0.0, &l.text);
		}
		if b.button.w > 0.0 {
			button(out, b.button, &b.button_text, s);
		}
	}
	if let Some(m) = &d.meter {
		rrect(out, m.sep, 0.0, HAIRLINE);
		for l in &m.label {
			label(out, l, m.label_w);
		}
		for (i, b) in m.bars.iter().enumerate() {
			rrect(out, *b, 2.0, if (i as u8) < m.level { s.accent } else { BAR_OFF });
		}
		text(out, m.text.right(), m.text.cy(), FontRole::SemiBold, ld::SMALL_PX, TEXT, Align::Right, 0.0, 0.0, &m.word);
	}
	if let Some(f) = &d.footer {
		rrect(out, f.sep, 0.0, HAIRLINE);
		for l in &f.note {
			label(out, l, f.note_w);
		}
		rrect(out, f.button, 8.0, CHIP_BG);
		for l in &f.leads {
			lead(out, l, s);
		}
		text(out, f.text.x, f.text.y, FontRole::SemiBold, ld::SMALL_PX, TEXT, Align::Left, 0.0, 0.0, &f.text.text);
	}
}

/// A small muted label in a box `max_w` wide (the renderer shrinks a wider one; `fit::overflows` reports it).
fn label(out: &mut Vec<Prim>, l: &Line, max_w: f32) {
	text(out, l.x, l.y, FontRole::Regular, ld::SMALL_PX, TEXT_MUTED, Align::Left, max_w, 0.0, &l.text);
}

/// The Now / Default pair: label and value each boxed to the column's right edge (at least 1, so one past it is still reported).
fn pair(out: &mut Vec<Prim>, p: &Pair, value_colour: crate::ui::prim::Rgba) {
	let room = |x: f32| (ld::X0 + ld::W - x).max(1.0);
	label(out, &p.label, room(p.label.x));
	text(out, p.value.x, p.value.y, FontRole::SemiBold, ld::SMALL_PX, value_colour, Align::Left, room(p.value.x), 0.0, &p.value.text);
}

fn button(out: &mut Vec<Prim>, rect: Rect, t: &Line, s: &Screen) {
	rrect(out, rect, 8.0, s.accent);
	text(out, t.x, t.y, FontRole::SemiBold, layout::CHIP_PX, INK_ON_ACCENT, Align::Center, 0.0, 0.0, &t.text);
}
