//! Geometry of the detail panel (right side): a column that flows down from the top (path, title, Now / Default, body, note, box)
//! with the meter row and the footer row anchored to the bottom, as in the mockup (padding 28, gap 16).
use super::data::{BoxData, Detail, Footer, Meter};
use super::layout::{leads, place_leads, Line, Lead};
use super::text::{run_width, upper, wrap, TextMeasure};
use crate::ui::input::{Device, GlyphSet};
use crate::ui::prim::{FontRole, Rect};

/// Inner column: x, width, top and bottom (panel 832,116 400x540, border 1, padding 28).
pub const X0: f32 = 861.0;
pub const W: f32 = 342.0;
const TOP: f32 = 145.0;
const BOTTOM: f32 = 627.0;
const GAP: f32 = 16.0;

pub const PATH_PX: f32 = 13.0;
pub const PATH_TRACK: f32 = 0.16;
pub const TITLE_PX: f32 = 38.0;
pub const TITLE_LH: f32 = 38.0 * 1.05;
pub const SMALL_PX: f32 = 15.0;
pub const BODY_PX: f32 = 17.0;
pub const BODY_LH: f32 = 17.0 * 1.55;
pub const NOTE_LH: f32 = 21.0;
pub const BOX_LH: f32 = 15.0 * 1.45;
pub const BTN_H: f32 = 36.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Pair {
	pub label: Line,
	pub value: Line,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BoxLayout {
	pub rect: Rect,
	pub lines: Vec<Line>,
	pub button: Rect,
	pub button_text: Line,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MeterLayout {
	pub sep: Rect,
	/// The label ("Speed impact"), wrapped into its box like the footer note: two lines in a long language rather than running into
	/// the bars.
	pub label: Vec<Line>,
	/// The label's box width: from the column's left edge to a gap before the bars (drawn as `max_w`, so `fit::overflows` checks it).
	pub label_w: f32,
	pub bars: Vec<Rect>,
	pub level: u8,
	/// Right aligned in this box.
	pub text: Rect,
	pub word: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FooterLayout {
	pub sep: Rect,
	pub note: Vec<Line>,
	/// The note's box width (left of the button; drawn as `max_w`).
	pub note_w: f32,
	pub button: Rect,
	pub leads: Vec<Lead>,
	pub text: Line,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DetailLayout {
	pub panel: Rect,
	pub path: Line,
	pub title: Vec<Line>,
	pub now: Option<Pair>,
	pub default: Option<Pair>,
	pub body: Vec<Line>,
	pub note: Vec<Line>,
	pub boxed: Option<BoxLayout>,
	pub meter: Option<MeterLayout>,
	pub footer: Option<FooterLayout>,
}

impl DetailLayout {
	/// The lowest a body / note line or the box may reach: above the meter's separator, else the footer's, else the panel's inner
	/// bottom (as `layout` cuts them, R1-8).
	pub fn floor(&self) -> f32 {
		match (&self.meter, &self.footer) {
			(Some(mt), _) => mt.sep.y - GAP,
			(None, Some(f)) => f.sep.y - GAP,
			(None, None) => BOTTOM,
		}
	}

	/// The bottom of the lowest body / note line or of the box (None = the column has none of them).
	pub fn text_bottom(&self) -> Option<f32> {
		let body = self.body.last().map(|l| l.y + BODY_LH * 0.5);
		let note = self.note.last().map(|l| l.y + NOTE_LH * 0.5);
		let boxed = self.boxed.as_ref().map(|b| b.rect.y + b.rect.h);
		[body, note, boxed].into_iter().flatten().reduce(f32::max)
	}
}

fn lines(texts: Vec<String>, x: f32, top: f32, lh: f32) -> Vec<Line> {
	texts.into_iter().enumerate().map(|(i, text)| Line { text, x, y: top + (i as f32 + 0.5) * lh }).collect()
}

pub fn layout(d: &Detail, device: Device, glyphs: GlyphSet, m: &dyn TextMeasure) -> DetailLayout {
	let mut bottom = BOTTOM;
	let footer = d.footer.as_ref().map(|f| footer(f, device, glyphs, m, bottom));
	if let Some(f) = &footer {
		bottom = f.sep.y - GAP;
	}
	let meter = d.meter.as_ref().map(|mt| meter(mt, m, bottom));
	// The lowest a text line or the box may reach: above the meter's separator, else the footer's, else the panel's inner bottom.
	let floor = meter.as_ref().map_or(bottom, |mt| mt.sep.y - GAP);
	let mut y = TOP;
	let path = Line { text: upper(&d.path), x: X0, y: y + 8.0 };
	y += 16.0 + GAP;
	let tl = wrap(m, FontRole::Condensed, TITLE_PX, &d.title, W);
	let title = lines(tl.clone(), X0, y, TITLE_LH);
	y += tl.len() as f32 * TITLE_LH + GAP;
	let (now, default) = now_default(d, m, y + 9.0);
	if d.now.is_some() || d.default.is_some() {
		y += 18.0 + GAP;
	}
	let mut bl = wrap(m, FontRole::Regular, BODY_PX, &d.body, W);
	let mut nl = d.note.as_ref().map(|n| wrap(m, FontRole::Regular, SMALL_PX, n, W)).unwrap_or_default();
	// A long language must not run into the meter / footer (R1-8): the body, then the note, is cut short with an ellipsis.
	let tail = tail_h(d.boxed.as_ref().map(|b| boxed(b, m, 0.0).rect.h));
	let body_max = ((floor - y - tail(nl.len())) / BODY_LH).floor().max(1.0) as usize;
	cut_lines(&mut bl, body_max, m, BODY_PX);
	let body_end = y + bl.len() as f32 * BODY_LH;
	let note_max = (0..=nl.len()).rev().find(|&n| body_end + tail(n) <= floor).unwrap_or(0);
	cut_lines(&mut nl, note_max, m, SMALL_PX);
	let body = lines(bl.clone(), X0, y, BODY_LH);
	y += bl.len() as f32 * BODY_LH + GAP;
	let note = lines(nl.clone(), X0, y, NOTE_LH);
	if !nl.is_empty() {
		y += nl.len() as f32 * NOTE_LH + GAP;
	}
	let boxed = d.boxed.as_ref().map(|b| boxed(b, m, y));
	DetailLayout { panel: super::layout::DETAIL, path, title, now, default, body, note, boxed, meter, footer }
}

/// The height below the body's last line: the gap, `n` note lines and the box (of height `box_h`), as `layout` stacks them.
fn tail_h(box_h: Option<f32>) -> impl Fn(usize) -> f32 {
	move |n| {
		let note = n as f32 * NOTE_LH;
		match (n > 0, box_h) {
			(false, None) => 0.0,
			(true, None) => GAP + note,
			(false, Some(h)) => GAP + h,
			(true, Some(h)) => GAP + note + GAP + h,
		}
	}
}

/// Keeps the first `max` lines; when lines were dropped the last one kept ends with an ellipsis (its last words go until it fits).
fn cut_lines(ls: &mut Vec<String>, max: usize, m: &dyn TextMeasure, px: f32) {
	if ls.len() <= max {
		return;
	}
	ls.truncate(max);
	let Some(last) = ls.last_mut() else { return };
	let mut t = last.trim_end().to_string();
	loop {
		let cut = format!("{t}\u{2026}");
		if m.width(FontRole::Regular, px, &cut) <= W || !t.contains(' ') {
			*last = cut;
			return;
		}
		t.truncate(t.rfind(' ').unwrap_or(0));
		t.truncate(t.trim_end().len());
	}
}

/// The Now and Default pairs on one line, 20 units apart; `cy` = the line's centre.
fn now_default(d: &Detail, m: &dyn TextMeasure, cy: f32) -> (Option<Pair>, Option<Pair>) {
	let mut x = X0;
	let space = m.width(FontRole::Regular, SMALL_PX, " ");
	let mut pair = |p: &Option<(String, String)>| {
		p.as_ref().map(|(l, v)| {
			let lw = m.width(FontRole::Regular, SMALL_PX, l);
			let vw = m.width(FontRole::SemiBold, SMALL_PX, v);
			let out = Pair { label: Line { text: l.clone(), x, y: cy }, value: Line { text: v.clone(), x: x + lw + space, y: cy } };
			x += lw + space + vw + 20.0;
			out
		})
	};
	let now = pair(&d.now);
	let default = pair(&d.default);
	(now, default)
}

fn boxed(b: &BoxData, m: &dyn TextMeasure, top: f32) -> BoxLayout {
	let ls = wrap(m, FontRole::Regular, SMALL_PX, &b.text, W - 32.0);
	let text_h = ls.len() as f32 * BOX_LH;
	let h = 14.0 + text_h + 10.0 + BTN_H + 14.0;
	let rect = Rect::new(X0, top, W, h);
	// [PC] No button text = no button (a zero rect: never drawn or hit; ui/screen.rs, nothing to switch to).
	let bw = if b.button.is_empty() { 0.0 } else { run_width(m, FontRole::SemiBold, SMALL_PX, &b.button, 0.0) + 28.0 };
	let button = Rect::new(X0 + 16.0, top + 14.0 + text_h + 10.0, bw, if bw > 0.0 { BTN_H } else { 0.0 });
	BoxLayout {
		rect,
		lines: lines(ls, X0 + 16.0, top + 14.0, BOX_LH),
		button,
		button_text: Line { text: b.button.clone(), x: button.cx(), y: button.cy() },
	}
}

fn meter(mt: &Meter, m: &dyn TextMeasure, bottom: f32) -> MeterLayout {
	let tw = run_width(m, FontRole::SemiBold, SMALL_PX, &mt.text, 0.0).max(60.0);
	let n = mt.max as f32;
	let bars_w = n * 22.0 + (n - 1.0).max(0.0) * 4.0;
	// The label's box ends 12 before the bars (which end 10 before the level word). One line keeps the row 18 high; a wrapped label
	// makes it as high as its lines, so the separator, and the column's floor with it, moves up.
	let label_w = W - tw - 10.0 - bars_w - 12.0;
	let label = wrap(m, FontRole::Regular, SMALL_PX, &mt.label, label_w);
	let label_h = label.len() as f32 * NOTE_LH;
	let row_h = if label.len() > 1 { label_h } else { 18.0 };
	let sep_y = bottom - (14.0 + row_h) - 1.0;
	let cy = sep_y + 1.0 + 14.0 + row_h * 0.5;
	let text = Rect::new(X0 + W - tw, cy - 9.0, tw, 18.0);
	let x0 = text.x - 10.0 - bars_w;
	MeterLayout {
		sep: Rect::new(X0, sep_y, W, 1.0),
		label: match label.len() {
			0 | 1 => label.into_iter().map(|text| Line { text, x: X0, y: cy }).collect(),
			_ => lines(label, X0, cy - label_h * 0.5, NOTE_LH),
		},
		label_w,
		bars: (0..mt.max).map(|i| Rect::new(x0 + i as f32 * 26.0, cy - 4.0, 22.0, 8.0)).collect(),
		level: mt.level,
		text,
		word: mt.text.clone(),
	}
}

fn footer(f: &Footer, device: Device, glyphs: GlyphSet, m: &dyn TextMeasure, bottom: f32) -> FooterLayout {
	let whats = leads(f.lead, device, glyphs);
	let (_, lead_end) = place_leads(m, &whats, 0.0, 0.0, 22.0, 22.0, 6.0);
	let tw = run_width(m, FontRole::SemiBold, SMALL_PX, &f.button, 0.0);
	let bw = 12.0 + lead_end + 8.0 + tw + 12.0;
	let note_w = W - bw - 12.0;
	let note = wrap(m, FontRole::Regular, SMALL_PX, &f.note, note_w);
	let note_h = note.len() as f32 * NOTE_LH;
	let row_h = note_h.max(38.0);
	let sep_y = bottom - (14.0 + row_h) - 1.0;
	let cy = sep_y + 1.0 + 14.0 + row_h * 0.5;
	let button = Rect::new(X0 + W - bw, cy - 19.0, bw, 38.0);
	let (ls, end) = place_leads(m, &whats, button.x + 12.0, cy, 22.0, 22.0, 6.0);
	FooterLayout {
		sep: Rect::new(X0, sep_y, W, 1.0),
		note: lines(note, X0, cy - note_h * 0.5, NOTE_LH),
		note_w,
		button,
		leads: ls,
		text: Line { text: f.button.clone(), x: end + 8.0, y: cy },
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ui::kit::text::FixedAdvance;

	fn detail(body: &str) -> Detail {
		Detail {
			path: "Options / Graphics".into(),
			title: "Lighting".into(),
			now: Some(("Now".into(), "On".into())),
			default: Some(("Default".into(), "On".into())),
			body: body.into(),
			note: Some("A note under the body text.".into()),
			boxed: Some(BoxData { text: "Restart to apply.".into(), button: "Restart".into() }),
			meter: Some(Meter { label: "Cost".into(), level: 2, max: 4, text: "Medium".into() }),
			footer: None,
		}
	}

	#[test]
	fn a_long_body_is_cut_above_the_meter_and_a_short_one_is_not() {
		let m = FixedAdvance(0.5);
		let long = layout(&detail(&"word ".repeat(400)), Device::Keyboard, GlyphSet::PlayStation, &m);
		assert!(long.body.last().is_some_and(|l| l.text.ends_with('\u{2026}')), "the cut body ends with an ellipsis");
		assert!(long.text_bottom().unwrap() <= long.floor() + 0.5, "{} > {}", long.text_bottom().unwrap(), long.floor());
		let short = layout(&detail("Two words."), Device::Keyboard, GlyphSet::PlayStation, &m);
		assert_eq!(short.body.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["Two words."]);
		assert_eq!(short.note.len(), 1);
		assert!(short.text_bottom().unwrap() <= short.floor());
	}

	#[test]
	fn a_long_meter_label_wraps_inside_its_box_and_lifts_the_separator() {
		let m = FixedAdvance(0.5);
		let short = layout(&detail("Two words."), Device::Keyboard, GlyphSet::PlayStation, &m).meter.unwrap();
		let mut d = detail("Two words.");
		d.meter.as_mut().unwrap().label = "Einfluss auf die Geschwindigkeit".into();
		let long = layout(&d, Device::Keyboard, GlyphSet::PlayStation, &m).meter.unwrap();
		assert_eq!(short.label.len(), 1);
		assert_eq!(short.label[0].y, short.bars[0].cy(), "one line: on the bars' centre line, as before");
		assert_eq!(long.label.len(), 2);
		assert!(long.label.iter().all(|t| m.width(FontRole::Regular, SMALL_PX, &t.text) <= long.label_w));
		assert!(X0 + long.label_w < long.bars[0].x, "the box ends before the bars");
		assert_eq!(short.sep.y - long.sep.y, 2.0 * NOTE_LH - 18.0);
	}
}
