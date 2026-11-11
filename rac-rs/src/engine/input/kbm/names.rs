//! Input names: the file names (`kbm=`) and the short names on screen.

use super::types::*;
use crate::input::binds::{key_file_name, key_from_name, key_shown};

const MICE: [(Mouse, &str, &str); 5] = [(Mouse::Left, "MouseLeft", "LMB"), (Mouse::Right, "MouseRight", "RMB"), (Mouse::Middle, "MouseMiddle", "MMB"), (Mouse::Back, "MouseBack", "Mouse 4"), (Mouse::Forward, "MouseForward", "Mouse 5")];
const WHEELS: [(Wheel, &str, &str); 2] = [(Wheel::Up, "WheelUp", "Wheel up"), (Wheel::Down, "WheelDown", "Wheel down")];

/// The name in the options file; None for a key that cannot be bound (not in the key list).
pub fn file_name(i: Input) -> Option<&'static str> {
	match i.norm() {
		Input::Key(k) => key_file_name(k),
		Input::Mouse(m) => MICE.iter().find(|e| e.0 == m).map(|e| e.1),
		Input::Wheel(w) => WHEELS.iter().find(|e| e.0 == w).map(|e| e.1),
	}
}

/// The input an options-file name says (case does not matter); Numpad Enter reads as Enter.
pub fn from_name(s: &str) -> Option<Input> {
	let s = s.trim();
	if let Some(e) = MICE.iter().find(|e| e.1.eq_ignore_ascii_case(s)) {
		return Some(Input::Mouse(e.0));
	}
	if let Some(e) = WHEELS.iter().find(|e| e.1.eq_ignore_ascii_case(s)) {
		return Some(Input::Wheel(e.0));
	}
	key_from_name(s).map(|k| Input::Key(k).norm())
}

/// The short name on screen (font-safe ASCII, at most 10 bytes): "LMB", "Mouse 4", "Wheel up", "G", "Space". A letter or digit key
/// shows the cap the player's keyboard layout prints on that physical key (`ui::keycaps::layout_cap`, by scan code; an AZERTY
/// player's key at the QWERTY Q spot says A), else the QWERTY name.
pub fn short_name(i: Input) -> String {
	match i.norm() {
		Input::Key(k) => crate::ui::keycaps::layout_cap(k).map_or_else(|| key_shown(k).to_string(), |c| c.to_string()),
		Input::Mouse(m) => MICE.iter().find(|e| e.0 == m).map_or("?", |e| e.2).to_string(),
		Input::Wheel(w) => WHEELS.iter().find(|e| e.0 == w).map_or("?", |e| e.2).to_string(),
	}
}

/// Whether the Controls page may capture `i`: a listed key other than Esc (it cancels), any mouse button or wheel notch.
pub fn capturable(i: Input) -> bool {
	match i {
		Input::Key(k) => crate::input::binds::bindable(k),
		_ => true,
	}
}
