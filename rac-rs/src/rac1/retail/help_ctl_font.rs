//! The game's own menu font for the repainted Help > Controls labels (`help_ctl.rs`): the glyph bitmaps of font 1 (the level's font
//! sheet, 256x128 indices + palette, the sheet `glyphs.rs` keeps for the GS capture) with retail's glyph table (u, v, dy, advance;
//! boot data in RAM), area-downscaled to the art's cap height (8 px) and letter-spaced like the art's own words. The sheet is
//! read at run time from the player's disc data; nothing of it is stored here. Pure: tests and the planner use `measure` / `line`.

use super::menu_model::text::font_char;
use std::sync::{Arc, Mutex};

/// Rows of a line's canvas: caps start at row 1, the baseline is the bottom edge of row `CAP`, descenders reach row 12.
pub const LINE_H: u32 = 13;
/// Rows above a line's cell that accents (umlauts) may use: a line's canvas starts `TOP` rows above its `y`.
pub const TOP: u32 = 2;
/// Cap height in pixels the art uses.
pub const CAP: u32 = 8;
/// Extra space between letters in output pixels (the art's words are letter-spaced), at `Face::Normal`.
const TRACK: f32 = 1.6;
/// Ink strength: the font's strokes are thin, the art's are drawn near full white.
const BOOST: f32 = 2.0;

/// Width of the letters: the art's letter-spacing at full width, then two narrower faces for tight boxes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
	Normal,
	Tight,
	Packed,
}

impl Face {
	fn sx(self) -> f32 {
		match self {
			Face::Normal => 1.11,
			Face::Tight => 0.96,
			Face::Packed => 0.83,
		}
	}
}

struct Gl {
	cov: [u8; 256],
	dy: i16,
	adv: i16,
	/// First inked column of the 16x16 cell.
	left: u8,
}

/// Font 1 as coverage cells.
pub struct Font {
	gl: Vec<Gl>,
	cap_top: i32,
	/// Output pixels per source pixel in y (and in x at `Face::Normal`).
	s: f32,
	by_char: Vec<(char, u8)>,
}

/// Retail's glyph table of font 1 (u, v, dy, adv), read from the boot executable on the disc by the host at each level load (`hudart.rs`):
/// the boot data in RAM is overwritten by the level overlay, so it cannot be read from RAM.
static TABLE: Mutex<Vec<(u8, u8, i8, u8)>> = Mutex::new(Vec::new());

pub fn set_table(t: Vec<(u8, u8, i8, u8)>) {
	*TABLE.lock().unwrap_or_else(|e| e.into_inner()) = t;
}

pub fn table() -> Vec<(u8, u8, i8, u8)> {
	TABLE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

static FONT: Mutex<Option<(usize, Arc<Font>)>> = Mutex::new(None);

/// The font the planner measures with; `key` names the source (a changed key rebuilds).
pub fn install(key: usize, make: impl FnOnce() -> Option<Font>) -> Option<Arc<Font>> {
	let mut g = FONT.lock().unwrap_or_else(|e| e.into_inner());
	if let Some((k, f)) = g.as_ref() {
		if *k == key {
			return Some(f.clone());
		}
	}
	let f = Arc::new(make()?);
	*g = Some((key, f.clone()));
	Some(f)
}

pub fn get() -> Option<Arc<Font>> {
	FONT.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|x| x.1.clone())
}

impl Font {
	/// `sheet`: 256x128 palette indices, `alpha`: coverage per palette index, `table`: (u, v, dy, adv) per code (232 entries).
	pub fn new(sheet: &[u8], alpha: &[u8; 256], table: &[(u8, u8, i8, u8)]) -> Option<Font> {
		if sheet.len() < 256 * 128 || table.len() < 0x80 {
			return None;
		}
		let gl: Vec<Gl> = table
			.iter()
			.map(|&(u, v, dy, adv)| {
				let mut cov = [0u8; 256];
				for y in 0..16usize {
					for x in 0..16usize {
						let (sx, sy) = (u as usize + x, v as usize + y);
						if sx < 256 && sy < 128 {
							cov[y * 16 + x] = alpha[sheet[sy * 256 + sx] as usize];
						}
					}
				}
				let left = (0..16).find(|&x| (0..16).any(|y| cov[y * 16 + x] > 60)).unwrap_or(0) as u8;
				Gl { cov, dy: dy as i16, adv: adv as i16, left }
			})
			.collect();
		let h = &gl[b'H' as usize];
		let (hdy, rows): (i32, Vec<i32>) = (h.dy as i32, (0..16).filter(|&y| (0..16).any(|x| h.cov[y as usize * 16 + x] > 60)).collect());
		let (top, bot) = (*rows.first()?, *rows.last()?);
		let cap_h = (bot - top + 1) as f32;
		let by_char = (0x20u16..=0xff).filter_map(|b| font_char(b as u8).map(|c| (c, b as u8))).collect();
		Some(Font { gl, cap_top: top + hdy, s: CAP as f32 / cap_h, by_char })
	}

