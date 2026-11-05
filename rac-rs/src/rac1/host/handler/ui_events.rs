//! The window's keys, mouse and focus loss for the modern Options page (v0.5; `modern_menu/host.rs`). Called first in `window_event`
//! (after the cursor-visibility step): while the page is up its events do not reach the retail menu behind it (no hover, no click, no
//! tap latch), key and button RELEASES always go on so nothing stays held in the game.
use super::*;
use crate::modern_menu::host;
use rac_engine::ui::{self, MouseButton as UiButton, MouseEvent};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::keyboard::PhysicalKey;

/// A window mouse button as the UI's (None = a button the page ignores).
fn map_button(b: MouseButton) -> Option<UiButton> {
	match b {
		MouseButton::Left => Some(UiButton::Left),
		MouseButton::Right => Some(UiButton::Right),
		MouseButton::Middle => Some(UiButton::Middle),
		_ => None,
	}
}

/// The wheel in notches (a pixel delta is ~120 per notch), positive = list up.
fn wheel_lines(d: &MouseScrollDelta) -> f32 {
	match d {
		MouseScrollDelta::LineDelta(_, y) => *y,
		MouseScrollDelta::PixelDelta(p) => p.y as f32 / 120.0,
	}
}

impl App {
	/// True = the event is the page's alone. Nothing happens (false) while the page is down.
	pub(super) fn ui_event(&mut self, event: &WindowEvent) -> bool {
		// The tab keys' caps follow the keyboard layout (R1-1): read it on this (the window's) thread at a key and on focus.
		if matches!(event, WindowEvent::KeyboardInput { .. } | WindowEvent::Focused(true)) {
			ui::keycaps::refresh();
		}
		let screens = crate::modern_menu::title_host::up() || crate::modern_menu::pause_host::up() || crate::modern_menu::vendor_host::up() || crate::modern_menu::ship_host::up() || crate::modern_menu::map_host::up();
		if !ui::is_open() && !screens {
			return false;
		}
		match event {
			WindowEvent::KeyboardInput { event, .. } => match event.physical_key {
				PhysicalKey::Code(k) => host::key(k, event.state == ElementState::Pressed, event.repeat),
				_ => false,
			},
			WindowEvent::CursorMoved { position, .. } => {
				self.cursor = Some((position.x as f32, position.y as f32));
				host::mouse(MouseEvent::Move { x: position.x as f32, y: position.y as f32 })
			}
			WindowEvent::MouseInput { state, button, .. } => {
				let (Some(b), Some((x, y))) = (map_button(*button), self.cursor) else { return false };
				if *state == ElementState::Pressed {
					host::mouse(MouseEvent::Down { button: b, x, y })
				} else {
					host::mouse(MouseEvent::Up { button: b });
					false // the release goes on: the game lets go of its own button state
				}
			}
			WindowEvent::MouseWheel { delta, .. } => host::mouse(MouseEvent::Wheel { lines: wheel_lines(delta) }),
			WindowEvent::Focused(false) => {
				host::lose_focus();
				false
			}
			_ => false,
		}
	}
}
