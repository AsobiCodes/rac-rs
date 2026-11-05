//! Draw the scrolling list: panel, clipped rows (label + control), section titles, banner, scrollbar.
use super::data::{Chip, Control, Item, RowData, Screen};
use super::draw::{rrect, rrect_b, text};
use super::layout::{self, CtrlBox, ItemBox, Layout, THUMB_R};
use crate::ui::prim::{Align, FontRole, Prim, Rect, Rgba};
use crate::ui::theme::*;

pub(super) fn list(out: &mut Vec<Prim>, s: &Screen, l: &Layout) {
	rrect_b(out, l.list, 16.0, PANEL, 1.0, HAIRLINE);
	out.push(Prim::ClipPush { rect: l.viewport, radius: 15.0 });
	for (i, (item, boxed)) in s.items.iter().zip(&l.items).enumerate() {
		if !boxed.rect().overlaps(&l.viewport) {
			continue;
		}
		match (item, boxed) {
			(Item::Section(_), ItemBox::Section { text: t, .. }) => {
				text(out, t.x, t.y, FontRole::Condensed, layout::SECTION_PX, TEXT_SECTION, Align::Left, 0.0, layout::SECTION_TRACK, &t.text);
			}
			(Item::Banner(_), ItemBox::Banner { rect, text: t, button, button_text }) => {
				rrect(out, *rect, 12.0, BOX_BG);
				text(out, t.x, t.y, FontRole::Medium, layout::LABEL_PX - 2.0, TEXT_BOX, Align::Left, button.x - t.x - 12.0, 0.0, &t.text);
				if button.w > 0.0 {
					rrect(out, *button, 8.0, s.accent);
					text(out, button_text.x, button_text.y, FontRole::SemiBold, layout::CHIP_PX, INK_ON_ACCENT, Align::Center, 0.0, 0.0, &button_text.text);
				}
			}
			(Item::Row(r), ItemBox::Row { rect, label, label_w, ctrl, .. }) => {
				row(out, s, r, s.focus == Some(i), rect, (label, *label_w), ctrl);
			}
			_ => {}
		}
	}
	out.push(Prim::ClipPop);
	scrollbar(out, l);
}

fn row(out: &mut Vec<Prim>, s: &Screen, r: &RowData, focused: bool, rect: &Rect, label: (&layout::Line, f32), ctrl: &CtrlBox) {
	if focused {
		rrect_b(out, *rect, 10.0, s.accent.tint(0.13), 1.5, s.accent.tint(0.6));
	}
	let colour = if focused { WHITE } else if r.dim { TEXT_DIM } else { TEXT_LABEL };
	text(out, label.0.x, label.0.y, FontRole::Medium, layout::LABEL_PX, colour, Align::Left, label.1, 0.0, &label.0.text);
	let value_colour = if focused { s.accent } else { TEXT };
	match (&r.control, ctrl) {
		(Control::Choice { value, index, .. }, CtrlBox::Choice { prev, next, value: vb, pips }) => {
			arrow_button(out, *prev, false);
			arrow_button(out, *next, true);
			text(out, vb.cx(), rect.y + 21.0, FontRole::SemiBold, layout::VALUE_PX, value_colour, Align::Center, vb.w, 0.0, value);
			for (i, p) in pips.iter().enumerate() {
				let fill = if i == *index { s.accent } else if i < *index { s.accent.tint(0.4) } else { PIP_OFF };
				rrect(out, *p, 1.5, fill);
			}
		}
		(Control::Toggle { value, on }, CtrlBox::Toggle { switch, value: vb }) => {
			text(out, vb.right(), rect.cy(), FontRole::SemiBold, layout::VALUE_PX, value_colour, Align::Right, 0.0, 0.0, value);
			rrect(out, *switch, 15.0, if *on { s.accent } else { SWITCH_OFF });
			let kx = switch.x + 3.0 + if *on { 26.0 } else { 0.0 };
			rrect(out, Rect::new(kx, switch.y + 3.0, 24.0, 24.0), 12.0, WHITE);
		}
		(Control::Slider { value, t }, CtrlBox::Slider { track, value: vb }) => slider(out, s, track, *t, value, vb, value_colour),
		(Control::Chips(chips), CtrlBox::Chips(rects)) => {
			for (c, rc) in chips.iter().zip(rects) {
				chip(out, s.accent, c, *rc);
			}
		}
		_ => {}
	}
}

fn arrow_button(out: &mut Vec<Prim>, rect: Rect, right: bool) {
	if rect.w <= 0.0 {
		return; // [PC] a Choice row with one value has no arrows (layout.rs `ctrl_box`)
	}
	rrect(out, rect, 8.0, BUTTON_BG);
	out.push(Prim::Chevron { rect: Rect::new(rect.cx() - 7.0, rect.cy() - 7.0, 14.0, 14.0), right, colour: CHEVRON });
}

fn slider(out: &mut Vec<Prim>, s: &Screen, track: &Rect, t: f32, value: &str, vb: &Rect, value_colour: Rgba) {
	let cy = track.cy();
	rrect(out, Rect::new(track.x, cy - 2.0, track.w, 4.0), 2.0, SLIDER_TRACK);
	let cx = track.x + THUMB_R + t.clamp(0.0, 1.0) * (track.w - 2.0 * THUMB_R);
	rrect(out, Rect::new(track.x, cy - 2.0, cx - track.x, 4.0), 2.0, s.accent);
	rrect_b(out, Rect::new(cx - THUMB_R, cy - THUMB_R, 2.0 * THUMB_R, 2.0 * THUMB_R), THUMB_R, WHITE, 2.0, s.accent);
	text(out, vb.right(), vb.cy(), FontRole::SemiBold, layout::VALUE_PX, value_colour, Align::Right, 0.0, 0.0, value);
}

fn chip(out: &mut Vec<Prim>, accent: Rgba, c: &Chip, rc: Rect) {
	let ink = if c.selected {
		rrect(out, rc, 8.0, accent);
		INK_ON_ACCENT
	} else if c.disabled {
		rrect_b(out, rc, 8.0, Rgba::rgba(0, 0, 0, 0.0), 1.0, Rgba::rgba(255, 255, 255, 0.14));
		TEXT_DISABLED
	} else {
		rrect(out, rc, 8.0, CHIP_BG);
		TEXT_LABEL
	};
	text(out, rc.cx(), rc.cy(), FontRole::SemiBold, layout::CHIP_PX, ink, Align::Center, 0.0, 0.0, &c.text);
}

/// A thin scrollbar at the right edge of the viewport while the content overflows.
fn scrollbar(out: &mut Vec<Prim>, l: &Layout) {
	if l.max_scroll <= 0.0 {
		return;
	}
	let vp = l.viewport;
	let h = (vp.h * vp.h / (vp.h + l.max_scroll)).max(30.0);
	let y = vp.y + (vp.h - h) * (l.scroll / l.max_scroll);
	rrect(out, Rect::new(vp.right() - 8.0, y, 4.0, h), 2.0, SCROLLBAR);
}
