//! [PC] The description line of the pause menu's Help page (id 7) for the modern Help tab (v0.5.1, lane kn). Retail's page shows, left of
//! its list, one text per row ("See helpful information from Gadgetron" for Help Log) and the row's picture right of it. Every field
//! the page looks its title (0x4f5e "Help Menu") up first and then the description of the row under the cursor, then the row labels
//! [C: RAC_PROMPT_LOG on fg1/3_0044, 2026-11-05: 0x4f5e 0x4fd6 0x4fd5 0x4ef1 0x4ef2 0x4ee2 0x4ee3 ...]. The text hook
//! (`prompts::lookup`) hands every id it sees to `note`; the worker's snapshot reads `desc` into `Extra::help_desc`
//! (menu_model/extra.rs). The text is the game's own, so it is in the disc's language and on both discs. Nothing in RAM changes.
use std::sync::atomic::{AtomicU64, Ordering};

/// The Help page's title text id (the first lookup of every field of the page).
const TITLE: u32 = 0x4f5e;
const NONE: u64 = u64::MAX;

/// The field of the last title lookup, and the last description: (field << 32) | text id.
static TITLE_AT: AtomicU64 = AtomicU64::new(NONE);
static DESC: AtomicU64 = AtomicU64::new(NONE);

/// Worker, for every text id the game looks up in field `field`: the id right after the title is the row's description.
pub fn note(id: u32, field: u32) {
	if id == TITLE {
		TITLE_AT.store(field as u64, Ordering::Relaxed);
		return;
	}
	if TITLE_AT.load(Ordering::Relaxed) == field as u64 {
		TITLE_AT.store(NONE, Ordering::Relaxed);
		DESC.store((field as u64) << 32 | id as u64, Ordering::Relaxed);
	}
}

/// Worker, the snapshot of field `field`: the description text id the page looked up in this field or the one before.
pub fn desc(field: u32) -> Option<u32> {
	desc_of(DESC.load(Ordering::Relaxed), field)
}

/// Pure core of `desc`: a lookup counts in its own field and the next (the snapshot may be read before the next field runs).
fn desc_of(w: u64, field: u32) -> Option<u32> {
	(w != NONE).then_some(w).filter(|w| field.checked_sub((w >> 32) as u32).is_some_and(|d| d <= 1)).map(|w| w as u32)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_id_after_the_title_is_the_description_for_its_field_and_the_next() {
		let w = 7u64 << 32 | 0x4fd6;
		assert_eq!(desc_of(w, 7), Some(0x4fd6));
		assert_eq!(desc_of(w, 8), Some(0x4fd6));
		assert_eq!(desc_of(w, 9), None, "an old lookup is not the page on screen");
		assert_eq!(desc_of(w, 6), None, "a lookup from a later field (a stale counter) does not count");
		assert_eq!(desc_of(NONE, 7), None);
	}

	#[test]
	fn note_takes_only_the_id_that_follows_the_title() {
		// The statics are this file's alone: no other test writes them.
		note(0x4f10, 900); // before any title: nothing
		assert_eq!(desc(900), None);
		note(TITLE, 901);
		note(0x4fd6, 901);
		note(0x4fd5, 901); // the row labels after it do not replace it
		assert_eq!(desc(901), Some(0x4fd6));
		note(TITLE, 902);
		note(0x4fd8, 902);
		assert_eq!(desc(902), Some(0x4fd8));
	}
}
