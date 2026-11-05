//! Hit testing: which widget of a laid-out `Screen` sits under a design-space point. Item numbers are indices in `Screen::items`.
use super::data::HintKind;
use super::layout::{CtrlBox, ItemBox, Layout, THUMB_R};
use crate::ui::prim::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
	None,
	/// Inside the list on nothing clickable.
	List,
	Tab(usize),
	TabPrev,
	TabNext,
	/// A row (its label, or any part of it that is not a control).
	Row(usize),
	/// The chevrons of a Choice row.
	Prev(usize),
	Next(usize),
	Switch(usize),
	Slider(usize),
	Chip(usize, usize),
	/// The button of a banner item.
	Banner(usize),
	/// The button of the detail panel's amber box.
	DetailBox,
	/// The button of the detail panel's footer ("View game").
	DetailFooter,
	/// A clickable hint bar entry (Reset, Peek).
	Hint(HintKind),
	/// The pill shown while peeking.
	PeekPill,
}

/// Where a point in the 0..1 range of a slider track puts the thumb: the thumb centre travels `THUMB_R` inside the track.
pub fn slider_t(track: &Rect, x: f32) -> f32 {
	let span = (track.w - 2.0 * THUMB_R).max(1.0);
	((x - (track.x + THUMB_R)) / span).clamp(0.0, 1.0)
}

pub fn hit(l: &Layout, peek: bool, x: f32, y: f32) -> Hit {
	if peek {
		return if l.pill.contains(x, y) { Hit::PeekPill } else { Hit::None };
	}
	if l.head.tab_prev.rect.contains(x, y) {
		return Hit::TabPrev;
	}
	if l.head.tab_next.rect.contains(x, y) {
		return Hit::TabNext;
	}
	if let Some(i) = l.head.tabs.iter().position(|t| t.rect.contains(x, y)) {
		return Hit::Tab(i);
	}
	if let Some(h) = l.hints.iter().find(|h| h.kind.is_button() && h.rect.contains(x, y)) {
		return Hit::Hint(h.kind);
	}
	if let Some(d) = &l.detail {
		if d.boxed.as_ref().is_some_and(|b| b.button.contains(x, y)) {
			return Hit::DetailBox;
		}
		if d.footer.as_ref().is_some_and(|f| f.button.contains(x, y)) {
			return Hit::DetailFooter;
		}
	}
	if l.viewport.contains(x, y) {
		return list_hit(l, x, y);
	}
	Hit::None
}

fn list_hit(l: &Layout, x: f32, y: f32) -> Hit {
	for (i, it) in l.items.iter().enumerate() {
		match it {
			ItemBox::Banner { button, .. } if button.contains(x, y) => return Hit::Banner(i),
			ItemBox::Row { rect, ctrl, .. } if rect.contains(x, y) => return row_hit(i, ctrl, x, y),
			_ => {}
		}
	}
	Hit::List
}

fn row_hit(i: usize, ctrl: &CtrlBox, x: f32, y: f32) -> Hit {
	match ctrl {
		CtrlBox::Choice { prev, .. } if prev.contains(x, y) => Hit::Prev(i),
		CtrlBox::Choice { next, .. } if next.contains(x, y) => Hit::Next(i),
		CtrlBox::Toggle { switch, .. } if switch.contains(x, y) => Hit::Switch(i),
		CtrlBox::Slider { track, .. } if track.contains(x, y) => Hit::Slider(i),
		CtrlBox::Chips(chips) => chips.iter().position(|c| c.contains(x, y)).map_or(Hit::Row(i), |c| Hit::Chip(i, c)),
		_ => Hit::Row(i),
	}
}
