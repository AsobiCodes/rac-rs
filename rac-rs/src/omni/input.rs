//! Keyboard, mouse, window and pointer for the mods: the host entries `key`, `pressed`, `mouse`, `window`, `pointer`,
//! `take_input` and `capture_mouse` (their C callbacks are in `input_host.rs`).
//!
//! Shape (`omni_input` / keys.h): keys by HID usage with a held and a pressed bit, mouse buttons as OMNI_MOUSE_* bits,
//! the mouse's raw motion and the wheel in notches since the last tick, focused / captured / overlay, the pointer over the 4:3 picture.
//!
//! Flow, so a replay stays deterministic: the window thread feeds `LIVE` (events: `note_window_event`, `note_mouse_motion`); the
//! main thread takes one `InputSnap` per field (`take`, in `Retail::tick`) and hands it to the worker inside `FieldIn::omni`; the
//! worker publishes it (`set_field`) before the field runs, and every host entry reads that snapshot. A replay builds its
//! `FieldIn`s without this module, so mods see a neutral snapshot (nothing held, not focused) whatever the window did.
//!
//! Nothing is fed or taken until mods are loaded (`ACTIVE`): without mods every entry point is one atomic load.
//!
//! `take_input` / `capture_mouse` (the loader passes no mod id): see `Holds`.

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Mutex, MutexGuard};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::keyboard::{KeyCode as K, PhysicalKey};

/// OMNI_MOUSE_* (keys.h: bit `bind - 0x200` of MouseLeft, MouseMiddle, MouseRight, MouseBack, MouseForward).
pub const MOUSE_LEFT: u8 = 1;
pub const MOUSE_MIDDLE: u8 = 2;
pub const MOUSE_RIGHT: u8 = 4;
pub const MOUSE_BACK: u8 = 8;
pub const MOUSE_FORWARD: u8 = 16;
/// `take_input`'s `kinds_mask` (the loader: bit 1 keyboard, bit 2 mouse).
pub const TAKE_KEYBOARD: u32 = 1;
pub const TAKE_MOUSE: u32 = 2;
/// A wheel direction counts as held / pressed for the tick it turned in once it turned half a notch (keys.h `omni_bind_wheel`).
const WHEEL_HELD: f32 = 0.5;
/// Pixel-precise scroll units per notch (the same factor the viewer's own wheel uses).
const PIXELS_PER_NOTCH: f32 = 80.0;

