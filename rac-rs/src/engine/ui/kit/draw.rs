//! `Screen` + `Layout` -> `Vec<Prim>` in design space: backdrop, header with tabs, list (draw_list.rs), detail panel
//! (draw_detail.rs), hint bar, peek pill. Pure: no device, no font, no clock.
use super::data::Screen;
use super::layout::{self, Lead, LeadWhat, Layout};
use crate::ui::prim::{Align, Border, FontRole, Prim, Rect, Rgba, TextRun};
use crate::ui::theme::*;

/// Appends a text run.
#[allow(clippy::too_many_arguments)]
pub(super) fn text(out: &mut Vec<Prim>, x: f32, y: f32, role: FontRole, px: f32, colour: Rgba, align: Align, max_w: f32, track: f32, t: &str) {
	out.push(Prim::Text(TextRun { x, y, role, px, colour, align, max_w, track, text: crate::localization::ui_text(t).to_string() }));
}

pub(super) fn rrect(out: &mut Vec<Prim>, rect: Rect, radius: f32, fill: Rgba) {
	out.push(Prim::RRect { rect, radius, fill, border: None });
}

pub(super) fn rrect_b(out: &mut Vec<Prim>, rect: Rect, radius: f32, fill: Rgba, width: f32, colour: Rgba) {
	out.push(Prim::RRect { rect, radius, fill, border: Some(Border { width, colour }) });
}

/// A keycap: rounded box with a hairline border and the key name in the middle.
pub(super) fn keycap(out: &mut Vec<Prim>, rect: Rect, label: &str) {
	rrect_b(out, rect, 6.0, CAP_BG, 1.0, CAP_BORDER);
	text(out, rect.cx(), rect.cy(), FontRole::SemiBold, layout::KEY_PX, TEXT, Align::Center, 0.0, 0.0, label);
}

/// A lead: keycap or pad glyph (letters or shapes by the glyph set).
pub(super) fn lead(out: &mut Vec<Prim>, l: &Lead, s: &Screen) {
	match &l.what {
		LeadWhat::Key(k) => keycap(out, l.rect, k),
		LeadWhat::Pad(sym) => out.push(Prim::Glyph { rect: l.rect, kind: s.glyphs.glyph(*sym), colour: symbol_colour(*sym) }),
	}
}

pub fn prims(s: &Screen, l: &Layout) -> Vec<Prim> {
	let mut out = Vec::new();
	if s.peek {
		out.push(Prim::Backdrop { blur_px: BLUR_PEEK, scrim: SCRIM_PEEK });
		pill(&mut out, s, l);
		return out;
	}
	out.push(Prim::Backdrop { blur_px: BLUR_MENU, scrim: SCRIM_MENU });
	header(&mut out, s, l);
	super::draw_list::list(&mut out, s, l);
	if let Some(d) = &l.detail {
		super::draw_detail::detail(&mut out, s, d);
	}
	hint_bar(&mut out, s, l);
	out
}

fn header(out: &mut Vec<Prim>, s: &Screen, l: &Layout) {
	let h = &l.head;
	text(out, h.title.x, h.title.y, FontRole::SemiBold, layout::TITLE_PX, TEXT_MUTED, Align::Left, 0.0, layout::TITLE_TRACK, &h.title.text);
	keycap(out, h.tab_prev.rect, &h.tab_prev.text);
	for (i, t) in h.tabs.iter().enumerate() {
		let on = i == s.tab;
		let colour = if on { WHITE } else { TEXT_MUTED };
		text(out, t.text.x, t.text.y, FontRole::Condensed, layout::TAB_PX, colour, Align::Left, 0.0, layout::TAB_TRACK, &t.text.text);
		if on {
			rrect(out, t.bar, 2.0, s.accent);
		}
	}
	keycap(out, h.tab_next.rect, &h.tab_next.text);
	if let Some((rect, line)) = &h.notice {
		rrect(out, *rect, rect.h * 0.5, s.accent);
		text(out, line.x, line.y, FontRole::SemiBold, layout::STATUS_PX, INK_ON_ACCENT, Align::Left, 0.0, 0.0, &line.text);
	}
	text(out, h.status.x, h.status.y, FontRole::Regular, layout::STATUS_PX, TEXT_MUTED, Align::Right, 0.0, 0.0, &h.status.text);
}

fn hint_bar(out: &mut Vec<Prim>, s: &Screen, l: &Layout) {
	for h in &l.hints {
		for ld in &h.leads {
			lead(out, ld, s);
		}
		text(out, h.text.x, h.text.y, FontRole::Medium, layout::HINT_PX, TEXT_BODY, Align::Left, 0.0, 0.0, &h.text.text);
	}
}

fn pill(out: &mut Vec<Prim>, s: &Screen, l: &Layout) {
	rrect_b(out, l.pill, l.pill.h * 0.5, PANEL, 1.0, PILL_BORDER);
	for ld in &l.pill_leads {
		lead(out, ld, s);
	}
	text(out, l.pill_text.x, l.pill_text.y, FontRole::SemiBold, layout::PILL_PX, TEXT, Align::Left, 0.0, 0.0, &l.pill_text.text);
}
