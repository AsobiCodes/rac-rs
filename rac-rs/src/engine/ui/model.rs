//! The state of the options screen (layer 2), pure: which tab and row are focused, the values of all rows, the scroll, peek, and the
//! list of changes and sounds the host has not taken yet. No window, no game, no clock of its own (`now` is set by the host in
//! milliseconds). Value editing is in `model_edit.rs`; the kit data is built from this by `screen.rs`.
use super::input::{Device, GlyphSet};
use super::rows::{UiMenu, UiRow, UiSound};
use std::ops::Range;

/// A row of the flattened table (all tabs, in order).
#[derive(Clone, Copy)]
pub struct RowRef {
	pub tab: usize,
	pub sec: usize,
	pub row: &'static UiRow,
}

/// A value the user changed; the host passes it to the game (`UiPack::set`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
	pub id: &'static str,
	pub value: i32,
}

/// Rows the user touched this recently (ms) keep their value when the game's values are read back (the game applies a change at
/// its next field).
pub const TOUCH_GRACE_MS: u64 = 300;

pub struct Model {
	pub menu: &'static UiMenu,
	pub lang: usize,
	/// Opened from the title screen (caption variant).
	pub front: bool,
	pub tab: usize,
	/// Global row index of the focused row.
	pub focus: usize,
	pub scroll: f32,
	pub peek: bool,
	pub device: Device,
	pub glyphs: GlyphSet,
	pub now: u64,
	/// A slider being dragged with the mouse (global row index).
	pub drag: Option<usize>,
	pub rows: Vec<RowRef>,
	pub tab_rows: Vec<Range<usize>>,
	pub vals: Vec<i32>,
	pub touched: Vec<Option<u64>>,
	pub changes: Vec<Change>,
	pub sounds: Vec<UiSound>,
}

impl Model {
	pub fn new(menu: &'static UiMenu, lang: usize) -> Model {
		let rows: Vec<RowRef> = menu.rows().map(|(tab, sec, row)| RowRef { tab, sec, row }).collect();
		let mut tab_rows: Vec<Range<usize>> = vec![0..0; menu.tabs.len()];
		for (i, r) in rows.iter().enumerate() {
			let t = &mut tab_rows[r.tab];
			if t.start == t.end {
				*t = i..i + 1;
			} else {
				t.end = i + 1;
			}
		}
		let vals = rows.iter().map(|r| if r.row.kind == super::rows::RowKind::Preset { 0 } else { r.row.def }).collect();
		let touched = vec![None; rows.len()];
		let focus = tab_rows.first().map_or(0, |r| r.start);
		let mut m = Model {
			menu,
			lang,
			front: false,
			tab: 0,
			focus,
			scroll: 0.0,
			peek: false,
			device: Device::Keyboard,
			glyphs: GlyphSet::PlayStation,
			now: 0,
			drag: None,
			rows,
			tab_rows,
			vals,
			touched,
			changes: Vec::new(),
			sounds: Vec::new(),
		};
		m.focus = m.first_shown();
		m
	}

	pub fn row_index(&self, id: &str) -> Option<usize> {
		self.rows.iter().position(|r| r.row.id == id)
	}

	pub fn value(&self, i: usize) -> i32 {
		let v = self.vals.get(i).copied().unwrap_or(0);
		// [PC] A Choice value the engine withdrew (ui/live.rs `offer`: an upscaler this PC cannot run) reads as the row's default.
		match self.rows.get(i).map(|r| r.row) {
			Some(r) if r.kind == super::rows::RowKind::Choice && !super::live::offered(r.id, v) => r.def,
			_ => v,
		}
	}

	pub fn value_of(&self, id: &str) -> Option<i32> {
		self.row_index(id).map(|i| self.value(i))
	}

	pub fn sound(&mut self, s: UiSound) {
		self.sounds.push(s);
	}

	pub fn take_changes(&mut self) -> Vec<Change> {
		std::mem::take(&mut self.changes)
	}

	pub fn take_sounds(&mut self) -> Vec<UiSound> {
		std::mem::take(&mut self.sounds)
	}

	/// The rows of the current tab (shown and hidden).
	pub fn cur_rows(&self) -> Range<usize> {
		self.tab_rows.get(self.tab).cloned().unwrap_or(0..0)
	}

	/// The rows of the current tab the screen shows, in order: a row whose dependency is unmet (`blocked`: Upscale quality while the
	/// Upscaler is Off) is hidden, not dimmed (user 2026-11-08: "too many options").
	pub fn cur_shown(&self) -> Vec<usize> {
		self.cur_rows().filter(|&i| self.shown(i)).collect()
	}

