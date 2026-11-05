//! [PC] The list as plain data for a game's own page (lane nach, 2026-11-03: a pack's modern pause menu shows the list in its own look;
//! the HUD-pass list of `screen.rs` stays the fallback and the Classic style's). The rows (masked while hidden and locked), the counts,
//! the cheat latch, and the page's focus and first visible row (reset by `screen::open`). While a page shows the list (`set_hosted`,
//! set by the game every display frame) the HUD pass draws neither the list nor its hint, and the list's keys and pad are the page's
//! (`screen::ui_key` keeps only F6, `screen::ui_pad` lets the pad through to the page's host). Game-neutral: no game texts here.
use super::{Def, Entry, Grade};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// One row of the list.
#[derive(Clone, Debug, PartialEq)]
pub struct RowView {
	pub name: &'static str,
	pub desc: &'static str,
	pub grade: Grade,
	/// A hidden entry not unlocked yet: its name and description stay secret (the page shows its own words).
	pub masked: bool,
	pub unlocked: bool,
	/// Unix seconds of the unlock; 0 = unknown.
	pub at: u64,
	pub progress: u32,
	/// Progress goal; 0 = no counter.
	pub max: u32,
}

/// The whole list as the page draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct ListView {
	pub rows: Vec<RowView>,
	pub got: usize,
	pub all: usize,
	/// The session cheated (`super::cheated`): nothing unlocks; the page says so over the full list.
	pub cheated: bool,
}

pub(super) fn row_of(d: &Def, e: &Entry) -> RowView {
	RowView { name: d.name, desc: d.desc, grade: d.grade, masked: d.hidden && !e.unlocked, unlocked: e.unlocked, at: e.at, progress: e.progress, max: d.max }
}

/// The list now; None = this game has no list or its file is not loaded yet.
pub fn list_view() -> Option<ListView> {
	let hub = super::hub();
	let h = hub.as_ref()?;
	let (got, all) = h.store.counts();
	let rows = h.pack.defs.iter().zip(&h.store.entries).map(|(d, e)| row_of(d, e)).collect();
	Some(ListView { rows, got, all, cheated: super::cheated() })
}

static HOSTED: AtomicBool = AtomicBool::new(false);
static FOCUS: AtomicUsize = AtomicUsize::new(0);
static FIRST: AtomicUsize = AtomicUsize::new(0);

/// The game's page shows the open list this display frame (true) or not (false: the HUD pass draws it, as in the Classic style).
pub fn set_hosted(on: bool) {
	HOSTED.store(on, Ordering::Relaxed);
}

pub fn hosted() -> bool {
	HOSTED.load(Ordering::Relaxed)
}

/// The list opened: focus and window back to the top.
pub(super) fn reset() {
	FOCUS.store(0, Ordering::Relaxed);
	FIRST.store(0, Ordering::Relaxed);
}

/// The page's (focus, first visible row).
pub fn cursor() -> (usize, usize) {
	(FOCUS.load(Ordering::Relaxed), FIRST.load(Ordering::Relaxed))
}

/// The focus moved by `d` rows over `n` rows (clamped; isize::MIN / MAX = the first / the last row).
pub fn step(focus: usize, d: isize, n: usize) -> usize {
	let last = n.saturating_sub(1);
	match d {
		isize::MIN => 0,
		isize::MAX => last,
		d => (focus as isize).saturating_add(d).clamp(0, last as isize) as usize,
	}
}

/// The first visible row that keeps `focus` in a window of `page` rows over `n` rows, moving the old `first` as little as possible.
pub fn window(first: usize, focus: usize, page: usize, n: usize) -> usize {
	let page = page.max(1);
	let max = n.saturating_sub(page);
	let first = if focus < first { focus } else if focus >= first + page { focus + 1 - page } else { first };
	first.min(max)
}

/// Moves the focus by `d` (see `step`) and the window with it; `page` = the rows the page shows, `n` = all rows.
pub fn move_focus(d: isize, page: usize, n: usize) {
	let (focus, first) = cursor();
	let to = step(focus, d, n);
	FOCUS.store(to, Ordering::Relaxed);
	FIRST.store(window(first, to, page, n), Ordering::Relaxed);
}

/// Puts the focus on row `i` (a hover or click on a visible row); the window follows.
pub fn focus_on(i: usize, page: usize, n: usize) {
	if i < n {
		FOCUS.store(i, Ordering::Relaxed);
		let first = FIRST.load(Ordering::Relaxed);
		FIRST.store(window(first, i, page, n), Ordering::Relaxed);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_window_follows_the_focus_and_never_runs_past_the_end() {
		assert_eq!(window(0, 0, 5, 35), 0);
		assert_eq!(window(0, 4, 5, 35), 0, "still visible");
		assert_eq!(window(0, 5, 5, 35), 1, "one past the bottom scrolls one");
		assert_eq!(window(10, 9, 5, 35), 9, "one above the top scrolls one");
		assert_eq!(window(0, 34, 5, 35), 30, "the end");
		assert_eq!(window(40, 34, 5, 35), 30, "clamped");
		assert_eq!(window(0, 2, 5, 3), 0, "fewer rows than the page");
		assert_eq!(window(3, 3, 0, 35), 3, "a zero page counts as one row");
	}

	#[test]
	fn the_focus_steps_and_clamps() {
		assert_eq!(step(0, -1, 35), 0);
		assert_eq!(step(0, 1, 35), 1);
		assert_eq!(step(33, 5, 35), 34);
		assert_eq!(step(10, isize::MIN, 35), 0);
		assert_eq!(step(10, isize::MAX, 35), 34);
		assert_eq!(step(0, 1, 0), 0, "an empty list");
	}

	#[test]
	fn a_hidden_locked_row_is_masked_and_an_unlocked_one_is_not() {
		let d = Def { id: "x", name: "N", desc: "D", grade: Grade::Gold, hidden: true, max: 0, platinum: false };
		let mut e = Entry { unlocked: false, at: 0, progress: 0, carry: 0, epoch: 0 };
		assert!(row_of(&d, &e).masked);
		e.unlocked = true;
		e.at = 7;
		let r = row_of(&d, &e);
		assert!(!r.masked && r.unlocked && r.at == 7 && r.grade == Grade::Gold);
	}
}
