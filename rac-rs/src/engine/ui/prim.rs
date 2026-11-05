//! The draw list of the modern UI: what a screen turns into (`kit::draw`), in the 1280x720 design space. No device, no font: the
//! renderer (stage S2) maps design units to the window with `kit::Scale`, rasterises the text runs and fills the shapes. Everything
//! here is plain data so the layout is testable without a GPU (`dump` gives the lines of the layout snapshot).

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
	pub x: f32,
	pub y: f32,
	pub w: f32,
	pub h: f32,
}

impl Rect {
	pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
		Rect { x, y, w, h }
	}
	pub fn right(&self) -> f32 {
		self.x + self.w
	}
	pub fn bottom(&self) -> f32 {
		self.y + self.h
	}
	pub fn cx(&self) -> f32 {
		self.x + self.w * 0.5
	}
	pub fn cy(&self) -> f32 {
		self.y + self.h * 0.5
	}
	pub fn contains(&self, px: f32, py: f32) -> bool {
		px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
	}
	pub fn inset(&self, d: f32) -> Rect {
		Rect::new(self.x + d, self.y + d, (self.w - 2.0 * d).max(0.0), (self.h - 2.0 * d).max(0.0))
	}
	pub fn shifted(&self, dy: f32) -> Rect {
		Rect::new(self.x, self.y + dy, self.w, self.h)
	}
	pub fn overlaps(&self, o: &Rect) -> bool {
		self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
	}
}

/// Straight (non-premultiplied) colour, alpha 0..255.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba {
	pub r: u8,
	pub g: u8,
	pub b: u8,
	pub a: u8,
}

impl Rgba {
	/// Opaque colour from 0xRRGGBB.
	pub const fn rgb(hex: u32) -> Rgba {
		Rgba { r: (hex >> 16) as u8, g: (hex >> 8) as u8, b: hex as u8, a: 255 }
	}
	/// Colour with alpha as a fraction (CSS `rgba(r,g,b,0.84)`).
	pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Rgba {
		Rgba { r, g, b, a: (a * 255.0 + 0.5) as u8 }
	}
	/// The same colour with another alpha (fraction): the mockup's `tint(hex, a)`.
	pub const fn tint(self, a: f32) -> Rgba {
		Rgba { r: self.r, g: self.g, b: self.b, a: (a * 255.0 + 0.5) as u8 }
	}
	pub fn to_f32(self) -> [f32; 4] {
		[self.r as f32 / 255.0, self.g as f32 / 255.0, self.b as f32 / 255.0, self.a as f32 / 255.0]
	}
	pub fn hex(self) -> String {
		format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
	}
}

/// The four faces of the mockup: Barlow Regular / Medium / SemiBold and Barlow Condensed SemiBold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontRole {
	Regular,
	Medium,
	SemiBold,
	Condensed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
	Left,
	Center,
	Right,
}

/// One line of text. `x` is the anchor of `align`, `y` the vertical centre of the line, `px` the font size in design units,
/// `track` the letter spacing in em, `max_w` the width the run must fit in (0 = unlimited; the renderer shrinks or clips).
/// Upper-casing is already done in `text`.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
	pub x: f32,
	pub y: f32,
	pub role: FontRole,
	pub px: f32,
	pub colour: Rgba,
	pub align: Align,
	pub max_w: f32,
	pub track: f32,
	pub text: String,
}

/// The four button symbols of a pad (PlayStation naming; the Letters glyph sets map them to A / B / X / Y).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Symbol {
	Cross,
	Circle,
	Square,
	Triangle,
}

/// A pad button on its dark disc: the symbol drawn as a shape, or a letter in the symbol's colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlyphKind {
	Symbol(Symbol),
	Letter(char),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Border {
	pub width: f32,
	pub colour: Rgba,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Prim {
	/// The window behind the UI: blurred by `blur_px` (design units) and covered by `scrim`; covers the whole window.
	Backdrop { blur_px: f32, scrim: Rgba },
	/// Everything up to the matching `ClipPop` is clipped to this rounded rectangle (the scrolling list).
	ClipPush { rect: Rect, radius: f32 },
	ClipPop,
	/// Rounded rectangle; the border is drawn inside the rectangle.
	RRect { rect: Rect, radius: f32, fill: Rgba, border: Option<Border> },
	Text(TextRun),
	/// A pad button disc with its symbol or letter (`rect` is the disc's square).
	Glyph { rect: Rect, kind: GlyphKind, colour: Rgba },
	/// A chevron stroke (2 px round caps) in a 14 x 14 box.
	Chevron { rect: Rect, right: bool, colour: Rgba },
	/// A picture of the registered set (`images`, by the pack's id), stretched over `rect`, corners rounded by `radius`; `tint` multiplies
	/// the picture (white = as is, a lower alpha fades it). Nothing is drawn for an id that is not registered.
	Image { rect: Rect, id: u32, radius: f32, tint: Rgba },
}

fn n(v: f32) -> String {
	let s = format!("{v:.2}");
	let s = s.trim_end_matches('0').trim_end_matches('.');
	if s == "-0" || s.is_empty() { "0".to_string() } else { s.to_string() }
}

fn r(rc: &Rect) -> String {
	format!("{} {} {} {}", n(rc.x), n(rc.y), n(rc.w), n(rc.h))
}

impl Prim {
	/// One line of text for the layout snapshot.
	pub fn dump(&self) -> String {
		match self {
			Prim::Backdrop { blur_px, scrim } => format!("backdrop blur {} scrim {}", n(*blur_px), scrim.hex()),
			Prim::ClipPush { rect, radius } => format!("clip {} r {}", r(rect), n(*radius)),
			Prim::ClipPop => "clip-end".to_string(),
			Prim::RRect { rect, radius, fill, border } => {
				let b = border.map(|b| format!(" border {} {}", n(b.width), b.colour.hex())).unwrap_or_default();
				format!("rrect {} r {} fill {}{}", r(rect), n(*radius), fill.hex(), b)
			}
			Prim::Text(t) => format!(
				"text {} {} {:?} {} {} {:?} max {} track {} {:?}",
				n(t.x),
				n(t.y),
				t.role,
				n(t.px),
				t.colour.hex(),
				t.align,
				n(t.max_w),
				n(t.track),
				t.text
			),
			Prim::Glyph { rect, kind, colour } => format!("glyph {} {:?} {}", r(rect), kind, colour.hex()),
			Prim::Chevron { rect, right, colour } => {
				format!("chevron {} {} {}", r(rect), if *right { "right" } else { "left" }, colour.hex())
			}
			Prim::Image { rect, id, radius, tint } => format!("image {} id {} r {} tint {}", r(rect), id, n(*radius), tint.hex()),
		}
	}
}

/// The lines of a draw list, one per prim.
pub fn dump(prims: &[Prim]) -> String {
	let mut s = String::new();
	for p in prims {
		s.push_str(&p.dump());
		s.push('\n');
	}
	s
}