	/// The first shown row of the current tab (the tab's first row when it shows none).
	pub fn first_shown(&self) -> usize {
		self.cur_shown().first().copied().unwrap_or(self.cur_rows().start)
	}

	pub fn is_first(&self) -> bool {
		self.cur_shown().first().is_none_or(|&f| self.focus == f)
	}

	pub fn is_last(&self) -> bool {
		self.cur_shown().last().is_none_or(|&l| self.focus == l)
	}

	/// A focus on a row that is hidden now (a value the game changed meanwhile, a focus written directly) moves to the nearest shown
	/// row of the tab above it, else below it; no sound.
	pub fn settle_focus(&mut self) {
		let shown = self.cur_shown();
		if shown.is_empty() || shown.contains(&self.focus) {
			return;
		}
		let up = shown.iter().rev().find(|&&i| i < self.focus);
		self.focus = up.or_else(|| shown.iter().find(|&&i| i > self.focus)).copied().unwrap_or(shown[0]);
	}

	/// Opens the tab with this id (focus on its first row); false if there is none.
	pub fn open_tab(&mut self, id: &str) -> bool {
		match self.menu.tabs.iter().position(|t| t.id == id) {
			Some(t) => {
				self.go_tab(t, false);
				true
			}
			None => false,
		}
	}

	fn go_tab(&mut self, t: usize, sound: bool) {
		self.tab = t;
		self.focus = self.first_shown();
		self.scroll = 0.0;
		if sound {
			self.sound(UiSound::Move);
		}
	}

	pub fn set_tab(&mut self, t: usize) {
		if t < self.menu.tabs.len() && t != self.tab {
			self.go_tab(t, true);
		}
	}

	pub fn tab_step(&mut self, dir: i32) {
		let n = self.menu.tabs.len() as i32;
		if n > 1 {
			self.go_tab((self.tab as i32 + dir).rem_euclid(n) as usize, true);
		}
	}

	/// Moves the focus up / down one shown row, wrapping at the ends of the tab (hidden rows are skipped).
	pub fn focus_step(&mut self, dir: i32) {
		let s = self.cur_shown();
		if s.len() > 1 {
			// A focus that is not a shown row of the tab (`focus` is pub and written directly) counts as its first one.
			let k = s.iter().position(|&i| i == self.focus).unwrap_or(0) as i32;
			self.focus = s[(k + dir).rem_euclid(s.len() as i32) as usize];
			self.sound(UiSound::Move);
		}
	}

	/// Focus on global row `i` (its tab becomes the current one).
	pub fn focus_to(&mut self, i: usize) {
		if i < self.rows.len() && i != self.focus {
			if self.rows[i].tab != self.tab {
				self.tab = self.rows[i].tab;
			}
			self.focus = i;
			self.sound(UiSound::Move);
		}
	}

	pub fn focus_edge(&mut self, last: bool) {
		let s = self.cur_shown();
		if let (Some(&first), Some(&end)) = (s.first(), s.last()) {
			let to = if last { end } else { first };
			if to != self.focus {
				self.focus = to;
				self.sound(UiSound::Move);
			}
		}
	}

	/// Page up / down: `n` shown rows, clamped to the tab.
	pub fn focus_page(&mut self, dir: i32, n: usize) {
		let s = self.cur_shown();
		if !s.is_empty() {
			let k = s.iter().position(|&i| i == self.focus).unwrap_or(0) as i64;
			let to = s[(k + dir as i64 * n as i64).clamp(0, s.len() as i64 - 1) as usize];
			if to != self.focus {
				self.focus = to;
				self.sound(UiSound::Move);
			}
		}
	}

	/// Takes over the game's current values (`get` by row id) except rows the user changed within `TOUCH_GRACE_MS`.
	pub fn refresh(&mut self, get: &dyn Fn(&str) -> Option<i32>) {
		for i in 0..self.rows.len() {
			let row = self.rows[i].row;
			if row.kind == super::rows::RowKind::Preset {
				continue;
			}
			if self.touched[i].is_some_and(|t| self.now.saturating_sub(t) < TOUCH_GRACE_MS) {
				continue;
			}
			if let Some(v) = get(row.id) {
				if self.valid(i, v) {
					self.vals[i] = v;
				}
			}
		}
		self.settle_focus(); // a value the game changed may hide the focused row
	}
}
