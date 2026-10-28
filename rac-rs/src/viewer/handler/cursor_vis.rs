//! Mouse cursor visibility (retail mode). The cursor is shown only while the mouse is the device in use.
//!
//! Before 2026-10-26 the cursor was shown whenever the Modern camera's grab ended: a controller player who paused the game saw
//! the OS cursor appear in the middle of the screen (the grab hides it, the pause menu releases it, and nothing had moved it).
//! Now:
//! - hidden at start and while the pad or the keyboard is used; shown by real mouse movement (raw motion of the device, not a
//!   window resize that moves the client area under a still cursor), a click or the wheel;
//! - a pad input hides it again; a key press hides it too, except while the cursor is captured for mouse look (a K+M player
//!   presses Esc to pause with the mouse as the device: the cursor must be there for the pause menu);
//! - captured (Modern camera, aim, free camera, mod capture) it is hidden, as before; nothing ever moves the cursor
//!   (no `set_cursor_position` anywhere).
//! `CursorVis` is the pure state machine (unit tests); `window_event` / `raw_motion` and the pad pulse of retail/prompts.rs feed the one live instance and
//! `apply` (frame start) sets the winit visibility when it changed and logs the change ("cursor: visible|hidden (why)").
//! Dev: `RAC_CURSOR_DEV=from-to:mouse|key|pad;...` (frame numbers, `to` may be empty) feeds scripted events instead of real ones,
//! to test the logic in hidden runs without input injection.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use std::time::Instant;
use winit::event::{ElementState, WindowEvent};
use winit::window::Window;

/// Raw mouse counts within `GAP_MS` that count as "the mouse is being used" (a desk bump or sensor noise is below).
const MOVE_COUNTS: f32 = 6.0;
const GAP_MS: u64 = 300;

/// Who used which device last, as far as the cursor is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Why {
	/// Nothing used yet.
	#[default]
	Start,
	Mouse,
	Key,
	Pad,
}

#[derive(Default)]
pub struct CursorVis {
	mouse: bool,
	why: Why,
	last_ms: u64,
	sum: f32,
}

impl CursorVis {
	pub const fn new() -> Self {
		CursorVis { mouse: false, why: Why::Start, last_ms: 0, sum: 0.0 }
	}

	/// Raw mouse movement of `dist` counts at `now_ms`: the mouse is in use once it adds up within the gap.
	pub fn motion(&mut self, now_ms: u64, dist: f32) {
		if now_ms.saturating_sub(self.last_ms) > GAP_MS {
			self.sum = 0.0;
		}
		self.last_ms = now_ms;
		self.sum += dist;
		if self.sum >= MOVE_COUNTS {
			self.sum = 0.0;
			self.click();
		}
	}

	/// A mouse button press or the wheel.
	pub fn click(&mut self) {
		self.mouse = true;
		self.why = Why::Mouse;
	}

	/// A key press. `captured`: the cursor is captured for mouse look right now, so the keys are the mouse player's WASD / Esc.
	pub fn key(&mut self, captured: bool) {
		if !captured {
			self.mouse = false;
			self.why = Why::Key;
		}
	}

	/// Real pad input (a button, a stick out of its deadzone).
	pub fn pad(&mut self) {
		self.mouse = false;
		self.why = Why::Pad;
	}

	/// Whether the OS cursor is shown; `captured` = the game holds the cursor (always hidden then).
	pub fn visible(&self, captured: bool) -> bool {
		self.mouse && !captured
	}

	pub fn why(&self) -> Why {
		self.why
	}

	/// The mouse is the device in use (its cursor is shown unless the game holds it): the page menus aim at the row under it
	/// only then. A pad / keyboard player's desk bump below `MOVE_COUNTS` moves a hidden cursor the player cannot see.
	pub fn mouse_in_use(&self) -> bool {
		self.mouse
	}
}

static LIVE: Mutex<CursorVis> = Mutex::new(CursorVis::new());
/// The visibility last given to the window: 0 hidden, 1 visible, 2 not yet set.
static APPLIED: AtomicU8 = AtomicU8::new(2);
static FRAME: AtomicU32 = AtomicU32::new(0);

