//! Mouse button and key state changes of the viewer window (`play_mouse`, `key_change`).

use super::*;

impl App {
	/// Mouse button `b` (0 left, 1 middle) went down or up in play. Retail: left = Circle (fire the weapon), middle =
	/// Triangle (Quick Select), no mouse Square (F is the wrench); `tap` also latches a click (`Retail::press_mouse`) so
	/// one shorter than a frame still reaches the game. The clean-room game: left = wrench, middle = weapon.
	pub(super) fn play_mouse(&mut self, b: usize, down: bool, tap: bool) {
		let flag = match (b, self.retail.is_some()) {
			(0, true) | (1, false) => &mut self.fire_mouse,
			(1, true) => &mut self.quick_mouse,
			_ => &mut self.attack_mouse,
		};
		*flag = down;
		if let (true, true, Some(r)) = (down, tap, &mut self.retail) {
			r.press_mouse(b);
		}
	}

	/// A key went down or up: the held set, and in retail play the tap latch (`Retail::press_key`), so a tap that begins
	/// and ends between two frames still reaches the game.
	pub(super) fn key_change(&mut self, k: KeyCode, pressed: bool, repeat: bool) {
		if pressed {
			self.keys.insert(k);
			if let (false, Some(r)) = (repeat, &mut self.retail) {
				r.press_key(k);
			}
		} else {
			self.keys.remove(&k);
		}
	}
}
