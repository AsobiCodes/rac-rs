//! A mod's draw list -> overlay `Quad`s in window pixels (draw_map.rs places the game's 512 x 448 space in the window).
//!
//! Text uses the game's default font (draw_art.rs `Art::font`: the level's glyph cells and advances, size 1 = the font's own 16-texel line, as
//! the retail HUD and menus draw it) and falls back to the built-in 5x7 font of `overlay.rs` (slot `overlay::FONT`, always uploaded) while the
//! art is not loaded (title, before the first level). The fallback's size 1 = a 16 game-pixel line; its glyph advance is 0.643 of the height so
//! that glyphs are 6:8 on screen at the game's own 4:3 stretch (a game pixel is 1.167 times wider than tall there).
//! Order: every `hud`-layer command in list order, then every `menu`-layer command (the GS overlay holds retail's HUD and menus as one picture,
//! so both layers sit above both; see HANDOFF_D.md).

use super::cabi::OmniDrawCmd;
use super::draw::Frame;
use super::draw_art::Art;
use super::draw_map::Layout;
use crate::hud::Glyph;
use crate::overlay::{Quad, FONT};

pub const KIND_TEXT: u32 = 1;
pub const KIND_RECT: u32 = 2;
pub const KIND_BAR: u32 = 3;
pub const KIND_ICON: u32 = 4;

/// Height in game pixels of a text line at size 1.
pub const LINE: f32 = 16.0;
/// Glyph advance as a share of the line height.
pub const ADVANCE: f32 = 0.643;

// The font atlas of overlay.rs (16 glyphs a row, 6 x 8 cells from ' ').
const COLS: usize = 16;
const CELL_W: f32 = 6.0;
const CELL_H: f32 = 8.0;
const FIRST: u8 = 32;
const COUNT: usize = 96;
const ATLAS_W: f32 = COLS as f32 * CELL_W;
const ATLAS_H: f32 = (COUNT.div_ceil(COLS)) as f32 * CELL_H;

/// 0xRRGGBBAA to straight RGBA floats.
pub fn rgba(c: u32) -> [f32; 4] {
	[(c >> 24) as f32 / 255.0, ((c >> 16) & 255) as f32 / 255.0, ((c >> 8) & 255) as f32 / 255.0, (c & 255) as f32 / 255.0]
}

/// The glyph a character draws: ASCII as is (lowercase shows as the font's capitals), anything else as '?'.
fn glyph_of(ch: char) -> u8 {
	if ch.is_ascii() {
		(ch as u8).to_ascii_uppercase().clamp(FIRST, FIRST + COUNT as u8 - 2)
	} else {
		b'?'
	}
}

/// Width in game pixels of `text` at `size`.
pub fn text_width(text: &str, size: f32) -> f32 {
	text.chars().count() as f32 * LINE * size * ADVANCE
}

pub(super) fn solid(out: &mut Vec<Quad>, l: &Layout, x: f32, y: f32, w: f32, h: f32, colour: [f32; 4]) {
	if colour[3] <= 0.0 || w <= 0.0 || h <= 0.0 {
		return;
	}
	let [x0, y0] = l.to_window(x, y);
	let [x1, y1] = l.to_window(x + w, y + h);
	// The font's block glyph (127), inset one texel so the edges stay solid.
	let g = (127 - FIRST) as usize;
	let (u, v) = (((g % COLS) * 6) as f32 + 1.0, ((g / COLS) * 8) as f32 + 1.0);
	out.push(Quad { tex: FONT, rect: [x0, y0, x1 - x0, y1 - y0], uv: [u / ATLAS_W, v / ATLAS_H, (u + 3.0) / ATLAS_W, (v + 5.0) / ATLAS_H], colour, angle: 0.0, corners: None });
}

