//! Buttons inside a line of text. A sentence that names a button ("jump with [Cross]") carries an inline chip in its string, and
//! the renderer draws the chip as the same pad disc or keycap the hint bar uses, in the flow of the text. A chip is
//! `OPEN tag payload CLOSE` with private-use characters, so a text stays a plain `String` through every page: the word wrap
//! (`kit::text::wrap`, words split at whitespace) keeps a chip whole, `FontMeasure` counts the chip's box, and `instances::build`
//! turns a run that holds chips into text pieces plus discs / keycaps (`expand`). Tags: `P` + symbol letter (X O S T) + glyph set
//! digit (0 PlayStation, 1 Letters, 2 Switch) = a pad disc; `K` + label = a keycap (a space of the label travels as `SPACE`).
//! Nothing here names a game: the pack decides which chip a button gets.
use super::input::GlyphSet;
use super::kit::text::TextMeasure;
use super::prim::{Align, Border, FontRole, Prim, Rect, Symbol, TextRun};
use super::theme::{symbol_colour, CAP_BG, CAP_BORDER};

pub const OPEN: char = '\u{E000}';
pub const CLOSE: char = '\u{E001}';
/// A space inside a keycap label (a real space would let the word wrap split the chip).
const SPACE: char = '\u{E002}';
/// The chip's box height (disc diameter, keycap height) in em of the run.
const BOX_EM: f32 = 1.3;
/// Keycap label size and side padding in em of the run.
const LABEL_EM: f32 = 0.72;
const PAD_EM: f32 = 0.35;
/// Room on each side of a chip in em (retail's glyph cells were wider than their picture as well).
const GAP_EM: f32 = 0.1;
/// A run too wide for its box shrinks to this fraction, then it is cut and ends with an ellipsis (as `instances` does for text).
const MIN_SHRINK: f32 = 0.8;
const ELLIPSIS: &str = "\u{2026}";

/// What one chip shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Chip {
	/// A pad button disc in a glyph set (letters or shapes, the symbol's colour).
	Pad(Symbol, GlyphSet),
	/// A keycap with this label.
	Key(String),
}

/// A piece of a run: plain text or a chip.
#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
	Text(String),
	Chip(Chip),
}

/// The chip of a pad button.
pub fn pad(s: Symbol, set: GlyphSet) -> String {
	let sym = match s {
		Symbol::Cross => 'X',
		Symbol::Circle => 'O',
		Symbol::Square => 'S',
		Symbol::Triangle => 'T',
	};
	let set = match set {
		GlyphSet::PlayStation => '0',
		GlyphSet::Letters => '1',
		GlyphSet::Switch => '2',
	};
	[OPEN, 'P', sym, set, CLOSE].iter().collect()
}

/// The chip of a keycap.
pub fn key(label: &str) -> String {
	let mut s = String::with_capacity(label.len() + 8);
	s.push(OPEN);
	s.push('K');
	s.extend(label.chars().filter(|&c| c != OPEN && c != CLOSE).map(|c| if c == ' ' { SPACE } else { c }));
	s.push(CLOSE);
	s
}

/// Whether `s` holds a chip.
pub fn has(s: &str) -> bool {
	s.contains(OPEN)
}

fn chip_of(body: &str) -> Option<Chip> {
	let mut c = body.chars();
	match c.next()? {
		'P' => {
			let sym = match c.next()?.to_ascii_uppercase() {
				'X' => Symbol::Cross,
				'O' => Symbol::Circle,
				'S' => Symbol::Square,
				'T' => Symbol::Triangle,
				_ => return None,
			};
			let set = GlyphSet::from_index(c.next()?.to_digit(10)? as u8);
			Some(Chip::Pad(sym, set))
		}
		'K' => Some(Chip::Key(c.map(|ch| if ch == SPACE { ' ' } else { ch }).collect())),
		_ => None,
	}
}

