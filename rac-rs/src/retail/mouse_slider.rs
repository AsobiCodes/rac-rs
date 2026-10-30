//! The Sound Options volume bars with the mouse (lane mmenu, v0.4.2): a click or drag on a bar and the wheel over it set the volume the
//! way the D-pad would, without holding Right / Left for hundreds of fields.
//!
//! Retail [C: Veldin FUN_0027a108, twin on every level; decomp/notes/ingame_pause_menu.md "Sound"]: with the page's row 0 (sound
//! effects, word 0x15edf0) or row 1 (music, 0x15edec) selected, the HELD Right bit adds 3 each field and the held Left bit subtracts 3,
//! clamped to 0..0x400; the same update recomputes the mixer variables (0x13e598..0x13e5ac) when the volume changed. A full sweep is 342
//! fields. The request here never writes a mixer variable or the selection: the volume word is preset to ONE step short of the wanted
//! value and Right (or Left) is held for ONE field, so the game's own code makes the last step and everything it derives. The wanted
//! value is snapped to the values held keys reach from where the volume is now (steps of 3, the ends 0 and 0x400 included).
//!
//! Geometry [C: the draw FUN_0027a3d8, leftovers P1-d]: the item rect (x, y, w, h) holds the rows at h/4, h/2, 3h/4 (`Kind::Value`); each
//! bar's fill starts at x + w/2 + 7 and is (w - (w/2 + 0x4a)) pixels wide at the full volume 0x400 (182 px: 265..447 on the PAL
//! 512 px picture), its frame ends at x + w - 0x3f.

use super::mouse_menu::{Kind, MenuSnap, LEFT, RIGHT};

/// The Sound Options page id (the in-level pause menu; the title's own page ids differ and has no bars here).
pub const PAGE: u32 = 0x20;
/// Volume range and what one held field adds.
pub const MAX: i32 = 0x400;
pub const STEP: i32 = 3;
/// One wheel notch: 16 held fields' worth.
pub const WHEEL: i32 = 16 * STEP;

/// What a request wants of the volume.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Slide {
	/// Near this absolute volume (0..=0x400): a click or drag at a bar position.
	To(i32),
	/// This many volume units up (positive) or down: wheel notches.
	By(i32),
}

/// The volume word of Sound Options row `row` (0 effects, 1 music; the PAL address through `disc::va`).
pub fn volume_addr(row: u32) -> Option<u32> {
	match row {
		0 => Some(super::disc::va(0x15edf0)),
		1 => Some(super::disc::va(0x15edec)),
		_ => None,
	}
}

/// The rectangle of the item whose row `sub` is a volume bar (None: not the in-level Sound Options page, or the stereo row).
pub fn bar_item(m: &MenuSnap, item: u32, sub: u32) -> Option<[i32; 4]> {
	if m.front || m.id != PAGE || sub > 1 {
		return None;
	}
	m.items.iter().find(|i| i.addr == item && matches!(i.kind, Kind::Value { n: 3, .. })).map(|i| i.rect)
}

/// The bar's fill start and its width at the full volume, overlay pixels.
fn bar(rect: [i32; 4]) -> (f32, f32) {
	let [x, _, w, _] = rect;
	((x + w / 2 + 7) as f32, (w - (w / 2 + 0x4a)) as f32)
}

/// Is overlay pixel column `x` over the bar (a little slack around it)?
pub fn on_bar(rect: [i32; 4], x: f32) -> bool {
	let (x0, bw) = bar(rect);
	x >= x0 - 6.0 && x <= (rect[0] + rect[2] - 0x3f) as f32 + 4.0 && bw > 0.0
}

/// The volume under pixel column `x` (clamped to 0..=0x400).
pub fn value_at(rect: [i32; 4], x: f32) -> i32 {
	let (x0, bw) = bar(rect);
	(((x - x0) / bw.max(1.0)) * MAX as f32).round().clamp(0.0, MAX as f32) as i32
}

/// The overlay pixel column of volume `v` (the inverse of `value_at`; tests aim the pointer with it).
#[cfg(test)]
pub fn x_of(rect: [i32; 4], v: i32) -> f32 {
	let (x0, bw) = bar(rect);
	x0 + bw * v.clamp(0, MAX) as f32 / MAX as f32
}

/// The volume word to preset and the direction to hold for one field so that the volume goes from `v0` to the value `slide` asks for,
/// snapped to what held keys reach (`v0 + 3k`, clamped). None when it is there already.
pub fn step(v0: i32, slide: Slide) -> Option<(i32, u16)> {
	let v0 = v0.clamp(0, MAX);
	let end = match slide {
		// The ends are always reachable (a held key clamps there).
		Slide::To(t) if t <= 0 => 0,
		Slide::To(t) if t >= MAX => MAX,
		Slide::To(t) => (v0 + STEP * ((t - v0) as f32 / STEP as f32).round() as i32).clamp(0, MAX),
		Slide::By(d) => (v0 + STEP * (d / STEP)).clamp(0, MAX),
	};
	if end == v0 {
		return None;
	}
	// One held field adds / subtracts 3 and clamps: from end - 3 (Right) the result is `end`, also at 0x400; below 3 Left from end + 3.
	Some(if end >= STEP { (end - STEP, RIGHT) } else { (end + STEP, LEFT) })
}

