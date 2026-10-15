//! ARI-15b / BRIDGE-69 (decomp/notes/aridia_flow.md "ARI-15b port spec"): the counter widget of a HUD slot as Skid
//! requests it for slots 5 (sharks, sheet 2000) and 7 (nests, sheet 2001): request FUN_002387c0, release FUN_00238ab8,
//! the per-tick master FUN_0023cec8 with the commit FUN_002388d0 and the update callback FUN_0023c838; the draw
//! FUN_0023c918 is `viewer/hud.rs` `count_widget`.

/// Slide limits.
const SLIDE_MIN: i32 = -6;

/// One HUD slot's counter widget: `owner` = the slot still holds Skid's definition (a marker request takes it),
/// `pending` / `pending_held` the definition waiting for the slide to return to -6, `held` its committed flags 0x10,
/// `live` the update callback installed, the lifetime and slide, fades f1 (bar) / f2 (text), the shown value and max.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CountSlot {
	owner: bool,
	pending: bool,
	pending_held: bool,
	held: bool,
	live: bool,
	lifetime: i32,
	slide: i32,
	f1: u8,
	f2: u8,
	display: i32,
	max: i32,
}

impl Default for CountSlot {
	fn default() -> Self {
		CountSlot { owner: false, pending: false, pending_held: false, held: false, live: false, lifetime: 0, slide: SLIDE_MIN, f1: 0, f2: 0, display: 0, max: 0 }
	}
}

impl CountSlot {




	/// What the draw reads: (display, max, f1, f2) while drawn (draw gate f1 != 0, OV:41170).
	pub fn snapshot(&self) -> Option<(i32, i32, u8, u8)> {
		(self.owner && self.f1 != 0).then_some((self.display, self.max, self.f1, self.f2))
	}
}

