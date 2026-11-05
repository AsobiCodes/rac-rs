//! Geometry of a `Screen` in the 1280x720 design space, read off the approved mockup (header 48,22; list 48,116 760x540; detail
//! 832,116 400x540; hint bar 48,670; rows 52 high; ...). `build` turns the plain data into rectangles and positioned text lines,
//! once; `draw` only emits prims from it and `hit` only searches it, so what is drawn is what is clicked. Text widths come from a
//! `TextMeasure`. Item rectangles are already shifted by the (clamped) scroll.
use super::data::{Control, HintKind, Item, RowData, Screen};
use super::layout_detail::{self, DetailLayout};
use super::text::{run_width, upper, TextMeasure};
use crate::ui::input::{Device, GlyphSet};
use crate::ui::keycaps::{self, Cap};
use crate::ui::prim::{FontRole, Rect, Symbol};

pub const LIST: Rect = Rect::new(48.0, 116.0, 760.0, 540.0);
pub const DETAIL: Rect = Rect::new(832.0, 116.0, 400.0, 540.0);
pub const CONTENT_X: f32 = 61.0;
pub const CONTENT_W: f32 = 734.0;
pub const CONTENT_Y: f32 = 123.0;
pub const ROW_H: f32 = 52.0;
pub const ROW_STEP: f32 = 54.0;
pub const SECTION_H: f32 = 44.0;
pub const BANNER_H: f32 = 56.0;
/// Right edge of the hint bar and the header.
pub const RIGHT_EDGE: f32 = 1232.0;
pub const HINT_CY: f32 = 688.0;

// Font sizes (design units) and letter spacing (em) of the mockup.
pub const TITLE_PX: f32 = 13.0;
pub const TITLE_TRACK: f32 = 0.18;
pub const TAB_PX: f32 = 27.0;
pub const TAB_TRACK: f32 = 0.06;
pub const STATUS_PX: f32 = 14.0;
pub const SECTION_PX: f32 = 15.0;
pub const SECTION_TRACK: f32 = 0.16;
pub const LABEL_PX: f32 = 18.0;
pub const VALUE_PX: f32 = 18.0;
pub const CHIP_PX: f32 = 15.0;
pub const KEY_PX: f32 = 12.0;
pub const HINT_PX: f32 = 15.0;
pub const PILL_PX: f32 = 16.0;

/// A text line: `x` is the anchor of the alignment `draw` gives it (left unless said), `y` the vertical centre.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
	pub text: String,
	pub x: f32,
	pub y: f32,
}

