//! Text measuring for the kit's layout. The kit never owns a font: stage S2's font implements `TextMeasure`, the S1 tests use
//! `FixedAdvance`. Wrapping is done here (greedy, at spaces) so a renderer only draws single lines.
use crate::ui::prim::FontRole;

pub trait TextMeasure {
	/// Advance width of `text` at `px` design units in the face of `role` (no letter spacing).
	fn width(&self, role: FontRole, px: f32, text: &str) -> f32;
}

/// Every character is `.0` em wide (tests).
pub struct FixedAdvance(pub f32);

impl TextMeasure for FixedAdvance {
	fn width(&self, role: FontRole, px: f32, text: &str) -> f32 {
		if crate::ui::inline::has(text) {
			return crate::ui::inline::width(self, role, px, text);
		}
		text.chars().count() as f32 * self.0 * px
	}
}

/// Width of a run with letter spacing `track` (em) between its characters.
pub fn run_width(m: &dyn TextMeasure, role: FontRole, px: f32, text: &str, track: f32) -> f32 {
	m.width(role, px, text) + track * px * text.chars().count() as f32
}

/// The mockup's `text-transform: uppercase` (German sharp s becomes SS, accepted).
pub fn upper(s: &str) -> String {
	s.to_uppercase()
}

/// Greedy word wrap into lines no wider than `max_w` (a single word wider than `max_w` stays on a line of its own). Explicit
/// `\n` breaks. An empty text gives no lines.
pub fn wrap(m: &dyn TextMeasure, role: FontRole, px: f32, text: &str, max_w: f32) -> Vec<String> {
	let mut lines = Vec::new();
	for para in crate::localization::ui_text(text).split('\n') {
		let mut cur = String::new();
		for word in para.split_whitespace() {
			// The word goes on the line in place (no new String per word); a line that got too wide gives it back.
			let keep = cur.len();
			if keep > 0 {
				cur.push(' ');
			}
			cur.push_str(word);
			if keep > 0 && m.width(role, px, &cur) > max_w {
				cur.truncate(keep);
				lines.push(std::mem::replace(&mut cur, word.to_string()));
			}
		}
		if !cur.is_empty() {
			lines.push(cur);
		}
	}
	lines
}