/// `s` cut into text pieces and chips. A chip that is not closed (a text cut short in the middle of one) and an unknown tag
/// show nothing; a stray `CLOSE` is dropped.
pub fn pieces(s: &str) -> Vec<Piece> {
	let mut out = Vec::new();
	let mut rest = s;
	while !rest.is_empty() {
		let Some(at) = rest.find(OPEN) else {
			out.push(Piece::Text(rest.replace(CLOSE, "")));
			break;
		};
		if at > 0 {
			out.push(Piece::Text(rest[..at].replace(CLOSE, "")));
		}
		let body = &rest[at + OPEN.len_utf8()..];
		let Some(end) = body.find(CLOSE) else { break };
		if let Some(c) = chip_of(&body[..end]) {
			out.push(Piece::Chip(c));
		}
		rest = &body[end + CLOSE.len_utf8()..];
	}
	out.retain(|p| !matches!(p, Piece::Text(t) if t.is_empty()));
	out
}

/// `s` with each chip as its words (keycap label; a pad chip as nothing): for a place that cannot draw a chip.
pub fn plain(s: &str) -> String {
	pieces(s)
		.into_iter()
		.map(|p| match p {
			Piece::Text(t) => t,
			Piece::Chip(Chip::Key(k)) => k,
			Piece::Chip(Chip::Pad(..)) => String::new(),
		})
		.collect()
}

/// The box of a chip in a run of `px`: (width, height); a disc is square.
pub fn size(m: &dyn TextMeasure, px: f32, c: &Chip) -> (f32, f32) {
	let h = px * BOX_EM;
	match c {
		Chip::Pad(..) => (h, h),
		Chip::Key(k) => ((m.width(FontRole::SemiBold, px * LABEL_EM, k) + 2.0 * PAD_EM * px).max(h), h),
	}
}

/// The advance of a chip: its box and the room on both sides.
fn advance(m: &dyn TextMeasure, px: f32, c: &Chip) -> f32 {
	size(m, px, c).0 + 2.0 * GAP_EM * px
}

/// Width of a run that may hold chips (no letter spacing), the measure of `FontMeasure` and `FixedAdvance` for such a run.
pub fn width(m: &dyn TextMeasure, role: FontRole, px: f32, s: &str) -> f32 {
	pieces(s)
		.iter()
		.map(|p| match p {
			Piece::Text(t) => m.width(role, px, t),
			Piece::Chip(c) => advance(m, px, c),
		})
		.sum()
}

fn piece_w(m: &dyn TextMeasure, t: &TextRun, px: f32, p: &Piece) -> f32 {
	match p {
		Piece::Text(s) => m.width(t.role, px, s) + t.track * px * s.chars().count() as f32,
		Piece::Chip(c) => advance(m, px, c),
	}
}

/// The longest prefix of `s` that fits `room` with the ellipsis after it.
fn cut(m: &dyn TextMeasure, role: FontRole, px: f32, s: &str, room: f32) -> String {
	let mut keep = 0;
	for (at, _) in s.char_indices().skip(1).chain(std::iter::once((s.len(), ' '))) {
		if m.width(role, px, &format!("{}{ELLIPSIS}", &s[..at])) <= room {
			keep = at;
		} else {
			break;
		}
	}
	format!("{}{ELLIPSIS}", s[..keep].trim_end())
}

/// The pieces that fit `max_w` at `px` (all of them when they fit), the last one cut and ended with an ellipsis otherwise.
fn fitted(m: &dyn TextMeasure, t: &TextRun, px: f32, all: Vec<Piece>) -> Vec<Piece> {
	let total: f32 = all.iter().map(|p| piece_w(m, t, px, p)).sum();
	if t.max_w <= 0.0 || total <= t.max_w + 0.01 {
		return all;
	}
	let room = t.max_w - m.width(t.role, px, ELLIPSIS);
	let (mut out, mut pen) = (Vec::new(), 0.0);
	for p in all {
		let w = piece_w(m, t, px, &p);
		if pen + w <= room {
			pen += w;
			out.push(p);
			continue;
		}
		out.push(Piece::Text(match p {
			Piece::Text(s) => cut(m, t.role, px, &s, room - pen),
			Piece::Chip(_) => ELLIPSIS.to_string(),
		}));
		break;
	}
	out
}

