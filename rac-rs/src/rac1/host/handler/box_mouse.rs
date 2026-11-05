//! [PC] The mouse in the Aspect ratio box (engine viewer/aspect_box.rs, GitHub #45). With a box, the frame is drawn at the box size
//! (`Gpu::win_size` = the box), so every cursor position goes on as box pixels: `box_event` rewrites `CursorMoved` once, before the
//! modern menus, the mods and the retail page menus see it (window pixel - the box's corner). The last window position is kept
//! (`LAST`): when the box changes under a still cursor (the Aspect ratio row, a window resize), `box_follow` moves the cursor to its
//! place in the new box before the next event, so the hover, `App::cursor` (a click's place) and `ON_BAR` follow. On a bar the
//! position lies outside the box: the retail page menus keep their last aim (`cursor_moved`), except while the left button is held
//! there (a Sound Options bar drag), which gets the position clamped into the box; a LEFT press on a bar while a menu is up is dropped
//! (the right button's Back and the middle button go on, the release goes on). Mouse look reads raw motion and is not touched.
//! Without a box (Auto, or a window within 1 % of the shape) every event goes on unchanged.

use super::*;
use rac_engine::viewer::aspect_box::{self, Rect};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};

/// The cursor's last position was on a bar (main thread: written and read by the window events).
static ON_BAR: AtomicBool = AtomicBool::new(false);

/// The last `CursorMoved` the OS sent: its device, the window pixel position, and the box it was translated with (None = no box).
#[derive(Clone, Copy)]
struct Last {
	device_id: DeviceId,
	x: f64,
	y: f64,
	rect: Option<Rect>,
}

thread_local! {
	/// MAIN only (the window events): `box_event` writes it, `box_follow` reads it and keeps its box.
	static LAST: Cell<Option<Last>> = const { Cell::new(None) };
}

/// The cursor is on a bar of the box (always false without a box).
pub(super) fn on_bar() -> bool {
	ON_BAR.load(Ordering::Relaxed)
}

/// A press of `button` (`pressed`) on a bar while a menu is up (`menu`) is dropped: only the left button, the positional select
/// (retail's right click is Back anywhere, `Retail::mouse_back`, and so is the modern pages' right button).
fn drops_press(button: MouseButton, pressed: bool, bar: bool, menu: bool) -> bool {
	button == MouseButton::Left && pressed && bar && menu
}

/// A window pixel position as a `CursorMoved` in the box `rect`'s pixels (`ON_BAR` set for it).
fn moved(device_id: DeviceId, rect: Rect, x: f64, y: f64) -> WindowEvent {
	let (x, y) = aspect_box::to_box(rect, x, y);
	ON_BAR.store(!aspect_box::inside(rect, x, y), Ordering::Relaxed);
	WindowEvent::CursorMoved { device_id, position: winit::dpi::PhysicalPosition::new(x, y) }
}

/// A box position on a bar clamped into the `w` x `h` box (0..w-1, 0..h-1).
fn clamp_into(w: u32, h: u32, x: f64, y: f64) -> (f64, f64) {
	(x.clamp(0.0, w.max(1) as f64 - 1.0), y.clamp(0.0, h.max(1) as f64 - 1.0))
}

impl App {
	/// The box the frame is drawn in, in window pixels (None without a box).
	fn box_now(&self) -> Option<Rect> {
		self.gpu.as_ref().map(|g| (g.box_rect(), g.real_size())).filter(|(r, real)| !r.is_full(*real)).map(|(r, _)| r)
	}

	/// The window event with its cursor position in box pixels; None = a left press on a bar while a menu is up (nothing happens).
	pub(super) fn box_event(&mut self, event: WindowEvent) -> Option<WindowEvent> {
		let rect = self.box_now();
		if let WindowEvent::CursorMoved { device_id, position } = event {
			LAST.set(Some(Last { device_id, x: position.x, y: position.y, rect }));
		}
		let Some(rect) = rect else {
			ON_BAR.store(false, Ordering::Relaxed);
			return Some(event);
		};
		match event {
			WindowEvent::CursorMoved { device_id, position } => Some(moved(device_id, rect, position.x, position.y)),
			WindowEvent::MouseInput { state, button, .. } if drops_press(button, state == ElementState::Pressed, on_bar(), self.menu_up()) => None,
			e => Some(e),
		}
	}

