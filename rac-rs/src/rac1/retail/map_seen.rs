//! [PC] Which planet the planet map (page 0xb) and its mission list (0xc) show, for the modern map page (v0.5 build 12,
//! `modern_menu/map.rs`). The game keeps it in an overlay word whose address moves with the level (the map item update reads it
//! first: Veldin 0x184414, Kerwan 0x184494, Novalis 0x184814; decomp/notes/ingame_menu_leftovers.md P1-c), but both pages look the
//! shown planet's texts up in every field they draw: its location (0x4eb9 + id) and its name (0x4ecc + id) for the two titles
//! [C: RAC_PROMPT_LOG on fg1/3_0044, 2026-11-04: every field of pages 0xb and 0xc looks up 0x4ebc 0x4ecf 0x4ecc (Kerwan, id 3); after
//! R1, 0x4ebb 0x4ece (Aridia, id 2)]. A planet the game hides is titled '????' (0x4f54, ingame_pause_menu.md section 4). The text hook
//! (`prompts::lookup`) hands every id it sees to `note`; the worker's snapshot reads `shown` into `Extra::map`
//! (menu_model/extra.rs). Nothing in RAM changes.
use std::sync::atomic::{AtomicU64, Ordering};

/// The location text of planet id 0 (`modern_menu::ship_marks::LOCATION_BASE`).
const LOCATION_BASE: u32 = 0x4eb9;
/// The last planet id (Veldin's finale).
const LAST: u32 = 18;
/// '????': the title of a planet the game hides.
const UNKNOWN: u32 = 0x4f54;
const NONE: u64 = u64::MAX;

/// What the map's titles named in the last field or the one before.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Seen {
	/// The planet id of the location title.
	pub planet: Option<u32>,
	/// A title was '????' (the planet is not unlocked).
	pub unknown: bool,
}

/// The last location lookup, (field << 32) | planet id; and the field of the last '????' lookup (<< 32). `NONE` = never.
static LOC: AtomicU64 = AtomicU64::new(NONE);
static UNK: AtomicU64 = AtomicU64::new(NONE);

/// Worker, for every text id the game looks up in field `field`.
pub fn note(id: u32, field: u32) {
	if (LOCATION_BASE..=LOCATION_BASE + LAST).contains(&id) {
		LOC.store((field as u64) << 32 | (id - LOCATION_BASE) as u64, Ordering::Relaxed);
	} else if id == UNKNOWN {
		UNK.store((field as u64) << 32, Ordering::Relaxed);
	}
}

/// Worker, the snapshot of field `field`: what the titles named (None = no title text was looked up lately: not a map page).
pub fn shown(field: u32) -> Option<Seen> {
	seen_of(LOC.load(Ordering::Relaxed), UNK.load(Ordering::Relaxed), field)
}

/// Pure core of `shown`: a lookup counts in its own field and the next (the snapshot may be read after the field or before the
/// next one runs); a '????' as new as the location wins over it.
fn seen_of(loc: u64, unk: u64, field: u32) -> Option<Seen> {
	let at = |w: u64| (w != NONE).then_some((w >> 32) as u32).filter(|&f| field.checked_sub(f).is_some_and(|d| d <= 1));
	let (l, u) = (at(loc), at(unk));
	let unknown = u.is_some_and(|u| l.is_none_or(|l| u >= l));
	let planet = l.map(|_| loc as u32).filter(|_| !unknown);
	(planet.is_some() || unknown).then_some(Seen { planet, unknown })
}

#[cfg(test)]
mod tests {
	use super::*;

	fn w(field: u32, v: u32) -> u64 {
		(field as u64) << 32 | v as u64
	}

	#[test]
	fn the_location_lookup_names_the_planet_for_its_field_and_the_next() {
		assert_eq!(seen_of(w(10, 3), NONE, 10), Some(Seen { planet: Some(3), unknown: false }));
		assert_eq!(seen_of(w(10, 3), NONE, 11), Some(Seen { planet: Some(3), unknown: false }));
		assert_eq!(seen_of(w(10, 3), NONE, 12), None, "an old lookup is not the page on screen");
		assert_eq!(seen_of(w(10, 3), NONE, 9), None, "a lookup from a later field (a stale counter) does not count");
		assert_eq!(seen_of(NONE, NONE, 10), None);
	}

	#[test]
	fn a_hidden_planet_is_unknown_unless_a_newer_location_was_looked_up() {
		assert_eq!(seen_of(w(10, 3), w(10, 0), 10), Some(Seen { planet: None, unknown: true }));
		assert_eq!(seen_of(NONE, w(10, 0), 11), Some(Seen { planet: None, unknown: true }));
		assert_eq!(seen_of(w(11, 2), w(10, 0), 11), Some(Seen { planet: Some(2), unknown: false }), "R1 moved on to a known planet");
	}

	#[test]
	fn note_keeps_only_location_and_unknown_ids() {
		// The statics are this file's alone: no other test writes them.
		note(LOCATION_BASE + 3, 500);
		note(0x4ecf, 500); // the name and the "Planet" word do not move the location
		note(0x4ecc, 500);
		assert_eq!(shown(500), Some(Seen { planet: Some(3), unknown: false }));
		note(LOCATION_BASE + LAST + 1, 501); // past the last planet: not a location
		assert_eq!(shown(501), Some(Seen { planet: Some(3), unknown: false }));
		note(UNKNOWN, 502);
		assert_eq!(shown(502), Some(Seen { planet: None, unknown: true }));
	}
}
