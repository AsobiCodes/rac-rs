//! Gate, text-table and text-lookup hand checks (moved out of pcext.rs 2026-10-27, lane fx5, to keep it under its size cap; a pure move).
use super::*;

#[test]
fn strafe_gate_truth_table() {
	// Extension on: the Strafe aiming row decides, with either camera.
	assert!(gate(true, true));
	assert!(!gate(true, false));
	// Extension off: the old switches decide, the menu does not restrict.
	assert!(gate(false, false));
}

#[test]
fn texts_fit_the_font_and_are_nonempty() {
	for row in TEXTS {
		for s in row {
			assert!(!s.is_empty() && s.len() < 0x3f);
			// Printable ASCII, the game's accent bytes (0x80..0xb0, see TEXTS) or the plate's newline (1) and button glyphs (0x10, 0x13).
			assert!(s.iter().all(|&b| (0x20..0x7f).contains(&b) || (0x80..=0xb0).contains(&b) || [1, 0x10, 0x13].contains(&b)));
		}
	}
}

/// No other hand sits on one of the text lookups (the PC hand would replace it; its decline path runs the retail code).
#[test]
fn no_other_hand_on_a_text_lookup() {
	ee_game::register();
	let n: Vec<_> = ee_oracle::rec::hands().into_iter().filter(|h| HANDS.iter().any(|p| p.entry == h.entry && !std::ptr::eq(*h, p))).map(|h| h.entry).collect();
	eprintln!("hands on a text lookup entry besides ours: {n:x?}");
	assert!(n.is_empty(), "another hand sits on a text lookup: {n:x?}");
}