/// A run that holds chips as plain prims in design space: each text piece a left-aligned run, each chip a pad disc (`Prim::Glyph`)
/// or a keycap (rounded box + centred label in the run's colour), all on the run's centre line and placed by its alignment. A run
/// wider than its `max_w` shrinks (to `MIN_SHRINK` at most), then is cut with an ellipsis, as a plain run is.
pub fn expand(t: &TextRun, m: &dyn TextMeasure) -> Vec<Prim> {
	let all = pieces(&t.text);
	let natural: f32 = all.iter().map(|p| piece_w(m, t, t.px, p)).sum();
	let px = if t.max_w > 0.0 && natural > t.max_w + 0.01 { t.px * (t.max_w / natural).max(MIN_SHRINK) } else { t.px };
	let shown = fitted(m, t, px, all);
	let total: f32 = shown.iter().map(|p| piece_w(m, t, px, p)).sum();
	let mut x = t.x
		- match t.align {
			Align::Left => 0.0,
			Align::Center => total * 0.5,
			Align::Right => total,
		};
	let mut out = Vec::with_capacity(shown.len() * 2);
	for p in &shown {
		let w = piece_w(m, t, px, p);
		match p {
			Piece::Text(s) => out.push(Prim::Text(TextRun { x, px, align: Align::Left, max_w: 0.0, text: s.clone(), ..t.clone() })),
			Piece::Chip(c) => {
				let (bw, bh) = size(m, px, c);
				let rect = Rect::new(x + GAP_EM * px, t.y - bh * 0.5, bw, bh);
				match c {
					Chip::Pad(sym, set) => out.push(Prim::Glyph { rect, kind: set.glyph(*sym), colour: symbol_colour(*sym) }),
					Chip::Key(k) => {
						out.push(Prim::RRect { rect, radius: bh * 0.25, fill: CAP_BG, border: Some(Border { width: 1.0, colour: CAP_BORDER }) });
						let label = TextRun { x: rect.cx(), y: t.y, role: FontRole::SemiBold, px: px * LABEL_EM, colour: t.colour, align: Align::Center, max_w: 0.0, track: 0.0, text: k.clone() };
						out.push(Prim::Text(label));
					}
				}
			}
		}
		x += w;
	}
	out
}

