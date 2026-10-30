//! Button glyphs option (v0.4.2, lane glyph; F2 + F3 of QOL_TRIAGE.md). Graphics -> System -> Button glyphs, key `glyphs=` of
//! `pc_options.txt` (`playstation` default, `letters`, `switch`): which pictures the ten button cells of the game's font sheet show.
//!
//! - PlayStation (default): retail's own cells, nothing is touched (a default RAM and picture stay what they were).
//! - Letters: A bottom, B right, X left, Y top; bumpers "LB" / "RB", triggers "LT" / "RT", sticks "L3" / "R3".
//! - Letters (Switch layout): only the letters move: A right, B bottom, X top, Y left.
//!
//! The pictures are OURS, drawn from code (`render`: a neutral grey disc / pill / dark disc with a letter set in a built-in 5x7 pixel
//! font), never an image file and never another project's artwork. They go into the same 24x16 cells of the font sheet as the retail
//! glyphs, in two places: the sheet the retail code uploads to the GS (`gscap/font.rs` `patch_font`, on every upload: retail re-sends
//! the three font sheets each frame, so a change shows within a frame; the pictures are quantised into the font's own 256-colour
//! palette and cached per style, `cells_for`) and the RGBA copy of the sheet the viewer's own screens draw (`blit`, viewer/hudart/
//! glyphtex.rs: the title and front-end menus, front dialogs, omni mod text). A cell shows the picture of the physical button its
//! function is played on NOW (`binds::pad_map`), so a rebound function is shown as the button it was moved to; with retail's
//! pictures too (decision for the user: with no rebinds and PlayStation nothing is touched at all).
//!
//! Ported ideas (DeSynkro, github DeSynkro/rac-rs-linux, commits 7cd8b7c9 / b1dfb689 and the xglyphs.rs / gscap font.rs they build
//! on; their code was only read, ours is re-derived, their pictures are not used): the ten cells (retail glyph code, sheet place),
//! quantising into the font palette with the sheet's dithered background, recognising the upload by its content, the row/option
//! structure, following the rebind map, and the guard that no transfer is written from inside a GPU-capture job.
use super::binds::{DEFAULT_PAD, NP};
use super::pcctl::CBASE;
use super::pcopts::PcOptions;
use ee_oracle::mem::Mem;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

/// The Button glyphs row's byte (0 PlayStation, 1 Letters, 2 Letters (Switch layout)); pcctl's block.
pub(super) const GLYPH_B: u32 = CBASE + 0x1d;
/// Text ids: the label and the three values (0x7fab..0x7faf are free in the Debug pool range `pcctl::ID_DBG0..ID_DBG_END`).
pub(super) const ID_GLYPHS: u32 = 0x7fab;
pub(super) const ID_PS: u32 = 0x7fac;
pub(super) const ID_LETTERS: u32 = 0x7fad;
pub(super) const ID_SWITCH: u32 = 0x7fae;
pub(super) const IDS: std::ops::RangeInclusive<u32> = ID_GLYPHS..=ID_SWITCH;

/// The names the options file uses, in the row's value order.
pub const NAMES: [&str; 3] = ["playstation", "letters", "switch"];

static STYLE: AtomicU8 = AtomicU8::new(0);

/// The style the prompts use: 0 PlayStation (retail's), 1 Letters, 2 Letters (Switch layout). Retail's with the PC options off.
pub fn style() -> u8 {
	if !super::pcext::enabled() {
		return 0;
	}
	STYLE.load(Ordering::Relaxed).min(2)
}

/// What the cells should show now: the style and the controller bindings.
pub fn wanted() -> (u8, [u8; NP]) {
	(style(), if style() == 0 && !super::pcext::enabled() { DEFAULT_PAD } else { super::binds::pad_map() })
}

/// Whether `(style, map)` is retail's own sheet (nothing to draw).
pub fn is_retail(w: &(u8, [u8; NP])) -> bool {
	w.0 == 0 && w.1 == DEFAULT_PAD
}

// ---- the cells ----------------------------------------------------------------------------------------------------------

pub const CELL_W: usize = 24;
pub const CELL_H: usize = 16;
/// The art sits in the 16-wide window at x 2 of its cell (where retail's own glyphs are); the rest is the font's background.
const ART_X: usize = 2;

