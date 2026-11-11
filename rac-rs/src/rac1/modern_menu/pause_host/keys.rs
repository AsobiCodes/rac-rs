//! The modern Controls table's input, a child of `pause_host.rs` (it uses the host's private state): `pause_keys.rs` is the table and
//! its cell logic, this file is the cursor (Up / Down rows, Left / Right slot), Enter / click = capture that cell (`kbm::capture_begin`),
//! Tab = capture slot 2, Delete / Backspace / the pad's Square = clear the cell (Pause's last binding is refused with a notice), Esc /
//! right click = back (or cancel a capture), hover = focus. MAIN thread. The capture itself runs in the worker (`kbm::capture_poll`).
use super::{back, click, State};
use crate::modern_menu::pause_keys::{self as pk, Clear, Effect, Row};
use crate::retail::binds;
use rac_engine::input::kbm;
use rac_engine::ui::input::{Action, MouseButton, MouseEvent};
use rac_engine::ui::kit::Scale;
use std::sync::atomic::{AtomicBool, Ordering};
use winit::keyboard::KeyCode;

/// The modern Controls table is the page on screen (MAIN writes in `end`; the worker's classic `pcctl::keys_on_row` reads it).
static TABLE: AtomicBool = AtomicBool::new(false);

/// Fields after a capture ended during which pad / key presses do not act (the press that bound a button must not choose a cell).
const AFTER_CAPTURE_MS: u64 = 250;
/// How long a mouse invert step asked for counts as the current mode while the worker has not applied it yet.
const PENDING_MS: u64 = 600;

/// The table is on screen: the hidden classic page's Tab / Delete / Square rows must not act.
pub fn table_up() -> bool {
	TABLE.load(Ordering::Relaxed)
}

pub(super) fn set_table(on: bool) {
	if !on {
		pk::set_view(None); // the next visit opens on the device in use again
	}
	// The table went away (a watchdog trip, a level change) with a capture waiting: cancel it, or the game gets a neutral pad until
	// the timeout. Only on the true -> false change (the classic page uses the same capture).
	if TABLE.swap(on, Ordering::Relaxed) && !on {
		kbm::capture_cancel();
	}
}

/// MAIN reads the pad on its own, apart from the worker that binds the press: while a capture waits, the page's presses are the
/// capture's (stamped); on the first poll after it ended, every button held at that moment is absorbed (it is the press that bound,
/// not a Confirm / Back / Clear on the table) and the after-capture grace starts.
pub(super) fn capture_gate(st: &mut State, mask: u16, now: u64) {
	if kbm::capturing() {
		st.cap_seen = true;
		st.keys_cap_at = now;
	} else if std::mem::take(&mut st.cap_seen) {
		st.pad_prev |= mask;
		st.keys_cap_at = now;
	}
}

pub(super) fn is_keys(st: &State) -> bool {
	st.built.as_ref().is_some_and(|b| b.keys.is_some())
}

fn rows(st: &State) -> &[Row] {
	st.built.as_ref().and_then(|b| b.keys.as_ref()).map_or(&[], |k| &k.rows)
}

/// The focus for the drawing: followed over the rows (the first selectable row on first use).
pub(super) fn frame(st: &mut State, r: &[Row]) -> pk::Ui {
	st.keys.follow(r);
	st.keys.clone()
}

fn begin(c: kbm::Cap) {
	kbm::capture_begin(c);
}

/// Does the cell effect: a capture, or a reset.
fn run(e: Effect) {
	match e {
		Effect::None => {}
		Effect::Capture(c) => begin(c),
		Effect::View(pad) => pk::set_view(Some(pad)),
		Effect::Reset(false) => binds::reset_keys(),
		Effect::Reset(true) => binds::reset_pad(),
		Effect::Mouse(axis, mode) => crate::retail::pcext::ui_bridge::set(["mouse_ud", "mouse_lr"][axis], mode as i32),
	}
}

