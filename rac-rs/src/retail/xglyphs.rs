//! The Xbox button artwork, as the game can draw it.
//!
//! The button prompts can show the buttons the way an Xbox pad does instead of the retail's own PlayStation
//! glyphs. The pictures come from `xboxglyphs/` (drawn for this fork, GPL-3.0-or-later) and are cut down to one
//! 16x16 cell per button, the size of a cell in the retail's 256x128 font sheet. The script that makes them is
//! `tools/make_xbox_glyphs.py`.
//!
//! The artwork goes in two places. The fork's own menus (the title screen, the pause menu's root page, the shop,
//! the help) draw the font it decoded off the disc, so the pictures are blitted into the ten cells the retail
//! keeps its button glyphs in (`blit`). The retail's own prompts (the in-game tutorial text, the Quick Select
//! ring, the paged menus) are drawn out of the graphics memory, where the font sheet is 8-bit indexed, so there
//! the pictures are quantised into the font's own palette (`quantise`) and written over the ten cells in the
//! sheet's upload (`retail::gscap`'s font patch).
//!
//! A cell holds the picture of the physical button its function is played by, so a rebound function is shown as
//! the button it was bound to (the map of `binds.rs`): the Cross cell holds the A picture until the Cross is
//! bound to another button.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::binds::{DEFAULT_PAD, NP};

/// The cell size: the retail's button cells are 24 wide and 16 tall. The art sits in a 16-wide
/// window at x 2, the text after a button glyph starts 18 columns in, and the rest of the cell is
/// overdrawn by it.
pub const CELL_W: usize = 24;
pub const CELL_H: usize = 16;
/// Where the art sits in the cell: the 16-wide window the retail's own art is in.
pub const ART_X: usize = 2;
pub const ART_W: usize = 16;

/// The ten button cells: the retail's glyph code and the cell's place in the sheet (u, v). The order is the
/// sheet's, and the order of the cells in the artwork files.
pub const CELLS: [(u8, u16, u16); 10] = [
	(0x10, 0, 96),   // Cross
	(0x13, 24, 96),  // Square
	(0x12, 48, 96),  // Triangle
	(0x11, 72, 96),  // Circle
	(0x14, 96, 96),  // L1
	(0x16, 120, 96), // L2
	(0x15, 144, 96), // R1
	(0x17, 168, 96), // R2
	(0x18, 208, 96), // the right stick
	(0x19, 232, 96), // the left stick
];

/// The artwork: the cells as RGBA, one 24x16 cell after the other, in the order of `CELLS`.
static XBOX: &[u8] = include_bytes!("../../assets/glyphs/xbox.rgba");
static SWITCH: &[u8] = include_bytes!("../../assets/glyphs/switch.rgba");

/// The artwork for the style: 1 Xbox, 2 Switch; None for the PlayStation style (the retail's own glyphs, no
/// artwork).
pub fn cells(style: u8) -> Option<&'static [u8]> {
	match style {
		1 => Some(XBOX),
		2 => Some(SWITCH),
		_ => None,
	}
}

/// The artwork's cell `k` (in the order of `CELLS`) as RGBA pixels.
pub fn cell(style: u8, k: usize) -> Option<&'static [u8]> {
	cells(style).map(|c| &c[k * CELL_W * CELL_H * 4..(k + 1) * CELL_W * CELL_H * 4])
}

/// The cell the picture of the function `f` comes from: the cell of the physical button the function is played
/// by (`map`), which is the cell of the function that button is the default for. None when the button has no
/// picture of its own (Start, Back, the d-pad): the cell keeps the retail's own glyph then.
fn source_cell(f: usize, map: &[u8; NP]) -> Option<usize> {
	let p = map[f] as usize;
	CELLS.iter().position(|&(c, _, _)| {
		super::prompts::bound_function(c).is_some_and(|g| DEFAULT_PAD[g] as usize == p)
	})
}

