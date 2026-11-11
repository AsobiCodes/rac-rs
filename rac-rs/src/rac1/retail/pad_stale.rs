//! The stale-button rule of the controller (v0.5.1 pad lane, review S1 / R2-S1): under a remap the layout flips between the player's
//! (gameplay) and the standard one (menu context) when a menu opens or closes. A button held at that moment would be a fresh press of
//! a different action on the other side (a fire button that turns into a Confirm in a shop, a Start that reopens the pause). So
//! the buttons down at a flip, and those down when a movie skip ended, give nothing until they are released (the keyboard has the same
//! rule: `menu_keys::Layer`). Without a remap the two layouts are equal and nothing is held back.

use std::sync::Mutex;

/// The state between two polls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stale {
	menu: Option<bool>,
	/// Physical buttons (`binds::PHYS` masks) held back until released.
	mask: u32,
}

impl Stale {
	/// One poll: `phys` is what is down, `menu` the context this field, `remapped` whether the layouts differ. Returns `phys` without the
	/// held-back buttons.
	pub fn step(&mut self, phys: u32, menu: bool, remapped: bool) -> u32 {
		if remapped && self.menu.is_some_and(|m| m != menu) {
			self.mask |= phys;
		}
		self.menu = Some(menu);
		self.mask &= phys;
		if !remapped {
			self.mask = 0;
		}
		phys & !self.mask
	}

	/// The buttons down now give nothing until released (a movie skip ended).
	pub fn hold(&mut self, phys: u32) {
		self.mask |= phys;
	}
}

static STATE: Mutex<Stale> = Mutex::new(Stale { menu: None, mask: 0 });

/// `Stale::step` on the shared state (`prompts::poll_pad`).
pub fn step(phys: u32, menu: bool, remapped: bool) -> u32 {
	STATE.lock().map_or(phys, |mut s| s.step(phys, menu, remapped))
}

/// The buttons down now stay away from the game until released (after a movie skip). Only under a remap (see `step`).
pub fn hold_now() {
	if let Ok(mut s) = STATE.lock() {
		s.hold(super::prompts::phys_now());
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_button_held_across_the_flip_gives_nothing_until_released() {
		let mut s = Stale::default();
		assert_eq!(s.step(0b01, false, true), 0b01, "gameplay: A fires");
		assert_eq!(s.step(0b01, true, true), 0, "a shop opens with A held: no fresh Confirm");
		assert_eq!(s.step(0b11, true, true), 0b10, "B pressed in the shop acts, A is still held back");
		assert_eq!(s.step(0b10, true, true), 0b10, "A released");
		assert_eq!(s.step(0b11, true, true), 0b11, "pressed again: counts");
	}

	#[test]
	fn defaults_hold_nothing_back() {
		let mut s = Stale::default();
		s.step(1, false, false);
		assert_eq!(s.step(1, true, false), 1);
		s.hold(1);
		assert_eq!(s.step(1, true, false), 1);
	}

	#[test]
	fn a_movie_skip_holds_the_button_while_remapped() {
		let mut s = Stale::default();
		s.step(0b100, true, true);
		s.hold(0b100);
		assert_eq!(s.step(0b100, false, true), 0);
		assert_eq!(s.step(0, false, true), 0);
		assert_eq!(s.step(0b100, false, true), 0b100);
	}
}