/// Clears the focused cell (the notice says why a refusal changed nothing).
fn clear(st: &mut State, now: u64) {
	let Some(row) = rows(st).get(st.keys.row).cloned() else { return };
	let slot = st.keys.slot;
	let c = pk::clear_effect(kbm::table(), &kbm::binds(), &row, slot);
	match (c, &row) {
		(Clear::Cleared, Row::Action { a, pad, .. }) => {
			if *pad {
				kbm::clear_pad(*a, slot.min(1));
			} else {
				kbm::clear(*a, slot.min(1));
			}
			st.keys.notice = None;
		}
		(Clear::Refused, _) => st.keys.notice = Some((c, now)),
		_ => {}
	}
}

/// Capture slot 2 of the focused action (Tab).
fn second(st: &mut State) {
	if let Some(Row::Action { a, pad, .. }) = rows(st).get(st.keys.row) {
		let (a, pad) = (*a, *pad);
		st.keys.slot = 1;
		begin(if pad { kbm::Cap::Pad(a, 1) } else { kbm::Cap::Kbm(a, 1) });
	}
}

/// Steps a mouse invert row from the mode asked for last when that is newer than the published one (two quick presses step twice).
fn step_mouse(st: &mut State, axis: usize, mode: u8, by: u8, now: u64) {
	let cur = st.keys.mouse_pending[axis].filter(|&(p, t)| p != mode && now.saturating_sub(t) < PENDING_MS).map_or(mode, |(p, _)| p);
	let next = (cur + by) % 3;
	st.keys.mouse_pending[axis] = Some((next, now));
	run(Effect::Mouse(axis, next));
}

fn choose(st: &mut State, now: u64) {
	let Some(r) = rows(st).get(st.keys.row).cloned() else { return };
	match r {
		Row::Mouse { axis, mode, .. } => step_mouse(st, axis, mode, 1, now),
		_ => run(pk::cell_effect(&r, st.keys.slot)),
	}
}

/// A menu action on the table. While a capture waits (and shortly after) every press is the capture's, none is a menu key.
pub(super) fn act(st: &mut State, a: Action, now: u64) {
	if kbm::capturing() {
		st.keys_cap_at = now;
		// A pad player cancels a keyboard capture with the page's back button (Circle / Triangle); the opening Cross is not a back.
		if a == Action::Back && matches!(kbm::capture_state(), kbm::Cap::Kbm(..)) {
			kbm::capture_cancel();
		}
		return;
	}
	if st.keys_cap_at != 0 && now.saturating_sub(st.keys_cap_at) < AFTER_CAPTURE_MS {
		return;
	}
	let r = rows(st).to_vec();
	match a {
		Action::Up => st.keys.step(&r, -1),
		Action::Down => st.keys.step(&r, 1),
		Action::PageUp => (0..5).for_each(|_| st.keys.step(&r, -1)),
		Action::PageDown => (0..5).for_each(|_| st.keys.step(&r, 1)),
		Action::Home => st.keys = pk::Ui::default(),
		Action::Left | Action::Right if matches!(r.get(st.keys.row), Some(Row::Device { .. })) => {
			pk::set_view(Some(a == Action::Right));
			st.keys = pk::Ui::default();
		}
		Action::Left | Action::Right if matches!(r.get(st.keys.row), Some(Row::Mouse { .. })) => {
			// Left / Right step a mouse invert row back / forward through its three values.
			if let Some(Row::Mouse { axis, mode, .. }) = r.get(st.keys.row) {
				step_mouse(st, *axis, *mode, if a == Action::Right { 1 } else { 2 }, now);
			}
		}
		Action::Left => st.keys.slot = 0,
		Action::Right => st.keys.slot = usize::from(matches!(r.get(st.keys.row), Some(Row::Action { .. }))),
		Action::Confirm | Action::Peek => choose(st, now),
		Action::Reset => clear(st, now),
		Action::Back => back(st),
		_ => return,
	}
	st.keys.follow(&r);
}

