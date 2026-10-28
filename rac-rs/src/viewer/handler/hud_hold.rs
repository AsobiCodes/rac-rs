//! The guard of the overlay picture on screen (`retail::HudGuard`, measured with that picture), with two steadiers (`Held::next` runs once per
//! NEW picture, `Gpu::set_retail_overlay`) that work in LIVE gameplay only (`hud_place::live`: the cells are used nowhere else; a pause menu,
//! a shop panel, a cutscene or the front end records its guard and the 4:3 flag but holds nothing, so nothing of it reaches into gameplay):
//!
//! - the HOLD carries a prompt's last guard over its fade-out: the prompt's text fades later than its bar (proven 2026-10-28 on the USA
//! vendor with every guard paired to its own picture: without the hold the last two fields tear), so for a few pictures the bar is not
//! drawn (no centred widget in the guard) while text still is, and the full-height cell would tear it;
//! - the 4:3 FALLBACK (`hud_place::shape` is None: Gadgetron title letters over the boxes, a corner widget against a prompt, two wide
//! widgets) needs two pictures in a row: a one-picture wide sprite must not flip the whole HUD from the corners to its 4:3 place and back.
//! Not so when the picture's highest centred widget is a top help prompt's bar (`HudGuard::centre_bar`): the bar fades in over several
//! fields, it is never a one-picture transient, and a debounced first picture would draw the prompt cut by the cell shape (torn). A prompt
//! that comes while a race bar's reach is still held therefore falls back on its FIRST picture.

use crate::retail::HudGuard;

/// Pictures a guard is kept after its widget is no longer drawn (8 pictures = 0.16 s at 50 Hz).
pub const HOLD: u8 = 8;

#[derive(Clone, Copy, Debug, Default)]
pub struct Held {
	guard: HudGuard,
	centre: u8,
	left: u8,
	right: u8,
	/// The last picture asked for the fallback (and was not used yet, or is being used).
	odd: bool,
}

/// `new` replaces `held` (and restarts the hold when `hold`); without one `held` is kept while `left` pictures remain, then dropped.
fn keep<T: Copy>(held: &mut Option<T>, left: &mut u8, new: Option<T>, hold: bool) {
	match new {
		Some(v) => {
			*held = Some(v);
			*left = if hold { HOLD } else { 0 };
		}
		None if *left > 0 => *left -= 1,
		None => *held = None,
	}
}

impl Held {
	/// A new picture arrived with guard `g`; `live` = it is a gameplay picture (see the header). A centred widget that starts above the boxes
	/// (no cell fits: the 4:3 fallback) is not held, so it is used for its own pictures only. Not live: nothing is held (all three counters 0).
	fn apply(&mut self, g: HudGuard, usa: bool, live: bool) {
		let hold_centre = live && g.centre.is_some_and(|c| super::hud_place::fits_above_boxes(c, usa));
		keep(&mut self.guard.centre, &mut self.centre, g.centre, hold_centre);
		// The bar flag goes with the centred guard it describes (held with it, gone with it).
		if g.centre.is_some() {
			self.guard.centre_bar = g.centre_bar;
		} else if self.guard.centre.is_none() {
			self.guard.centre_bar = false;
		}
		keep(&mut self.guard.left, &mut self.left, g.left, live);
		keep(&mut self.guard.right, &mut self.right, g.right, live);
		if !live {
			(self.centre, self.left, self.right) = (0, 0, 0);
		}
	}

	/// A new picture arrived with guard `g` (`live`: see the header). When the guard it makes would put the HUD back in its 4:3 place, the
	/// first such picture keeps the layout of the picture before (nothing else changes); the second one in a row uses it. No debounce for a
	/// picture that is not live (the flag is recorded all the same) or whose highest centred widget is a prompt bar (`g.centre_bar`).
	pub fn next(&mut self, g: HudGuard, usa: bool, live: bool) {
		let mut n = *self;
		n.apply(g, usa, live);
		let odd = super::hud_place::shape(&n.guard, usa).is_none();
		if live && odd && !self.odd && !g.centre_bar {
			self.odd = true;
			return;
		}
		n.odd = odd;
		*self = n;
	}

