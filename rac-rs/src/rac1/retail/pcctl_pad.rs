//! The controller half of the classic Controls page (v0.5.1, design `V051_PAD_DESIGN.md`): sections "Controller 1/3", "2/3", "3/3"
//! list the controller-bindable ACTIONS of `kbm_actions.rs` (the same rows as the keyboard, two slots each). A row's value reads
//! "A / -" (slot 1 / slot 2; "-" for an empty slot; Xbox or PlayStation names; " x2" when another action has a button too).
//! Cross / Enter / click captures slot 1, Square / Tab slot 2, L3 / Delete clears (`pcctl_kbm.rs` notes the keys; the pad's Square is
//! read by `pcctl::keys_on_row`). The menus themselves always read the STANDARD layout, so the page works whatever the remap.

use super::pcctl::VALUE_MAX;
use crate::kbm_actions as ka;
use rac_engine::input::binds::pad_shown;
use rac_engine::input::kbm;

type T5 = [&'static [u8]; 5];

/// Controller sections (after the keyboard ones): the first controller section index of the page.
pub const FIRST: usize = ka::SECTIONS;
/// Rows per section (the page fits six).
const PER: usize = 6;
/// Number of controller sections.
pub const NSEC: usize = 3;

/// The sections' names, En Fr De Es It.
pub static SECTION_NAMES: [T5; NSEC] = [
	[b"Controller 1/3", b"Manette 1/3", b"Controller 1/3", b"Mando 1/3", b"Joypad 1/3"],
	[b"Controller 2/3", b"Manette 2/3", b"Controller 2/3", b"Mando 2/3", b"Joypad 2/3"],
	[b"Controller 3/3", b"Manette 3/3", b"Controller 3/3", b"Mando 3/3", b"Joypad 3/3"],
];

/// The hint line while a controller section is shown: which button captures slot 2 and which key clears.
pub static HINT: T5 = [
	b"Square: 2nd, L3 or Del: clear",
	b"Carr\x8c: 2e, L3/Suppr: effacer",
	b"Quadrat: 2., L3/Entf: leeren",
	b"Cuadrado: 2o, L3/Supr: borrar",
	b"Quadrato: 2o, L3/Canc: cancella",
];

/// Sticks cannot be remapped.
pub static STICKS: T5 = [b"Sticks are fixed", b"Sticks fixes", b"Sticks fest", b"Sticks fijos", b"Stick fissi"];

/// The controller-bindable actions (table indices), in table order.
fn all() -> Vec<usize> {
	ka::install();
	kbm::table().actions.iter().enumerate().filter(|(_, a)| a.pad_ok).map(|(i, _)| i).collect()
}

/// The actions of controller section `sec` (0 based within the controller sections; at most six).
pub fn rows(sec: usize) -> Vec<usize> {
	all().chunks(PER).nth(sec).map(<[usize]>::to_vec).unwrap_or_default()
}

/// The value cell of action `a`: "A / -", "A" ... as the keyboard cells; empty both = "-".
pub fn value(a: usize, playstation: bool) -> Vec<u8> {
	ka::install();
	let (t, b) = (kbm::table(), kbm::binds());
	let Some(slots) = b.1.get(a) else { return Vec::new() };
	let n = |p: Option<u8>| p.map_or("-".to_string(), |p| pad_shown(p, playstation).to_string());
	let mut s = n(slots[0]);
	if slots[1].is_some() {
		s = format!("{s} / {}", n(slots[1]));
	}
	let shared = b.shared_pad(t, a).len();
	let mark = format!(" x{}", shared + 1);
	if shared > 0 && s.len() + mark.len() <= VALUE_MAX {
		s.push_str(&mark);
	}
	s.truncate(VALUE_MAX);
	s.into_bytes()
}

/// Delete on action `a`: empties slot 2, else slot 1 (false when nothing changed: empty, or Pause's last button).
pub fn clear(a: usize) -> bool {
	let has2 = kbm::binds().1.get(a).is_some_and(|s| s[1].is_some());
	kbm::clear_pad(a, has2 as usize)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_sections_list_every_bindable_action_once_and_values_follow_the_slots() {
		let _g = crate::retail::pcopts::globals_guard();
		kbm::init(None, None, None, None);
		ka::install();
		let all: Vec<usize> = (0..NSEC).flat_map(rows).collect();
		assert_eq!(all.len(), 17);
		assert!((0..NSEC).all(|s| (1..=PER).contains(&rows(s).len())));
		let jump = kbm::table().index("jump").unwrap();
		let aim = kbm::table().index("aim").unwrap();
		assert_eq!(value(jump, false), b"A");
		assert_eq!(value(jump, true), b"Cross");
		kbm::assign_pad(jump, 1, 1);
		assert_eq!(value(jump, false), b"A / B x2");
		assert!(clear(jump) && value(jump, false) == b"A");
		assert!(value(aim, false).starts_with(b"RT"), "Aim shares RT with Crouch (R2)");
		kbm::init(None, None, None, None);
	}
}