/// The artwork in the font texture's button cells: the fork's own menus draw the font it decoded off the disc,
/// so the pictures go into the ten cells the retail keeps its button glyphs in. The texture is the sheet (256 by
/// 128) and is changed in place.
///
/// The picture in a cell is the one of the physical button its function is played by (`map`), so a rebound
/// function is shown as the button it was bound to. `orig` is the sheet as it was decoded: the PlayStation style
/// takes the retail's own symbols out of it (the Xbox and Switch artwork comes from the files), and the texture
/// is written from it, so the style can be changed back and forth.
pub fn blit(orig: &crate::level::Texture, live: &mut crate::level::Texture, style: u8, map: &[u8; NP]) -> bool {
	if orig.w < 256 || orig.h < 128 || live.w < 256 || live.h < 128 {
		return false;
	}
	let art = cells(style);
	for (k, &(_, u, v)) in CELLS.iter().enumerate() {
		let Some(f) = super::prompts::bound_function(CELLS[k].0) else { continue };
		let Some(s) = source_cell(f, map) else { continue };
		let (su, sv) = (CELLS[s].1 as usize, CELLS[s].2 as usize);
		for y in 0..CELL_H {
			for x in 0..CELL_W {
				let px = match art {
					Some(art) => {
						let a = (s * CELL_H * CELL_W + y * CELL_W + x) * 4;
						[art[a], art[a + 1], art[a + 2], art[a + 3]]
					}
					// The PlayStation style: the retail's own symbol of the button, out of the sheet as it was decoded.
					None => orig.rgba[(sv + y) * orig.w + su + x],
				};
				live.rgba[(v as usize + y) * live.w + u as usize + x] = px;
			}
		}
	}
	true
}

/// The artwork as indices into the font's palette: each pixel takes the palette's nearest colour. The font sheet
/// is 8-bit indexed with a 256-colour palette the text already uses most of, so the pictures are quantised into
/// what is there rather than given colours of their own. The palette is in the GS's index order (the CSM1's
/// index bits 3 and 4 are swapped, `hud::palette_rgba`).
pub fn quantise(art: &[u8], palette: &[[u8; 4]]) -> Vec<u8> {
	art.chunks(4).map(|px| nearest(px, palette)).collect()
}

/// The font's background: its "transparent" is a dither of two colours in 4x4 blocks, aligned to the sheet's origin, so the
/// colour at (x, y) is the even one when (x/4 + y/4) is even and the odd one when it is not. The two are the sheet's own texels in
/// the gap between a cell's art (columns 2..17) and the next cell (column 18).
fn background_indices(sheet: &[u8], u: usize, v: usize) -> [u8; 2] {
	let at = |x: usize| sheet.get(v * 256 + u + x).copied().unwrap_or(0);
	let even = |x: usize, y: usize| ((x / 4) + (y / 4)) % 2 == 0;
	let (a, b) = (at(18), at(20));
	if even(u + 18, v) {
		[a, b]
	} else {
		[b, a]
	}
}

/// The artwork's cells as indices into the font's palette: the picture's pixels take the nearest colour and the transparent ones take
/// the font's own dithered background, so a button sits on the same background as the text around it. The picture in a cell is the
/// one of the physical button its function is played by (`map`).
pub fn quantised_cells(style: u8, palette: &[[u8; 4]], sheet: &[u8], map: &[u8; NP]) -> Vec<Vec<u8>> {
	let Some(art) = cells(style) else { return Vec::new() };
	CELLS
		.iter()
		.enumerate()
		.map(|(k, &(_, u, v))| {
			let Some(f) = super::prompts::bound_function(CELLS[k].0) else { return Vec::new() };
			let Some(s) = source_cell(f, map) else { return Vec::new() };
			let bg = background_indices(sheet, u as usize, v as usize);
			art[s * CELL_H * CELL_W * 4..(s + 1) * CELL_H * CELL_W * 4]
				.chunks(4)
				.enumerate()
				.map(|(i, px)| {
					if px[3] == 0 {
						let x = u as usize + i % CELL_W;
						let y = v as usize + i / CELL_W;
						return bg[(((x / 4) + (y / 4)) % 2) as usize];
					}
					nearest(px, palette)
				})
				.collect()
		})
		.collect()
}

/// The palette entry nearest to a colour.
fn nearest(px: &[u8], palette: &[[u8; 4]]) -> u8 {
	let mut best = 0u8;
	let mut best_d = u32::MAX;
	for (i, c) in palette.iter().enumerate().take(256) {
		let d: u32 = (0..4).map(|k| (px[k] as i32 - c[k] as i32).unsigned_abs() as u32).sum();
		if d < best_d {
			best_d = d;
			best = i as u8;
		}
	}
	best
}

/// The font patch the graphics memory capture uses: the font sheet's own texels, so the upload that carries the
/// sheet can be recognised, and the artwork's cells as indices into the font's palette.
type Patch = Arc<PatchData>;