/// Raw keys before the menu mapping: Tab captures slot 2, Delete clears. True = taken.
pub(super) fn raw_key(st: &mut State, code: KeyCode, now: u64) -> bool {
	if !is_keys(st) || !matches!(code, KeyCode::Tab | KeyCode::Delete) {
		return false;
	}
	if !kbm::capturing() {
		match code {
			KeyCode::Tab => second(st),
			_ => clear(st, now),
		}
	}
	true
}

/// While a capture waits, a click or a wheel notch is its answer: the host leaves them to the window handler (`kbm_capture_click` /
/// `kbm_capture_wheel`). The opening click ran before the capture began, so it never gets here.
pub(super) fn hands_to_capture(ev: &MouseEvent) -> bool {
	kbm::capturing() && matches!(ev, MouseEvent::Down { .. } | MouseEvent::Wheel { .. })
}

/// A mouse event on the table: hover focuses the cell under the pointer, a left click focuses and chooses it. Everything else (tabs, the
/// hint bar, right click = back, the wheel) is the host's.
pub(super) fn mouse(st: &mut State, ev: MouseEvent, now: u64) -> bool {
	let at = |st: &State, x: f32, y: f32| {
		let (dx, dy) = Scale::fit(st.win.0, st.win.1).to_design(x, y);
		pk::hit(rows(st), st.keys.top, dx, dy)
	};
	match ev {
		MouseEvent::Move { x, y } => {
			let hit = at(st, x, y);
			if let Some((i, s)) = hit.filter(|&(i, s)| (i, s) != (st.keys.row, st.keys.slot)) {
				let r = rows(st).to_vec();
				st.keys.focus(&r, i, s);
			}
			hit.is_some() // off the table the pointer is the host's (tab bar, hint bar hover)
		}
		MouseEvent::Down { button: MouseButton::Left, x, y } => {
			if kbm::capturing() {
				return true;
			}
			match at(st, x, y) {
				Some((i, s)) => {
					let r = rows(st).to_vec();
					st.keys.focus(&r, i, s);
					choose(st, now);
				}
				None => click(st, x, y, now),
			}
			true
		}
		_ => false,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use rac_engine::input::binds::PHYS;

	/// The worker binds the press and ends the capture BEFORE MAIN's next pad poll sees it: that press must not act on the table (R2-S1).
	#[test]
	fn the_press_that_bound_a_controller_button_never_acts_on_the_table() {
		let _l = crate::retail::pcopts::globals_guard();
		kbm::init(None, None, None, None);
		let mut st = State { primed: true, ..State::default() };
		kbm::capture_begin(kbm::Cap::Pad(0, 0));
		capture_gate(&mut st, 0, 10); // MAIN polls while the capture waits
		assert!(st.cap_seen && st.keys_cap_at == 10);
		kbm::capture_poll(&[], 0);
		kbm::capture_poll(&[], PHYS[1].1); // the worker: B is fresh twice -> bound, capture over
		kbm::capture_poll(&[], PHYS[1].1);
		assert!(!kbm::capturing() && kbm::binds().1[0][0] == Some(1));
		let circle = 0x20u16;
		capture_gate(&mut st, circle, 20); // MAIN's first poll after: B is held
		assert_eq!(st.pad_prev & circle, circle, "the held button is absorbed: no fresh edge");
		assert_eq!(st.keys_cap_at, 20, "and the grace starts");
		capture_gate(&mut st, circle, 30);
		assert_eq!(st.keys_cap_at, 20, "only once per capture");
		kbm::init(None, None, None, None);
	}

	/// The table going away cancels a waiting capture, once; a classic page's capture is left alone.
	#[test]
	fn closing_the_table_cancels_its_capture_only_on_the_change() {
		let _l = crate::retail::pcopts::globals_guard();
		kbm::init(None, None, None, None);
		set_table(false);
		kbm::capture_begin(kbm::Cap::Pad(0, 0));
		set_table(false);
		assert!(kbm::capturing(), "the table was not up: the classic page's capture stays");
		set_table(true);
		set_table(false);
		assert!(!kbm::capturing(), "the table went away: cancelled");
	}
}
