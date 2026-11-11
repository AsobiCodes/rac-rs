//! Button pictures and names under a controller remap (v0.5.1, design `V051_PAD_DESIGN.md` "BUTTON PICTURES").
//!
//! The ten button cells of the font sheet always show the STANDARD layout (`glyphs::wanted`): menus read the controller that way, so
//! a menu legend ("Cross Select") is always right. A text that describes a GAMEPLAY action (a tutorial or hint box in play, a pause
//! Help / Moves body, a weapon description) must show the button that does it NOW: its glyph code is changed to the code of the cell
//! that shows the bound button (slot 1, else slot 2 of the action whose output is that glyph's pad bit), or, where no cell shows that
//! button (Start, Back, the d-pad, a button the sheet has no picture for), to its name ("Start", "D-pad up"). Nothing is rewritten
//! while the controller is not remapped, so a default RAM and picture stay what they were.

use super::glyphs;
use crate::kbm_actions as ka;
use rac_engine::input::binds::pad_shown;
use rac_engine::input::kbm::{self, Output};

/// What a glyph shows after the remap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shown {
	/// The sheet's cell with this glyph code.
	Cell(u8),
	/// No cell shows the button: its name.
	Name(String),
	/// The action has no controller button at all.
	Unbound,
}

/// Whether any controller slot differs from the defaults.
pub fn remapped() -> bool {
	ka::install();
	kbm::pad_remapped()
}

/// The cell (glyph code) that shows physical button `p` in the standard layout, if there is one.
pub fn cell_of(p: u8) -> Option<u8> {
	glyphs::std_code(p)
}

/// What the glyph of the PS2 pad bit `bit` (0x40 Cross .. 2 R2) shows now; None for a bit no controller action drives.
pub fn shown(bit: u16, playstation: bool) -> Option<Shown> {
	ka::install();
	let (t, b) = (kbm::table(), kbm::binds());
	let a = t.actions.iter().position(|a| a.pad_ok && a.output == Output::Pad(bit))?;
	Some(match b.1[a][0].or(b.1[a][1]) {
		None => Shown::Unbound,
		Some(p) => cell_of(p).map_or_else(|| Shown::Name(pad_shown(p, playstation).to_string()), Shown::Cell),
	})
}

/// The PS2 pad bit a button glyph code stands for (None for the stick glyphs and other bytes).
pub fn bit_of(code: u8) -> Option<u16> {
	super::prompts::BINDINGS.iter().find(|k| k.code == code && k.bit != 0).map(|k| k.bit)
}

/// `s` with every button glyph changed to what its action shows now; None when nothing changes.
pub fn rewrite(s: &[u8]) -> Option<Vec<u8>> {
	if !remapped() {
		return None;
	}
	let ps = glyphs::style() == 0;
	let mut out = Vec::with_capacity(s.len() + 8);
	let mut changed = false;
	for &b in s {
		match bit_of(b).and_then(|bit| shown(bit, ps)) {
			Some(Shown::Cell(c)) => {
				changed |= c != b;
				out.push(c);
			}
			Some(Shown::Name(n)) => {
				changed = true;
				out.extend_from_slice(n.as_bytes());
			}
			Some(Shown::Unbound) => {
				changed = true;
				out.push(b'-');
			}
			None => out.push(b),
		}
	}
	changed.then_some(out)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn defaults_change_nothing_and_a_moved_jump_shows_its_new_cell_or_name() {
		let _g = crate::retail::pcopts::globals_guard();
		kbm::init(None, None, None, None);
		ka::install();
		assert!(!remapped() && rewrite(b"\x10 Jump").is_none(), "no remap: untouched");
		let jump = kbm::table().index("jump").unwrap();
		kbm::assign_pad(jump, 0, 1); // B = the Circle cell
		assert_eq!(rewrite(b"Press \x10 to jump").unwrap(), b"Press \x11 to jump");
		assert!(rewrite(b"\x11 fire").is_none(), "Fire is still on B: nothing changes");
		kbm::assign_pad(jump, 0, 6); // Start has no cell
		let name = rac_engine::input::binds::pad_shown(6, glyphs::style() == 0);
		assert_eq!(rewrite(b"\x10 Jump").unwrap(), format!("{name} Jump").into_bytes());
		kbm::init(None, None, None, None);
	}
}