fn glyphs(out: &mut Vec<Quad>, l: &Layout, text: &str, x: f32, y: f32, size: f32, colour: [f32; 4]) {
	if colour[3] <= 0.0 {
		return;
	}
	let (h, adv) = (LINE * size, LINE * size * ADVANCE);
	for (i, ch) in text.chars().enumerate() {
		let g = glyph_of(ch);
		if g == b' ' {
			continue;
		}
		let k = (g - FIRST) as usize;
		let (u, v) = (((k % COLS) * 6) as f32, ((k / COLS) * 8) as f32);
		let [x0, y0] = l.to_window(x + i as f32 * adv, y);
		let [x1, y1] = l.to_window(x + (i + 1) as f32 * adv, y + h);
		out.push(Quad { tex: FONT, rect: [x0, y0, x1 - x0, y1 - y0], uv: [u / ATLAS_W, v / ATLAS_H, (u + CELL_W) / ATLAS_W, (v + CELL_H) / ATLAS_H], colour, angle: 0.0, corners: None });
	}
}

/// The game font's glyph for a character: ASCII and the font's own codes as is, anything else as '?'; control codes without a glyph give None.
fn game_glyph(font: &(usize, Vec<Glyph>), ch: char) -> Option<Glyph> {
	let at = |c: char| font.1.get(c as usize).copied().filter(|g| g.adv != 0);
	at(ch).or_else(|| at('?'))
}

/// Width in game pixels of `text` in the game font at `size` (1 = the font's own 16-texel cell, one texel one game pixel).
pub fn game_text_width(font: &(usize, Vec<Glyph>), text: &str, size: f32) -> f32 {
	text.chars().filter_map(|ch| game_glyph(font, ch)).map(|g| g.adv as f32 * size).sum()
}

/// `text` in the game's default font (the same cells and advances as the retail HUD text, viewer/hudart.rs `HudArt::font`).
fn game_glyphs(out: &mut Vec<Quad>, l: &Layout, font: &(usize, Vec<Glyph>), text: &str, x: f32, y: f32, size: f32, colour: [f32; 4]) {
	if colour[3] <= 0.0 {
		return;
	}
	// The font sheet is mid-grey: the PS2 draws it with vertex colour 0x80 = x1 and up to 0xFF = x2, so a mod's 0xFF (white) must double.
	let colour = [colour[0] * 2.0, colour[1] * 2.0, colour[2] * 2.0, colour[3]];
	let mut cx = x;
	for ch in text.chars() {
		let Some(g) = game_glyph(font, ch) else { continue };
		if ch != ' ' {
			let [x0, y0] = l.to_window(cx, y + g.dy as f32 * size);
			let [x1, y1] = l.to_window(cx + 16.0 * size, y + (g.dy as f32 + 16.0) * size);
			let (u, v) = (g.u as f32, g.v as f32);
			out.push(Quad { tex: font.0, rect: [x0, y0, x1 - x0, y1 - y0], uv: [u / 256.0, v / 128.0, (u + 16.0) / 256.0, (v + 16.0) / 128.0], colour, angle: 0.0, corners: None });
		}
		cx += g.adv as f32 * size;
	}
}

pub(super) fn text(out: &mut Vec<Quad>, l: &Layout, art: Option<&Art>, f: &Frame, c: &OmniDrawCmd) {
	let s = f.text(c);
	let size = if c.w.is_finite() && c.w > 0.0 { c.w.min(16.0) } else { 1.0 };
	let font = art.and_then(|a| a.font.as_ref());
	let w = font.map_or_else(|| text_width(s, size), |ft| game_text_width(ft, s, size));
	let x = match c.align {
		1 => c.x - w / 2.0,
		2 => c.x - w,
		_ => c.x,
	};
	let mut put = |dx: f32, colour: [f32; 4]| match font {
		Some(ft) => game_glyphs(out, l, ft, s, x + dx, c.y + dx, size, colour),
		None => glyphs(out, l, s, x + dx, c.y + dx, size, colour),
	};
	// Shadow (colour2, 0 = none): one game pixel down and right, scaled with the size.
	if c.color2 & 255 != 0 {
		put(size, rgba(c.color2));
	}
	put(0.0, rgba(c.color));
}