/// The keyboard keys of keys.h's table: winit's key, the bind name (the W3C `code` name; winit calls the Meta keys Super) and the
/// USB HID usage, which is the key's index in `omni_input.keys`. IntlHash (usage 50) has no winit key.
pub static KEYS: &[(K, &str, u8)] = &[
	(K::KeyA, "KeyA", 4),
	(K::KeyB, "KeyB", 5),
	(K::KeyC, "KeyC", 6),
	(K::KeyD, "KeyD", 7),
	(K::KeyE, "KeyE", 8),
	(K::KeyF, "KeyF", 9),
	(K::KeyG, "KeyG", 10),
	(K::KeyH, "KeyH", 11),
	(K::KeyI, "KeyI", 12),
	(K::KeyJ, "KeyJ", 13),
	(K::KeyK, "KeyK", 14),
	(K::KeyL, "KeyL", 15),
	(K::KeyM, "KeyM", 16),
	(K::KeyN, "KeyN", 17),
	(K::KeyO, "KeyO", 18),
	(K::KeyP, "KeyP", 19),
	(K::KeyQ, "KeyQ", 20),
	(K::KeyR, "KeyR", 21),
	(K::KeyS, "KeyS", 22),
	(K::KeyT, "KeyT", 23),
	(K::KeyU, "KeyU", 24),
	(K::KeyV, "KeyV", 25),
	(K::KeyW, "KeyW", 26),
	(K::KeyX, "KeyX", 27),
	(K::KeyY, "KeyY", 28),
	(K::KeyZ, "KeyZ", 29),
	(K::Digit1, "Digit1", 30),
	(K::Digit2, "Digit2", 31),
	(K::Digit3, "Digit3", 32),
	(K::Digit4, "Digit4", 33),
	(K::Digit5, "Digit5", 34),
	(K::Digit6, "Digit6", 35),
	(K::Digit7, "Digit7", 36),
	(K::Digit8, "Digit8", 37),
	(K::Digit9, "Digit9", 38),
	(K::Digit0, "Digit0", 39),
	(K::Enter, "Enter", 40),
	(K::Escape, "Escape", 41),
	(K::Backspace, "Backspace", 42),
	(K::Tab, "Tab", 43),
	(K::Space, "Space", 44),
	(K::Minus, "Minus", 45),
	(K::Equal, "Equal", 46),
	(K::BracketLeft, "BracketLeft", 47),
	(K::BracketRight, "BracketRight", 48),
	(K::Backslash, "Backslash", 49),
	(K::Semicolon, "Semicolon", 51),
	(K::Quote, "Quote", 52),
	(K::Backquote, "Backquote", 53),
	(K::Comma, "Comma", 54),
	(K::Period, "Period", 55),
	(K::Slash, "Slash", 56),
	(K::CapsLock, "CapsLock", 57),
	(K::F1, "F1", 58),
	(K::F2, "F2", 59),
	(K::F3, "F3", 60),
	(K::F4, "F4", 61),
	(K::F5, "F5", 62),
	(K::F6, "F6", 63),
	(K::F7, "F7", 64),
	(K::F8, "F8", 65),
	(K::F9, "F9", 66),
	(K::F10, "F10", 67),
	(K::F11, "F11", 68),
	(K::F12, "F12", 69),
	(K::PrintScreen, "PrintScreen", 70),
	(K::ScrollLock, "ScrollLock", 71),
	(K::Pause, "Pause", 72),
	(K::Insert, "Insert", 73),
	(K::Home, "Home", 74),
	(K::PageUp, "PageUp", 75),
	(K::Delete, "Delete", 76),
	(K::End, "End", 77),
	(K::PageDown, "PageDown", 78),
	(K::ArrowRight, "ArrowRight", 79),
	(K::ArrowLeft, "ArrowLeft", 80),
	(K::ArrowDown, "ArrowDown", 81),
	(K::ArrowUp, "ArrowUp", 82),
	(K::NumLock, "NumLock", 83),
	(K::NumpadDivide, "NumpadDivide", 84),
	(K::NumpadMultiply, "NumpadMultiply", 85),
	(K::NumpadSubtract, "NumpadSubtract", 86),
	(K::NumpadAdd, "NumpadAdd", 87),
	(K::NumpadEnter, "NumpadEnter", 88),
	(K::Numpad1, "Numpad1", 89),
	(K::Numpad2, "Numpad2", 90),
	(K::Numpad3, "Numpad3", 91),
	(K::Numpad4, "Numpad4", 92),
	(K::Numpad5, "Numpad5", 93),
	(K::Numpad6, "Numpad6", 94),
	(K::Numpad7, "Numpad7", 95),
	(K::Numpad8, "Numpad8", 96),
	(K::Numpad9, "Numpad9", 97),
	(K::Numpad0, "Numpad0", 98),
	(K::NumpadDecimal, "NumpadDecimal", 99),
	(K::IntlBackslash, "IntlBackslash", 100),
	(K::ContextMenu, "ContextMenu", 101),
	(K::NumpadEqual, "NumpadEqual", 103),
	(K::ControlLeft, "ControlLeft", 224),
	(K::ShiftLeft, "ShiftLeft", 225),
	(K::AltLeft, "AltLeft", 226),
	(K::SuperLeft, "MetaLeft", 227),
	(K::ControlRight, "ControlRight", 228),
	(K::ShiftRight, "ShiftRight", 229),
	(K::AltRight, "AltRight", 230),
	(K::SuperRight, "MetaRight", 231),
];