/// What a hint bar entry or button starts with.
#[derive(Clone, Debug, PartialEq)]
pub enum LeadWhat {
	Key(String),
	Pad(Symbol),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lead {
	pub rect: Rect,
	pub what: LeadWhat,
}

/// The lead(s) of a hint kind for a device: keycaps for the keyboard (named in the screen's language, the tab keys as the keyboard
/// layout prints them: `keycaps.rs`), pad glyphs (and L1 / R1 keycaps) for a pad.
pub fn leads(kind: HintKind, device: Device, glyphs: GlyphSet) -> Vec<LeadWhat> {
	let key = |s: &str| LeadWhat::Key(s.to_string());
	let cap = |c: Cap| LeadWhat::Key(keycaps::name(c).to_string());
	match (device, kind) {
		(Device::Keyboard, HintKind::Change) => vec![cap(Cap::Enter)],
		(Device::Keyboard, HintKind::Back) => vec![cap(Cap::Esc)],
		(Device::Keyboard, HintKind::Reset) => vec![cap(Cap::Backspace)],
		(Device::Keyboard, HintKind::Peek) => vec![cap(Cap::Space)],
		(Device::Keyboard, HintKind::Tabs) => {
			let (prev, next) = keycaps::tab_keys();
			vec![LeadWhat::Key(prev), LeadWhat::Key(next)]
		}
		(Device::Pad, HintKind::Change) => vec![LeadWhat::Pad(Symbol::Cross)],
		(Device::Pad, HintKind::Back) => vec![LeadWhat::Pad(Symbol::Circle)],
		(Device::Pad, HintKind::Reset) => vec![LeadWhat::Pad(Symbol::Square)],
		(Device::Pad, HintKind::Peek) => vec![LeadWhat::Pad(Symbol::Triangle)],
		(Device::Pad, HintKind::Tabs) => {
			let (l, r) = glyphs.shoulders();
			vec![key(l), key(r)]
		}
	}
}

/// Width of a keycap: its text and 16 of padding and border, at least `min_w`.
pub fn cap_w(m: &dyn TextMeasure, text: &str, min_w: f32) -> f32 {
	(run_width(m, FontRole::SemiBold, KEY_PX, text, 0.0) + 16.0).max(min_w)
}

/// Places leads left to right from `x`, centred on `cy`; returns them and the x after the last.
#[allow(clippy::too_many_arguments)]
pub fn place_leads(m: &dyn TextMeasure, whats: &[LeadWhat], x: f32, cy: f32, glyph: f32, key_h: f32, gap: f32) -> (Vec<Lead>, f32) {
	let mut out = Vec::new();
	let mut x = x;
	for (i, w) in whats.iter().enumerate() {
		if i > 0 {
			x += gap;
		}
		let (wd, h) = match w {
			LeadWhat::Key(t) => (cap_w(m, t, 0.0), key_h),
			LeadWhat::Pad(_) => (glyph, glyph),
		};
		out.push(Lead { rect: Rect::new(x, cy - h * 0.5, wd, h), what: w.clone() });
		x += wd;
	}
	(out, x)
}

#[derive(Clone, Debug, PartialEq)]
pub struct CapBox {
	pub rect: Rect,
	pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TabBox {
	pub rect: Rect,
	pub text: Line,
	pub bar: Rect,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Header {
	pub title: Line,
	/// Right aligned at `x`.
	pub status: Line,
	pub notice: Option<(Rect, Line)>,
	pub tab_prev: CapBox,
	pub tab_next: CapBox,
	pub tabs: Vec<TabBox>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CtrlBox {
	None,
	Choice { prev: Rect, next: Rect, value: Rect, pips: Vec<Rect> },
	Toggle { switch: Rect, value: Rect },
	/// `track` is the whole slider (hit area); the thumb travels inside `track.x + THUMB_R .. track.right - THUMB_R`.
	Slider { track: Rect, value: Rect },
	Chips(Vec<Rect>),
}

pub const THUMB_R: f32 = 9.0;

#[derive(Clone, Debug, PartialEq)]
pub enum ItemBox {
	Banner { rect: Rect, text: Line, button: Rect, button_text: Line },
	Section { rect: Rect, text: Line },
	/// `label_hit` is the left part of the row (focus on click); `label_w` the width the label may take.
	Row { rect: Rect, label: Line, label_hit: Rect, label_w: f32, ctrl: CtrlBox },
}

impl ItemBox {
	pub fn rect(&self) -> Rect {
		match self {
			ItemBox::Banner { rect, .. } | ItemBox::Section { rect, .. } | ItemBox::Row { rect, .. } => *rect,
		}
	}
}

#[derive(Clone, Debug, PartialEq)]
pub struct HintBox {
	pub kind: HintKind,
	pub rect: Rect,
	pub leads: Vec<Lead>,
	pub text: Line,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
	pub head: Header,
	pub list: Rect,
	/// The clip of the scrolling list (inside the border).
	pub viewport: Rect,
	/// The scroll the item rectangles are shifted by (the screen's, clamped).
	pub scroll: f32,
	pub max_scroll: f32,
	pub items: Vec<ItemBox>,
	pub detail: Option<DetailLayout>,
	pub hints: Vec<HintBox>,
	pub pill: Rect,
	pub pill_leads: Vec<Lead>,
	pub pill_text: Line,
}

pub fn build(s: &Screen, m: &dyn TextMeasure) -> Layout {
	let max_scroll = list_max_scroll(s);
	let scroll = s.scroll.clamp(0.0, max_scroll);
	let (items, pass_max) = list_pass(s, m, scroll);
	debug_assert!((pass_max - max_scroll).abs() < 0.01, "list_max_scroll {max_scroll} != list_pass {pass_max}");
	let (pill, pill_leads, pill_text) = pill(s, m);
	Layout {
		head: header(s, m),
		list: LIST,
		viewport: LIST.inset(1.0),
		scroll,
		max_scroll,
		items,
		detail: s.detail.as_ref().map(|d| layout_detail::layout(d, s.device, s.glyphs, m)),
		hints: hints(s, m),
		pill,
		pill_leads,
		pill_text,
	}
}

impl Layout {
	/// The scroll that shows item `i` fully (8 units of margin); the first / last row of a list goes to the very top / bottom.
	pub fn scroll_to_show(&self, i: usize, first: bool, last: bool) -> f32 {
		if first {
			return 0.0;
		}
		if last {
			return self.max_scroll;
		}
		let Some(it) = self.items.get(i) else { return self.scroll };
		let (r, vp) = (it.rect(), self.viewport);
		let mut sc = self.scroll;
		if r.y < vp.y + 8.0 {
			sc -= vp.y + 8.0 - r.y;
		} else if r.bottom() > vp.bottom() - 8.0 {
			sc += r.bottom() - (vp.bottom() - 8.0);
		}
		sc.clamp(0.0, self.max_scroll)
	}
}

fn header(s: &Screen, m: &dyn TextMeasure) -> Header {
	let (pt, nt) = match leads(HintKind::Tabs, s.device, s.glyphs).as_slice() {
		[LeadWhat::Key(a), LeadWhat::Key(b)] => (a.clone(), b.clone()),
		_ => (String::new(), String::new()),
	};
	let cap = |t: &str, x: f32| CapBox { rect: Rect::new(x, 68.5, cap_w(m, t, 30.0), 24.0), text: t.to_string() };
	let tab_prev = cap(&pt, 48.0);
	let mut x = tab_prev.rect.right() + 10.0;
	let mut tabs = Vec::new();
	for name in &s.tabs {
		let t = upper(name);
		let tw = run_width(m, FontRole::Condensed, TAB_PX, &t, TAB_TRACK);
		tabs.push(TabBox {
			rect: Rect::new(x, 59.0, tw + 28.0, 43.0),
			text: Line { text: t, x: x + 14.0, y: 79.0 },
			bar: Rect::new(x + 14.0, 99.0, tw, 3.0),
		});
		x += tw + 28.0 + 10.0;
	}
	let tab_next = cap(&nt, x);
	let notice = s.notice.as_ref().map(|n| {
		let w = run_width(m, FontRole::SemiBold, STATUS_PX, n, 0.0) + 24.0;
		(Rect::new(RIGHT_EDGE - w, 38.0, w, 27.0), Line { text: n.clone(), x: RIGHT_EDGE - w + 12.0, y: 51.5 })
	});
	Header {
		title: Line { text: upper(&s.title), x: 48.0, y: 47.0 },
		status: Line { text: s.status.clone(), x: RIGHT_EDGE, y: 81.5 },
		notice,
		tab_prev,
		tab_next,
		tabs,
	}
}

/// Lays the list items out from the top of the content, shifted up by `scroll`; also returns the largest useful scroll.
/// How far the list scrolls at most: the height of its items (banner, sections, rows) past the list box. Counts heights only (no
/// boxes are built: `list_pass` gives the same number).
fn list_max_scroll(s: &Screen) -> f32 {
	let mut h = 0.0;
	for it in &s.items {
		h += match it {
			Item::Banner(_) => 8.0 + BANNER_H,
			Item::Section(_) => SECTION_H,
			Item::Row(_) => ROW_STEP,
		};
	}
	(h + 6.0 + 16.0 - (LIST.h - 2.0)).max(0.0)
}

fn list_pass(s: &Screen, m: &dyn TextMeasure, scroll: f32) -> (Vec<ItemBox>, f32) {
	let mut y = CONTENT_Y - scroll;
	let mut out = Vec::with_capacity(s.items.len());
	for it in &s.items {
		match it {
			Item::Banner(b) => {
				y += 8.0;
				let rect = Rect::new(CONTENT_X, y, CONTENT_W, BANNER_H);
				// [PC] No button text = no button (a zero rect: never drawn or hit; ui/screen.rs, nothing to switch to).
				let bw = if b.button.is_empty() { 0.0 } else { run_width(m, FontRole::SemiBold, CHIP_PX, &b.button, 0.0) + 28.0 };
				let button = Rect::new(rect.right() - 16.0 - bw, rect.cy() - 18.0, bw, if bw > 0.0 { 36.0 } else { 0.0 });
				out.push(ItemBox::Banner {
					rect,
					text: Line { text: b.text.clone(), x: rect.x + 16.0, y: rect.cy() },
					button,
					button_text: Line { text: b.button.clone(), x: button.cx(), y: button.cy() },
				});
				y += BANNER_H;
			}
			Item::Section(t) => {
				let rect = Rect::new(CONTENT_X, y, CONTENT_W, SECTION_H);
				out.push(ItemBox::Section { rect, text: Line { text: upper(t), x: rect.x + 16.0, y: rect.y + 27.0 } });
				y += SECTION_H;
			}
			Item::Row(r) => {
				out.push(row_box(r, Rect::new(CONTENT_X, y, CONTENT_W, ROW_H), m));
				y += ROW_STEP;
			}
		}
	}
	let content_h = (y + scroll - CONTENT_Y) + 6.0 + 16.0;
	(out, (content_h - (LIST.h - 2.0)).max(0.0))
}

fn row_box(r: &RowData, rect: Rect, m: &dyn TextMeasure) -> ItemBox {
	let right = rect.right() - 10.0;
	let (ctrl, left) = ctrl_box(&r.control, rect, right, m);
	let hit_w = (left - 12.0 - rect.x).max(0.0);
	ItemBox::Row {
		rect,
		label: Line { text: r.label.clone(), x: rect.x + 16.0, y: rect.cy() },
		label_hit: Rect::new(rect.x, rect.y, hit_w, ROW_H),
		label_w: (hit_w - 16.0).max(0.0),
		ctrl,
	}
}

/// The control's boxes and the x where its column starts.
fn ctrl_box(c: &Control, row: Rect, right: f32, m: &dyn TextMeasure) -> (CtrlBox, f32) {
	match c {
		Control::None => (CtrlBox::None, right),
		Control::Choice { count, .. } => {
			// [PC] One value (or none) to choose from (ui/screen.rs: only the values the engine offers, e.g. an Upscaler row on a PC
			// with no upscaler): no arrows and no pips, zero rects that are never drawn or hit (the column stays where it is).
			let live = *count > 1;
			let b = if live { 34.0 } else { 0.0 };
			let next = Rect::new(right - 34.0, row.y + 9.0, b, b);
			let value = Rect::new(right - 34.0 - 8.0 - 176.0, row.y, 176.0, ROW_H);
			let prev = Rect::new(value.x - 8.0 - 34.0, row.y + 9.0, b, b);
			let n = *count as f32;
			let x0 = value.cx() - (n * 18.0 + (n - 1.0).max(0.0) * 4.0) * 0.5;
			let pips = (0..*count).filter(|_| live).map(|i| Rect::new(x0 + i as f32 * 22.0, row.y + 38.0, 18.0, 3.0)).collect();
			(CtrlBox::Choice { prev, next, value, pips }, prev.x)
		}
		Control::Toggle { value, .. } => {
			let switch = Rect::new(right - 56.0, row.y + 11.0, 56.0, 30.0);
			let vw = run_width(m, FontRole::SemiBold, VALUE_PX, value, 0.0).max(40.0);
			let value = Rect::new(switch.x - 8.0 - vw, row.y, vw, ROW_H);
			(CtrlBox::Toggle { switch, value }, value.x)
		}
		Control::Slider { .. } => {
			let value = Rect::new(right - 60.0, row.y, 60.0, ROW_H);
			let track = Rect::new(value.x - 8.0 - 190.0, row.y + 11.0, 190.0, 30.0);
			(CtrlBox::Slider { track, value }, track.x)
		}
		Control::Chips(chips) => {
			let mut x = right;
			let mut rects = Vec::new();
			for c in chips.iter().rev() {
				let w = run_width(m, FontRole::SemiBold, CHIP_PX, &c.text, 0.0) + 24.0;
				x -= w;
				rects.push(Rect::new(x, row.y + 9.0, w, 34.0));
				x -= 8.0;
			}
			rects.reverse();
			let left = rects.first().map_or(right, |r| r.x);
			(CtrlBox::Chips(rects), left)
		}
	}
}

fn hints(s: &Screen, m: &dyn TextMeasure) -> Vec<HintBox> {
	// widths first (right-aligned bar), then positions from the right edge
	let items: Vec<_> = s
		.hints
		.iter()
		.map(|h| {
			let whats = leads(h.kind, s.device, s.glyphs);
			let (_, lead_end) = place_leads(m, &whats, 0.0, 0.0, 24.0, 22.0, 6.0);
			let tw = run_width(m, FontRole::Medium, HINT_PX, &h.text, 0.0);
			let pad = if h.kind.is_button() { 6.0 } else { 0.0 };
			(h, whats, lead_end, tw, pad)
		})
		.collect();
	let mut out = Vec::new();
	let mut end = RIGHT_EDGE;
	for (h, whats, lead_w, tw, pad) in items.into_iter().rev() {
		let w = pad * 2.0 + lead_w + 8.0 + tw;
		let x = end - w;
		let (ls, lead_end) = place_leads(m, &whats, x + pad, HINT_CY, 24.0, 22.0, 6.0);
		let rect = if h.kind.is_button() { Rect::new(x, HINT_CY - 17.0, w, 34.0) } else { Rect::new(x, HINT_CY - 12.0, w, 24.0) };
		out.push(HintBox { kind: h.kind, rect, leads: ls, text: Line { text: h.text.clone(), x: lead_end + 8.0, y: HINT_CY } });
		end = x - 26.0;
	}
	out.reverse();
	out
}

fn pill(s: &Screen, m: &dyn TextMeasure) -> (Rect, Vec<Lead>, Line) {
	let whats = leads(HintKind::Peek, s.device, s.glyphs);
	let (_, lead_end) = place_leads(m, &whats, 0.0, 0.0, 22.0, 22.0, 6.0);
	let tw = run_width(m, FontRole::SemiBold, PILL_PX, &s.peek_text, 0.0);
	let w = 36.0 + lead_end + 10.0 + tw;
	let rect = Rect::new((crate::ui::theme::DESIGN_W - w) * 0.5, 640.0, w, 44.0);
	let (ls, end) = place_leads(m, &whats, rect.x + 18.0, rect.cy(), 22.0, 22.0, 6.0);
	(rect, ls, Line { text: s.peek_text.clone(), x: end + 10.0, y: rect.cy() })
}

#[cfg(test)]
mod choice_tests {
	use super::*;
	use crate::ui::kit::text::FixedAdvance;

	/// Review R28 m1: a Choice row with one offered value (the Upscaler on a PC with no upscaler) has no arrows and no pips; with
	/// more the boxes are the old ones, and the value box and the column do not move.
	#[test]
	fn a_choice_with_one_value_has_no_arrows_and_no_pips() {
		let row = Rect::new(CONTENT_X, 200.0, CONTENT_W, ROW_H);
		let right = row.right() - 10.0;
		let boxes = |count| match ctrl_box(&Control::Choice { value: "Off".into(), index: 0, count }, row, right, &FixedAdvance(0.5)) {
			(CtrlBox::Choice { prev, next, value, pips }, left) => (prev, next, value, pips, left),
			_ => panic!("choice"),
		};
		let (prev, next, value, pips, left) = boxes(3);
		assert_eq!((prev.w, next.w, pips.len()), (34.0, 34.0, 3));
		assert_eq!(next, Rect::new(right - 34.0, row.y + 9.0, 34.0, 34.0));
		let (p1, n1, v1, pips1, left1) = boxes(1);
		assert_eq!((p1.w, p1.h, n1.w, n1.h, pips1.len()), (0.0, 0.0, 0.0, 0.0, 0), "no arrows, no pips");
		assert_eq!((v1, left1), (value, left), "value box and column unchanged");
		assert!(!p1.contains(p1.x, p1.y) && !n1.contains(n1.x, n1.y), "a zero rect is never hit");
	}
}
