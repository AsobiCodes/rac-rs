//! Rows where a GS primitive crosses the scissor edge of the overlay, for the widescreen presenter.
//!
//! The GS scissor only discards pixels: a HUD piece that reaches past the 4:3 picture's right (or left) edge is cut there.
//! The viewer fills a wider window's side margins with the picture's stretched edge columns (fades, black bars and flat
//! panels need that); on a row where a piece is cut, that stretch would repeat the cut column over the margin (the
//! Pokitaru breath tank sliding in drew as a huge block). The capture notes the rows of every primitive that crosses an
//! edge here; a later primitive that spans the whole width on those rows (a fill over them) clears them again.
//!
//! The same pass finds the HUD pieces that would be torn by the HUD position's top corner cells (viewer/handler/hud_place.rs): `guard`
//! (the top help prompts, the Gadgetron title letters, the hoverboard race's bars). Each published picture carries its own `Guard`.

pub mod guard;
pub use guard::Guard;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Row space of the marks: the most rows a frame has (PAL front-end screens: 512; gameplay and the USA disc: 448, see `publish`).
pub const ROWS: usize = 512;
/// Row flag: a primitive crosses the right scissor edge.
pub const RIGHT: u8 = 1;
/// Row flag: a primitive crosses the left scissor edge.
pub const LEFT: u8 = 2;

struct Marks {
	cur: [u8; ROWS],
	any: bool,
	done: [u8; ROWS],
	/// Rows of the frame `done` belongs to (what `published` cuts it to).
	done_rows: usize,
}

static MARKS: Mutex<Marks> = Mutex::new(Marks { cur: [0; ROWS], any: false, done: [0; ROWS], done_rows: 0 });

/// `Marks::any` outside the lock: a full-width primitive (most fills and fades) only has rows to clear when something is marked, so it
/// does not take the lock at all while nothing is (the common case). Set and cleared under the lock, with `any`.
static ANY: AtomicBool = AtomicBool::new(false);

/// What a primitive with the screen-space vertices `v` (x, y in pixels) does to the rows of a scissor `sc`
/// (x0, x1, y0, y1, inclusive): `Some((y0, y1, flags))` when it crosses an edge, and `None` otherwise.
pub fn crossing(sc: [i32; 4], v: &[(f32, f32)]) -> Option<(i32, i32, u8)> {
	let (mut lx, mut hx, mut ly, mut hy) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
	for p in v {
		lx = lx.min(p.0);
		hx = hx.max(p.0);
		ly = ly.min(p.1);
		hy = hy.max(p.1);
	}
	let (sx0, sx1) = (sc[0] as f32, sc[1] as f32 + 1.0);
	// Cut: more than a pixel past an edge (a sprite at -0.5 still covers pixel 0 whole) AND some of it visible (a
	// prim wholly outside the scissor draws nothing; the retail HUD parks such sprites off screen).
	let right = hx > sx1 + 1.0 && lx < sx1 && hx > sx0 && lx > sx0;
	let left = lx < sx0 - 1.0 && hx > sx0 && hx < sx1;
	let (y0, y1) = ((ly.ceil() as i32).max(sc[2]).max(0), (hy.floor() as i32).min(sc[3]).min(ROWS as i32 - 1));
	if y1 < y0 || !(right || left) {
		return None;
	}
	Some((y0, y1, if right { RIGHT } else { 0 } | if left { LEFT } else { 0 }))
}

/// Whether a primitive can draw anything: not when it is textured by TFX 0 (modulate) or untextured, blends (ABE) and every vertex alpha is 0
/// (the blend then keeps the destination). The prompt bar is drawn every field with alpha 0 while no prompt shows, so only visible
/// primitives count for the guard. `tme` / `abe`: the PRIM register's TME and ABE bits. Not exact: ALPHA.C = FIX / Ad with TFX 0 or
/// untextured can draw at alpha 0 and is skipped here; not seen in the HUD (the prompt bar is modulated, alpha from the vertices).
pub fn visible(tme: bool, abe: bool, tex0: u64, mut alphas: impl Iterator<Item = u8>) -> bool {
	let tfx = (tex0 >> 35) & 3;
	(tme && tfx != 0) || !abe || alphas.any(|a| a != 0)
}

