//! Mouse button and key state changes of the viewer window (`play_mouse`, `key_change`).

use super::*;
use rac_engine::input::kbm::{self, Input, Mouse, Wheel};

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
		// The left button up ends a Sound Options bar drag (retail/mouse_slider.rs).
		if let (0, false, Some(r)) = (b, down, &mut self.retail) {
			r.mouse_release();
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
			if !repeat {
				self.kbm_pc_edge(Input::Key(k), true);
			}
		} else {
			self.keys.remove(&k);
			self.kbm_pc_edge(Input::Key(k), false);
		}
	}

	/// A PC action (Aim, Zoom) bound to `input` went down or up. Aim = `modcam::ui_aim` with the old right button's gating (the modern
	/// camera, or Classic with Strafe aiming, no free camera, no page menu); a zoom action bound to a key or button zooms one notch on
	/// the press (the wheel's own delta goes through `kbm_wheel`).
	pub(super) fn kbm_pc_edge(&mut self, input: Input, down: bool) {
		self.pc_edge(input, down, false);
	}

	/// `kbm_pc_edge`; `skip_aim`: the caller (`kbm_right`) decides the aim itself.
	fn pc_edge(&mut self, input: Input, down: bool, skip_aim: bool) {
		let Some(r) = self.retail.as_ref() else { return };
		if r.mouse_menu_active() || crate::retail::devtools::free_on() {
			return;
		}
		// A mod took the keyboard (omni/input.rs): the keys press nothing, whatever they are bound to.
		if matches!(input, Input::Key(_)) && crate::omni::input::keyboard_taken() {
			return;
		}
		crate::kbm_actions::install();
		let (t, b) = (kbm::table(), kbm::binds());
		for id in b.pc_actions(t, input) {
			match id {
				crate::kbm_actions::AIM if !skip_aim && crate::retail::modcam::classic::aim_button() => crate::retail::modcam::ui_aim(down),
				crate::kbm_actions::ZOOM_IN if down && self.grabbed => crate::retail::modcam::ui_zoom(1.0),
				crate::kbm_actions::ZOOM_OUT if down && self.grabbed => crate::retail::modcam::ui_zoom(-1.0),
				_ => {}
			}
		}
	}

	/// The right button outside the page menus: held state + tap latch for what it is bound to; the aim where bound (and allowed),
	/// else the orbit look flag as before.
	pub(super) fn kbm_right(&mut self, down: bool) {
		let mut aims = false;
		if let Some(r) = self.retail.as_mut() {
			r.set_mouse_held(Mouse::Right, down);
			if down {
				r.press_mouse(2);
			}
			crate::kbm_actions::install();
			aims = kbm::binds().pc_actions(kbm::table(), Input::Mouse(Mouse::Right)).any(|id| id == crate::kbm_actions::AIM) && crate::retail::modcam::classic::aim_button() && !crate::retail::devtools::free_on();
		}
		if aims {
			crate::retail::modcam::ui_aim(down);
		} else {
			self.looking = down;
		}
		// Any other PC action on the right button (Zoom in / out).
		self.pc_edge(Input::Mouse(Mouse::Right), down, true);
	}

	/// The Controls page waits for a key or button (the capture of `kbm`): mouse clicks and wheel notches are answers then.
	pub(super) fn kbm_capturing(&self) -> bool {
		self.retail.is_some() && kbm::capturing()
	}

	/// A left / right / middle press while capturing: a tap of that button (the menu does not click).
	pub(super) fn kbm_capture_click(&mut self, b: winit::event::MouseButton) {
		use winit::event::MouseButton as M;
		if let Some(r) = self.retail.as_mut() {
			r.press_mouse(match b {
				M::Left => 0,
				M::Middle => 1,
				_ => 2,
			});
		}
	}

	/// A wheel notch while capturing.
	pub(super) fn kbm_capture_wheel(&mut self, lines: f32) {
		if let (true, Some(r)) = (lines != 0.0, self.retail.as_mut()) {
			r.press_wheel(lines > 0.0);
		}
	}

	/// A side button (mouse 4 = back, 5 = forward).
	pub(super) fn kbm_side(&mut self, back: bool, down: bool) {
		let m = if back { Mouse::Back } else { Mouse::Forward };
		if let Some(r) = self.retail.as_mut() {
			r.set_mouse_held(m, down);
			if down {
				r.press_mouse(if back { 3 } else { 4 });
			}
		}
		self.kbm_pc_edge(Input::Mouse(m), down);
	}

	/// A wheel notch outside the page menus (`lines` > 0 = up): the tap of Wheel up / down, and where that is bound to a zoom action
	/// the wheel delta goes to the camera zoom as before (up = zoom in).
	pub(super) fn kbm_wheel(&mut self, lines: f32) {
		if lines == 0.0 {
			return;
		}
		let (w, up) = if lines > 0.0 { (Wheel::Up, true) } else { (Wheel::Down, false) };
		if let Some(r) = self.retail.as_mut() {
			r.press_wheel(up);
		}
		crate::kbm_actions::install();
		let mut zoom = 0.0f32;
		for id in kbm::binds().pc_actions(kbm::table(), Input::Wheel(w)) {
			match id {
				crate::kbm_actions::ZOOM_IN => zoom = lines.abs(),
				crate::kbm_actions::ZOOM_OUT => zoom = -lines.abs(),
				_ => {}
			}
		}
		if zoom != 0.0 && self.grabbed {
			crate::retail::modcam::ui_zoom(zoom);
		}
	}

	/// The window lost the focus in retail play: a key or button released while another window had the focus never reports its
	/// release here, so it would stay held in every recorded field. Lets go of every key and mouse button, and ends a Sound Options
	/// bar drag (its button's release goes to the other window; `Retail::mouse_release`).
	pub(super) fn lose_focus(&mut self) {
		self.keys.clear();
		self.attack_mouse = false;
		self.fire_mouse = false;
		self.quick_mouse = false;
		crate::retail::modcam::ui_release();
		if let Some(r) = self.retail.as_mut() {
			r.mouse_release();
			for m in [Mouse::Right, Mouse::Back, Mouse::Forward] {
				r.set_mouse_held(m, false);
			}
		}
	}
}