	/// The glyph code retail's text drawer uses for `c` (the text bank's own byte code: `font_char` is its table) and its cell.
	fn glyph(&self, c: char) -> Option<(usize, &Gl)> {
		let code = self.by_char.iter().find(|e| e.0 == c).map_or(b' ', |e| e.1) as usize;
		self.gl.get(code).map(|g| (code, g))
	}

	/// True when `c` has a real glyph cell (a space counts); false = `line` would skip it.
	pub fn knows(&self, c: char) -> bool {
		c == ' ' || self.by_char.iter().any(|e| e.0 == c && self.gl.get(e.1 as usize).is_some_and(|g| g.adv != 0))
	}

	/// The accent cell of codes 0x80..=0xa7: retail (FUN_0021cbf8) first blits table entry `code + 0x40` at (x + entry.adv as i8, y + entry.dy), then the base cell.
	fn accent(&self, code: usize) -> Option<&Gl> {
		if (0x80..=0xa7).contains(&code) { self.gl.get(code + 0x40) } else { None }
	}

	fn track(&self, face: Face) -> f32 {
		TRACK * face.sx()
	}

	fn step(&self, face: Face, g: &Gl) -> f32 {
		g.adv as f32 * self.s * face.sx() + self.track(face)
	}

	/// Width of `text` in px (first ink column to the last pen position).
	pub fn measure(&self, face: Face, text: &str) -> f32 {
		let mut w = 0.0;
		let mut first = true;
		for c in text.chars() {
			let Some((_, g)) = self.glyph(c) else { continue };
			if first {
				w -= g.left as f32 * self.s * face.sx();
				first = false;
			}
			w += self.step(face, g);
		}
		(w - self.track(face)).max(0.0)
	}

	/// `text` as coverage: (width, `LINE_H + TOP` rows of `width` bytes, row-major, the first `TOP` rows above the line's y). The first inked column is x = 1.
	pub fn line(&self, face: Face, text: &str) -> (u32, Vec<u8>) {
		let w = self.measure(face, text).ceil() as u32 + 3;
		let rows = (LINE_H + TOP) as i32;
		let mut acc = vec![0f32; (w * rows as u32) as usize];
		let sxe = self.s * face.sx();
		let mut pen = 1.0f32;
		let mut first = true;
		for c in text.chars() {
			let Some((code, g)) = self.glyph(c) else { continue };
			if first {
				pen -= g.left as f32 * sxe;
				first = false;
			}
			let acc_cell = self.accent(code).map(|e| (e, pen + (e.adv as i8) as f32 * sxe, e.dy as i32));
			let cells = acc_cell.into_iter().chain(std::iter::once((g, pen, g.dy as i32)));
			for (cell, cx, cdy) in cells {
				for gy in 0..if c == ' ' { 0 } else { 16i32 } {
					let top = (1 + TOP) as f32;
					let (y0, y1) = (top + (gy + cdy - self.cap_top) as f32 * self.s, top + (gy + 1 + cdy - self.cap_top) as f32 * self.s);
					for gx in 0..16i32 {
						let v = cell.cov[(gy * 16 + gx) as usize];
						if v == 0 {
							continue;
						}
						let (x0, x1) = (cx + gx as f32 * sxe, cx + (gx + 1) as f32 * sxe);
						for oy in y0.floor() as i32..y1.ceil() as i32 {
							let ho = (y1.min(oy as f32 + 1.0) - y0.max(oy as f32)).max(0.0);
							for ox in x0.floor() as i32..x1.ceil() as i32 {
								let wo = (x1.min(ox as f32 + 1.0) - x0.max(ox as f32)).max(0.0);
								if (0..w as i32).contains(&ox) && (0..rows).contains(&oy) {
									acc[(oy as u32 * w + ox as u32) as usize] += v as f32 / 255.0 * ho * wo;
								}
							}
						}
					}
				}
			}
			pen += self.step(face, g);
		}
		(w, acc.iter().map(|&a| (a * BOOST * 255.0 + 0.5).min(255.0) as u8).collect())
	}
}

/// Width of `text` with the installed font (a rough guess when none is installed: the planner then paints nothing anyway).
pub fn measure(face: Face, text: &str) -> f32 {
	get().map_or(text.chars().count() as f32 * 7.0, |f| f.measure(face, text))
}