/// A name of keys.h's table that is not in `KEYS`: usage 50 without a winit key.
const INTL_HASH: u8 = 50;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Wheel {
	Up,
	Down,
	Left,
	Right,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Bind {
	Key(u8),
	Mouse(u8),
	Wheel(Wheel),
}

/// What a bind name (`omni.key("KeyW")`, `"MouseLeft"`, `"WheelUp"`) is; None for a name not in the table.
fn bind(name: &str) -> Option<Bind> {
	if let Some(k) = KEYS.iter().find(|k| k.1 == name) {
		return Some(Bind::Key(k.2));
	}
	Some(match name {
		"IntlHash" => Bind::Key(INTL_HASH),
		"MouseLeft" => Bind::Mouse(MOUSE_LEFT),
		"MouseMiddle" => Bind::Mouse(MOUSE_MIDDLE),
		"MouseRight" => Bind::Mouse(MOUSE_RIGHT),
		"MouseBack" => Bind::Mouse(MOUSE_BACK),
		"MouseForward" => Bind::Mouse(MOUSE_FORWARD),
		"WheelUp" => Bind::Wheel(Wheel::Up),
		"WheelDown" => Bind::Wheel(Wheel::Down),
		"WheelLeft" => Bind::Wheel(Wheel::Left),
		"WheelRight" => Bind::Wheel(Wheel::Right),
		_ => return None,
	})
}

/// The HID usage of a winit key, None for a key the table does not have.
pub fn usage(k: K) -> Option<u8> {
	KEYS.iter().find(|e| e.0 == k).map(|e| e.2)
}

/// The OMNI_MOUSE_* bit of a winit button.
pub fn mouse_bit(b: MouseButton) -> Option<u8> {
	match b {
		MouseButton::Left => Some(MOUSE_LEFT),
		MouseButton::Middle => Some(MOUSE_MIDDLE),
		MouseButton::Right => Some(MOUSE_RIGHT),
		MouseButton::Back => Some(MOUSE_BACK),
		MouseButton::Forward => Some(MOUSE_FORWARD),
		MouseButton::Other(_) => None,
	}
}

fn bit_set(b: &mut [u64; 4], u: u8, on: bool) {
	let (w, m) = ((u >> 6) as usize, 1u64 << (u & 63));
	if on {
		b[w] |= m;
	} else {
		b[w] &= !m;
	}
}

fn bit_get(b: &[u64; 4], u: u8) -> bool {
	b[(u >> 6) as usize] >> (u & 63) & 1 != 0
}

/// The input of one field, as the mods see it (`omni_input`). Copy, so it rides in `FieldIn`; the default is neutral.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputSnap {
	/// Keys down now / went down since the last snapshot (a tap shorter than a tick too), bit = HID usage.
	pub held: [u64; 4],
	pub pressed: [u64; 4],
	pub mouse_held: u8,
	pub mouse_pressed: u8,
	/// Raw motion in pixels (y down) and the wheel in notches (x right, y away from the player) since the last snapshot.
	pub dx: f32,
	pub dy: f32,
	pub wheel_x: f32,
	pub wheel_y: f32,
	pub focused: bool,
	pub captured: bool,
	pub overlay: bool,
	/// The cursor over the 4:3 picture: 0..1 across and down (outside it below 0 or above 1), and whether it is over it.
	pub pointer: [f32; 2],
	pub inside: bool,
}

impl InputSnap {
	pub const NEUTRAL: InputSnap = InputSnap {
		held: [0; 4],
		pressed: [0; 4],
		mouse_held: 0,
		mouse_pressed: 0,
		dx: 0.0,
		dy: 0.0,
		wheel_x: 0.0,
		wheel_y: 0.0,
		focused: false,
		captured: false,
		overlay: false,
		pointer: [0.0; 2],
		inside: false,
	};

	/// `omni.key(name)`.
	pub fn key(&self, name: &str) -> bool {
		self.test(name, false)
	}

	/// `omni.pressed(name)`.
	pub fn pressed(&self, name: &str) -> bool {
		self.test(name, true)
	}

	fn test(&self, name: &str, pressed: bool) -> bool {
		match bind(name) {
			Some(Bind::Key(u)) => bit_get(if pressed { &self.pressed } else { &self.held }, u),
			Some(Bind::Mouse(b)) => (if pressed { self.mouse_pressed } else { self.mouse_held }) & b != 0,
			Some(Bind::Wheel(w)) => self.notches(w) >= WHEEL_HELD,
			None => false,
		}
	}

	fn notches(&self, w: Wheel) -> f32 {
		match w {
			Wheel::Up => self.wheel_y,
			Wheel::Down => -self.wheel_y,
			Wheel::Right => self.wheel_x,
			Wheel::Left => -self.wheel_x,
		}
	}

	/// `omni.mouse()`: dx, dy, wheel_x, wheel_y.
	pub fn mouse(&self) -> [f64; 4] {
		[self.dx as f64, self.dy as f64, self.wheel_x as f64, self.wheel_y as f64]
	}

	/// `omni.window()`: focused, captured, overlay.
	pub fn window(&self) -> [i32; 3] {
		[self.focused as i32, self.captured as i32, self.overlay as i32]
	}
}

impl Default for InputSnap {
	fn default() -> Self {
		Self::NEUTRAL
	}
}

/// What the window thread has seen since the last `take`.
struct Live {
	held: [u64; 4],
	pressed: [u64; 4],
	mouse_held: u8,
	mouse_pressed: u8,
	dx: f32,
	dy: f32,
	wheel_x: f32,
	wheel_y: f32,
	focused: bool,
	/// The cursor as fractions of the client area, None when it is outside the window.
	cursor: Option<[f32; 2]>,
}

