//! Widescreen sides of the retail overlay: the quads that draw the 4:3 picture and its stretched edge columns.
//!
//! In a window wider than 4:3 the overlay's edge columns are clamped out over the side margins (fades, black bars and flat
//! panels fill the whole width that way). On a row where the GS scissor cuts a HUD piece at the edge (the Pokitaru breath
//! tank sliding in from the right) that stretch would repeat the cut column over the margin: a huge block. The capture
//! notes those rows (`retail::gscap::edge_cut`); there the margin stays clear and the piece ends at the picture edge.

use crate::overlay::Quad;
use crate::retail::edge_cut_rows;

/// Overlay quads for a window `sw` x `sh` with `k` = window aspect / (4/3); `rows` = the overlay texture height (448, or the
/// window's height with the sharp HUD: the cut marks stay in the 448 PS2 rows, so the margins split in that row space).
pub(super) fn quads(tex: usize, sw: f32, sh: f32, k: f32, rows: u32) -> Vec<Quad> {
	match edge_cut_rows() {
		Some(cut) if k > 1.0 && rows > 0 => margins(&cut, tex, sw, sh, k, cut.len() as u32),
		_ => vec![Quad { tex, rect: [0.0, 0.0, sw, sh], uv: [0.5 - 0.5 * k, 0.0, 0.5 + 0.5 * k, 1.0], colour: [1.0; 4], angle: 0.0, corners: None }],
	}
}

/// The picture plus the side margins on the rows that are not cut.
pub(super) fn margins(cut: &[u8], tex: usize, sw: f32, sh: f32, k: f32, rows: u32) -> Vec<Quad> {
	let q = |rect: [f32; 4], uv: [f32; 4]| Quad { tex, rect, uv, colour: [1.0; 4], angle: 0.0, corners: None };
	let pw = sh * 4.0 / 3.0;
	let x0 = (sw - pw) * 0.5;
	let mut out = vec![q([x0, 0.0, pw, sh], [0.0, 0.0, 1.0, 1.0])];
	let n = rows as usize;
	for (flag, left) in [(2u8, true), (1u8, false)] {
		let mut r = 0;
		while r < n {
			if cut.get(r).is_some_and(|c| c & flag != 0) {
				r += 1;
				continue;
			}
			let s = r;
			while r < n && !cut.get(r).is_some_and(|c| c & flag != 0) {
				r += 1;
			}
			let (y0, y1) = (s as f32 / n as f32, r as f32 / n as f32);
			let (rect, uv) = if left {
				([0.0, y0 * sh, x0, (y1 - y0) * sh], [0.5 - 0.5 * k, y0, 0.0, y1])
			} else {
				([x0 + pw, y0 * sh, x0, (y1 - y0) * sh], [1.0, y0, 0.5 + 0.5 * k, y1])
			};
			out.push(q(rect, uv));
		}
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn cut_rows_get_no_right_margin() {
		let mut cut = [0u8; 448];
		cut[100..200].fill(1);
		let qs = margins(&cut, 7, 1280.0, 720.0, 1.7778 / (4.0 / 3.0), 448);
		// picture + left margin (all rows) + right margin in two runs (0..100, 200..448)
		assert_eq!(qs.len(), 4);
		assert!(qs.iter().all(|q| q.rect[3] > 0.0));
	}

	#[test]
	fn cut_rows_map_in_ps2_rows() {
		// rows 224..448 cut on the right: the right margin covers only the top half of the window, at any texture height
		let mut cut = [0u8; 448];
		cut[224..].fill(1);
		let qs = margins(&cut, 7, 1920.0, 1080.0, 1.7778 / (4.0 / 3.0), cut.len() as u32);
		let right: Vec<_> = qs.iter().filter(|q| q.uv[0] == 1.0).collect();
		assert_eq!(right.len(), 1);
		assert!((right[0].rect[3] - 540.0).abs() < 0.5 && (right[0].uv[3] - 0.5).abs() < 1e-6);
	}
}
