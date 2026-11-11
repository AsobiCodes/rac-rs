//! The keyboard + mouse half of the classic Controls page (`pcctl.rs` is the page; v0.5.1, design `V051_KBM_DESIGN.md`): sections
//! "Keyboard, mouse 1/2", "2/2", "Movement", "Camera", "D-pad" and "PC" list the game ACTIONS of `kbm_actions.rs`, each with two
//! slots. A row's value reads "LMB / G" (slot 1 / slot 2, "-" for an empty slot 1; an input another action of the same layer has too
//! adds " x2"). Cross / Enter / a left click on a row captures slot 1 (the retail item flips the row byte, `pcctl::sync_rows`);
//! Square or Tab captures slot 2, Delete clears slot 2 (slot 1 when slot 2 is empty; Pause keeps one binding). The capture accepts
//! a key, a mouse button or a wheel notch (`kbm::capture_poll`, fed by `Retail::tick_keys`); Esc or 8 s cancel it.
//!
//! Tab and Delete are not pad buttons on this page (`menu_keys` gives the Options page no alt button), so `Retail::tick_keys` notes
//! them (`note_key`) once their first poll is over, and `pcctl::field` reads them with the pad's Square on the row under the cursor.

use super::pcctl::VALUE_MAX;
use crate::kbm_actions as ka;
use rac_engine::input::kbm::{self, names};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering::Relaxed};
use winit::keyboard::KeyCode;

type T5 = [&'static [u8]; 5];

/// Controls sections that list actions (the first ones of the page).
pub const NSEC: usize = ka::SECTIONS;

/// The sections' names (the stepper shows them between `<` and `>`), En Fr De Es It in the game's font bytes.
pub static SECTION_NAMES: [T5; NSEC] = [
	[b"Keyboard, mouse 1/2", b"Clavier, souris 1/2", b"Tastatur, Maus 1/2", b"Teclado, rat\x9cn 1/2", b"Tastiera, mouse 1/2"],
	[b"Keyboard, mouse 2/2", b"Clavier, souris 2/2", b"Tastatur, Maus 2/2", b"Teclado, rat\x9cn 2/2", b"Tastiera, mouse 2/2"],
	[b"Movement", b"Mouvement", b"Bewegung", b"Movimiento", b"Movimento"],
	[b"Camera", b"Cam\x8cra", b"Kamera", b"C\x84mara", b"Camera"],
	[b"D-pad", b"Croix", b"Steuerkreuz", b"Cruceta", b"Croce"],
	[b"PC", b"PC", b"PC", b"PC", b"PC"],
];

/// The capture prompt of an action row.
pub static PRESS: T5 = [b"Press key or button", b"Touche ou bouton souris", b"Taste oder Maustaste", b"Tecla o bot\x9cn rat\x9cn", b"Tasto o pulsante mouse"];

/// The hint line (the Controls entry's description): which keys capture slot 2 and clear.
pub static HINT: T5 = [b"Tab: 2nd key, Del: clear", b"Tab: 2e touche, Suppr: effacer", b"Tab: 2. Taste, Entf: leeren", b"Tab: 2a tecla, Supr: borrar", b"Tab: 2o tasto, Canc: cancella"];

/// The actions (table indices) of section `sec`, in the table's order (at most six).
pub fn rows(sec: usize) -> Vec<usize> {
	ka::install();
	kbm::table().actions.iter().enumerate().filter(|(_, a)| a.section as usize == sec).map(|(i, _)| i).collect()
}

/// The label of action `a` in language `l`.
pub fn label(a: usize, l: usize) -> Vec<u8> {
	ka::LABELS.get(a).map_or_else(Vec::new, |t| t[l.min(4)].to_vec())
}

/// The value cell of action `a`: "LMB / G", "G" (slot 1 only), "- / G" (slot 2 only), "-" (none); " x2" when an input is shared.
pub fn value(a: usize) -> Vec<u8> {
	ka::install();
	let (t, b) = (kbm::table(), kbm::binds());
	let Some(slots) = b.0.get(a) else { return Vec::new() };
	let mut s = slots[0].map_or_else(|| "-".to_string(), names::short_name);
	if let Some(i) = slots[1] {
		s = format!("{s} / {}", names::short_name(i));
	}
	let shared = b.shared(t, a).len();
	if shared > 0 && s.len() + 3 <= VALUE_MAX {
		s = format!("{s} x{}", shared + 1);
	}
	s.truncate(VALUE_MAX);
	s.into_bytes()
}

// ---- Tab, Delete and Square on a row -------------------------------------------------------------------------------------

/// Tab (bit 0) and Delete (bit 1) pressed since the worker last looked.
static KEYS: AtomicU8 = AtomicU8::new(0);
/// The slot the next capture of a row byte flip takes (set by Square / Tab, taken once).
static NEXT_SLOT: AtomicUsize = AtomicUsize::new(0);
/// The pad's L3 was down last field.
static L3: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The pad's Square was down last field.
static SQUARE: AtomicBool = AtomicBool::new(false);

/// Host: a key's first poll is over (`Retail::tick_keys`); Tab and Delete are remembered for the Controls page.
pub fn note_key(k: KeyCode) {
	match k {
		KeyCode::Tab => KEYS.fetch_or(1, Relaxed),
		KeyCode::Delete => KEYS.fetch_or(2, Relaxed),
		_ => 0,
	};
}

/// What the player asked of the row under the cursor this field.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edit {
	Nothing,
	/// Capture slot 2 (Square, Tab).
	Capture2,
	/// Clear slot 2, else slot 1 (Delete).
	Clear,
}

/// Worker, every field the Controls page is built: the keys noted and the pad word (`buttons`: Square 0x80 = slot 2, a new L3 click
/// 0x200 = clear, so a controller-only player can undo a binding) as one request.
pub fn request(buttons: u16) -> Edit {
	let k = KEYS.swap(0, Relaxed);
	let sq = buttons & 0x80 != 0;
	let edge = sq && !SQUARE.swap(sq, Relaxed);
	if !sq {
		SQUARE.store(false, Relaxed);
	}
	let l3 = buttons & 0x200 != 0;
	let l3_edge = l3 && !L3.swap(l3, Relaxed);
	if !l3 {
		L3.store(false, Relaxed);
	}
	match (k & 2 != 0 || l3_edge, k & 1 != 0 || edge) {
		(true, _) => Edit::Clear,
		(false, true) => Edit::Capture2,
		_ => Edit::Nothing,
	}
}

/// The next capture binds slot 2 instead of slot 1.
pub fn want_slot2() {
	NEXT_SLOT.store(1, Relaxed);
}

/// The slot a capture that starts now binds (0 unless Square / Tab asked for slot 2); resets to slot 1.
pub fn take_slot() -> usize {
	NEXT_SLOT.swap(0, Relaxed)
}

/// Delete on action `a`: empties slot 2, else slot 1 (false when nothing changed: empty, or Pause's last binding).
pub fn clear(a: usize) -> bool {
	let has2 = kbm::binds().0.get(a).is_some_and(|s| s[1].is_some());
	kbm::clear(a, has2 as usize)
}

#[cfg(test)]
#[path = "pcctl_kbm_tests.rs"]
mod tests;
