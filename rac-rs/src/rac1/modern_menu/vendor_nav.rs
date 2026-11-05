//! [PC] The modern shop's "Buy all ammo" row as a list entry the keys and the pad reach like the item rows (user 2026-11-03: "make
//! the buy all ammo selectable just like other items"). Retail's shop has no such row: its browse reads only Right / Left (next /
//! previous item, new presses; < 8 rows clamp, 8+ wrap), Cross (buy) and Triangle (leave); Up / Down do nothing there [C: Novalis
//! FUN_002afce8]. While the modern page browses, `field` (WORKER thread, `Retail::tick_keys`, before the queued mouse pulses of
//! `menu_model::cmd::feed`, which are never remapped) turns the page's vertical list into retail's horizontal one (Up = Left,
//! Down = Right) and keeps one more place above the first row: Up on row 0 puts the focus on Buy all ammo (retail's cursor stays on
//! row 0; the browse sound is played by `retail::vendor::browse_cue`), Down there gives it back to row 0 (the same sound); with 8
//! rows or more (retail's list wraps) the row sits between the last row and the first one. Cross on the row is Square for the game
//! (the "Buy all ammo" hook, `retail::vendor`) while it is held. The left stick is the D-pad here: retail's pad writer ORs it into
//! the same held word (boot FUN_00216e20), so `field` folds it in with the same threshold and centres the axis it took (R23 M1).
//! One step per press, as retail: the shop browse reads the pad's edge word 0x13cb04 (`held & !prev`), not the auto-repeat word
//! 0x13cae4, so a held key or stick moves one row there too [C: boot FUN_00216e20, Novalis FUN_002afce8]; a key eaten here stays
//! away from the game until it is released (no late press). Catch-all: when retail's cursor leaves row 0 under the focus (a mouse
//! request in flight, a late snapshot) the focus goes, the page shows retail's row. Where the page lists no Buy all row
//! (`retail::vendor::buyall_listed` off: RAC_PC_EXT=0, USA without the extras) Up / Down only walk the list (R23 m1). The focus is
//! `FOCUS`, shared with the main thread's mouse (`vendor_host.rs` moves it on a hover / click / wheel, and `frame_vendor` draws it
//! through `vendor::Facts::buyall`). With the page down (Classic, or any other screen) the pad passes unchanged.
use super::vendor_host;
use crate::retail::mouse_menu::{stick_bits, Kind, MenuSnap, CROSS, DOWN, LEFT, RIGHT, UP};
use crate::retail::mouse_vendor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Square: the "Buy all ammo" hook reads it (`retail::vendor`).
const SQUARE: u16 = 0x80;
/// The Buy all ammo row has the focus (MAIN: the mouse, `set_up`; WK: the keys, `field`).
static FOCUS: AtomicBool = AtomicBool::new(false);
/// The keys' memory between fields (WK only).
static NAV: Mutex<Nav> = Mutex::new(Nav { focus: false, prev: 0, eat: 0, square: false, last: None });

/// Does the Buy all ammo row have the focus?
pub fn focused() -> bool {
	FOCUS.load(Ordering::Relaxed)
}

pub fn set_focus(on: bool) {
	FOCUS.store(on, Ordering::Relaxed);
}

/// The browsing list as the game holds it: its cursor (`sel`), its rows, whether it wraps (8 rows or more) and whether the page
/// lists the Buy all ammo row above it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct List {
	pub sel: u32,
	pub count: u32,
	pub cyclic: bool,
	pub row: bool,
}

/// The list of the shop snapshot while it browses (either kind); None in the prompt, while it animates and outside the shop.
pub fn list(m: Option<&MenuSnap>, row: bool) -> Option<List> {
	let m = m.filter(|m| mouse_vendor::stage(m) == 1)?;
	match m.items.first()?.kind {
		Kind::Grid { cols, cursor, .. } if cursor < cols => Some(List { sel: cursor, count: cols, cyclic: mouse_vendor::cyclic(m), row }),
		_ => None,
	}
}

