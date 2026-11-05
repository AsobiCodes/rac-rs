//! The four faces of the modern UI (Barlow Regular / Medium / SemiBold and Barlow Condensed SemiBold, SIL OFL 1.1, bundled from
//! `fonts/`), measuring and placing text. `FontMeasure` is the `kit::TextMeasure` the layout uses, so what the layout measures is
//! exactly what the renderer draws. Pure CPU: the parsed faces live in one `OnceLock`, widths are in em and scale with the size.
//! Latin only (En, Fr, De, Es, It): no shaping, kerning comes from the font's own pairs. Rasterising is `glyph_cache.rs`.
use super::kit::TextMeasure;
use super::prim::FontRole;
use ab_glyph::{Font, FontRef, GlyphId, PxScale};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

const REGULAR: &[u8] = include_bytes!("fonts/Barlow-Regular.ttf");
const MEDIUM: &[u8] = include_bytes!("fonts/Barlow-Medium.ttf");
const SEMI_BOLD: &[u8] = include_bytes!("fonts/Barlow-SemiBold.ttf");
const CONDENSED: &[u8] = include_bytes!("fonts/BarlowCondensed-SemiBold.ttf");

pub const ROLES: [FontRole; 4] = [FontRole::Regular, FontRole::Medium, FontRole::SemiBold, FontRole::Condensed];

/// Glyph ids and advances are tabulated for this range (Basic Latin, Latin-1, Latin Extended-A); other characters are looked up.
const TAB_LO: u32 = 0x20;
const TAB_HI: u32 = 0x180;

pub fn role_index(r: FontRole) -> usize {
	match r {
		FontRole::Regular => 0,
		FontRole::Medium => 1,
		FontRole::SemiBold => 2,
		FontRole::Condensed => 3,
	}
}

struct Face {
	font: FontRef<'static>,
	upem: f32,
	/// Ascent and descent (negative) in em.
	ascent: f32,
	descent: f32,
	/// Pixel scale of ab_glyph for an em of 1 px: ab_glyph's `PxScale` is the height (ascent - descent), not the em.
	height_per_em: f32,
	ids: Vec<GlyphId>,
	adv: Vec<f32>,
}

impl Face {
	fn new(data: &'static [u8]) -> Face {
		let font = FontRef::try_from_slice(data).expect("bundled font parses");
		let upem = font.units_per_em().unwrap_or(1000.0);
		let mut ids = Vec::new();
		let mut adv = Vec::new();
		for c in TAB_LO..TAB_HI {
			let id = font.glyph_id(char::from_u32(c).unwrap_or(' '));
			ids.push(id);
			adv.push(font.h_advance_unscaled(id) / upem);
		}
		Face {
			upem,
			ascent: font.ascent_unscaled() / upem,
			descent: font.descent_unscaled() / upem,
			height_per_em: font.height_unscaled() / upem,
			font,
			ids,
			adv,
		}
	}

	fn id(&self, c: char) -> GlyphId {
		let u = c as u32;
		if (TAB_LO..TAB_HI).contains(&u) { self.ids[(u - TAB_LO) as usize] } else { self.font.glyph_id(c) }
	}

	fn advance(&self, c: char) -> f32 {
		let u = c as u32;
		if (TAB_LO..TAB_HI).contains(&u) { self.adv[(u - TAB_LO) as usize] } else { self.font.h_advance_unscaled(self.font.glyph_id(c)) / self.upem }
	}
}

pub struct FontSet {
	faces: [Face; 4],
}

thread_local! {
	/// Kerning pairs already looked up: (face, first glyph, second glyph) -> em. The font's pair table is slow to walk.
	static KERN: RefCell<HashMap<(u8, u16, u16), f32>> = RefCell::new(HashMap::new());
}

/// One placed run: the characters with the pen position before each (px, from the start of the run) and the run's width.
#[derive(Clone, Debug, Default)]
pub struct Placed {
	pub chars: Vec<(char, f32)>,
	pub width: f32,
}