	/// The guard to lay the cells out with.
	pub fn guard(&self) -> HudGuard {
		self.guard
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::retail::HudReach;

	/// A top help prompt's bar (PAL vendor: cut at row 43.4).
	fn prompt() -> HudGuard {
		HudGuard { centre: Some(43.4 / 448.0), centre_bar: true, ..Default::default() }
	}

	fn race() -> HudGuard {
		HudGuard { left: Some(HudReach { u: 275.5 / 512.0, top: 14.5 / 448.0, bottom: 86.5 / 448.0 }), ..Default::default() }
	}

	/// Letters over the ammo box: no cell height fits, the picture keeps its 4:3 place. Not a bar.
	fn letters() -> HudGuard {
		HudGuard { centre: Some(0.05), ..Default::default() }
	}

	#[test]
	fn a_prompts_guard_outlives_its_bar_by_the_hold() {
		let mut h = Held::default();
		h.next(prompt(), false, true);
		assert_eq!(h.guard(), prompt());
		for _ in 0..HOLD {
			h.next(HudGuard::default(), false, true);
			assert_eq!(h.guard(), prompt(), "the text still fades while the bar is gone");
		}
		h.next(HudGuard::default(), false, true);
		assert_eq!(h.guard(), HudGuard::default());
	}

	/// The race bar's reach is held the same way, on either side.
	#[test]
	fn a_corner_widgets_reach_is_held_too() {
		let right = HudGuard { right: race().left, ..Default::default() };
		for g in [race(), right] {
			let mut h = Held::default();
			h.next(g, false, true);
			for _ in 0..HOLD {
				h.next(HudGuard::default(), false, true);
				assert_eq!(h.guard(), g);
			}
			h.next(HudGuard::default(), false, true);
			assert_eq!(h.guard(), HudGuard::default());
		}
	}

	/// The fallback of a non-bar widget needs two pictures in a row; a lone one changes nothing, and a held widget is not aged by it.
	#[test]
	fn a_fallback_needs_two_pictures_in_a_row() {
		let mut h = Held::default();
		h.next(prompt(), false, true);
		h.next(letters(), false, true);
		assert_eq!(h.guard(), prompt(), "one picture of letters: the layout stays");
		h.next(prompt(), false, true);
		assert_eq!(h.guard(), prompt());
		h.next(letters(), false, true);
		h.next(HudGuard::default(), false, true);
		assert_eq!(h.guard(), prompt(), "a lone spark in between, then the held prompt goes on");
		// Two in a row: the letters are used (their own pictures only, never held), then the picture returns to the cells at once.
		let mut h = Held::default();
		h.next(letters(), false, true);
		assert_eq!(h.guard(), HudGuard::default());
		h.next(letters(), false, true);
		assert_eq!(h.guard(), letters());
		h.next(HudGuard::default(), false, true);
		assert_eq!(h.guard(), HudGuard::default());
		// Letters against a held race reach: also debounced (they are no prompt bar).
		let mut h = Held::default();
		h.next(race(), false, true);
		h.next(letters(), false, true);
		assert_eq!(h.guard(), race(), "the first picture of letters keeps the race layout");
	}

	/// A prompt bar that comes while the race bar's reach is still held asks for the fallback (the corner widget would reach into the prompt's
	/// rows) and gets it on its FIRST picture: the bar fades in over several fields, and a debounced first picture was drawn torn.
	#[test]
	fn a_prompt_against_a_held_race_reach_falls_back_on_its_first_picture() {
		let mut h = Held::default();
		h.next(race(), false, true);
		h.next(HudGuard::default(), false, true);
		assert_eq!(h.guard(), race(), "the race reach is held");
		h.next(prompt(), false, true);
		assert_eq!(h.guard(), HudGuard { left: race().left, ..prompt() }, "the first prompt picture uses the merged guard");
		assert!(super::super::hud_place::shape(&h.guard(), false).is_none(), "which is the 4:3 fallback");
		// And it stays so while the prompt shows.
		h.next(prompt(), false, true);
		assert!(super::super::hud_place::shape(&h.guard(), false).is_none());
	}

	/// A picture that is not gameplay (pause menu, shop panel, cutscene, front end) holds nothing: its corner widget or centred piece is
	/// recorded but gone with the first gameplay picture, not 8 pictures later; a guard held from gameplay does not outlive a pause either.
	#[test]
	fn a_picture_that_is_not_gameplay_holds_nothing() {
		for g in [race(), HudGuard { right: race().left, ..Default::default() }, prompt()] {
			let mut h = Held::default();
			h.next(g, false, false);
			assert_eq!(h.guard(), g, "recorded");
			h.next(HudGuard::default(), false, true);
			assert_eq!(h.guard(), HudGuard::default(), "not held into gameplay");
		}
		let mut h = Held::default();
		h.next(prompt(), false, true);
		h.next(HudGuard::default(), false, false);
		h.next(HudGuard::default(), false, true);
		assert_eq!(h.guard(), HudGuard::default(), "a pause ends the hold");
	}

	/// Not live, the fallback is used at once (no debounce) and its flag is recorded: the first gameplay picture that asks for it again is
	/// not debounced.
	#[test]
	fn the_fallback_flag_is_recorded_when_not_live() {
		let mut h = Held::default();
		h.next(letters(), false, false);
		assert_eq!(h.guard(), letters(), "no debounce outside gameplay");
		h.next(letters(), false, true);
		assert_eq!(h.guard(), letters(), "the flag was recorded: used at once");
	}
}