	/// The box changed since the last cursor move was translated (the Aspect ratio row, a window resize) while the cursor stood
	/// still: the last window position as a `CursorMoved` in the new box's pixels (window pixels when the box is gone), once. None
	/// when nothing changed, and always while there is no box before or after (Auto: no event, exactly as before).
	pub(super) fn box_follow(&mut self) -> Option<WindowEvent> {
		let last = LAST.get()?;
		let rect = self.box_now();
		if rect == last.rect {
			return None;
		}
		LAST.set(Some(Last { rect, ..last }));
		Some(match rect {
			Some(r) => moved(last.device_id, r, last.x, last.y),
			None => {
				ON_BAR.store(false, Ordering::Relaxed);
				WindowEvent::CursorMoved { device_id: last.device_id, position: winit::dpi::PhysicalPosition::new(last.x, last.y) }
			}
		})
	}

	/// A cursor move in box pixels (`box_event`): the pointer (`App::cursor`, a menu click's place), and the retail page menus' aim
	/// (retail/mouse_menu.rs) as fractions of the box. It aims only while the mouse is in use (cursor shown); a hidden-cursor desk
	/// bump just keeps the place for a click. On a bar the aim stays, except while the left button is held on a page menu (a Sound
	/// Options bar drag): then the position clamped into the box goes on, so the bar stops at its end instead of freezing.
	pub(super) fn cursor_moved(&mut self, x: f64, y: f64) {
		self.cursor = Some((x as f32, y as f32));
		let (Some(r), Some(g)) = (self.retail.as_mut(), self.gpu.as_ref()) else { return };
		let (w, h) = g.win_size(); // box pixels (render scale: config is the render size)
		let (x, y) = match (on_bar(), r.mouse_held()) {
			(false, _) => (x, y),
			(true, true) => clamp_into(w, h, x, y),
			(true, false) => return,
		};
		let (fx, fy) = (x as f32 / w.max(1) as f32, y as f32 / h.max(1) as f32);
		if cursor_vis::mouse_in_use() { r.mouse_cursor(fx, fy) } else { r.mouse_track(fx, fy) }
	}

	/// A menu that the mouse points at is up: the modern Options page or screens, or a retail page menu.
	fn menu_up(&self) -> bool {
		use crate::modern_menu::{pause_host, ship_host, title_host, vendor_host};
		rac_engine::ui::is_open() || title_host::up() || pause_host::up() || vendor_host::up() || ship_host::up() || crate::modern_menu::map_host::up() || self.retail.as_ref().is_some_and(|r| r.mouse_menu_active())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn only_a_left_press_on_a_bar_in_a_menu_is_dropped() {
		assert!(drops_press(MouseButton::Left, true, true, true));
		assert!(!drops_press(MouseButton::Left, false, true, true), "the release goes on: nothing stays held");
		assert!(!drops_press(MouseButton::Left, true, false, true), "a click on the picture works");
		assert!(!drops_press(MouseButton::Left, true, true, false), "gameplay: the fire button works anywhere");
		assert!(!drops_press(MouseButton::Right, true, true, true), "the right click is Back anywhere (retail and modern pages)");
		assert!(!drops_press(MouseButton::Middle, true, true, true));
	}

	#[test]
	fn a_drag_on_a_bar_is_clamped_into_the_box() {
		assert_eq!(clamp_into(960, 720, -30.0, 300.0), (0.0, 300.0), "the left bar: the box's left edge");
		assert_eq!(clamp_into(960, 720, 1000.0, 300.0), (959.0, 300.0), "the right bar: the last pixel");
		assert_eq!(clamp_into(1280, 540, 640.0, -5.0), (640.0, 0.0));
		assert_eq!(clamp_into(1280, 540, 640.0, 600.0), (640.0, 539.0));
		assert_eq!(clamp_into(1280, 540, 12.5, 7.0), (12.5, 7.0), "inside: unchanged");
		assert_eq!(clamp_into(0, 0, 5.0, 5.0), (0.0, 0.0), "never a negative range");
	}

	/// The box changed under a still cursor (4:3 picked at 1280 x 720, then Auto again): `moved` puts the kept window position into
	/// the new box, `ON_BAR` included.
	#[test]
	fn a_new_box_moves_the_kept_position() {
		let dev = DeviceId::dummy();
		let pos = |e: WindowEvent| match e {
			WindowEvent::CursorMoved { position, .. } => (position.x, position.y),
			_ => panic!("a cursor move"),
		};
		let b43 = aspect_box::fit((1280, 720), aspect_box::Ratio::R4x3); // 160, 0, 960 x 720
		assert_eq!(pos(moved(dev, b43, 1100.0, 360.0)), (940.0, 360.0), "the Aspect row's arrow at window x 1100 = box x 940");
		assert!(!on_bar());
		assert_eq!(pos(moved(dev, b43, 1200.0, 360.0)), (1040.0, 360.0));
		assert!(on_bar(), "the right bar");
		ON_BAR.store(false, Ordering::Relaxed);
	}
}
