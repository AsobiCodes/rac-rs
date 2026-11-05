//! Retail text for the modern pages with its buttons kept. The game's strings name buttons with glyph bytes ("springst du mit
//! [Cross]", decomp/notes/button_prompts.md); a modern page used to drop them, which left holes in the sentences. Here every
//! button of a sentence becomes an engine inline chip (`rac_engine::ui::inline`) in the hint bar's style: with the keyboard the
//! play key's keycap (the name retail's own keyboard prompts give it, `prompts::key_name`, rebinds included), with a pad the
//! button disc of the Button glyphs set for the four face buttons and a keycap with the set's name for the shoulders, triggers
//! and sticks. A legend line ("[Triangle] Exit": short and starting with a button, or one of retail's menu legend ids) keeps
//! dropping its buttons: the modern page draws its own lead for it.
use crate::retail::menu_model::text::{Btn, RichText, Span, Strings};
use crate::retail::prompts;
use rac_engine::ui::inline;
use rac_engine::ui::input::{Device, GlyphSet};

/// How the page names a button: the device and glyph set of its hint bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
	pub device: Device,
	pub glyphs: GlyphSet,
}

/// A legend line is at most this many bytes long (retail's rule, `prompts` LEGEND_MAX_BYTES).
const LEGEND_MAX: usize = 24;

/// The glyph byte of a button (0x10 Cross .. 0x19 left stick).
fn code(b: Btn) -> u8 {
	(0x10..=0x19).find(|&c| Btn::from_glyph(c) == Some(b)).unwrap_or(0x10)
}

/// The glyph byte of a button (`prompts::menu_key` names it).
pub fn glyph_code(b: Btn) -> u8 {
	code(b)
}

/// A short line that starts with a button: a legend whose lead the modern page draws itself.
pub fn is_legend(r: &RichText) -> bool {
	let bytes: usize = r.0.iter().map(|s| if let Span::Text(t) = s { t.chars().count() } else { 1 }).sum();
	matches!(r.0.first(), Some(Span::Button(_))) && bytes <= LEGEND_MAX
}

/// The chip of one button in `style`; `key` names a button's keyboard key.
pub fn chip(b: Btn, style: Style, key: &dyn Fn(Btn) -> String) -> String {
	if style.device == Device::Keyboard {
		let k = key(b);
		return inline::key(if k.is_empty() { b.label() } else { &k });
	}
	if let Some(sym) = b.symbol() {
		return inline::pad(sym, style.glyphs);
	}
	let g = style.glyphs;
	inline::key(match b {
		Btn::L1 => g.shoulders().0,
		Btn::R1 => g.shoulders().1,
		Btn::L2 => g.triggers().0,
		Btn::R2 => g.triggers().1,
		Btn::StickL => g.sticks().0,
		_ => g.sticks().1,
	})
}

/// A decoded string as page text: breaks as spaces, highlight marks dropped, trimmed; each button a chip, or dropped in a legend.
pub fn page_text(r: &RichText, legend: bool, style: Style, key: &dyn Fn(Btn) -> String) -> String {
	let s = if legend { r.plain() } else { r.plain_with(|b| chip(b, style, key)) };
	s.trim().to_string()
}

/// The keyboard key of a button in play, as retail's keyboard prompts name it (Space, F, Ctrl, WASD, Mouse ...).
pub fn play_key(b: Btn) -> String {
	let ctx = prompts::Ctx { modern_camera: crate::retail::modcam::on(), menu: None };
	prompts::key_name(code(b), ctx).map(|n| String::from_utf8_lossy(&n).into_owned()).unwrap_or_default()
}

/// The text `id` of `strings` for a modern page in `style` (empty when the id has none).
pub fn text(strings: &Strings, id: u32, style: Style) -> String {
	let r = strings.rich(id);
	let legend = is_legend(&r) || prompts::MENU_TEXTS.contains(&(id as u16));
	page_text(&r, legend, style, &play_key)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::retail::menu_model::text::decode_bytes;
	use rac_engine::ui::inline::{pieces, Chip, Piece};
	use rac_engine::ui::prim::Symbol;

	const PAD: Style = Style { device: Device::Pad, glyphs: GlyphSet::PlayStation };

	fn kb_key(b: Btn) -> String {
		match b {
			Btn::Cross => "Space".into(),
			Btn::R1 => "Ctrl".into(),
			_ => String::new(),
		}
	}

	fn chips(s: &str) -> Vec<Chip> {
		pieces(s).into_iter().filter_map(|p| if let Piece::Chip(c) = p { Some(c) } else { None }).collect()
	}

	/// The German Hoverboard text of pp\itm.png ("springst du mit . Wenn du springst und , , und in") keeps its buttons.
	#[test]
	fn a_sentence_keeps_its_buttons_as_chips_in_the_hint_style() {
		let r = decode_bytes(b"Du springst mit \x10. Halte \x15, \x16 und \x19\x01gedr\xa7ckt.");
		let pad = page_text(&r, false, PAD, &kb_key);
		assert_eq!(inline::plain(&pad), "Du springst mit . Halte R1, L2 und LS gedr\u{fc}ckt.");
		let want = vec![Chip::Pad(Symbol::Cross, GlyphSet::PlayStation), Chip::Key("R1".into()), Chip::Key("L2".into()), Chip::Key("LS".into())];
		assert_eq!(chips(&pad), want);
		assert!(pad.starts_with("Du springst mit \u{e000}"), "the chip sits where the glyph was: {pad:?}");
		let letters = page_text(&r, false, Style { device: Device::Pad, glyphs: GlyphSet::Letters }, &kb_key);
		assert_eq!(chips(&letters)[..3], [Chip::Pad(Symbol::Cross, GlyphSet::Letters), Chip::Key("RB".into()), Chip::Key("LT".into())]);
		let kb = page_text(&r, false, Style { device: Device::Keyboard, glyphs: GlyphSet::PlayStation }, &kb_key);
		// The keyboard: the play keys; a key without a name falls back to the button's own name.
		assert_eq!(chips(&kb), vec![Chip::Key("Space".into()), Chip::Key("Ctrl".into()), Chip::Key("L2".into()), Chip::Key("Left stick".into())]);
	}

	#[test]
	fn a_legend_line_drops_its_lead_and_plain_text_is_unchanged() {
		for raw in [&b"\x12 Exit"[..], b"\x11 Ja", b"\x10 Ausr\xa7sten"] {
			let r = decode_bytes(raw);
			assert!(is_legend(&r), "{raw:?}");
			let s = page_text(&r, true, PAD, &kb_key);
			assert!(!inline::has(&s) && !s.starts_with(' '), "{s:?}");
		}
		let long = decode_bytes(b"\x10 und dann springst du weit nach vorn");
		assert!(!is_legend(&long), "a long sentence that starts with a button is not a legend");
		let r = decode_bytes(b"  Gold-Bolts\x01gefunden  ");
		assert_eq!(page_text(&r, false, PAD, &kb_key), "Gold-Bolts gefunden");
	}

	#[test]
	fn the_glyph_bytes_map_back_to_their_buttons() {
		for c in 0x10..=0x19u8 {
			assert_eq!(code(Btn::from_glyph(c).unwrap()), c);
		}
	}
}