/// The ten button cells of the sheet: retail's glyph code and the place (u, v). The order is the sheet's.
pub const CELLS: [(u8, usize, usize); 10] = [
	(0x10, 0, 96),   // Cross
	(0x13, 24, 96),  // Square
	(0x12, 48, 96),  // Triangle
	(0x11, 72, 96),  // Circle
	(0x14, 96, 96),  // L1
	(0x16, 120, 96), // L2
	(0x15, 144, 96), // R1
	(0x17, 168, 96), // R2
	(0x18, 208, 96), // right stick
	(0x19, 232, 96), // left stick
];

/// The binding function (`binds::CROSS` ..) a cell's glyph stands for; None for the stick cells (not rebindable).
fn function_of(k: usize) -> Option<usize> {
	use super::binds as b;
	[b::CROSS, b::SQUARE, b::TRIANGLE, b::CIRCLE, b::L1, b::L2, b::R1, b::R2].get(k).copied()
}

/// What a picture is: a letter on a disc, a bumper or trigger pill with two letters, a stick cell with two letters.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pic {
	Disc(u8),
	Bumper([u8; 2]),
	Trigger([u8; 2]),
	Stick([u8; 2]),
}

/// The picture of the physical button `p` (an index into `binds::PHYS`: 0 A, 1 B, 2 X, 3 Y are the bottom, right, left and top face
/// buttons) in `style`; None for a button without one (Start, Back, the d-pad), whose cell keeps a retail glyph.
pub fn pic_of(p: u8, style: u8) -> Option<Pic> {
	let face = if style == 2 { *b"BAYX" } else { *b"ABXY" };
	Some(match p {
		0..=3 => Pic::Disc(face[p as usize]),
		4 => Pic::Bumper(*b"LB"),
		5 => Pic::Bumper(*b"RB"),
		8 => Pic::Stick(*b"L3"),
		9 => Pic::Stick(*b"R3"),
		14 => Pic::Trigger(*b"LT"),
		15 => Pic::Trigger(*b"RT"),
		_ => return None,
	})
}

/// Where cell `k`'s pixels come from under `(style, map)`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Src {
	/// The sheet's own cell `k`.
	Own,
	/// The sheet's cell `s` (a retail picture of another button: the function was rebound).
	Sheet(usize),
	/// Our drawn picture.
	Art(Pic),
}

/// The source of cell `k`: the picture of the physical button its function is played by. The stick cells stand for the sticks and
/// follow no binding (they keep retail's in PlayStation).
pub fn source(k: usize, style: u8, map: &[u8; NP]) -> Src {
	let p = match function_of(k) {
		Some(f) => map[f],
		None if style == 0 => return Src::Own,
		None => return Src::Art(Pic::Stick(if CELLS[k].0 == 0x19 { *b"L3" } else { *b"R3" })),
	};
	if style == 0 {
		// retail's picture of the button `p`: the cell of the function that button is the default for
		return (0..8).find(|&s| function_of(s).is_some_and(|g| DEFAULT_PAD[g] == p)).map_or(Src::Own, Src::Sheet);
	}
	pic_of(p, style).map_or(Src::Own, Src::Art)
}

// ---- drawing ------------------------------------------------------------------------------------------------------------