fn now_ms() -> u64 {
	static T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
	T0.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Raw mouse motion (`DeviceEvent::MouseMotion`).
pub fn raw_motion(dx: f64, dy: f64) {
	if let Ok(mut c) = LIVE.lock() {
		c.motion(now_ms(), (dx.abs() + dy.abs()) as f32);
	}
}

/// Is the mouse the device in use right now (the OS cursor is shown, or only held by the game)? See `CursorVis::mouse_in_use`.
pub fn mouse_in_use() -> bool {
	LIVE.lock().map_or(true, |c| c.mouse_in_use())
}

/// Window events: key presses, mouse button presses and the wheel. `captured` = the cursor is captured right now.
pub fn window_event(e: &WindowEvent, captured: bool) {
	let Ok(mut c) = LIVE.lock() else { return };
	match e {
		WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => c.key(captured),
		WindowEvent::MouseInput { state: ElementState::Pressed, .. } | WindowEvent::MouseWheel { .. } => c.click(),
		_ => {}
	}
}

/// `RAC_CURSOR_DEV`: the scripted device of this frame.
fn scripted(frame: u32) -> Option<Why> {
	static S: std::sync::OnceLock<Vec<(u32, u32, Why)>> = std::sync::OnceLock::new();
	let s = S.get_or_init(|| parse_script(&std::env::var("RAC_CURSOR_DEV").unwrap_or_default()));
	s.iter().rev().find(|(a, z, _)| (*a..*z).contains(&frame)).map(|x| x.2)
}

fn parse_script(v: &str) -> Vec<(u32, u32, Why)> {
	v.split(';')
		.filter_map(|e| {
			let (r, d) = e.split_once(':')?;
			let (a, z) = r.split_once('-')?;
			let why = match d.trim() {
				"mouse" => Why::Mouse,
				"key" => Why::Key,
				"pad" => Why::Pad,
				_ => return None,
			};
			Some((a.trim().parse().ok()?, z.trim().parse().unwrap_or(u32::MAX), why))
		})
		.collect()
}

/// Frame start: sets the window's cursor visibility (`captured` = the grab of this frame, or not retail: the old rule).
pub fn apply(window: &Window, captured: bool, retail: bool) {
	let frame = FRAME.fetch_add(1, Ordering::Relaxed);
	let (visible, why) = if retail {
		let Ok(mut c) = LIVE.lock() else { return };
		if let Some(w) = scripted(frame) {
			// A scripted device acts every frame it covers (one event per frame is enough for every transition).
			match w {
				Why::Mouse => c.click(),
				Why::Key => c.key(false),
				Why::Pad => c.pad(),
				Why::Start => {}
			}
		}
		if crate::retail::prompts::take_pad_pulse() {
			c.pad();
			// Real pad input also resets the OS idle timers (a pad plays without the window focus; awake.rs).
			crate::awake::pad_input();
		}
		(c.visible(captured), c.why())
	} else {
		(!captured, Why::Start)
	};
	if APPLIED.swap(visible as u8, Ordering::Relaxed) != visible as u8 {
		window.set_cursor_visible(visible);
		let reason = if captured { "captured".to_string() } else { format!("{why:?}").to_lowercase() };
		eprintln!("cursor: {} (frame {frame}, {reason})", if visible { "visible" } else { "hidden" });
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn hidden_until_the_mouse_moves() {
		let mut c = CursorVis::new();
		assert!(!c.visible(false));
		c.motion(1000, 2.0);
		assert!(!c.visible(false), "a bump of 2 counts is no use of the mouse");
		c.motion(1100, 5.0);
		assert!(c.visible(false));
		assert_eq!(c.why(), Why::Mouse);
	}

	#[test]
	fn slow_drift_never_adds_up() {
		let mut c = CursorVis::new();
		for i in 0..100 {
			c.motion(1000 + i * 1000, 2.0);
		}
		assert!(!c.visible(false));
	}

	#[test]
	fn click_shows_pad_and_key_hide() {
		let mut c = CursorVis::new();
		c.click();
		assert!(c.visible(false));
		c.pad();
		assert!(!c.visible(false));
		c.click();
		c.key(false);
		assert!(!c.visible(false));
		assert_eq!(c.why(), Why::Key);
	}

	#[test]
	fn captured_is_always_hidden_and_keeps_mouse_use() {
		let mut c = CursorVis::new();
		c.click();
		assert!(!c.visible(true));
		// WASD / Esc while the cursor is captured: the mouse stays the device, so the pause menu shows the cursor.
		c.key(true);
		assert!(c.visible(false));
		// A pad input while captured does hide it for the pause menu.
		c.pad();
		assert!(!c.visible(false));
	}

	#[test]
	fn pad_player_pausing_never_sees_the_cursor() {
		// Gameplay (captured), Start on the pad, pause menu (not captured), pad menu navigation: never visible.
		let mut c = CursorVis::new();
		for captured in [true, true, false, false] {
			c.pad();
			assert!(!c.visible(captured));
		}
		// Keyboard player in the menu: arrow keys keep it hidden.
		c.key(false);
		assert!(!c.visible(false));
	}

	#[test]
	fn mouse_then_pad_then_mouse() {
		let mut c = CursorVis::new();
		c.motion(0, 50.0);
		assert!(c.visible(false));
		c.pad();
		assert!(!c.visible(false));
		c.motion(5000, 50.0);
		assert!(c.visible(false));
	}

	#[test]
	fn a_hidden_cursor_is_not_the_mouse_in_use() {
		let mut c = CursorVis::new();
		assert!(!c.mouse_in_use(), "nothing used yet");
		c.motion(1000, 2.0);
		assert!(!c.mouse_in_use(), "a desk bump");
		c.motion(1100, 5.0);
		assert!(c.mouse_in_use());
		c.pad();
		assert!(!c.mouse_in_use());
		c.click();
		c.key(true);
		assert!(c.mouse_in_use(), "WASD while the mouse holds the cursor");
		c.key(false);
		assert!(!c.mouse_in_use());
	}

	#[test]
	fn dev_script_parses() {
		let s = parse_script("10-20:mouse; 30-:pad;x-1:key;5-6:nonsense");
		assert_eq!(s, vec![(10, 20, Why::Mouse), (30, u32::MAX, Why::Pad)]);
	}
}
