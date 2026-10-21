//! The hero and his held gadgets' animation clocks: one `anim::PropAnim` (the retail 0x264df8 clock) each, stepped once
//! per 50 Hz game tick after `Game::update` (decomp/notes/moby_animation.md "Clock", hero_movement.md section 7).
use super::*;
use crate::anim::PropAnim;






pub(super) struct HeroRig {
	pub hero: PropAnim,
	pub wrench: PropAnim,
	/// REN-4: the second held object (retail iRam001404d4, mirrored from the hero's SetAnim); no model draws it yet.
	pub hand: PropAnim,
	/// REN-4b: the slot-3 back object (retail iRam001404d0, the mirror target of the held-object tables); no model draws it
	/// yet (BRIDGE-21).
	pub back: PropAnim,
	/// Slot-3 held-object kind (0x1404f8): 2 = wrench.
	kind: u8,
}




impl HeroRig {
	pub fn new() -> Self {
		let mut r = HeroRig {      hero: PropAnim::new(IDLE), wrench: PropAnim::new(WRENCH_HELD), hand: PropAnim::new(WRENCH_HELD), back: PropAnim::new(1), kind: 2,        };
		r.reset_wrench();
		r
	}







	pub fn reset_wrench(&mut self) {
		self.wrench.set(WRENCH_HELD, 0);
		self.wrench.hold(false);
	}





	/// Slot-3 held-object kind (2 = wrench).
	pub fn kind(&self) -> u8 {
		self.kind
	}

}

#[cfg(test)]
mod tests {
		// RNG-4c test 3: back_speed draws one small(m) for a kind-2 hero sequence with m table rows (21: one), none otherwise.
		// RNG-4c test 2: a request whose pick was drawn at the request site (the gold bolt) draws nothing in the mirror.
		// RNG-4c: the mirror queue drains in push order at flush_mirror and `tick` equals its three stages.
	use crate::particles::Rng;

	// NOV-8.5 test 5: whole-tick state timers convert exactly (the unfixed t * 50.0 lands short for 85 of 400).
}