/// Notes a primitive drawn into the overlay.
pub fn note(sc: [i32; 4], v: &[(f32, f32)]) {
	let (sx0, sx1) = (sc[0] as f32, sc[1] as f32 + 1.0);
	let full = v.iter().all(|p| p.0.is_finite()) && v.iter().any(|p| p.0 <= sx0) && v.iter().any(|p| p.0 >= sx1);
	match crossing(sc, v) {
		Some((y0, y1, f)) => {
			let mut m = MARKS.lock().unwrap_or_else(|e| e.into_inner());
			m.any = true;
			ANY.store(true, Ordering::Release);
			for y in y0..=y1 {
				m.cur[y as usize] |= f;
			}
		}
		None if full && ANY.load(Ordering::Acquire) => {
			let mut m = MARKS.lock().unwrap_or_else(|e| e.into_inner());
			if m.any {
				let (ly, hy) = v.iter().fold((f32::MAX, f32::MIN), |a, p| (a.0.min(p.1), a.1.max(p.1)));
				for y in (ly.ceil() as i32).max(sc[2]).max(0)..=(hy.floor() as i32).min(sc[3]).min(ROWS as i32 - 1) {
					m.cur[y as usize] = 0;
				}
			}
		}
		None => {}
	}
}

/// The overlay was published with `rows` rows: its marks become what the presenter reads, cut to that row count so the presenter maps row
/// r to r / rows of the picture, the next picture starts clean. Returns the picture's guard (to travel with it, `gscap::Overlay::guard`).
pub fn publish(rows: usize) -> Guard {
	let g = guard::finish(rows);
	let mut m = MARKS.lock().unwrap_or_else(|e| e.into_inner());
	if m.any || m.done.iter().any(|b| *b != 0) {
		m.done = m.cur;
		m.done_rows = rows.clamp(1, ROWS);
		m.cur = [0; ROWS];
		m.any = false;
		ANY.store(false, Ordering::Release);
	}
	g
}

/// The rows of the last published picture (flags `RIGHT` / `LEFT`; one entry per row of that picture), `None` when no row is marked.
pub fn published() -> Option<Vec<u8>> {
	let m = MARKS.lock().unwrap_or_else(|e| e.into_inner());
	m.done[..m.done_rows].iter().any(|b| *b != 0).then(|| m.done[..m.done_rows].to_vec())
}