/// The keys' memory: the focus, last field's (remapped) pad word, the keys eaten until released, Cross held as Square, and retail's
/// cursor of the last field (None: no browsing list).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Nav {
	pub focus: bool,
	prev: u16,
	eat: u16,
	square: bool,
	last: Option<u32>,
}

/// One field's answer: the pad word the game sees, and whether the browse sound is due (the focus moved, retail's cursor did not).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Out {
	pub pad: u16,
	pub cue: bool,
}

/// One field of the pad word `pad` (pure): `list` = the browsing list (None = the page is down, the prompt, or the shop animates).
pub fn step(n: &mut Nav, list: Option<List>, pad: u16) -> Out {
	let mut p = pad;
	if let Some(l) = list {
		// The page's list is vertical, retail's is horizontal: Up = previous, Down = next.
		p = p & !(UP | DOWN) | if pad & UP != 0 { LEFT } else { 0 } | if pad & DOWN != 0 { RIGHT } else { 0 };
		// No Buy all row, or retail's cursor left row 0 under the focus (the catch-all): the page shows retail's row.
		if n.focus && (!l.row || (n.last == Some(0) && l.sel != 0)) {
			n.focus = false;
		}
	}
	n.last = list.map(|l| l.sel);
	let new = p & !n.prev;
	n.prev = p;
	n.eat &= p;
	n.square &= p & CROSS != 0;
	let mut cue = false;
	match list {
		Some(l) if n.focus => {
			if new & CROSS != 0 {
				n.square = true; // Cross on Buy all ammo = Square for the game while it is held
			}
			if new & RIGHT != 0 {
				// Down: back to row 0, where retail's cursor still is.
				(n.focus, cue) = (false, true);
				n.eat |= RIGHT;
			} else if new & LEFT != 0 {
				if l.cyclic && l.sel == 0 {
					n.focus = false; // the wrapping list: retail goes to its last row with its own sound
				} else {
					n.eat |= LEFT; // the top of the list
				}
			}
		}
		Some(l) if l.row => {
			if new & LEFT != 0 && l.sel == 0 {
				(n.focus, cue) = (true, true);
				n.eat |= LEFT;
			} else if new & RIGHT != 0 && l.cyclic && l.sel + 1 == l.count {
				n.focus = true; // the wrapping list: retail goes to row 0 with its own sound, the focus is on the row above it
			}
		}
		_ => {}
	}
	let mut out = p & !n.eat;
	if n.square {
		out = out & !CROSS | SQUARE;
	}
	Out { pad: out, cue }
}

/// The pad of this field (WK, `Retail::tick_keys`, `snap` = the shop snapshot `Snap::vend`): buttons `b`, sticks `s`. The page down:
/// unchanged, but for a key eaten while it was up and still held.
pub fn field(b: &mut u16, s: &mut [u8; 4], snap: Option<&MenuSnap>) {
	field_in(b, s, snap, vendor_host::up(), crate::retail::vendor::buyall_listed());
}

/// `field` with the page's state given (`up` = the modern shop page is up, `row` = it lists Buy all ammo; tests).
pub fn field_in(b: &mut u16, s: &mut [u8; 4], snap: Option<&MenuSnap>, up: bool, row: bool) {
	let l = if up { list(snap, row) } else { None };
	if l.is_some() {
		// The left stick as the D-pad (retail's threshold): the axis it pushed is centred, its bits go through the steps below.
		let sb = stick_bits(*s);
		*b |= sb;
		if sb & (LEFT | RIGHT) != 0 {
			s[2] = 0x80;
		}
		if sb & (UP | DOWN) != 0 {
			s[3] = 0x80;
		}
	}
	let mut n = NAV.lock().unwrap_or_else(|e| e.into_inner());
	n.focus = up && focused();
	let before = n.focus;
	let out = step(&mut n, l, *b);
	if up && n.focus != before {
		// The mouse moved the focus on the main thread in the meantime: it wins.
		let _ = FOCUS.compare_exchange(before, n.focus, Ordering::Relaxed, Ordering::Relaxed);
	}
	*b = out.pad;
	if out.cue {
		crate::retail::vendor::browse_cue();
	}
}

#[cfg(test)]
mod tests;