/// The sheet's texels and the artwork's cells for each sheet and style.
pub struct PatchData {
	/// The font sheets' own texels (256x128, one byte each, row by row): the game has three, and the prompts use
	/// whichever it likes, so the upload is recognised by its content.
	pub sheets: Vec<Vec<u8>>,
	/// The palette of each sheet, so the artwork can be quantised into it again when the bindings change.
	pub palettes: Vec<Vec<[u8; 4]>>,
	/// The cells of each sheet, three styles apiece (0 PlayStation, 1 Xbox, 2 Switch), as indices into that sheet's palette.
	/// The cells of sheet `k` in style `s` are at `k * 3 + s`. They are the cells of the bindings as they were when the
	/// patch was made.
	pub cells: Vec<Vec<Vec<u8>>>,
	/// The bindings the cells were made for.
	pub map: [u8; NP],
}

impl PatchData {
	/// The cells of the sheet `which` in the style `style` for the bindings `map`: the ones made when the patch was built, or
	/// fresh ones when the bindings have changed since (a rebound function is shown as the button it was bound to).
	pub fn cells_for(&self, which: usize, style: usize, map: &[u8; NP]) -> Vec<Vec<u8>> {
		if map == &self.map {
			return self.cells[which * 3 + style].clone();
		}
		let sheet = &self.sheets[which];
		let palette = &self.palettes[which];
		match style {
			0 => original_cells(sheet, map),
			1 => quantised_cells(1, palette, sheet, map),
			_ => quantised_cells(2, palette, sheet, map),
		}
	}
}

/// The patch, shared with the capture thread (the sheet is uploaded per level, the patch with it).
static PATCH: Mutex<Option<Patch>> = Mutex::new(None);

/// Whether the artwork is in the font's cells in the graphics memory: the capture sets it when it has patched
/// the sheet. The prompts then draw the pictures; without it they fall back to the button names.
static ARTWORK: AtomicBool = AtomicBool::new(false);

/// Whether the artwork is in the font's cells.
pub fn artwork_in_place() -> bool {
	ARTWORK.load(Ordering::Relaxed)
}

/// The capture says the artwork is in the sheet's cells (or that it is not).
pub fn set_artwork_in_place(on: bool) {
	ARTWORK.store(on, Ordering::Relaxed)
}

/// Sets the patch: the font sheet's texels and the artwork's cells for each style.
pub fn set_patch(p: PatchData) {
	*PATCH.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(p));
}

