//! Retail menu text as the modern screens need it: the game's font byte table decoded (accents, button glyphs, line breaks,
//! highlight marks), per language, with no drawing. `frontend::Text` keeps every byte of a string as one char (U+0000..U+00FF),
//! so everything here works on those strings; the UI formats `%d` itself.
//!
//! Byte table [C: all five languages of all_text.bin; the (I) entries are the pattern of their vowel block]: 0x20..0x7e as ASCII,
//! 0x01 newline, 0x0c highlight on, 0x08 highlight off, 0x10..0x19 the button glyphs (Cross, Circle, Triangle, Square, L1, R1, L2,
//! R2, right stick, left stick), 0x80..0xb0 the accents (vowel blocks of eight: upper acute / grave / circumflex / umlaut, then the
//! same lower case; then N and C with tilde / cedilla, sharp s, em dash, inverted ! and ?, oe).
use crate::frontend::Text;
use rac_engine::ui::prim::Symbol;

/// A pad button a text names with a glyph byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Btn {
	Cross,
	Circle,
	Triangle,
	Square,
	L1,
	R1,
	L2,
	R2,
	StickR,
	StickL,
}

/// The glyph byte of Cross; the other buttons follow in `Btn` order.
const GLYPH_FIRST: u8 = 0x10;
const BTNS: [Btn; 10] = [Btn::Cross, Btn::Circle, Btn::Triangle, Btn::Square, Btn::L1, Btn::R1, Btn::L2, Btn::R2, Btn::StickR, Btn::StickL];

impl Btn {
	/// The button a glyph byte (0x10..=0x19) shows.
	pub fn from_glyph(b: u8) -> Option<Btn> {
		BTNS.get(b.checked_sub(GLYPH_FIRST)? as usize).copied()
	}

	/// The button of a pad word with exactly this one bit set (menu_keys / mouse_menu bits; sticks have none).
	pub fn from_pad(bits: u16) -> Option<Btn> {
		BTNS.iter().copied().find(|b| b.pad_bit() != 0 && b.pad_bit() == bits)
	}

	/// The PS2 pad bit (0 for the two sticks).
	pub fn pad_bit(self) -> u16 {
		match self {
			Btn::Cross => 0x40,
			Btn::Circle => 0x20,
			Btn::Triangle => 0x10,
			Btn::Square => 0x80,
			Btn::L1 => 4,
			Btn::R1 => 8,
			Btn::L2 => 1,
			Btn::R2 => 2,
			Btn::StickR | Btn::StickL => 0,
		}
	}

	/// The engine's shape for the four face buttons (it has no shoulder or stick symbols).
	pub fn symbol(self) -> Option<Symbol> {
		match self {
			Btn::Cross => Some(Symbol::Cross),
			Btn::Circle => Some(Symbol::Circle),
			Btn::Triangle => Some(Symbol::Triangle),
			Btn::Square => Some(Symbol::Square),
			_ => None,
		}
	}

	/// A plain-text name (used where no glyph can be drawn).
	pub fn label(self) -> &'static str {
		match self {
			Btn::Cross => "Cross",
			Btn::Circle => "Circle",
			Btn::Triangle => "Triangle",
			Btn::Square => "Square",
			Btn::L1 => "L1",
			Btn::R1 => "R1",
			Btn::L2 => "L2",
			Btn::R2 => "R2",
			Btn::StickR => "Right stick",
			Btn::StickL => "Left stick",
		}
	}
}

/// One piece of a decoded string.
#[derive(Clone, Debug, PartialEq)]
pub enum Span {
	Text(String),
	Button(Btn),
	/// A line break (byte 1).
	Break,
	/// The highlight colour switches on (true, byte 0x0c) or off (false, byte 0x08).
	Mark(bool),
}

/// A decoded string.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RichText(pub Vec<Span>);

impl RichText {
	/// The text alone: breaks as a space, buttons and marks dropped.
	pub fn plain(&self) -> String {
		self.plain_with(|_| String::new())
	}

	/// The text with each button spelled by `f`; breaks as a space, marks dropped.
	pub fn plain_with(&self, f: impl Fn(Btn) -> String) -> String {
		let mut out = String::new();
		for s in &self.0 {
			match s {
				Span::Text(t) => out.push_str(t),
				Span::Button(b) => out.push_str(&f(*b)),
				Span::Break => out.push(' '),
				Span::Mark(_) => {}
			}
		}
		out
	}

	/// The buttons in order of appearance.
	pub fn buttons(&self) -> Vec<Btn> {
		self.0.iter().filter_map(|s| if let Span::Button(b) = s { Some(*b) } else { None }).collect()
	}

