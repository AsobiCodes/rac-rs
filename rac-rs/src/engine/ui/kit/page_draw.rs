//! `PageScreen` + `PageLayout` -> `Vec<Prim>` in design space (see `page.rs`). No `Prim::Backdrop`: the scene behind stays sharp, a
//! left scrim darkens it under the menu column. Pure: no device, no font, no clock.
use super::draw::{keycap, rrect, rrect_b, text};
use super::layout::{Lead, LeadWhat, Line};
use super::page::*;
use crate::ui::prim::{Align, FontRole, Prim, Rect, Rgba};
use crate::ui::theme::*;

/// The scrim alpha at design x: 0.88 at 0, 0.55 at 576, 0 at 960 (linear between).
pub fn scrim_alpha(x: f32) -> f32 {
	let [a0, a1, a2] = SCRIM_ALPHA;
	if x <= 576.0 {
		a0 + (a1 - a0) * (x / 576.0).max(0.0)
	} else {
		(a1 + (a2 - a1) * ((x - 576.0) / 384.0)).max(0.0)
	}
}

fn lead(out: &mut Vec<Prim>, l: &Lead, s: &PageScreen) {
	match &l.what {
		LeadWhat::Key(k) => keycap(out, l.rect, k),
		LeadWhat::Pad(sym) => out.push(Prim::Glyph { rect: l.rect, kind: s.glyphs.glyph(*sym), colour: symbol_colour(*sym) }),
	}
}

#[allow(clippy::too_many_arguments)]
fn run(out: &mut Vec<Prim>, ln: &Line, role: FontRole, px: f32, colour: Rgba, align: Align, max_w: f32, track: f32) {
	text(out, ln.x, ln.y, role, px, colour, align, max_w, track, &ln.text);
}

fn select_look(out: &mut Vec<Prim>, rect: Rect, radius: f32, selected: bool, idle: Rgba, accent: Rgba) {
	if selected {
		rrect_b(out, rect, radius, accent.tint(0.13), 1.5, accent.tint(0.6));
	} else {
		rrect(out, rect, radius, idle);
	}
}

fn menu(out: &mut Vec<Prim>, s: &PageScreen, l: &PageLayout) {
	for (b, bx) in s.menu.iter().zip(&l.menu) {
		select_look(out, bx.rect, 12.0, b.selected, Rgba::rgba(10, 15, 27, 0.55), s.accent);
		let bar = if b.selected { s.accent } else { WHITE.tint(0.18) };
		rrect(out, bx.bar, 2.0, bar);
		let colour = if b.selected && b.live { WHITE } else if b.selected { TEXT_BODY } else { TEXT_MUTED };
		run(out, &bx.text, FontRole::Condensed, MENU_PX, colour, Align::Left, MENU_W - 38.0 - 16.0, MENU_TRACK);
	}
}

fn item(out: &mut Vec<Prim>, r: &PageRow, b: &ItemBox, accent: Rgba) {
	select_look(out, b.rect, if b.card { 12.0 } else { 10.0 }, r.selected, if b.card { WHITE.tint(0.03) } else { Rgba::rgba(255, 255, 255, 0.0) }, accent);
	let (title, body) = if r.dim { (TEXT_DISABLED, TEXT_DISABLED) } else { (WHITE, TEXT_BODY) };
	if b.card {
		run(out, &b.label, FontRole::Condensed, CARD_PX, title, Align::Left, b.label_w, 0.0);
		for (lab, val) in &b.stats {
			run(out, lab, FontRole::Regular, STAT_LABEL_PX, TEXT_DIM, Align::Right, 0.0, 0.0);
			run(out, val, FontRole::SemiBold, STAT_VALUE_PX, body, Align::Right, 0.0, 0.0);
		}
	} else {
		run(out, &b.label, FontRole::Medium, ROW_LABEL_PX, if r.dim { TEXT_DISABLED } else { TEXT }, Align::Left, b.label_w, 0.0);
		if let Some(v) = &b.value {
			run(out, v, FontRole::SemiBold, ROW_VALUE_PX, body, Align::Right, 0.0, 0.0);
		}
		if let (Some(t), Some(f)) = (b.bar, r.bar) {
			rrect(out, t, 4.0, WHITE.tint(0.12));
			rrect(out, Rect::new(t.x, t.y, t.w * f.clamp(0.0, 1.0), t.h), 4.0, if r.dim { TEXT_DISABLED } else { accent });
		}
	}
}