/// The patch, if there is one.
pub fn patch() -> Option<Patch> {
	PATCH.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The sheet's own texels in the ten button cells: the PlayStation style, so the retail's glyphs can be put back when the
/// player changes the style away from the artwork. A function bound to another button takes that button's symbol.
fn original_cells(sheet: &[u8], map: &[u8; NP]) -> Vec<Vec<u8>> {
	CELLS
		.iter()
		.enumerate()
		.map(|(k, _)| {
			let src = super::prompts::bound_function(CELLS[k].0).and_then(|f| source_cell(f, map)).unwrap_or(k);
			let (su, sv) = (CELLS[src].1 as usize, CELLS[src].2 as usize);
			let mut cell = Vec::with_capacity(CELL_W * CELL_H);
			for y in 0..CELL_H {
				for x in 0..CELL_W {
					let p = (sv + y) * 256 + su + x;
					cell.push(sheet.get(p).copied().unwrap_or(0));
				}
			}
			cell
		})
		.collect()
}

/// The patch for the font sheets of `level`: the sheets' texels and the artwork's cells for each style, already
/// quantised into the font's own palette. None when the style is the retail's own glyphs (no artwork) or the
/// sheets cannot be read.
pub fn patch_for(level: &crate::level::Level, glyphs: &[Vec<crate::hud::Glyph>]) -> Option<PatchData> {
	let fonts = crate::hud::level_fonts_raw(level, glyphs).ok()?;
	let map = super::binds::pad_map();
	let mut sheets = Vec::new();
	let mut palettes = Vec::new();
	let mut cells = Vec::new();
	for (_, pixels, palette) in fonts {
		sheets.push(pixels.clone());
		palettes.push(palette.clone());
		// The first of the three styles is the PlayStation one: the retail's own glyphs.
		cells.push(original_cells(&pixels, &map));
		cells.push(quantised_cells(1, &palette, &pixels, &map));
		cells.push(quantised_cells(2, &palette, &pixels, &map));
	}
	Some(PatchData { sheets, palettes, cells, map })
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_cells_match_the_prompts_glyph_codes() {
		use crate::retail::prompts::*;
		let codes: Vec<u8> = CELLS.iter().map(|&(c, _, _)| c).collect();
		assert_eq!(
			codes,
			vec![CROSS, SQUARE, TRIANGLE, CIRCLE, L1, L2, R1, R2, RSTICK, LSTICK]
		);
	}

	#[test]
	fn the_artwork_is_ten_cells_of_the_cell_size() {
		for style in [1u8, 2] {
			let art = cells(style).expect("the artwork");
			assert_eq!(art.len(), CELLS.len() * CELL_W * CELL_H * 4, "style {style}");
			for (k, _) in CELLS.iter().enumerate() {
				let c = cell(style, k).expect("the cell");
				assert_eq!(c.len(), CELL_W * CELL_H * 4);
				// The pictures have something in them (not a blank cell).
				assert!(c.chunks(4).any(|p| p[3] != 0), "cell {k} is empty");
			}
		}
		assert!(cells(0).is_none(), "the PlayStation style has no artwork");
	}

	#[test]
	fn the_artwork_is_blitted_into_the_font_sheet() {
		let orig = crate::level::Texture { w: 256, h: 128, rgba: vec![[0, 0, 0, 0]; 256 * 128] };
		let mut tex = orig.clone();
		assert!(blit(&orig, &mut tex, 1, &super::super::binds::DEFAULT_PAD));
		// The Cross cell (u = 0, v = 96) holds the artwork's first pixel.
		let art = cell(1, 0).expect("the cell");
		let p = 96 * 256;
		assert_eq!(tex.rgba[p], [art[0], art[1], art[2], art[3]]);
		// The cell is no longer blank (the disc is in the middle of it).
		assert!(tex.rgba[p..p + 24].iter().any(|p| p.iter().any(|&b| b != 0)));
		// The art is in the 16-wide window the retail's own art is in.
		let a = cell(1, 0).expect("the cell");
		assert!(
			a[2 * CELL_W..2 * CELL_W + ART_W].chunks(4).any(|p| p[3] != 0),
			"the art is at x {ART_X}"
		);
	}

	/// The cells of a texture, cell by cell.
	fn cells_of(tex: &crate::level::Texture) -> Vec<Vec<[u8; 4]>> {
		CELLS
			.iter()
			.map(|&(_, u, v)| {
				let mut c = Vec::with_capacity(CELL_W * CELL_H);
				for y in 0..CELL_H {
					for x in 0..CELL_W {
						c.push(tex.rgba[(v as usize + y) * 256 + u as usize + x]);
					}
				}
				c
			})
			.collect()
	}

	/// The artwork's cell as pixels.
	fn art_of(k: usize) -> Vec<[u8; 4]> {
		cell(1, k).expect("the cell").chunks(4).map(|p| [p[0], p[1], p[2], p[3]]).collect()
	}

	/// The picture in a cell is the one of the physical button its function is played by: the Circle bound to the RT shows
	/// the RT's picture, the other cells their own.
	#[test]
	fn the_picture_is_the_one_of_the_bound_button() {
		use super::super::binds;
		let orig = crate::level::Texture { w: 256, h: 128, rgba: vec![[0, 0, 0, 0]; 256 * 128] };
		// The defaults: every cell holds its own picture.
		let mut tex = orig.clone();
		assert!(blit(&orig, &mut tex, 1, &binds::DEFAULT_PAD));
		let cells = cells_of(&tex);
		for k in 0..CELLS.len() {
			assert_eq!(cells[k], art_of(k), "cell {k}");
		}
		// The Circle on the RT: the Circle's cell holds the R2 cell's picture (the RT), the rest their own. The R2 function
		// took the Circle's old button (the B) in the swap, so its cell holds the B's picture.
		let map = binds::pad_from_text("circle:rt");
		let mut tex = orig.clone();
		assert!(blit(&orig, &mut tex, 1, &map));
		let cells = cells_of(&tex);
		assert_eq!(cells[3], art_of(7), "the Circle cell holds the RT's picture");
		assert_eq!(cells[0], art_of(0), "the Cross cell holds the A's picture");
		assert_eq!(cells[7], art_of(3), "the R2 cell holds the B's picture");
	}

	/// The PlayStation style has no artwork of its own: a bound function takes the retail's own symbol of the button it is
	/// played by, out of the sheet as it was decoded.
	#[test]
	fn the_playstation_style_takes_the_retails_own_symbol() {
		use super::super::binds;
		let sheet: Vec<u8> = (0..256 * 128).map(|i| (i % 251) as u8).collect();
		let orig = crate::level::Texture { w: 256, h: 128, rgba: sheet.iter().map(|&b| [b, b, b, 255]).collect() };
		let mut tex = orig.clone();
		assert!(blit(&orig, &mut tex, 0, &binds::DEFAULT_PAD));
		assert_eq!(cells_of(&tex), cells_of(&orig), "the defaults change nothing");
		// The Circle on the RT: the Circle's cell holds the R2 cell's symbol.
		let map = binds::pad_from_text("circle:rt");
		let mut tex = orig.clone();
		assert!(blit(&orig, &mut tex, 0, &map));
		let cells = cells_of(&tex);
		assert_eq!(cells[3], cells_of(&orig)[7], "the Circle cell holds the RT's symbol");
		assert_eq!(cells[0], cells_of(&orig)[0], "the Cross cell holds its own symbol");
	}

	/// The font's background is a dither of two colours in 4x4 blocks, and the artwork's transparent parts take it, so a
	/// button sits on the same background as the text around it.
	#[test]
	fn the_transparent_parts_take_the_fonts_dither() {
		// A sheet whose background is two texels, 1 in the even blocks and 2 in the odd ones.
		let mut sheet = vec![2u8; 256 * 128];
		for y in 0..128 {
			for x in 0..256 {
				if ((x / 4) + (y / 4)) % 2 == 0 {
					sheet[y * 256 + x] = 1;
				}
			}
		}
		let bg = background_indices(&sheet, 0, 96);
		assert_eq!(bg, [1, 2], "the even colour and the odd one");
		// The artwork: a transparent cell, so every pixel takes the dither.
		let palette = vec![[0, 0, 0, 255]; 256];
		let cells = quantised_cells(1, &palette, &sheet, &super::super::binds::DEFAULT_PAD);
		assert_eq!(cells.len(), CELLS.len());
		for (k, &(_, u, v)) in CELLS.iter().enumerate() {
			for y in 0..CELL_H {
				for x in 0..CELL_W {
					let want = if ((u as usize + x) / 4 + (v as usize + y) / 4) % 2 == 0 { 1 } else { 2 };
					let art = &XBOX[k * CELL_W * CELL_H * 4..(k + 1) * CELL_W * CELL_H * 4];
					let px = &art[(y * CELL_W + x) * 4..(y * CELL_W + x) * 4 + 4];
					if px[3] == 0 {
						// The picture's transparent parts take the font's dither.
						assert_eq!(cells[k][y * CELL_W + x], want, "cell {k} at {u},{v}, pixel {x},{y}");
					} else {
						// The picture's own pixels take the palette's nearest colour.
						assert_eq!(cells[k][y * CELL_W + x], 0, "cell {k} at {u},{v}, pixel {x},{y}");
					}
				}
			}
		}
	}

	#[test]
	fn the_palette_chooses_the_nearest_colour() {
		let pal = vec![[0, 0, 0, 0], [255, 0, 0, 255], [0, 255, 0, 255]];
		let art = [255u8, 0, 0, 255, 0, 255, 0, 255, 10, 10, 10, 0];
		assert_eq!(quantise(&art, &pal), vec![1, 2, 0]);
	}

	#[test]
	fn the_playstation_cells_are_the_sheets_own_texels() {
		// A sheet with a number in every texel: the PlayStation cells are the sheet's own.
		let sheet: Vec<u8> = (0..256 * 128).map(|i| (i % 251) as u8).collect();
		let cells = original_cells(&sheet, &super::super::binds::DEFAULT_PAD);
		assert_eq!(cells.len(), CELLS.len());
		for (k, &(_, u, v)) in CELLS.iter().enumerate() {
			for y in 0..CELL_H {
				for x in 0..CELL_W {
					let p = (v as usize + y) * 256 + u as usize + x;
					assert_eq!(cells[k][y * CELL_W + x], sheet[p], "cell {k} at {u},{v}");
				}
			}
		}
	}
}