/// Tests only: the volumes and mixer variables as the worker saw them after its last field on the page (the main thread cannot read
/// RAM): [0x15edf0, 0x15edec, 0x13e598, 0x13e59c, 0x13e5a0, 0x13e5a4, 0x13e5a8, 0x13e5ac].
#[cfg(test)]
pub static SEEN: [std::sync::atomic::AtomicU32; 8] = [const { std::sync::atomic::AtomicU32::new(0) }; 8];

#[cfg(test)]
pub fn publish(m: &ee_oracle::Machine) {
	use std::sync::atomic::Ordering;
	for (slot, a) in SEEN.iter().zip([0x15edf0u32, 0x15edec, 0x13e598, 0x13e59c, 0x13e5a0, 0x13e5a4, 0x13e5a8, 0x13e5ac]) {
		slot.store(m.rd32(super::disc::va(a)), Ordering::Relaxed);
	}
}

#[cfg(not(test))]
pub fn publish(_: &ee_oracle::Machine) {}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::retail::mouse_menu::{Item, MenuSnap};

	const RECT: [i32; 4] = [2, 0, 512, 448];

	#[test]
	fn the_bar_maps_pixels_to_volumes() {
		// Fill from x = 2 + 256 + 7 = 265, 182 px wide at the full volume: 265..447 as measured in retail.
		assert_eq!(bar(RECT), (265.0, 182.0));
		assert_eq!(value_at(RECT, 265.0), 0);
		assert_eq!(value_at(RECT, 447.0), MAX);
		assert_eq!(value_at(RECT, 356.0), MAX / 2);
		assert_eq!((value_at(RECT, 100.0), value_at(RECT, 500.0)), (0, MAX));
		assert!(on_bar(RECT, 300.0) && on_bar(RECT, 262.0) && !on_bar(RECT, 200.0) && !on_bar(RECT, 470.0));
	}

	#[test]
	fn a_step_is_one_held_field_from_one_step_short() {
		// From the boot music volume 0x2cc = 716: 716 + 3k.
		assert_eq!(step(716, Slide::To(716)), None);
		assert_eq!(step(716, Slide::To(800)), Some((797, RIGHT)));
		// 801 is off the lattice 716 + 3k: it snaps to 800.
		assert_eq!(step(716, Slide::To(801)), Some((797, RIGHT)));
		assert_eq!(step(716, Slide::To(600)), Some((596, RIGHT)));
		// The ends are reached by clamping, as a held key does: 716 + 3 * 103 = 1025 -> 0x400, 716 - 3 * 239 = -1 -> 0.
		assert_eq!(step(716, Slide::To(MAX)), Some((MAX - STEP, RIGHT)));
		assert_eq!(step(716, Slide::To(0)), Some((STEP, LEFT)));
		assert_eq!(step(1, Slide::To(0)), Some((STEP, LEFT)));
		assert_eq!(step(MAX, Slide::To(MAX)), None);
		// Wheel notches: 48 units up or down, clamped.
		assert_eq!(step(716, Slide::By(WHEEL)), Some((716 + WHEEL - STEP, RIGHT)));
		assert_eq!(step(716, Slide::By(-WHEEL)), Some((716 - WHEEL - STEP, RIGHT)));
		assert_eq!(step(1000, Slide::By(WHEEL)), Some((MAX - STEP, RIGHT)));
	}

	#[test]
	fn every_result_is_one_a_held_key_reaches() {
		// Model of FUN_0027a108: Right: min(v + 3, 0x400); Left: max(v - 3, 0). The preset plus ONE held field equals `end`, and `end` is
		// v0 + 3k clamped (the value k held fields give).
		let held = |v: i32, d: u16, n: i32| (0..n).fold(v, |v, _| if d == RIGHT { (v + STEP).min(MAX) } else { (v - STEP).max(0) });
		for v0 in [0, 1, 2, 3, 100, 716, 1021, 1022, 1023, 1024] {
			for t in (0..=MAX).step_by(7) {
				let Some((pre, d)) = step(v0, Slide::To(t)) else { continue };
				let after = held(pre, d, 1);
				let (dir, n) = if after > v0 { (RIGHT, (after - v0 + 2) / 3) } else { (LEFT, (v0 - after + 2) / 3) };
				assert_eq!(held(v0, dir, n), after, "v0 {v0} -> {t}: preset {pre}");
				assert!((0..=MAX).contains(&pre));
			}
		}
	}

	#[test]
	fn only_the_in_level_sound_page_has_bars() {
		let it = Item { addr: 0x400, slot: 3, rect: RECT, kind: Kind::Value { n: 3, cursor: 0 }, texts: Vec::new() };
		let m = |id, front| MenuSnap { page: 0x1b0000, id, state: id, cur: 0x400, items: vec![it.clone()], front };
		assert_eq!(bar_item(&m(PAGE, false), 0x400, 1), Some(RECT));
		assert_eq!(bar_item(&m(PAGE, false), 0x400, 2), None);
		assert_eq!(bar_item(&m(PAGE, true), 0x400, 0), None);
		assert_eq!(bar_item(&m(0x1d, false), 0x400, 0), None);
	}
}