/// An icon of the game's HUD art (draw_art.rs table) tinted by `color` (0 = no tint); a width or height of 0 draws it at its own size.
/// An icon the level's art does not hold, or no art at all, draws nothing.
pub(super) fn icon(out: &mut Vec<Quad>, l: &Layout, art: Option<&Art>, c: &OmniDrawCmd) {
	let Some(&Some((slot, tw, th))) = art.and_then(|a| a.icons.get(c.icon as usize)) else { return };
	let (w, h) = if c.w > 0.0 && c.h > 0.0 { (c.w, c.h) } else { (tw, th) };
	let colour = if c.color == 0 { [1.0; 4] } else { rgba(c.color) };
	if colour[3] <= 0.0 || !w.is_finite() || !h.is_finite() {
		return;
	}
	let [x0, y0] = l.to_window(c.x, c.y);
	let [x1, y1] = l.to_window(c.x + w, c.y + h);
	out.push(Quad { tex: slot, rect: [x0, y0, x1 - x0, y1 - y0], uv: [0.0, 0.0, 1.0, 1.0], colour, angle: 0.0, corners: None });
}

fn bar(out: &mut Vec<Quad>, l: &Layout, c: &OmniDrawCmd) {
	solid(out, l, c.x, c.y, c.w, c.h, rgba(c.color2));
	let frac = if c.value.is_finite() { c.value.clamp(0.0, 1.0) } else { 0.0 };
	solid(out, l, c.x, c.y, c.w * frac, c.h, rgba(c.color));
}