impl Live {
	const NEW: Live = Live {
		held: [0; 4],
		pressed: [0; 4],
		mouse_held: 0,
		mouse_pressed: 0,
		dx: 0.0,
		dy: 0.0,
		wheel_x: 0.0,
		wheel_y: 0.0,
		focused: true,
		cursor: None,
	};

	fn key(&mut self, usage: u8, down: bool) {
		if down && !bit_get(&self.held, usage) {
			bit_set(&mut self.pressed, usage, true);
		}
		bit_set(&mut self.held, usage, down);
	}

	fn button(&mut self, bit: u8, down: bool) {
		if down && self.mouse_held & bit == 0 {
			self.mouse_pressed |= bit;
		}
		if down {
			self.mouse_held |= bit;
		} else {
			self.mouse_held &= !bit;
		}
	}

	fn wheel(&mut self, x: f32, y: f32) {
		self.wheel_x += x;
		self.wheel_y += y;
	}

	/// Losing the focus lets go of every key and button (their releases go to the other window); nothing is held while unfocused.
	fn focus(&mut self, on: bool) {
		self.focused = on;
		if !on {
			self.held = [0; 4];
			self.mouse_held = 0;
		}
	}

	/// The snapshot for the next field; "since the last tick" restarts (pressed, motion, wheel). `to_picture` maps the cursor
	/// (fractions of the client area) to fractions of the 4:3 picture.
	fn take(&mut self, captured: bool, to_picture: impl FnOnce(f32, f32) -> [f32; 2]) -> InputSnap {
		let pointer = match self.cursor.filter(|_| self.focused) {
			Some(c) => to_picture(c[0], c[1]),
			None => [0.0; 2],
		};
		let inside = self.focused && self.cursor.is_some() && (0.0..=1.0).contains(&pointer[0]) && (0.0..=1.0).contains(&pointer[1]);
		let s = InputSnap {
			held: self.held,
			pressed: self.pressed,
			mouse_held: self.mouse_held,
			mouse_pressed: self.mouse_pressed,
			dx: self.dx,
			dy: self.dy,
			wheel_x: self.wheel_x,
			wheel_y: self.wheel_y,
			focused: self.focused,
			captured,
			overlay: false,
			pointer,
			inside,
		};
		self.pressed = [0; 4];
		self.mouse_pressed = 0;
		(self.dx, self.dy, self.wheel_x, self.wheel_y) = (0.0, 0.0, 0.0, 0.0);
		s
	}
}

/// `take_input` / `capture_mouse`, with the book-keeping that lets a failed mod give them back.
///
/// The loader's two callbacks carry no mod id, and `take_input` only works while a mod's script first runs. The mods load one after
/// the other, and each one ends with a log line carrying its id: `loaded <version>` (Info) or `failed: <why>` (Error). So what was
/// asked since the last such line belongs to the mod of the next one: kept at `loaded`, dropped at `failed`. After the load a
/// `failed:` line removes that mod's hold. `capture_mouse` asked at run time (a tick, a hook) has no owner; any mod failing then
/// lets go of it (the latest asking of any mod counts, as the format says, and a mod that still wants it asks again).
struct Holds {
	loading: bool,
	pending_take: u32,
	pending_cap: Option<bool>,
	/// (mod id, take_input mask) of the mods that loaded.
	mods: Vec<(String, u32)>,
	cap_on: bool,
	/// Who asked last: Some(id) when it happened while loading, None at run time.
	cap_owner: Option<String>,
}

impl Holds {
	const NEW: Holds = Holds { loading: false, pending_take: 0, pending_cap: None, mods: Vec::new(), cap_on: false, cap_owner: None };

	fn take_input(&mut self, mask: u32) {
		if self.loading {
			self.pending_take |= mask;
		} else {
			self.add_take("", mask);
		}
	}

	fn add_take(&mut self, id: &str, mask: u32) {
		match self.mods.iter_mut().find(|m| m.0 == id) {
			Some(m) => m.1 |= mask,
			None => self.mods.push((id.to_string(), mask)),
		}
	}

	fn capture(&mut self, on: bool) {
		if self.loading {
			self.pending_cap = Some(on);
		} else {
			(self.cap_on, self.cap_owner) = (on, None);
		}
	}

