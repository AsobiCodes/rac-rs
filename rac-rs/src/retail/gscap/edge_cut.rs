//! Rows where a GS primitive crosses the scissor edge of the overlay, for the widescreen presenter.
//!
//! The GS scissor only discards pixels: a HUD piece that reaches past the 4:3 picture's right (or left) edge is cut there.
//! The viewer fills a wider window's side margins with the picture's stretched edge columns (fades, black bars and flat
//! panels need that); on a row where a piece is cut, that stretch would repeat the cut column over the margin (the
//! Pokitaru breath tank sliding in drew as a huge block). The capture notes the rows of every primitive that crosses an
//! edge here; a later primitive that spans the whole width on those rows (a fill over them) clears them again.

use std::sync::Mutex;

pub const ROWS: usize = 448;
/// Row flag: a primitive crosses the right scissor edge.
pub const RIGHT: u8 = 1;
/// Row flag: a primitive crosses the left scissor edge.
pub const LEFT: u8 = 2;

struct Marks {
	cur: [u8; ROWS],
	any: bool,
	done: [u8; ROWS],
}

static MARKS: Mutex<Marks> = Mutex::new(Marks { cur: [0; ROWS], any: false, done: [0; ROWS] });

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

/// Notes a primitive drawn into the overlay.
pub fn note(sc: [i32; 4], v: &[(f32, f32)]) {
	let (sx0, sx1) = (sc[0] as f32, sc[1] as f32 + 1.0);
	let full = v.iter().all(|p| p.0.is_finite()) && v.iter().any(|p| p.0 <= sx0) && v.iter().any(|p| p.0 >= sx1);
	match crossing(sc, v) {
		Some((y0, y1, f)) => {
			let mut m = MARKS.lock().unwrap_or_else(|e| e.into_inner());
			m.any = true;
			for y in y0..=y1 {
				m.cur[y as usize] |= f;
			}
		}
		None if full => {
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

/// The overlay was published: its rows become what the presenter reads, the next picture starts clean.
pub fn publish() {
	let mut m = MARKS.lock().unwrap_or_else(|e| e.into_inner());
	if m.any || m.done.iter().any(|b| *b != 0) {
		m.done = m.cur;
		m.cur = [0; ROWS];
		m.any = false;
	}
}

/// The rows of the last published picture (flags `RIGHT` / `LEFT`), `None` when no row is marked.
pub fn published() -> Option<[u8; ROWS]> {
	let m = MARKS.lock().unwrap_or_else(|e| e.into_inner());
	m.done.iter().any(|b| *b != 0).then_some(m.done)
}

#[cfg(test)]
mod tests {
	use super::*;

	const SC: [i32; 4] = [0, 511, 0, 447];

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
		assert_eq!(crossing(SC, &[(-20.0, 10.0), (30.0, 20.0)]), Some((10, 20, LEFT)));
		note(SC, &[(478.5, 111.5), (520.5, 303.5)]);
		note(SC, &[(-1.0, 100.0), (513.0, 150.0)]);
		publish();
		let rows = published().unwrap();
		assert_eq!(rows[111], 0);
		assert_eq!(rows[120], 0, "the fill over rows 100..150 cleared them");
		assert_eq!(rows[200], RIGHT);
		assert_eq!(rows[303], RIGHT);
	}
}