impl FontSet {
	/// The parsed faces (parsed once, on first use).
	pub fn get() -> &'static FontSet {
		static SET: OnceLock<FontSet> = OnceLock::new();
		SET.get_or_init(|| FontSet { faces: [Face::new(REGULAR), Face::new(MEDIUM), Face::new(SEMI_BOLD), Face::new(CONDENSED)] })
	}

	fn face(&self, r: FontRole) -> &Face {
		&self.faces[role_index(r)]
	}

	pub fn font(&self, r: FontRole) -> &FontRef<'static> {
		&self.face(r).font
	}

	/// Whether the face has a real glyph for `c` (glyph 0 is the missing-character box).
	pub fn has(&self, r: FontRole, c: char) -> bool {
		self.face(r).id(c).0 != 0
	}

	/// Whether all four faces have `c`.
	pub fn covers(&self, c: char) -> bool {
		ROLES.iter().all(|r| self.has(*r, c))
	}

	/// Advance of `c` in em.
	pub fn advance(&self, r: FontRole, c: char) -> f32 {
		self.face(r).advance(c)
	}

	/// Kerning between two characters in em (negative = closer).
	pub fn kern(&self, r: FontRole, a: char, b: char) -> f32 {
		let f = self.face(r);
		let (ia, ib) = (f.id(a), f.id(b));
		KERN.with(|k| *k.borrow_mut().entry((role_index(r) as u8, ia.0, ib.0)).or_insert_with(|| f.font.kern_unscaled(ia, ib) / f.upem))
	}

	/// Distance from the vertical centre of a line to its baseline, in em (CSS centres the content area ascent + descent).
	pub fn centre_to_baseline(&self, r: FontRole) -> f32 {
		let f = self.face(r);
		(f.ascent + f.descent) * 0.5
	}

	/// ab_glyph's scale for an em of `em_px` pixels.
	pub fn px_scale(&self, r: FontRole, em_px: f32) -> PxScale {
		PxScale::from(em_px * self.face(r).height_per_em)
	}

	/// Width of `text` at `px` without letter spacing (advances plus kerning).
	pub fn width(&self, r: FontRole, px: f32, text: &str) -> f32 {
		let mut em = 0.0;
		let mut prev: Option<char> = None;
		for c in text.chars() {
			if let Some(p) = prev {
				em += self.kern(r, p, c);
			}
			em += self.advance(r, c);
			prev = Some(c);
		}
		em * px
	}

	/// Places a run: pen positions in px from the run start, and the width including the letter spacing after every character
	/// (`track` in em; the same width `kit::text::run_width` gives the layout).
	pub fn place(&self, r: FontRole, px: f32, track: f32, text: &str) -> Placed {
		let mut out = Placed::default();
		let mut pen = 0.0;
		let mut prev: Option<char> = None;
		for c in text.chars() {
			if let Some(p) = prev {
				pen += self.kern(r, p, c) * px;
			}
			out.chars.push((c, pen));
			pen += (self.advance(r, c) + track) * px;
			prev = Some(c);
		}
		out.width = pen;
		out
	}

	/// `text` cut short and ended with an ellipsis so that the run fits `max_w` px: the longest proper prefix whose `place` width
	/// (with the ellipsis after it, kerning and letter spacing included) is at most `max_w`, else the ellipsis alone. One walk over
	/// the text with the same sums in the same order as `place`, so the result equals cutting one character at a time and placing
	/// the whole string again, without that loop's allocation per character.
	pub fn ellipsis_cut(&self, r: FontRole, px: f32, track: f32, text: &str, max_w: f32) -> String {
		const ELLIPSIS: char = '\u{2026}';
		let tail = (self.advance(r, ELLIPSIS) + track) * px;
		let mut keep = 0; // byte length of the longest prefix found to fit
		let mut pen = 0.0;
		let mut prev: Option<char> = None;
		for (at, c) in text.char_indices() {
			// The prefix `text[..at]` plus the ellipsis.
			let mut w = pen;
			if let Some(p) = prev {
				w += self.kern(r, p, ELLIPSIS) * px;
			}
			w += tail;
			if w <= max_w {
				keep = at;
			}
			if let Some(p) = prev {
				pen += self.kern(r, p, c) * px;
			}
			pen += (self.advance(r, c) + track) * px;
			prev = Some(c);
		}
		let mut cut = String::with_capacity(keep + ELLIPSIS.len_utf8());
		cut.push_str(&text[..keep]);
		cut.push(ELLIPSIS);
		cut
	}
}

/// The `TextMeasure` of the layout, backed by the bundled faces.
#[derive(Clone, Copy, Debug, Default)]
pub struct FontMeasure;

impl TextMeasure for FontMeasure {
	fn width(&self, role: FontRole, px: f32, text: &str) -> f32 {
		if super::inline::has(text) {
			return super::inline::width(self, role, px, text); // a run with button chips (`inline`)
		}
		FontSet::get().width(role, px, text)
	}
}