	/// A log line of the loader or a mod.
	fn on_log(&mut self, level: i32, id: &str, msg: &str) {
		if id.is_empty() {
			return;
		}
		if level == 0 && self.loading && msg.starts_with("loaded ") {
			self.commit(id);
		} else if level == 2 && msg.starts_with("failed: ") {
			self.fail(id);
		}
	}

	fn commit(&mut self, id: &str) {
		let mask = std::mem::take(&mut self.pending_take);
		if mask != 0 {
			self.add_take(id, mask);
		}
		if let Some(on) = self.pending_cap.take() {
			(self.cap_on, self.cap_owner) = (on, Some(id).filter(|i| !i.is_empty()).map(str::to_string));
		}
	}

	/// The load is over. Whatever is still pending (the loader's last line was not the one expected) is kept, owner unknown.
	fn end_load(&mut self) {
		self.commit("");
		self.loading = false;
	}

	fn fail(&mut self, id: &str) {
		self.mods.retain(|m| m.0 != id);
		if self.loading {
			(self.pending_take, self.pending_cap) = (0, None);
		}
		if self.cap_on && self.cap_owner.as_deref().is_none_or(|o| o == id) {
			(self.cap_on, self.cap_owner) = (false, None);
		}
	}

	fn taken(&self) -> u32 {
		self.mods.iter().fold(0, |a, m| a | m.1)
	}
}

/// Mods are loaded: the window feeds are on. Set when the load begins (before the mods' main scripts run), cleared again if it fails.
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// The window has the focus (the viewer's `focused`, mirrored for `mouse_owned`).
static FOCUSED: AtomicBool = AtomicBool::new(true);
/// The cursor is grabbed now (the viewer's `grabbed`: the modern camera's or a mod's), for `window()`.
static CAPTURED: AtomicBool = AtomicBool::new(false);
/// A mod took the mouse (`take_input`), or asks for the cursor captured (`capture_mouse`). The keyboard is the clash switch's KBM bit (clash.rs).
static TAKEN_MOUSE: AtomicBool = AtomicBool::new(false);
static CAPTURE: AtomicBool = AtomicBool::new(false);
static LIVE: Mutex<Live> = Mutex::new(Live::NEW);
/// The snapshot of the field the worker runs (what the host entries read).
static FIELD: Mutex<InputSnap> = Mutex::new(InputSnap::NEUTRAL);
static HOLDS: Mutex<Holds> = Mutex::new(Holds::NEW);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
	m.lock().unwrap_or_else(|e| e.into_inner())
}

fn publish(h: &Holds) {
	let t = h.taken();
	super::clash::set_keyboard_taken(t & TAKE_KEYBOARD != 0);
	TAKEN_MOUSE.store(t & TAKE_MOUSE != 0, Relaxed);
	CAPTURE.store(h.cap_on, Relaxed);
}

/// The mods' main scripts are about to run: from now on the window feeds are on and `take_input` / `capture_mouse` are collected.
pub fn begin_load() {
	ACTIVE.store(true, Relaxed);
	lock(&HOLDS).loading = true;
}

/// The load is over; when it failed no mod runs, so everything is switched off again.
pub fn end_load(ok: bool) {
	let mut h = lock(&HOLDS);
	if ok {
		h.end_load();
	} else {
		*h = Holds::NEW;
		ACTIVE.store(false, Relaxed);
		*lock(&FIELD) = InputSnap::NEUTRAL;
	}
	publish(&h);
}

/// Window events (main thread, first thing in `window_event`); `client` is the client area in pixels.
pub fn note_window_event(e: &WindowEvent, client: (u32, u32)) {
	if !ACTIVE.load(Relaxed) {
		return;
	}
	let mut l = lock(&LIVE);
	match e {
		WindowEvent::KeyboardInput { event, .. } => {
			if let Some(u) = (if let PhysicalKey::Code(k) = event.physical_key { usage(k) } else { None }) {
				l.key(u, event.state == ElementState::Pressed);
			}
		}
		WindowEvent::MouseInput { state, button, .. } => {
			if let Some(b) = mouse_bit(*button) {
				l.button(b, *state == ElementState::Pressed);
			}
		}
		WindowEvent::MouseWheel { delta, .. } => match delta {
			MouseScrollDelta::LineDelta(x, y) => l.wheel(*x, *y),
			MouseScrollDelta::PixelDelta(p) => l.wheel(p.x as f32 / PIXELS_PER_NOTCH, p.y as f32 / PIXELS_PER_NOTCH),
		},
		WindowEvent::CursorMoved { position, .. } => {
			l.cursor = Some([position.x as f32 / client.0.max(1) as f32, position.y as f32 / client.1.max(1) as f32]);
		}
		WindowEvent::CursorLeft { .. } => l.cursor = None,
		WindowEvent::Focused(f) => {
			l.focus(*f);
			FOCUSED.store(*f, Relaxed);
		}
		_ => {}
	}
}