/// The tests (also draw/guard_tests.rs) share the process-wide marks and pieces: they take this lock.
#[cfg(test)]
pub(crate) static TEST_SERIAL: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
	use super::*;

	const SC: [i32; 4] = [0, 511, 0, 447];
	use super::TEST_SERIAL as SERIAL;

	#[test]
	fn tank_strip_crossing_the_right_edge_marks_its_rows() {
		// The breath tank's strip, partly slid in: x 478..520, y 111..303.
		let tri = [(478.5, 111.5), (520.5, 111.5), (478.5, 303.5)];
		assert_eq!(crossing(SC, &tri), Some((112, 303, RIGHT)));
	}

	#[test]
	fn pieces_inside_flush_or_full_width_do_not_mark() {
		assert_eq!(crossing(SC, &[(450.0, 112.0), (492.0, 304.0)]), None);
		// Ends exactly at the edge (x = 512 covers pixel 511).
		assert_eq!(crossing(SC, &[(480.0, 0.0), (512.0, 100.0)]), None);
		// A fade over the whole width and beyond.
		assert_eq!(crossing(SC, &[(-1.0, 0.0), (513.0, 447.0)]), None);
		// Wholly outside the scissor (retail parks sprites off screen): draws nothing, marks nothing.
		assert_eq!(crossing(SC, &[(520.0, 0.0), (544.0, 100.0)]), None);
		assert_eq!(crossing(SC, &[(-99.0, 275.0), (-42.0, 341.0)]), None);
		// A sprite from -0.5 covers pixel 0 whole: not cut.
		assert_eq!(crossing(SC, &[(-0.5, -0.5), (31.5, 447.5)]), None);
	}

	#[test]
	fn left_edge_and_a_later_fill() {
		let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
		assert_eq!(crossing(SC, &[(-20.0, 10.0), (30.0, 20.0)]), Some((10, 20, LEFT)));
		note(SC, &[(478.5, 111.5), (520.5, 303.5)]);
		note(SC, &[(-1.0, 100.0), (513.0, 150.0)]);
		publish(448);
		let rows = published().unwrap();
		assert_eq!(rows.len(), 448);
		assert_eq!(rows[111], 0);
		assert_eq!(rows[120], 0, "the fill over rows 100..150 cleared them");
		assert_eq!(rows[200], RIGHT);
		assert_eq!(rows[303], RIGHT);
	}

	/// The prompt bar is drawn every field with vertex alpha 0 while no prompt shows (modulated: draws nothing): not a guard. Alpha 0 with
	/// the blend off draws, an untextured primitive ignores a stale TEX0's TFX.
	#[test]
	fn an_invisible_prompt_bar_is_not_a_guard() {
		let modulate = 0x8006f845_553077c6u64;
		assert!(!visible(true, true, modulate, [0u8, 0].into_iter()));
		assert!(visible(true, true, modulate, [16u8, 16].into_iter()), "a fading-in prompt counts");
		assert!(visible(true, true, modulate | (1 << 35), [0u8, 0].into_iter()), "decal textures take their alpha from the texture");
		assert!(!visible(false, true, modulate | (1 << 35), [0u8, 0].into_iter()), "TME off: the stale TEX0 does not count");
		assert!(visible(true, false, modulate, [0u8, 0].into_iter()), "no blend: alpha 0 draws");
		assert!(visible(false, false, modulate, std::iter::empty()));
	}

	/// A full-width fill with nothing marked changes nothing (and `ANY` mirrors `any`); with rows marked it clears the ones it covers.
	#[test]
	fn a_fill_with_nothing_marked_takes_no_rows_and_any_follows_the_marks() {
		let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
		publish(448);
		assert!(!ANY.load(Ordering::Acquire));
		note(SC, &[(-1.0, 10.0), (513.0, 50.0)]);
		assert!(!ANY.load(Ordering::Acquire), "a fill alone marks nothing");
		publish(448);
		assert_eq!(published(), None);
		note(SC, &[(478.5, 111.5), (520.5, 303.5)]);
		assert!(ANY.load(Ordering::Acquire));
		note(SC, &[(-1.0, 200.0), (513.0, 250.0)]);
		publish(448);
		assert!(!ANY.load(Ordering::Acquire), "publish clears it");
		let rows = published().unwrap();
		assert_eq!((rows[150], rows[220], rows[280]), (RIGHT, 0, RIGHT));
		publish(448);
		assert_eq!(published(), None, "the next picture starts clean");
	}

	/// A 512-row frame (PAL front end) keeps its marks in 512 rows, a 448-row frame in 448: the presenter divides by the length.
	#[test]
	fn the_published_marks_have_the_frames_row_count() {
		let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
		let sc = [0, 511, 0, 511];
		note(sc, &[(478.5, 480.5), (520.5, 500.5)]);
		publish(512);
		let rows = published().unwrap();
		assert_eq!(rows.len(), 512);
		assert_eq!((rows[481], rows[500], rows[400]), (RIGHT, RIGHT, 0), "rows past 448 can be marked");
		note(SC, &[(478.5, 111.5), (520.5, 303.5)]);
		publish(448);
		assert_eq!(published().unwrap().len(), 448);
		publish(448);
		assert_eq!(published(), None);
	}

	/// A picture's guard is the one computed from that picture's pieces, returned by its own `publish`: a piece that shows for one picture is
	/// in that picture's guard only, and the rows are normalised by the picture's row count.
	#[test]
	fn the_guard_goes_with_the_picture_it_was_measured_on() {
		let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
		publish(448);
		let bar = |y: f32| [[(107.0, y), (139.0, y + 32.0)], [(139.0, y), (373.0, y + 32.0)], [(373.0, y), (405.0, y + 32.0)]];
		for v in bar(41.5) {
			guard::note(SC, &v);
		}
		let first = publish(448);
		assert!((first.centre.unwrap() * 448.0 - 43.4).abs() < 1e-3, "{first:?}");
		// The next picture has the race bar instead: no leftover of the prompt (a one-picture piece), and the first guard is unchanged.
		guard::note(SC, &[(19.5, 14.5), (275.5, 78.5)]);
		let second = publish(448);
		assert_eq!(second.centre, None);
		assert!(second.left.is_some());
		assert!((first.centre.unwrap() * 448.0 - 43.4).abs() < 1e-3);
		assert_eq!(publish(448), Guard::default(), "the next picture starts clean");
		// Rows are divided by the published row count, not by the scissor.
		for v in bar(41.5) {
			guard::note([0, 511, 0, 511], &v);
		}
		assert!((publish(512).centre.unwrap() * 512.0 - 43.4).abs() < 1e-3);
		assert_eq!(guard::GUARD_COL, 0.34);
	}
}