fn panel(out: &mut Vec<Prim>, s: &PageScreen, p: &PanelBox) {
	rrect_b(out, p.rect, 16.0, PANEL, 1.0, HAIRLINE);
	run(out, &p.caption, FontRole::Condensed, CAPTION_PX, TEXT_SECTION, Align::Left, 0.0, CAPTION_TRACK);
	if let Some(side) = &p.side {
		run(out, side, FontRole::Regular, SIDE_PX, TEXT_DIM, Align::Right, 0.0, 0.0);
	}
	let rows: Vec<&PageRow> = match &s.panel {
		Panel::Cards { cards, extra, .. } => cards.iter().chain(extra.iter()).collect(),
		Panel::Rows { rows, .. } => rows.iter().collect(),
		Panel::None => Vec::new(),
	};
	for (r, b) in rows.into_iter().zip(&p.items) {
		item(out, r, b, s.accent);
	}
}

fn dialog(out: &mut Vec<Prim>, s: &PageScreen, d: &DialogBox) {
	rrect(out, Rect::new(-5000.0, -5000.0, 10000.0, 10000.0), 0.0, DIALOG_SCRIM);
	rrect_b(out, d.rect, 16.0, PANEL, 1.0, PILL_BORDER);
	for ln in &d.lines {
		run(out, ln, FontRole::Medium, DIALOG_PX, TEXT, Align::Center, DIALOG_W - 56.0, 0.0);
	}
	for b in &d.buttons {
		rrect_b(out, b.rect, 12.0, BUTTON_BG, 1.0, HAIRLINE);
		lead(out, &b.lead, s);
		run(out, &b.text, FontRole::Medium, BUTTON_PX, TEXT, Align::Left, 0.0, 0.0);
	}
}

pub fn prims(s: &PageScreen, l: &PageLayout) -> Vec<Prim> {
	let mut out = Vec::new();
	if let Some(sb) = &l.shell {
		super::shell::draw(&mut out, s, l, sb);
	} else {
		if !s.menu.is_empty() {
			for i in 0..32 {
				let (x, w) = (i as f32 * 30.0, 30.0);
				let a = scrim_alpha(x + w * 0.5);
				let (x, w) = if i == 0 { (-5000.0, w + 5000.0) } else { (x, w) };
				out.push(Prim::RRect { rect: Rect::new(x, -5000.0, w, 10000.0), radius: 0.0, fill: SCRIM_COLOUR.tint(a), border: None });
			}
		}
		menu(&mut out, s, l);
	}
	if let Some(p) = &l.panel {
		panel(&mut out, s, p);
	}
	for h in &l.hints {
		for ld in &h.leads {
			lead(&mut out, ld, s);
		}
		run(&mut out, &h.text, FontRole::Medium, 15.0, TEXT_BODY, Align::Left, 0.0, 0.0);
	}
	if let Some(f) = &l.footer {
		run(&mut out, f, FontRole::Regular, 13.0, TEXT_DIM, Align::Left, 0.0, 0.0);
	}
	if let Some(d) = &l.dialog {
		dialog(&mut out, s, d);
	}
	out
}

/// Layout and draw list of a page in one call.
pub fn frame(s: &PageScreen, m: &dyn super::TextMeasure) -> (PageLayout, Vec<Prim>) {
	let l = layout(s, m);
	let p = prims(s, &l);
	(l, p)
}