/// The built-in 5x7 pixel font: the letters and digit the pictures need, one byte per row (bit 4 = left column).
fn glyph5x7(c: u8) -> [u8; 7] {
	match c {
		b'A' => [0x0e, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
		b'B' => [0x1e, 0x11, 0x11, 0x1e, 0x11, 0x11, 0x1e],
		b'X' => [0x11, 0x11, 0x0a, 0x04, 0x0a, 0x11, 0x11],
		b'Y' => [0x11, 0x11, 0x0a, 0x04, 0x04, 0x04, 0x04],
		b'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1f],
		b'R' => [0x1e, 0x11, 0x11, 0x1e, 0x14, 0x12, 0x11],
		b'T' => [0x1f, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
		b'3' => [0x0e, 0x11, 0x01, 0x06, 0x01, 0x11, 0x0e],
		_ => [0; 7],
	}
}

/// Signed distance (pixels, negative inside) of the point `(px, py)` to the rounded box centred `(cx, cy)`, half sizes `(hx, hy)`.
fn rounded_box(px: f32, py: f32, c: (f32, f32), h: (f32, f32), r: f32) -> f32 {
	let (qx, qy) = ((px - c.0).abs() - (h.0 - r), (py - c.1).abs() - (h.1 - r));
	(qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r
}

const FILL: [u8; 4] = [228, 228, 228, 255];
const FILL_TRIGGER: [u8; 4] = [196, 196, 196, 255];
const RIM: [u8; 4] = [96, 96, 96, 255];
const INK: [u8; 4] = [16, 16, 16, 255];
const DARK: [u8; 4] = [64, 64, 64, 255];
const DARK_RIM: [u8; 4] = [210, 210, 210, 255];
const LIGHT: [u8; 4] = [244, 244, 244, 255];

/// The picture as a 24x16 RGBA cell (alpha 0 = transparent). Opaque pixels are fully opaque: the font's palette has no gradient to
/// quantise a soft edge into, so the edge is hard.
pub fn render(pic: Pic) -> Vec<[u8; 4]> {
	let (shape, fill, rim, ink, text): ((f32, f32), [u8; 4], [u8; 4], [u8; 4], Vec<u8>) = match pic {
		Pic::Disc(c) => ((7.75, 7.75), FILL, RIM, INK, vec![c]),
		Pic::Bumper(t) => ((8.0, 5.5), FILL, RIM, INK, t.to_vec()),
		Pic::Trigger(t) => ((8.0, 7.5), FILL_TRIGGER, RIM, INK, t.to_vec()),
		Pic::Stick(t) => ((7.75, 7.75), DARK, DARK_RIM, LIGHT, t.to_vec()),
	};
	let radius = match pic {
		Pic::Disc(_) | Pic::Stick(_) => 7.75,
		Pic::Bumper(_) => 5.5,
		Pic::Trigger(_) => 4.0,
	};
	let centre = (ART_X as f32 + 8.0, 8.0 + if matches!(pic, Pic::Bumper(_) | Pic::Trigger(_)) { 0.5 } else { 0.0 });
	let mut out = vec![[0u8; 4]; CELL_W * CELL_H];
	for y in 0..CELL_H {
		for x in 0..CELL_W {
			let d = rounded_box(x as f32 + 0.5, y as f32 + 0.5, centre, shape, radius);
			if d <= 0.0 {
				out[y * CELL_W + x] = if d > -1.0 { rim } else { fill };
			}
		}
	}
	// the text: 5x7 letters, one pixel apart, centred on the shape
	let w = text.len() * 5 + (text.len() - 1);
	let x0 = (centre.0 - w as f32 / 2.0 + 0.5).floor() as usize;
	let y0 = (centre.1 - 3.5).floor() as usize;
	for (i, &c) in text.iter().enumerate() {
		for (row, bits) in glyph5x7(c).iter().enumerate() {
			for col in 0..5 {
				if bits >> (4 - col) & 1 != 0 {
					out[(y0 + row) * CELL_W + x0 + i * 6 + col] = ink;
				}
			}
		}
	}
	out
}

// ---- the native copy of the sheet ---------------------------------------------------------------------------------------

/// The viewer's RGBA copy of a font sheet (`live`, 256x128) gets the ten cells for `(style, map)`; `orig` is the sheet as decoded
/// (retail's cells come out of it, so the style can be changed back and forth). A transparent pixel of a drawn picture keeps the
/// sheet's own texel (the font's background). False for a sheet of another size.
pub fn blit(orig: &crate::level::Texture, live: &mut crate::level::Texture, style: u8, map: &[u8; NP]) -> bool {
	if (orig.w, orig.h, live.w, live.h) != (256, 128, 256, 128) {
		return false;
	}
	for (k, &(_, u, v)) in CELLS.iter().enumerate() {
		let art;
		let (from, su, sv) = match source(k, style, map) {
			Src::Own => (None, u, v),
			Src::Sheet(s) => (None, CELLS[s].1, CELLS[s].2),
			Src::Art(p) => {
				art = render(p);
				(Some(&art), 0, 0)
			}
		};
		for y in 0..CELL_H {
			for x in 0..CELL_W {
				live.rgba[(v + y) * 256 + u + x] = match from {
					Some(a) if a[y * CELL_W + x][3] != 0 => a[y * CELL_W + x],
					Some(_) => orig.rgba[(v + y) * 256 + u + x],
					None => orig.rgba[(sv + y) * 256 + su + x],
				};
			}
		}
	}
	true
}

// ---- the sheet in the GS memory -----------------------------------------------------------------------------------------

/// The font sheets of the level as the GS holds them (index bytes, palette), to recognise the upload and to quantise into.
pub struct Patch {
	pub sheets: Vec<Vec<u8>>,
	pub palettes: Vec<Vec<[u8; 4]>>,
	/// The sheets as the qwords of a PSMT8 upload (16 indices per qword, the first in the low byte): the upload is compared with these.
	qwords: Vec<Vec<u128>>,
}

impl Patch {
	pub fn new(sheets: Vec<Vec<u8>>, palettes: Vec<Vec<[u8; 4]>>) -> Patch {
		let qwords = sheets.iter().map(|s| s.chunks(16).map(|c| c.iter().enumerate().fold(0u128, |w, (i, b)| w | (*b as u128) << (8 * i))).collect()).collect();
		Patch { sheets, palettes, qwords }
	}

	/// Which sheet an upload's qwords are (the first 256x128 indices equal), no copy.
	pub fn find(&self, data: &[u128]) -> Option<usize> {
		self.qwords.iter().position(|q| data.len() >= q.len() && data[..q.len()] == q[..])
	}
}

static PATCH: Mutex<Option<Arc<Patch>>> = Mutex::new(None);
/// The cells of the last style asked for, per sheet (cleared when the style, the bindings or the level's sheets change).
static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

struct Cache {
	patch: usize,
	want: (u8, [u8; NP]),
	cells: [Option<Arc<Vec<Vec<u8>>>>; 3],
}

/// viewer/hudart.rs, at a level's HUD art: the level's three font sheets (the retail code uploads them with the level).
pub fn set_patch_for(level: &crate::level::Level) {
	let patch = crate::hud::level_fonts_raw(level).ok().map(|f| {
		let (sheets, palettes) = f.into_iter().unzip();
		Arc::new(Patch::new(sheets, palettes))
	});
	*CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
	*PATCH.lock().unwrap_or_else(|e| e.into_inner()) = patch;
}

pub fn patch() -> Option<Arc<Patch>> {
	PATCH.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The ten cells of sheet `which` for `w`, computed once per (style, bindings): the capture asks on every font upload.
pub fn cells_for(patch: &Arc<Patch>, which: usize, w: &(u8, [u8; NP])) -> Arc<Vec<Vec<u8>>> {
	let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
	let key = Arc::as_ptr(patch) as usize;
	if g.as_ref().is_none_or(|c| c.patch != key || c.want != *w) {
		*g = Some(Cache { patch: key, want: *w, cells: Default::default() });
	}
	let c = g.as_mut().expect("just set");
	c.cells[which.min(2)].get_or_insert_with(|| Arc::new(gs_cells(patch, which, w))).clone()
}

/// Writes the ten cells into the qwords of a 256x128 PSMT8 upload (`data` is the whole sheet, row 0 first, at least 2048 qwords).
pub fn patch_qwords(data: &mut [u128], cells: &[Vec<u8>]) {
	for (k, cell) in cells.iter().enumerate() {
		let (u, v) = (CELLS[k].1, CELLS[k].2);
		for y in 0..CELL_H {
			for x in 0..CELL_W {
				let i = (v + y) * 256 + u + x;
				let b = 8 * (i % 16);
				data[i / 16] = (data[i / 16] & !(0xffu128 << b)) | ((cell[y * CELL_W + x] as u128) << b);
			}
		}
	}
}

/// The font's background: its "transparent" is a dither of two palette entries in 4x4 blocks aligned to the sheet's origin. The two
/// are the sheet's own texels in the gap behind a cell's art (columns 18 and 20).
fn background(sheet: &[u8], u: usize, v: usize) -> [u8; 2] {
	let (a, b) = (sheet[v * 256 + u + 18], sheet[v * 256 + u + 20]);
	if ((u + 18) / 4 + v / 4) % 2 == 0 {
		[a, b]
	} else {
		[b, a]
	}
}

/// The palette entry nearest to a colour (sum of the four channel differences).
fn nearest(px: [u8; 4], palette: &[[u8; 4]]) -> u8 {
	let mut best = (u32::MAX, 0u8);
	for (i, c) in palette.iter().enumerate().take(256) {
		let d: u32 = (0..4).map(|k| (px[k] as i32 - c[k] as i32).unsigned_abs()).sum();
		if d < best.0 {
			best = (d, i as u8);
		}
	}
	best.1
}

/// The ten cells of sheet `which` as palette indices (24x16 each, row by row) for `(style, map)`.
pub fn gs_cells(patch: &Patch, which: usize, w: &(u8, [u8; NP])) -> Vec<Vec<u8>> {
	let (sheet, palette) = (&patch.sheets[which], &patch.palettes[which]);
	let copy = |s: usize| -> Vec<u8> {
		let (u, v) = (CELLS[s].1, CELLS[s].2);
		(0..CELL_H).flat_map(|y| (0..CELL_W).map(move |x| sheet[(v + y) * 256 + u + x])).collect()
	};
	(0..CELLS.len())
		.map(|k| match source(k, w.0, &w.1) {
			Src::Own => copy(k),
			Src::Sheet(s) => copy(s),
			Src::Art(p) => {
				let (u, v) = (CELLS[k].1, CELLS[k].2);
				let bg = background(sheet, u, v);
				render(p)
					.iter()
					.enumerate()
					.map(|(i, px)| {
						if px[3] == 0 {
							bg[((u + i % CELL_W) / 4 + (v + i / CELL_W) / 4) % 2]
						} else {
							nearest(*px, palette)
						}
					})
					.collect()
			}
		})
		.collect()
}

// ---- the row ------------------------------------------------------------------------------------------------------------

/// `pcext::init`: the file's choice is the row's start value.
pub fn init(opts: &PcOptions) {
	let s = opts.extra_get("glyphs").and_then(|v| NAMES.iter().position(|n| v.trim().eq_ignore_ascii_case(n))).unwrap_or(0);
	STYLE.store(s as u8, Ordering::Relaxed);
}

/// The row's start byte (`pcctl::build_gfx` on fresh RAM).
pub(super) fn byte() -> u8 {
	STYLE.load(Ordering::Relaxed).min(2)
}

/// Worker, every field (`pcext::sync`): a changed row byte becomes the style and goes into the options file (true = the caller saves).
/// A three-valued row: the pause page's item cycles the byte, the title's value item only flips it, so any change is one press and
/// the row steps on from its last value on both pages (like Stick dead zone, pcinput.rs).
pub fn sync(mem: &mut Mem, opts: &mut PcOptions) -> bool {
	if !super::pcctl::gfx_built(mem) {
		return false;
	}
	let (b, last) = (mem.rd8(GLYPH_B), STYLE.load(Ordering::Relaxed));
	if b == last {
		return false;
	}
	let next = (last + 1) % 3;
	mem.wr8(GLYPH_B, next);
	STYLE.store(next, Ordering::Relaxed);
	eprintln!("retail: PC options: Button glyphs {}", NAMES[next as usize]);
	opts.extra_set("glyphs", (next != 0).then(|| NAMES[next as usize].to_string()));
	true
}

type T5 = [&'static [u8]; 5];

#[rustfmt::skip]
static LABEL: T5 = [b"Button glyphs", b"Symboles boutons", b"Tastensymbole", b"Iconos de botones", b"Simboli tasti"];
#[rustfmt::skip]
static LETTERS: T5 = [b"Letters", b"Lettres", b"Buchstaben", b"Letras", b"Lettere"];
#[rustfmt::skip]
static SWITCH: T5 = [b"Letters (Switch)", b"Lettres (Switch)", b"Buchst. (Switch)", b"Letras (Switch)", b"Lettere (Switch)"];

/// The text of `id` (`IDS`) in language `l` (0 En .. 4 It); empty for any other id.
pub fn text(id: u32, l: usize) -> Vec<u8> {
	let l = l.min(4);
	match id {
		ID_GLYPHS => LABEL[l].to_vec(),
		ID_PS => b"PlayStation".to_vec(),
		ID_LETTERS => LETTERS[l].to_vec(),
		ID_SWITCH => SWITCH[l].to_vec(),
		_ => Vec::new(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn tex() -> crate::level::Texture {
		// a recognisable fake sheet: every texel carries its own position
		crate::level::Texture { w: 256, h: 128, rgba: (0..256 * 128).map(|i| [(i % 256) as u8, (i / 256) as u8, 7, 255]).collect() }
	}

	/// The default (PlayStation, no rebinds) leaves every cell of the sheet exactly as it was.
	#[test]
	fn the_default_is_retails_own_sheet() {
		let (orig, mut live) = (tex(), tex());
		assert!(blit(&orig, &mut live, 0, &DEFAULT_PAD));
		assert_eq!(orig.rgba, live.rgba);
		assert!((0..10).all(|k| source(k, 0, &DEFAULT_PAD) == Src::Own || source(k, 0, &DEFAULT_PAD) == Src::Sheet(k)));
		assert!(is_retail(&(0, DEFAULT_PAD)));
	}

	/// Letters: A bottom (Cross), B right (Circle), X left (Square), Y top (Triangle); Switch layout moves only the letters.
	#[test]
	fn the_letters_sit_where_the_buttons_are() {
		let d = DEFAULT_PAD;
		let cell = |style, k| match source(k, style, &d) {
			Src::Art(p) => p,
			s => panic!("{s:?}"),
		};
		let (cross, square, triangle, circle) = (0, 1, 2, 3);
		assert_eq!([cell(1, cross), cell(1, circle), cell(1, square), cell(1, triangle)], [Pic::Disc(b'A'), Pic::Disc(b'B'), Pic::Disc(b'X'), Pic::Disc(b'Y')]);
		assert_eq!([cell(2, cross), cell(2, circle), cell(2, square), cell(2, triangle)], [Pic::Disc(b'B'), Pic::Disc(b'A'), Pic::Disc(b'Y'), Pic::Disc(b'X')]);
		for style in [1, 2] {
			assert_eq!([cell(style, 4), cell(style, 6), cell(style, 5), cell(style, 7)], [Pic::Bumper(*b"LB"), Pic::Bumper(*b"RB"), Pic::Trigger(*b"LT"), Pic::Trigger(*b"RT")]);
			assert_eq!([cell(style, 8), cell(style, 9)], [Pic::Stick(*b"R3"), Pic::Stick(*b"L3")]);
		}
	}

	/// A rebound function shows the picture of the button it is on now, in retail's style too.
	#[test]
	fn a_rebound_function_shows_its_new_button() {
		let mut m = DEFAULT_PAD;
		super::super::binds::pad_assign(&mut m, super::super::binds::CROSS, 1); // Cross on the right face button (B)
		assert_eq!(source(0, 1, &m), Src::Art(Pic::Disc(b'B')));
		assert_eq!(source(0, 0, &m), Src::Sheet(3), "retail's Circle picture");
		super::super::binds::pad_assign(&mut m, super::super::binds::CROSS, 6); // Start has no picture: the cell keeps retail's
		assert_eq!(source(0, 1, &m), Src::Own);
	}

	/// Every picture draws something inside the 16-wide window and nothing outside it.
	#[test]
	fn the_pictures_stay_in_their_window() {
		for style in [1, 2] {
			for p in [0u8, 1, 2, 3, 4, 5, 8, 9, 14, 15] {
				let px = render(pic_of(p, style).unwrap());
				assert_eq!(px.len(), CELL_W * CELL_H);
				assert!(px.iter().any(|c| c == &INK || c == &LIGHT), "p{p}: no letter");
				for y in 0..CELL_H {
					for x in (0..ART_X).chain(ART_X + 16..CELL_W) {
						assert_eq!(px[y * CELL_W + x][3], 0, "p{p}: pixel ({x},{y}) outside the window");
					}
				}
			}
		}
	}

	/// Writes the drawn cells of both styles at 4x zoom to the PPM file named by `GLYPH_SHEET_OUT` (a proof picture, not a check).
	#[test]
	#[ignore]
	fn write_the_zoom_sheet() {
		let Ok(path) = std::env::var("GLYPH_SHEET_OUT") else { return };
		let (z, gap) = (4usize, 4usize);
		let (w, h) = (10 * (CELL_W * z + gap) + gap, 2 * (CELL_H * z + gap) + gap);
		let mut img = vec![[70u8, 70, 90]; w * h];
		for (row, style) in [1u8, 2].into_iter().enumerate() {
			for (col, p) in [0u8, 1, 2, 3, 4, 5, 14, 15, 8, 9].into_iter().enumerate() {
				let px = render(pic_of(p, style).unwrap());
				for y in 0..CELL_H * z {
					for x in 0..CELL_W * z {
						let c = px[(y / z) * CELL_W + x / z];
						if c[3] > 0 {
							img[(gap + row * (CELL_H * z + gap) + y) * w + gap + col * (CELL_W * z + gap) + x] = [c[0], c[1], c[2]];
						}
					}
				}
			}
		}
		let mut out = format!("P6
{w} {h}
255
").into_bytes();
		out.extend(img.iter().flatten());
		std::fs::write(path, out).unwrap();
	}

	/// The row byte becomes the style and the file's key, three values in a ring; the default removes the key.
	#[test]
	fn the_row_and_the_key() {
		let _g = crate::retail::pcopts::globals_guard();
		init(&PcOptions::parse("glyphs=SWITCH\n"));
		assert_eq!(byte(), 2);
		init(&PcOptions::parse("glyphs=nonsense\n"));
		assert_eq!(byte(), 0);
		init(&PcOptions::parse("glyphs=letters\n"));
		assert_eq!(byte(), 1);
		assert!(IDS.all(|id| !text(id, 0).is_empty() && (0..5).all(|l| text(id, l).len() < 22)));
	}

	/// A fake level: three 256x128 sheets whose background is one index (7) with the cell windows holding index 9, and a grey palette.
	fn fake_patch() -> Arc<Patch> {
		let sheet = |n: u8| -> Vec<u8> {
			let mut s = vec![7u8; 256 * 128];
			for &(_, u, v) in &CELLS {
				for y in 0..CELL_H {
					for x in ART_X..ART_X + 16 {
						s[(v + y) * 256 + u + x] = 9;
					}
				}
			}
			s[0] = n;
			s
		};
		let pal: Vec<[u8; 4]> = (0..256).map(|i| [i as u8, i as u8, i as u8, 255]).collect();
		Arc::new(Patch::new((0..3).map(sheet).collect(), vec![pal; 3]))
	}

	fn qwords_of(sheet: &[u8]) -> Vec<u128> {
		sheet.chunks(16).map(|c| c.iter().enumerate().fold(0u128, |w, (i, b)| w | (*b as u128) << (8 * i))).collect()
	}

	/// The upload is recognised by its texels (one of three sheets, nothing else), without a copy.
	#[test]
	fn an_upload_is_recognised_by_its_texels() {
		let p = fake_patch();
		for k in 0..3 {
			assert_eq!(p.find(&qwords_of(&p.sheets[k])), Some(k));
		}
		let mut other = qwords_of(&p.sheets[1]);
		other[2047] ^= 1;
		assert_eq!(p.find(&other), None);
		assert_eq!(p.find(&other[..100]), None, "a short transfer is no sheet");
	}

	/// A drawn cell is quantised into the palette; the transparent part is the sheet's own background index; retail copies
	/// (PlayStation with a rebind) come out of the sheet's own cells.
	#[test]
	fn the_cells_are_quantised_into_the_font() {
		let p = fake_patch();
		let cells = gs_cells(&p, 0, &(1, DEFAULT_PAD));
		assert_eq!(cells.len(), 10);
		for c in &cells {
			assert_eq!(c.len(), CELL_W * CELL_H);
			assert_eq!(c[0], 7, "outside the art: the sheet's background");
			assert!(c.iter().any(|&i| [16, 228, 196, 64, 244].contains(&i)), "ink or fill came out of the palette (grey = its own value)");
		}
		// Cross rebound onto the Circle button, PlayStation: Cross's cell shows Circle's retail cell, which here is the window colour.
		let mut m = DEFAULT_PAD;
		super::super::binds::pad_assign(&mut m, super::super::binds::CROSS, 1);
		let sheet0 = &p.sheets[0];
		let cells = gs_cells(&p, 0, &(0, m));
		assert_eq!(cells[0], gs_cells(&p, 0, &(0, DEFAULT_PAD))[3]);
		assert_eq!(gs_cells(&p, 0, &(0, DEFAULT_PAD))[9], (0..CELL_H).flat_map(|y| (0..CELL_W).map(move |x| sheet0[(96 + y) * 256 + 232 + x])).collect::<Vec<u8>>());
	}

	/// The cells land at their place in the sheet's qwords and touch nothing else; the second ask is the cached one.
	#[test]
	fn the_patch_writes_only_the_ten_cells() {
		let p = fake_patch();
		set_patch_for_test(&p);
		let w = (2u8, DEFAULT_PAD);
		let cells = cells_for(&p, 1, &w);
		assert!(Arc::ptr_eq(&cells, &cells_for(&p, 1, &w)), "cached");
		assert!(!Arc::ptr_eq(&cells, &cells_for(&p, 1, &(1, DEFAULT_PAD))), "another style, another cells");
		let mut data = qwords_of(&p.sheets[1]);
		let before = data.clone();
		patch_qwords(&mut data, &cells);
		let out: Vec<u8> = data.iter().flat_map(|q| q.to_le_bytes()).collect();
		let mut touched = 0;
		for (i, (a, b)) in out.iter().zip(&p.sheets[1]).enumerate() {
			let (x, y) = (i % 256, i / 256);
			let in_cell = CELLS.iter().any(|&(_, u, v)| x >= u && x < u + CELL_W && y >= v && y < v + CELL_H);
			assert!(in_cell || a == b, "pixel ({x},{y}) outside every cell changed");
			touched += (a != b) as usize;
		}
		assert!(touched > 500, "the art is there ({touched})");
		assert_ne!(data, before);
		for (k, c) in cells.iter().enumerate() {
			let (u, v) = (CELLS[k].1, CELLS[k].2);
			assert!((0..CELL_H).all(|y| (0..CELL_W).all(|x| out[(v + y) * 256 + u + x] == c[y * CELL_W + x])), "cell {k}");
		}
		*PATCH.lock().unwrap() = None;
	}

	fn set_patch_for_test(p: &Arc<Patch>) {
		*CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
		*PATCH.lock().unwrap_or_else(|e| e.into_inner()) = Some(p.clone());
	}

	/// The native copy: PlayStation without rebinds is the sheet untouched, Letters draws, back to PlayStation restores retail's cells.
	#[test]
	fn the_native_copy_follows_the_style() {
		let (orig, mut live) = (tex(), tex());
		assert!(blit(&orig, &mut live, 1, &DEFAULT_PAD));
		assert_ne!(orig.rgba, live.rgba);
		let cell0 = |t: &crate::level::Texture, y: usize, x: usize| t.rgba[(96 + y) * 256 + x];
		assert_eq!(cell0(&live, 0, 0), cell0(&orig, 0, 0), "a transparent picture pixel keeps the sheet's texel");
		assert!((0..CELL_H).any(|y| (0..CELL_W).any(|x| cell0(&live, y, x) == FILL)), "the disc is drawn");
		let mut back = tex();
		assert!(blit(&orig, &mut back, 0, &DEFAULT_PAD));
		assert_eq!(back.rgba, orig.rgba);
		let mut small = crate::level::Texture { w: 128, h: 128, rgba: vec![[0; 4]; 128 * 128] };
		assert!(!blit(&orig, &mut small, 1, &DEFAULT_PAD), "another size is left alone");
	}

	/// The title page's item only flips the byte (1 -> 0, 2 -> 3, ...): every change is one press and the row steps round the ring.
	#[test]
	fn the_row_steps_round_the_ring_from_any_write() {
		let _g = crate::retail::pcopts::globals_guard();
		init(&PcOptions::parse(""));
		let mut mem = Mem::new();
		let mut o = PcOptions::default();
		assert!(!sync(&mut mem, &mut o), "no Graphics page yet");
		let rows: Vec<[u32; 6]> = (0..15u32).map(|i| [0x100 + i, 0x200 + i, 0x300, 0x301, 0, 0]).collect();
		super::super::pcctl::build_gfx(&mut mem, super::super::pcext::BASE + 0xaa0, &rows, true, false);
		assert_eq!(mem.rd8(GLYPH_B), 0, "default RAM stays zero");
		for (write, expect) in [(1u8, 1u8), (2, 2), (3, 0), (1, 1), (0, 2), (5, 0)] {
			mem.wr8(GLYPH_B, write);
			assert!(sync(&mut mem, &mut o), "write {write}");
			assert_eq!((mem.rd8(GLYPH_B), byte()), (expect, expect), "write {write}");
			assert_eq!(o.extra_get("glyphs"), (expect != 0).then(|| NAMES[expect as usize]));
		}
		assert!(!sync(&mut mem, &mut o), "once");
		init(&PcOptions::parse(""));
	}
}
