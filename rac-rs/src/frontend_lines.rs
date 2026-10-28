//! Two-line page headers of the front-end mirror (v0.4.1). The mirror draws a page's heading as one line (`frontend::fit_text`); the
//! Graphics page's Culling warning (`retail/pcext/cullwarn.rs`) has two, split by the game's new-line byte 1. One line is drawn
//! exactly as before; several lines share one text size (the smallest that fits every line into the bar) and are centred.

use crate::overlay::Quad;

type Font = (usize, Vec<crate::hud::Glyph>);

/// The width of `s` in the font's advance units (letters the font has no glyph for are skipped, as `fit_text` does).
fn width(font: &Font, s: &str) -> f32 {
	s.chars().filter_map(|c| font.1.get(c as usize).filter(|g| g.adv != 0)).map(|g| g.adv as f32).sum()
}

/// Draws the heading `s` in the bar `r` = [x, y, w, h]. Only the Culling warning (`split` true: the caller passes it from the heading it draws, which has the new-line byte 1 only for this text) is split
/// into lines by byte 1; every other heading, and a single line, goes straight to `fit_text` as before.
pub fn fit_header(out: &mut Vec<Quad>, font: &Font, s: &str, r: [f32; 4], k: f32, ox: f32, colour: [f32; 4], split: bool) {
	let lines: Vec<&str> = s.split('\x01').collect();
	if !split || lines.len() < 2 {
		return crate::frontend::fit_text(out, font, s, r, k, ox, colour);
	}
	let lh = r[3] / lines.len() as f32;
	let scale = lines.iter().map(|l| width(font, l)).filter(|w| *w > 0.0).map(|w| r[2] / w).fold(lh / 16.0, f32::min).min(1.5);
	for (i, l) in lines.iter().enumerate() {
		let w = width(font, l) * scale;
		if w > 0.0 {
			crate::frontend::fit_text(out, font, l, [r[0] + (r[2] - w) / 2.0, r[1] + lh * i as f32, w, lh], k, ox, colour);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn font() -> Font {
		(3, (0..128).map(|_| crate::hud::Glyph { u: 0, v: 0, dy: 0, adv: 8 }).collect())
	}

	/// One line is `fit_text`'s own picture; two lines draw both lines' letters, one above the other, inside the bar.
	#[test]
	fn one_line_is_unchanged_and_two_lines_stack_inside_the_bar() {
		let (f, bar) = (font(), [10.0, 20.0, 100.0, 40.0]);
		let (mut a, mut b) = (Vec::new(), Vec::new());
		fit_header(&mut a, &f, "AB", bar, 1.0, 0.0, [1.0; 4], true);
		crate::frontend::fit_text(&mut b, &f, "AB", bar, 1.0, 0.0, [1.0; 4]);
		assert_eq!(a.len(), b.len());
		assert!(a.iter().zip(&b).all(|(p, q)| p.rect == q.rect));
		let mut two = Vec::new();
		fit_header(&mut two, &f, "ABCD\x01EF", bar, 1.0, 0.0, [1.0; 4], true);
		assert_eq!(two.len(), 2 * 6, "a shadow and a coloured quad per letter, 6 letters");
		for q in &two {
			assert!(q.rect[0] >= 10.0 - 1.0 && q.rect[0] + q.rect[2] <= 110.0 + 3.0, "inside the bar across: {:?}", q.rect);
			assert!(q.rect[1] >= 20.0 - 1.0 && q.rect[1] + q.rect[3] <= 60.0 + 3.0, "inside the bar down: {:?}", q.rect);
		}
		let top = two[2].rect[1];
		let bottom = two[10].rect[1];
		assert!(bottom > top, "line two sits below line one");
	}

	/// Any other page's heading (`split` false) is drawn by `fit_text` alone, even with byte 1 in it, exactly as before the warning.
	#[test]
	fn other_headers_are_never_split() {
		let (f, bar) = (font(), [10.0, 20.0, 100.0, 40.0]);
		let (mut a, mut b) = (Vec::new(), Vec::new());
		fit_header(&mut a, &f, "AB\x01CD", bar, 1.0, 0.0, [1.0; 4], false);
		crate::frontend::fit_text(&mut b, &f, "AB\x01CD", bar, 1.0, 0.0, [1.0; 4]);
		assert_eq!(a.len(), b.len());
		assert!(a.iter().zip(&b).all(|(p, q)| p.rect == q.rect));
	}
}