/// Raw mouse motion (`DeviceEvent::MouseMotion`, main thread).
pub fn note_mouse_motion(dx: f64, dy: f64) {
	if ACTIVE.load(Relaxed) {
		let mut l = lock(&LIVE);
		l.dx += dx as f32;
		l.dy += dy as f32;
	}
}

/// The viewer grabbed or freed the cursor (once per frame).
pub fn note_captured(grabbed: bool) {
	if ACTIVE.load(Relaxed) {
		CAPTURED.store(grabbed, Relaxed);
	}
}

/// The snapshot for the field about to be handed to the worker (main thread, `Retail::tick`); the motion, wheel and pressed bits
/// since the previous one. `to_picture` maps the cursor (fractions of the client area) to fractions of the 4:3 picture.
pub fn take(to_picture: impl FnOnce(f32, f32) -> [f32; 2]) -> InputSnap {
	if !ACTIVE.load(Relaxed) {
		return InputSnap::NEUTRAL;
	}
	lock(&LIVE).take(CAPTURED.load(Relaxed), to_picture)
}

/// The worker is about to run a field with this input: what the host entries answer until the next field.
pub fn set_field(s: &InputSnap) {
	if ACTIVE.load(Relaxed) {
		*lock(&FIELD) = *s;
	}
}

/// The snapshot the host entries answer from.
pub fn current() -> InputSnap {
	*lock(&FIELD)
}

/// `take_input(mask)` from a mod's main script.
pub fn take_input(mask: u32) {
	let mut h = lock(&HOLDS);
	h.take_input(mask);
	publish(&h);
}

/// `capture_mouse(on)` from a mod.
pub fn capture_mouse(on: bool) {
	let mut h = lock(&HOLDS);
	h.capture(on);
	publish(&h);
}

/// A log line from the loader or a mod (host.rs `log_cb`): the loader's `loaded` / `failed:` lines tell which mod owns what was asked.
pub fn note_log(level: i32, id: &str, msg: &str) {
	if (level == 0 && msg.starts_with("loaded ")) || (level == 2 && msg.starts_with("failed: ")) {
		let mut h = lock(&HOLDS);
		h.on_log(level, id, msg);
		publish(&h);
	}
}

/// A mod took the keyboard: the game's pad is built without it (`Retail::tick_keys`). This is the clash switch's keyboard-table bit
/// (clash.rs: it also covers a mod whose manifest `conflicts` names rac-rs.kbm).
pub fn keyboard_taken() -> bool {
	super::clash::off(super::clash::KBM)
}

/// The ids of the mods that took the keyboard (a take asked at run time has no owner and is not listed), for the clash switch's log line.
pub fn keyboard_owners() -> Vec<String> {
	lock(&HOLDS).mods.iter().filter(|m| m.1 & TAKE_KEYBOARD != 0 && !m.0.is_empty()).map(|m| m.0.clone()).collect()
}

/// A mod asks for the cursor captured (`capture_mouse(true)`): the viewer grabs it while the window has the focus.
pub fn capture_wanted() -> bool {
	CAPTURE.load(Relaxed)
}

/// A mod has the mouse: it took it (`take_input("mouse")`) or captured the cursor while the window has the focus. The game's
/// own mouse (camera look, aim, clicks, menu cursor) stays out meanwhile.
pub fn mouse_owned() -> bool {
	TAKEN_MOUSE.load(Relaxed) || (CAPTURE.load(Relaxed) && FOCUSED.load(Relaxed))
}

/// Events the game must not see while a mod owns the mouse: presses, the wheel, the cursor (releases pass, so nothing sticks).
pub fn swallows(e: &WindowEvent) -> bool {
	mouse_owned() && matches!(e, WindowEvent::MouseInput { state: ElementState::Pressed, .. } | WindowEvent::MouseWheel { .. } | WindowEvent::CursorMoved { .. })
}

/// The keys the pad is built from: none while a mod has the keyboard (`taken`).
pub fn gated(taken: bool, held: impl Fn(K) -> bool) -> impl Fn(K) -> bool {
	move |k| !taken && held(k)
}

#[cfg(test)]
mod tests;