	pub fn is_empty(&self) -> bool {
		self.0.is_empty()
	}
}

/// 0x80..=0xb0 of the font.
const ACCENTS: [char; 49] = [
	'\u{c1}', '\u{c0}', '\u{c2}', '\u{c4}', '\u{e1}', '\u{e0}', '\u{e2}', '\u{e4}', // A
	'\u{c9}', '\u{c8}', '\u{ca}', '\u{cb}', '\u{e9}', '\u{e8}', '\u{ea}', '\u{eb}', // E
	'\u{cd}', '\u{cc}', '\u{ce}', '\u{cf}', '\u{ed}', '\u{ec}', '\u{ee}', '\u{ef}', // I
	'\u{d3}', '\u{d2}', '\u{d4}', '\u{d6}', '\u{f3}', '\u{f2}', '\u{f4}', '\u{f6}', // O
	'\u{da}', '\u{d9}', '\u{db}', '\u{dc}', '\u{fa}', '\u{f9}', '\u{fb}', '\u{fc}', // U
	'\u{d1}', '\u{f1}', '\u{c7}', '\u{e7}', '\u{df}', '\u{2014}', '\u{a1}', '\u{bf}', '\u{153}',
];

/// The character a printable font byte shows; None for the control bytes (they are not characters). Bytes above the table pass
/// through as the same Latin-1 char (text the port added itself, such as the PC options' rows, is real Unicode).
pub fn font_char(b: u8) -> Option<char> {
	if let Some(c) = crate::translation_probe::localized_char(b) { return Some(c); }
	match b {
		0x20..=0x7e => Some(b as char),
		0x80..=0xb0 => Some(ACCENTS[(b - 0x80) as usize]),
		0xb1..=0xff => Some(b as char),
		_ => None,
	}
}

/// Decodes a string whose chars are the font bytes (what `frontend::Text::get` returns). A char above U+00FF is a real Unicode
/// char and stays as it is.
pub fn decode(s: &str) -> RichText {
	let mut spans = Vec::new();
	let mut run = String::new();
	for c in s.chars() {
		if c as u32 > 0xff {
			run.push(c);
			continue;
		}
		let b = c as u8;
		let piece = match (b, font_char(b), Btn::from_glyph(b)) {
			(_, Some(ch), _) => {
				run.push(ch);
				continue;
			}
			(1, ..) => Span::Break,
			(0x0c, ..) => Span::Mark(true),
			(0x08, ..) => Span::Mark(false),
			(_, _, Some(btn)) => Span::Button(btn),
			// An unknown control byte (none occurs in the five languages) shows nothing.
			_ => continue,
		};
		if !run.is_empty() {
			spans.push(Span::Text(std::mem::take(&mut run)));
		}
		spans.push(piece);
	}
	if !run.is_empty() {
		spans.push(Span::Text(run));
	}
	RichText(spans)
}

/// Decodes raw bytes (text straight out of RAM or all_text.bin), up to the first zero.
pub fn decode_bytes(raw: &[u8]) -> RichText {
	let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
	decode(&raw[..end].iter().map(|&b| b as char).collect::<String>())
}

/// The UI's text index (0..=4: English, French, German, Spanish, Italian) of the game's language word (0, 2, 3, 4, 5).
pub fn lang_index(word: u32) -> usize {
	(word.saturating_sub(1) as usize).min(4)
}

/// The strings of one language, decoded on demand.
#[derive(Clone, Copy)]
pub struct Strings<'a>(pub &'a Text);

impl Strings<'_> {
	/// The decoded text; empty when the id has none.
	pub fn rich(&self, id: u32) -> RichText {
		decode(self.0.get(id))
	}

	/// `rich(id).plain()`.
	pub fn plain(&self, id: u32) -> String {
		self.rich(id).plain()
	}

	/// Does the id have a text in this language?
	pub fn has(&self, id: u32) -> bool {
		!self.0.get(id).is_empty()
	}
}

/// The five languages' text banks, each loaded the first time it is asked for.
#[derive(Default)]
pub struct Bank([Option<Text>; 5]);

impl Bank {
	/// The bank of the game language word `lang_word`; `loader(index)` reads it from the disc once. None when the loader fails.
	pub fn get_or_load(&mut self, lang_word: u32, loader: impl FnOnce(usize) -> Option<Text>) -> Option<&Text> {
		let i = lang_index(lang_word);
		if self.0[i].is_none() {
			self.0[i] = loader(i);
		}
		self.0[i].as_ref()
	}
}
