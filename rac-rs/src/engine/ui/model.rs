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
		Model {
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
		}
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

	/// The rows of the current tab.
	pub fn cur_rows(&self) -> Range<usize> {
		self.tab_rows.get(self.tab).cloned().unwrap_or(0..0)
	}

	pub fn is_first(&self) -> bool {
		self.focus == self.cur_rows().start
	}

	pub fn is_last(&self) -> bool {
		let r = self.cur_rows();
		r.is_empty() || self.focus + 1 == r.end
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
		self.focus = self.cur_rows().start;
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

	/// Moves the focus up / down one row, wrapping at the ends of the tab (no row is skipped, blocked rows included).
	pub fn focus_step(&mut self, dir: i32) {
		let r = self.cur_rows();
		if r.len() > 1 {
			// A focus outside the tab (`focus` is pub and written directly) counts as its first row: never an underflow.
			let k = (self.focus.saturating_sub(r.start).min(r.len() - 1)) as i32;
			self.focus = r.start + (k + dir).rem_euclid(r.len() as i32) as usize;
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
		let r = self.cur_rows();
		if !r.is_empty() {
			let to = if last { r.end - 1 } else { r.start };
			if to != self.focus {
				self.focus = to;
				self.sound(UiSound::Move);
			}
		}
	}

	/// Page up / down: `n` rows, clamped to the tab.
	pub fn focus_page(&mut self, dir: i32, n: usize) {
		let r = self.cur_rows();
		if !r.is_empty() {
			let to = (self.focus as i64 + dir as i64 * n as i64).clamp(r.start as i64, r.end as i64 - 1) as usize;
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
	}
}