/// `prims` with every run that holds chips expanded (`expand`); the list itself when none does.
pub fn expand_all<'a>(prims: &'a [Prim], m: &dyn TextMeasure) -> std::borrow::Cow<'a, [Prim]> {
	if !prims.iter().any(|p| matches!(p, Prim::Text(t) if has(&t.text))) {
		return std::borrow::Cow::Borrowed(prims);
	}
	let mut out = Vec::with_capacity(prims.len() + 8);
	for p in prims {
		match p {
			Prim::Text(t) if has(&t.text) => out.extend(expand(t, m)),
			_ => out.push(p.clone()),
		}
	}
	std::borrow::Cow::Owned(out)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ui::kit::text::{wrap, FixedAdvance};
	use crate::ui::prim::{GlyphKind, Rgba};

	fn run(text: &str, align: Align, max_w: f32) -> TextRun {
		TextRun { x: 100.0, y: 50.0, role: FontRole::Regular, px: 10.0, colour: Rgba::rgb(0xFFFFFF), align, max_w, track: 0.0, text: text.to_string() }
	}

	#[test]
	fn chips_read_back_and_unknown_or_open_chips_show_nothing() {
		let s = format!("Jump with {}. Aim {} now{}", pad(Symbol::Cross, GlyphSet::Letters), key("Left stick"), &key("Q")[..4]);
		assert!(!key("Left stick").contains(' '), "a keycap label keeps no real space");
		let p = pieces(&s);
		assert_eq!(
			p,
			vec![
				Piece::Text("Jump with ".into()),
				Piece::Chip(Chip::Pad(Symbol::Cross, GlyphSet::Letters)),
				Piece::Text(". Aim ".into()),
				Piece::Chip(Chip::Key("Left stick".into())),
				Piece::Text(" now".into()),
			]
		);
		assert_eq!(pieces(&format!("a{OPEN}Zq{CLOSE}b{CLOSE}")), vec![Piece::Text("a".into()), Piece::Text("b".into())]);
		assert_eq!(plain(&s), "Jump with . Aim Left stick now");
		assert!(!has("plain") && has(&s));
	}

	#[test]
	fn the_wrap_keeps_a_chip_whole_and_measures_its_box() {
		let m = FixedAdvance(0.5);
		let chip = key("Left stick");
		let s = format!("press {chip} to run");
		// Each char 5 px at 10 px; the keycap: label 10 chars x 0.5 x 7.2 = 36 + 7 padding = 43 px box + 2 px room.
		assert!((width(&m, FontRole::Regular, 10.0, &chip) - 45.0).abs() < 0.01);
		let lines = wrap(&m, FontRole::Regular, 10.0, &s, 55.0);
		assert_eq!(lines, vec!["press".to_string(), chip.clone(), "to run".to_string()], "the chip stays one word");
	}

	#[test]
	fn a_run_with_a_chip_becomes_text_pieces_and_a_disc_on_its_line() {
		let m = FixedAdvance(0.5);
		let t = run(&format!("ab{}cd", pad(Symbol::Circle, GlyphSet::PlayStation)), Align::Left, 0.0);
		let p = expand(&t, &m);
		assert_eq!(p.len(), 3);
		let Prim::Text(a) = &p[0] else { panic!("text first") };
		assert_eq!((a.x, a.text.as_str(), a.align), (100.0, "ab", Align::Left));
		let Prim::Glyph { rect, kind, colour } = &p[1] else { panic!("then the disc") };
		assert_eq!((rect.x, rect.w, rect.h, rect.cy()), (111.0, 13.0, 13.0, 50.0));
		assert_eq!((*kind, *colour), (GlyphKind::Symbol(Symbol::Circle), symbol_colour(Symbol::Circle)));
		let Prim::Text(c) = &p[2] else { panic!("text last") };
		assert_eq!((c.x, c.text.as_str()), (125.0, "cd"));
		// Centred: the whole run (10 + 15 + 10 px) is centred on x.
		let Prim::Text(a) = &expand(&run(&t.text, Align::Center, 0.0), &m)[0] else { panic!() };
		assert!((a.x - 82.5).abs() < 0.01);
		// The Letters set draws the letter of the button.
		let Prim::Glyph { kind, .. } = &expand(&run(&pad(Symbol::Cross, GlyphSet::Letters), Align::Left, 0.0), &m)[0] else { panic!() };
		assert_eq!(*kind, GlyphKind::Letter('A'));
	}

	#[test]
	fn a_keycap_chip_is_a_box_with_its_label_and_a_long_run_is_cut() {
		let m = FixedAdvance(0.5);
		let p = expand(&run(&key("Space"), Align::Left, 0.0), &m);
		assert!(matches!(&p[0], Prim::RRect { border: Some(_), .. }));
		assert!(matches!(&p[1], Prim::Text(l) if l.text == "Space" && l.align == Align::Center));
		// 40 chars of text (200 px) and a chip in a 100 px box: shrunk to 0.8, then cut with an ellipsis before the chip.
		let long = format!("{} {}", "x".repeat(40), pad(Symbol::Cross, GlyphSet::PlayStation));
		let p = expand(&run(&long, Align::Left, 100.0), &m);
		assert_eq!(p.len(), 1);
		let Prim::Text(t) = &p[0] else { panic!() };
		assert!(t.text.ends_with(ELLIPSIS) && (t.px - 8.0).abs() < 1e-4);
		assert!(m.width(t.role, t.px, &t.text) <= 100.0);
		// A list without chips is passed through untouched.
		let plain = vec![Prim::Text(run("plain", Align::Left, 0.0))];
		assert!(matches!(expand_all(&plain, &m), std::borrow::Cow::Borrowed(_)));
	}
}