/// All quads of a list, `hud` layer first.
pub fn build(f: &Frame, l: &Layout, art: Option<&Art>) -> Vec<Quad> {
	let mut out = Vec::with_capacity(f.cmds.len() * 4);
	for layer in [0u32, 1] {
		for c in f.cmds.iter().filter(|c| (c.layer != 0) as u32 == layer) {
			match c.kind {
				KIND_TEXT => text(&mut out, l, art, f, c),
				KIND_RECT => solid(&mut out, l, c.x, c.y, c.w, c.h, rgba(c.color)),
				KIND_BAR => bar(&mut out, l, c),
				KIND_ICON => icon(&mut out, l, art, c),
				// Unknown kinds are ignored.
				_ => {}
			}
		}
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::omni::draw::Slot;

	fn frame(cmds: &[OmniDrawCmd], pool: &[u8]) -> std::sync::Arc<Frame> {
		let s = Slot::new();
		s.submit(cmds, pool);
		s.current().expect("frame")
	}

	fn cmd(kind: u32, layer: u32) -> OmniDrawCmd {
		OmniDrawCmd { kind, layer, ..Default::default() }
	}

	#[test]
	fn colours_are_rrggbbaa() {
		assert_eq!(rgba(0xFF8000FF), [1.0, 128.0 / 255.0, 0.0, 1.0]);
		assert_eq!(rgba(0x00000000)[3], 0.0);
	}

	#[test]
	fn a_rect_lands_on_the_mapped_pixels() {
		let mut r = cmd(KIND_RECT, 0);
		(r.x, r.y, r.w, r.h, r.color) = (0.0, 0.0, 512.0, 448.0, 0xFFFFFFFF);
		let l = Layout::plain(1920.0, 1080.0);
		let q = build(&frame(&[r], &[]), &l, None);
		assert_eq!(q.len(), 1);
		assert!((q[0].rect[0] - 240.0).abs() < 0.01 && (q[0].rect[2] - 1440.0).abs() < 0.01 && (q[0].rect[3] - 1080.0).abs() < 0.01);
	}

	#[test]
	fn a_bar_is_a_back_rect_and_a_fraction_of_fill() {
		let mut b = cmd(KIND_BAR, 0);
		(b.x, b.y, b.w, b.h, b.value, b.color, b.color2) = (100.0, 50.0, 200.0, 10.0, 0.25, 0x00FF00FF, 0x000000FF);
		let l = Layout::plain(1024.0, 768.0);
		let q = build(&frame(&[b], &[]), &l, None);
		assert_eq!(q.len(), 2);
		assert!((q[1].rect[2] / q[0].rect[2] - 0.25).abs() < 1e-4);
		// A fraction outside 0..1 or NaN is clamped.
		b.value = f32::NAN;
		assert_eq!(build(&frame(&[b], &[]), &l, None).len(), 1);
		b.value = 7.0;
		let q = build(&frame(&[b], &[]), &l, None);
		assert!((q[1].rect[2] - q[0].rect[2]).abs() < 1e-3);
	}

	#[test]
	fn text_aligns_and_shadows() {
		let mut t = cmd(KIND_TEXT, 0);
		(t.x, t.y, t.w, t.text_len, t.color) = (256.0, 20.0, 1.0, 4, 0xFFFFFFFF);
		let l = Layout::plain(1024.0, 768.0);
		let left = build(&frame(&[t], b"TEST"), &l, None);
		assert_eq!(left.len(), 4);
		t.align = 1;
		let centre = build(&frame(&[t], b"TEST"), &l, None);
		let width = text_width("TEST", 1.0);
		assert!((centre[0].rect[0] - (l.to_window(256.0 - width / 2.0, 0.0)[0])).abs() < 0.01);
		t.align = 2;
		let right = build(&frame(&[t], b"TEST"), &l, None);
		assert!((right[3].rect[0] + right[3].rect[2] - l.to_window(256.0, 0.0)[0]).abs() < 0.01);
		// A shadow doubles the glyph quads and is drawn first.
		t.color2 = 0x000000FF;
		let shadowed = build(&frame(&[t], b"TEST"), &l, None);
		assert_eq!(shadowed.len(), 8);
		assert_eq!(shadowed[0].colour[3], 1.0);
		assert_eq!(shadowed[0].colour[0], 0.0);
	}

	#[test]
	fn layers_and_unknown_kinds() {
		let mut a = cmd(KIND_RECT, 1);
		(a.w, a.h, a.color, a.x) = (10.0, 10.0, 0xFF0000FF, 1.0);
		let mut b = cmd(KIND_RECT, 0);
		(b.w, b.h, b.color, b.x) = (10.0, 10.0, 0x00FF00FF, 2.0);
		let l = Layout::plain(512.0, 448.0 * 0.75 * 1.0);
		let q = build(&frame(&[a, b, cmd(99, 0), cmd(KIND_ICON, 0)], &[]), &l, None);
		// Two rects: the hud layer one (green) first although it came second in the list.
		assert_eq!(q.len(), 2);
		assert_eq!(q[0].colour[1], 1.0);
		assert_eq!(q[1].colour[0], 1.0);
	}

	/// One window pixel per game pixel.
	fn ident() -> Layout {
		Layout { rect: [0.0, 0.0, 512.0, 448.0], uv: [0.0, 0.0, 1.0, 1.0] }
	}

	/// A font with 'A' (code 65, adv 10, drawn 2 lower) and '?' (63, adv 8) and nothing else.
	fn art() -> Art {
		let mut glyphs = vec![Glyph { u: 0, v: 0, dy: 0, adv: 0 }; 128];
		glyphs[65] = Glyph { u: 32, v: 16, dy: 2, adv: 10 };
		glyphs[63] = Glyph { u: 48, v: 0, dy: 0, adv: 8 };
		glyphs[32] = Glyph { u: 0, v: 0, dy: 0, adv: 5 };
		Art { font: Some((7, glyphs)), icons: vec![Some((9, 32.0, 16.0)), None] }
	}

	#[test]
	fn game_font_text_uses_the_glyph_cells_and_advances() {
		let mut t = cmd(KIND_TEXT, 0);
		(t.x, t.y, t.w, t.color) = (100.0, 50.0, 2.0, 0xFFFFFFFF);
		let s = b"A A?z";
		t.text_len = s.len() as u32;
		let l = ident();
		let a = art();
		let q = build(&frame(&[t], s), &l, Some(&a));
		// 'A', 'A', '?', and 'z' (no glyph) as '?': four quads; the space draws none.
		assert_eq!(q.len(), 4);
		assert!(q.iter().all(|q| q.tex == 7));
		// The mid-grey font sheet needs x2 colour for white (PS2 vertex 0x80 = x1); alpha stays.
		assert_eq!(q[0].colour, [2.0, 2.0, 2.0, 1.0]);
		assert_eq!(q[0].uv, [32.0 / 256.0, 16.0 / 128.0, 48.0 / 256.0, 32.0 / 128.0]);
		// Size 2: 32 x 32 game pixels (one window pixel per game pixel), 'A' dy 2 -> y 50 + 4.
		assert!((q[0].rect[2] - 32.0).abs() < 1e-3 && (q[0].rect[3] - 32.0).abs() < 1e-3, "{:?}", q[0].rect);
		assert!((q[0].rect[1] - 54.0).abs() < 1e-3);
		// The second 'A' starts one advance (10 * 2) plus the space (5 * 2) later.
		assert!((q[1].rect[0] - q[0].rect[0] - 30.0).abs() < 1e-3);
		assert!((game_text_width(a.font.as_ref().unwrap(), "A A", 1.0) - 25.0).abs() < 1e-4);
	}

	#[test]
	fn game_font_text_aligns_and_shadows_with_the_game_widths() {
		let mut t = cmd(KIND_TEXT, 0);
		(t.x, t.y, t.w, t.color, t.align) = (256.0, 10.0, 1.0, 0xFFFFFFFF, 1);
		let s = b"AA";
		t.text_len = 2;
		let l = ident();
		let a = art();
		let q = build(&frame(&[t], s), &l, Some(&a));
		assert!((q[0].rect[0] - (256.0 - 10.0)).abs() < 1e-3);
		t.color2 = 0x000000FF;
		let q = build(&frame(&[t], s), &l, Some(&a));
		assert_eq!(q.len(), 4);
		assert_eq!(q[0].colour[0], 0.0);
		assert!((q[0].rect[0] - q[2].rect[0] - 1.0).abs() < 1e-3);
		// Without the art the 5x7 font answers (slot FONT).
		let q = build(&frame(&[t], s), &l, None);
		assert!(q.iter().all(|q| q.tex == FONT));
	}

	#[test]
	fn icons_use_their_slot_size_and_tint() {
		let mut i = cmd(KIND_ICON, 0);
		(i.x, i.y, i.w, i.h, i.icon, i.color) = (10.0, 20.0, 0.0, 0.0, 0, 0);
		let l = ident();
		let a = art();
		let q = build(&frame(&[i], &[]), &l, Some(&a));
		assert_eq!(q.len(), 1);
		assert_eq!(q[0].tex, 9);
		assert_eq!(q[0].rect, [10.0, 20.0, 32.0, 16.0]);
		assert_eq!(q[0].colour, [1.0; 4]);
		// Given size and tint.
		(i.w, i.h, i.color) = (64.0, 64.0, 0xFF000080);
		let q = build(&frame(&[i], &[]), &l, Some(&a));
		assert_eq!(q[0].rect, [10.0, 20.0, 64.0, 64.0]);
		assert!((q[0].colour[3] - 128.0 / 255.0).abs() < 1e-4);
		// An icon the level does not hold, an index outside the table, and no art at all draw nothing.
		i.icon = 1;
		assert!(build(&frame(&[i], &[]), &l, Some(&a)).is_empty());
		i.icon = 500;
		assert!(build(&frame(&[i], &[]), &l, Some(&a)).is_empty());
		i.icon = 0;
		assert!(build(&frame(&[i], &[]), &l, None).is_empty());
	}

	#[test]
	fn non_ascii_text_draws_question_marks_and_never_panics() {
		let mut t = cmd(KIND_TEXT, 0);
		(t.w, t.color) = (1.0, 0xFFFFFFFF);
		let s = "a\u{e9}\u{4e2d}".as_bytes();
		t.text_len = s.len() as u32;
		let q = build(&frame(&[t], s), &Layout::plain(640.0, 480.0), None);
		assert_eq!(q.len(), 3);
	}
}
