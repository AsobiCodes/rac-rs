//! Button remap: which keyboard key or controller button drives which PS2 pad function. The model (this file), the options-file
//! keys, the keyboard and controller input it steers (`retail::pad_from_keys`, `prompts::map_xinput`) and the key names of the
//! prompts; the menu page that edits it is `pcctl.rs` (Pause -> Options -> Controls).
//!
//! Functions. The keyboard drives 24: the twelve buttons (Cross, Square, Triangle, Circle, L1, R1, L2, R2, L3, R3, Start,
//! Select), the four d-pad directions and the eight stick directions (the left stick: forward, back, left, right, which is
//! movement; the right stick: camera up, down, left, right). A controller drives the first 16 (the sticks are analog and stay
//! as they are). Every function has one key (Start starts with three: Esc, Enter, Numpad Enter) or one controller button;
//! a key or button may drive SEVERAL functions (v0.4.1, 2026-10-27: "binding one button moved two other actions"): binding
//! changes only the function being bound, nothing else moves (no swap, no chain), so no function is ever left unbound by the
//! menu, and e.g. Fire (Square) can sit on the trigger that is also the strafe aim (R2) or on a second key next to Circle. The
//! Controls page shows a shared key or button on every row it drives (`shared`, `pad_shared`). The defaults are exactly the
//! bindings the port had before the menu existed.
//!
//! What stays fixed: the menu keys (`menu_keys.rs`: arrows / WASD, Enter / Space, Esc / Backspace, Q / E, Tab), so a bad remap
//! never locks the player out of a menu; Esc and Space skip scenes and movies; the mouse (left button = Circle, middle =
//! Triangle, right = aim / menu back, `modcam.rs`); F1..F12 and the keys of the OS cannot be bound. Esc cannot be captured (it
//! cancels the capture), so it keeps no binding once it has been taken off Start, and "Reset" brings it back.
//!
//! File (`pc_options.txt`, see `pcopts.rs`): only what differs from the defaults is written, so a player who never opens the page has
//! no line. `keys=cross:KeyK,start:Escape+Enter` (function:winit key names, `+` between the keys of one function) and
//! `pad=cross:b,l2:lb` (function:controller button). Function names are `ID`, key names `KEYS`, button names `PHYS`; anything
//! that cannot be read is skipped; every entry binds its function alone, as the menu does (a file written before v0.4.1 lists the
//! functions a swap had moved, so it reads as it did).
use std::sync::Mutex;
use winit::keyboard::KeyCode as K;

/// Functions the keyboard drives, and the ones a controller drives (the first `NP`).
pub const NK: usize = 24;
pub const NP: usize = 16;

/// Function indices (the order of every table here and of the menu's rows): the 12 buttons, the d-pad (up, down, left, right),
/// then the left stick (forward, back, left, right) and the right stick (up, down, left, right).
pub const CROSS: usize = 0;
pub const SQUARE: usize = 1;
pub const TRIANGLE: usize = 2;
pub const CIRCLE: usize = 3;
pub const L1: usize = 4;
pub const R1: usize = 5;
pub const L2: usize = 6;
pub const R2: usize = 7;
pub const DPAD: usize = 12;
pub const LSTICK: usize = 16;
pub const RSTICK: usize = 20;
/// The stick buttons (the L3 and R3 presses; the sticks' directions are `LSTICK` and `RSTICK`).
pub const L3: usize = 8;
pub const R3: usize = 9;

/// The PS2 pad bit of each button function (the d-pad in the order up, down, left, right); the stick functions have none.
pub const BIT: [u16; NP] = [0x40, 0x80, 0x10, 0x20, 4, 8, 1, 2, 0x200, 0x400, 0x800, 0x100, 0x1000, 0x4000, 0x8000, 0x2000];

/// The function names of the options file.
pub const ID: [&str; NK] = [
	"cross", "square", "triangle", "circle", "l1", "r1", "l2", "r2", "l3", "r3", "start", "select", "up", "down", "left", "right", "forward", "back", "strafe_left", "strafe_right", "camera_up", "camera_down", "camera_left", "camera_right",
];

/// The stick axis (index into `[rx, ry, lx, ly]`) and direction (the axis moves from 0x80 by 0x7f times this) of stick function `f - LSTICK`.
const STICK: [(usize, i32); 8] = [(3, -1), (3, 1), (2, -1), (2, 1), (1, -1), (1, 1), (0, -1), (0, 1)];

const fn k1(a: K) -> [Option<K>; 3] {
	[Some(a), None, None]
}

/// Today's keyboard: Space = Cross, F = Square, T = Triangle, G = Circle, C = L1, Left Ctrl = R1, V = L2, B = R2, Left Shift =
/// L3, R = R3, Esc / Enter / Numpad Enter = Start, M = Select, arrows = d-pad, WASD = left stick, X / Z = right stick up / down,
/// E / Q = right stick left / right.
pub const DEFAULT_KEYS: [[Option<K>; 3]; NK] = [
	k1(K::Space),
	k1(K::KeyF),
	k1(K::KeyT),
	k1(K::KeyG),
	k1(K::KeyC),
	k1(K::ControlLeft),
	k1(K::KeyV),
	k1(K::KeyB),
	k1(K::ShiftLeft),
	k1(K::KeyR),
	[Some(K::Escape), Some(K::Enter), Some(K::NumpadEnter)],
	k1(K::KeyM),
	k1(K::ArrowUp),
	k1(K::ArrowDown),
	k1(K::ArrowLeft),
	k1(K::ArrowRight),
	k1(K::KeyW),
	k1(K::KeyS),
	k1(K::KeyA),
	k1(K::KeyD),
	k1(K::KeyX),
	k1(K::KeyZ),
	k1(K::KeyE),
	k1(K::KeyQ),
];

/// The keys a player can bind: (key, name in the file, name on screen). Escape is listed for the file (Start's default) but is
/// not capturable; F1..F12, the Windows keys and the like are not listed.
macro_rules! keys {
	($($k:ident $s:expr),* $(,)?) => { &[$((K::$k, stringify!($k), $s)),*] };
}
pub static KEYS: &[(K, &str, &str)] = keys![
	KeyA "A", KeyB "B", KeyC "C", KeyD "D", KeyE "E", KeyF "F", KeyG "G", KeyH "H", KeyI "I", KeyJ "J", KeyK "K", KeyL "L", KeyM "M",
	KeyN "N", KeyO "O", KeyP "P", KeyQ "Q", KeyR "R", KeyS "S", KeyT "T", KeyU "U", KeyV "V", KeyW "W", KeyX "X", KeyY "Y", KeyZ "Z",
	Digit0 "0", Digit1 "1", Digit2 "2", Digit3 "3", Digit4 "4", Digit5 "5", Digit6 "6", Digit7 "7", Digit8 "8", Digit9 "9",
	Numpad0 "Num0", Numpad1 "Num1", Numpad2 "Num2", Numpad3 "Num3", Numpad4 "Num4", Numpad5 "Num5", Numpad6 "Num6", Numpad7 "Num7",
	Numpad8 "Num8", Numpad9 "Num9", NumpadAdd "Num+", NumpadSubtract "Num-", NumpadMultiply "Num*", NumpadDivide "Num/",
	NumpadDecimal "Num.", NumpadEnter "NumEnter",
	ArrowUp "Up", ArrowDown "Down", ArrowLeft "Left", ArrowRight "Right",
	Space "Space", Enter "Enter", Tab "Tab", Backspace "Bksp", Escape "Esc",
	ShiftLeft "LShift", ShiftRight "RShift", ControlLeft "Ctrl", ControlRight "RCtrl", AltLeft "LAlt", AltRight "RAlt", CapsLock "Caps",
	Minus "-", Equal "=", BracketLeft "[", BracketRight "]", Backslash "\\", Semicolon ";", Quote "'", Backquote "`", Comma ",", Period ".", Slash "/",
	Insert "Ins", Delete "Del", Home "Home", End "End", PageUp "PgUp", PageDown "PgDn",
];

/// The key named `s` in the options file (case does not matter; a few short names too).
pub fn key_from_name(s: &str) -> Option<K> {
	let s = s.trim();
	KEYS.iter().find(|k| k.1.eq_ignore_ascii_case(s)).map(|k| k.0)
}

/// A key's name in the file; None for a key that cannot be bound.
pub fn key_file_name(k: K) -> Option<&'static str> {
	KEYS.iter().find(|e| e.0 == k).map(|e| e.1)
}

/// A key's name on screen.
pub fn key_shown(k: K) -> &'static str {
	KEYS.iter().find(|e| e.0 == k).map_or("?", |e| e.2)
}

/// Whether the menu may bind `k`: listed, and not Esc (it cancels).
pub fn bindable(k: K) -> bool {
	k != K::Escape && KEYS.iter().any(|e| e.0 == k)
}

/// The keyboard's bindings: up to three keys per function.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KbBinds(pub [[Option<K>; 3]; NK]);

impl Default for KbBinds {
	fn default() -> KbBinds {
		KbBinds(DEFAULT_KEYS)
	}
}

/// The keys of `a` moved to the front.
fn norm(a: [Option<K>; 3]) -> [Option<K>; 3] {
	let mut o = [None; 3];
	for (i, k) in a.iter().flatten().enumerate() {
		o[i] = Some(*k);
	}
	o
}

impl KbBinds {
	/// The DualShock report of the keys `held` (plus the mouse's `circle` / `triangle`): (buttons, [rx, ry, lx, ly]).
	pub fn pad(&self, held: impl Fn(K) -> bool, circle: bool, triangle: bool) -> (u16, [u8; 4]) {
		let on = |f: usize| self.0[f].iter().flatten().any(|&k| held(k));
		let mut b = 0u16;
		for f in 0..NP {
			if on(f) {
				b |= BIT[f];
			}
		}
		if circle {
			b |= BIT[CIRCLE];
		}
		if triangle {
			b |= BIT[TRIANGLE];
		}
		let mut d = [0i32; 4];
		for (i, (axis, dir)) in STICK.iter().enumerate() {
			if on(LSTICK + i) {
				d[*axis] += dir;
			}
		}
		(b, d.map(|d| (0x80 + d * 0x7f).clamp(0, 0xff) as u8))
	}

	/// Function `f` gets `new`; every other function keeps what it has (a key may drive several functions, nothing else moves).
	pub fn assign(&mut self, f: usize, new: [Option<K>; 3]) {
		self.0[f] = norm(new);
	}

	/// The other functions that have a key `f` has (the rows the Controls page shows the key on besides `f`'s own).
	pub fn shared(&self, f: usize) -> Vec<usize> {
		(0..NK).filter(|&g| g != f && self.0[g].iter().flatten().any(|k| self.0[f].contains(&Some(*k)))).collect()
	}

	/// The options-file value: what differs from the defaults; None when nothing does.
	pub fn to_text(&self) -> Option<String> {
		let v: Vec<String> = (0..NK)
			.filter(|&f| self.0[f] != DEFAULT_KEYS[f])
			.map(|f| format!("{}:{}", ID[f], self.0[f].iter().flatten().filter_map(|&k| key_file_name(k)).collect::<Vec<_>>().join("+")))
			.collect();
		(!v.is_empty()).then(|| v.join(","))
	}

	/// The bindings an options-file value says (`to_text`'s inverse): the defaults with each readable entry applied as the menu would.
	pub fn from_text(s: &str) -> KbBinds {
		let mut b = KbBinds::default();
		for e in s.split(',') {
			let Some((f, ks)) = e.split_once(':') else { continue };
			let Some(f) = ID.iter().position(|n| n.eq_ignore_ascii_case(f.trim())) else { continue };
			let keys: Vec<K> = ks.split('+').filter_map(key_from_name).take(3).collect();
			if keys.is_empty() {
				continue;
			}
			let mut a = [None; 3];
			for (i, k) in keys.iter().enumerate() {
				a[i] = Some(*k);
			}
			b.assign(f, a);
		}
		b
	}

	/// The name of function `f` on screen: its keys, at most two ("Esc/Enter").
	pub fn shown(&self, f: usize) -> String {
		let v: Vec<&str> = self.0[f].iter().flatten().take(2).map(|&k| key_shown(k)).collect();
		if v.is_empty() {
			"-".into()
		} else {
			v.join("/")
		}
	}
}

/// The controller's physical buttons: (name in the file, XInput mask, name on an Xbox pad, name on a PlayStation pad). Masks are
/// XInput's buttons; the triggers (`prompts::TRIGGER`) are bits 16 and 17. A PlayStation controller arrives as an XInput report
/// with its buttons in their PS2 places (pspad.rs).
pub const PHYS: [(&str, u32, &str, &str); NP] = [
	("a", 0x1000, "A", "Cross"),
	("b", 0x2000, "B", "Circle"),
	("x", 0x4000, "X", "Square"),
	("y", 0x8000, "Y", "Triangle"),
	("lb", 0x100, "LB", "L1"),
	("rb", 0x200, "RB", "R1"),
	("start", 0x10, "Start", "Options"),
	("back", 0x20, "Back", "Share"),
	("ls", 0x40, "L3", "L3"),
	("rs", 0x80, "R3", "R3"),
	("dup", 1, "D-Up", "D-Up"),
	("ddown", 2, "D-Down", "D-Down"),
	("dleft", 4, "D-Left", "D-Left"),
	("dright", 8, "D-Right", "D-Right"),
	("lt", 1 << 16, "LT", "L2"),
	("rt", 1 << 17, "RT", "R2"),
];

/// Today's controller: A = Cross, X = Square, Y = Triangle, B = Circle, LB = L1, RB = R1, LT = L2, RT = R2, L3, R3, Start,
/// Back = Select, the d-pad as is: the index into `PHYS` of each function.
pub const DEFAULT_PAD: [u8; NP] = [0, 2, 3, 1, 4, 5, 14, 15, 8, 9, 6, 7, 10, 11, 12, 13];

/// The PS2 buttons a controller's pressed physical buttons (`PHYS` masks) give, under `map` (function -> `PHYS` index).
pub fn pad_buttons(phys: u32, map: &[u8; NP]) -> u16 {
	let mut b = 0;
	for f in 0..NP {
		if phys & PHYS[map[f] as usize % NP].1 != 0 {
			b |= BIT[f];
		}
	}
	b
}

/// `map` with function `f` on button `p`; every other function keeps its button (a button may drive several functions).
pub fn pad_assign(map: &mut [u8; NP], f: usize, p: usize) {
	map[f] = p as u8;
}

/// The other controller functions on the button `f` is on.
pub fn pad_shared(map: &[u8; NP], f: usize) -> Vec<usize> {
	(0..NP).filter(|&g| g != f && map[g] == map[f]).collect()
}

pub fn pad_to_text(map: &[u8; NP]) -> Option<String> {
	let v: Vec<String> = (0..NP).filter(|&f| map[f] != DEFAULT_PAD[f]).map(|f| format!("{}:{}", ID[f], PHYS[map[f] as usize % NP].0)).collect();
	(!v.is_empty()).then(|| v.join(","))
}

pub fn pad_from_text(s: &str) -> [u8; NP] {
	let mut m = DEFAULT_PAD;
	for e in s.split(',') {
		let Some((f, p)) = e.split_once(':') else { continue };
		let (Some(f), Some(p)) = (ID[..NP].iter().position(|n| n.eq_ignore_ascii_case(f.trim())), PHYS.iter().position(|e| e.0.eq_ignore_ascii_case(p.trim()))) else { continue };
		pad_assign(&mut m, f, p);
	}
	m
}

/// The physical button `p`'s name on screen; PlayStation names when a PlayStation controller is the one connected.
pub fn pad_shown(p: u8, playstation: bool) -> &'static str {
	let e = &PHYS[p as usize % NP];
	if playstation {
		e.3
	} else {
		e.2
	}
}

// ---- the live bindings and the capture ---------------------------------------------------------------------------------

/// What the Controls page waits for: the next fresh key (`Key`) or controller button (`Pad`) for function `f`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cap {
	Idle,
	Key(usize),
	Pad(usize),
}

/// Fields (one per `tick_keys`, 20 ms) a capture waits before it gives up: 8 s.
pub const CAP_FIELDS: u32 = 400;

/// Everything the page and the input share: the bindings, the change flag and the capture.
struct Live {
	kb: KbBinds,
	pad: [u8; NP],
	/// A binding changed since the worker last looked (it writes the options file).
	dirty: bool,
	cap: Cap,
	/// Fields the capture has waited.
	fields: u32,
	/// The capture ended (bound, cancelled or timed out) since the worker last looked.
	ended: bool,
}

impl Live {
	const START: Live = Live { kb: KbBinds(DEFAULT_KEYS), pad: DEFAULT_PAD, dirty: false, cap: Cap::Idle, fields: 0, ended: false };

	fn assign_key(&mut self, f: usize, k: K) {
		if f < NK && bindable(k) {
			self.kb.assign(f, [Some(k), None, None]);
			self.dirty = true;
		}
	}

	fn assign_pad(&mut self, f: usize, p: usize) {
		if f < NP && p < NP {
			pad_assign(&mut self.pad, f, p);
			self.dirty = true;
		}
	}
}

/// The one live state: a mutex in the game; in tests a thread-local (each test runs on a thread of its own, so a test that remaps
/// never disturbs another one reading the defaults).
#[cfg(not(test))]
fn live<R>(f: impl FnOnce(&mut Live) -> R) -> R {
	static L: Mutex<Live> = Mutex::new(Live::START);
	f(&mut L.lock().unwrap_or_else(|e| e.into_inner()))
}
#[cfg(test)]
fn live<R>(f: impl FnOnce(&mut Live) -> R) -> R {
	thread_local! { static L: std::cell::RefCell<Live> = const { std::cell::RefCell::new(Live::START) }; }
	L.with(|l| f(&mut l.borrow_mut()))
}

/// The keyboard bindings now (the defaults until `init`, and always with the extension off).
pub fn kb() -> KbBinds {
	live(|l| l.kb)
}

pub fn pad_map() -> [u8; NP] {
	live(|l| l.pad)
}

/// pcext::init: the options file's `keys` / `pad` values.
pub fn init(keys: Option<&str>, pad: Option<&str>) {
	live(|l| {
		l.kb = keys.map_or_else(KbBinds::default, KbBinds::from_text);
		l.pad = pad.map_or(DEFAULT_PAD, pad_from_text);
		l.dirty = false;
	});
}

/// The menu bound `k` to keyboard function `f`.
pub fn assign_key(f: usize, k: K) {
	live(|l| l.assign_key(f, k));
}

/// The menu bound physical button `p` to controller function `f`.
pub fn assign_pad(f: usize, p: usize) {
	live(|l| l.assign_pad(f, p));
}

pub fn reset_keys() {
	live(|l| {
		l.kb = KbBinds::default();
		l.dirty = true;
	});
}

pub fn reset_pad() {
	live(|l| {
		l.pad = DEFAULT_PAD;
		l.dirty = true;
	});
}

/// Worker: Some((`keys` value, `pad` value)) once after a change (a None value = the defaults: the line goes).
pub fn take_changes() -> Option<(Option<String>, Option<String>)> {
	live(|l| std::mem::take(&mut l.dirty).then(|| (l.kb.to_text(), pad_to_text(&l.pad))))
}

/// Worker: the player pressed Confirm on a row: wait for the next key or button.
pub fn capture_begin(c: Cap) {
	live(|l| {
		l.cap = c;
		l.fields = 0;
		l.ended = false;
	});
}

/// Worker: stops a capture (the page was left, the section changed).
pub fn capture_cancel() {
	live(|l| {
		if l.cap != Cap::Idle {
			l.cap = Cap::Idle;
			l.ended = true;
		}
	});
}

/// Window thread: a capture is waiting, so the game gets a neutral pad this field.
pub fn capturing() -> bool {
	live(|l| l.cap != Cap::Idle)
}

/// Worker: true once when the capture ended (the row's byte goes back to 0).
pub fn capture_ended() -> bool {
	live(|l| std::mem::take(&mut l.ended))
}

/// Window thread, every field while `capturing`: `keys` are the keys pressed fresh this field, `phys` the controller buttons
/// (`PHYS` masks) that went down this field. Binds the first, or ends on Esc / the time limit.
pub fn capture_poll(keys: &[K], phys: u32) {
	live(|l| {
		if l.cap == Cap::Idle {
			return;
		}
		l.fields += 1;
		let done = if keys.contains(&K::Escape) || l.fields > CAP_FIELDS {
			true
		} else {
			match l.cap {
				Cap::Key(f) => keys.iter().copied().find(|&k| bindable(k)).map(|k| l.assign_key(f, k)).is_some(),
				Cap::Pad(f) => (0..NP).find(|&p| phys & PHYS[p].1 != 0).map(|p| l.assign_pad(f, p)).is_some(),
				Cap::Idle => false,
			}
		};
		if done {
			l.cap = Cap::Idle;
			l.ended = true;
		}
	});
}

// ---- prompts ---------------------------------------------------------------------------------------------------------

/// The key name a prompt shows for the keyboard function `f` (a button glyph): None while `f` has today's keys (the caller's own
/// name stands: "Space", "LMB" for Circle, ...), else its first key.
pub fn prompt_name(f: usize) -> Option<String> {
	let b = kb();
	(b.0[f] != DEFAULT_KEYS[f]).then(|| b.shown(f))
}

/// The name of a stick glyph (the four directions of the left stick `LSTICK` or the right one `RSTICK`) when remapped: the four
/// keys joined ("I/J/K/L", in the order up, left, down, right of the default's "W/A/S/D").
pub fn stick_name(first: usize) -> Option<String> {
	let b = kb();
	(b.0[first..first + 4] != DEFAULT_KEYS[first..first + 4]).then(|| [0, 2, 1, 3].iter().map(|&i| b.shown(first + i)).collect::<Vec<_>>().join("/"))
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The keyboard mapping this port had before the menu: `retail::pad_from_keys` as it was.
	fn legacy(held: impl Fn(K) -> bool, circle: bool, triangle: bool) -> (u16, [u8; 4]) {
		let map = [
			(K::Space, 0x40u16),
			(K::ControlLeft, 8),
			(K::KeyC, 4),
			(K::KeyV, 1),
			(K::KeyB, 2),
			(K::Escape, 0x800),
			(K::Enter, 0x800),
			(K::NumpadEnter, 0x800),
			(K::KeyM, 0x100),
			(K::ShiftLeft, 0x200),
			(K::KeyR, 0x400),
			(K::ArrowUp, 0x1000),
			(K::ArrowRight, 0x2000),
			(K::ArrowDown, 0x4000),
			(K::ArrowLeft, 0x8000),
		];
		let mut b = map.iter().filter(|(k, _)| held(*k)).fold(0, |b, (_, m)| b | m);
		if held(K::KeyF) {
			b |= 0x80;
		}
		if circle || held(K::KeyG) {
			b |= 0x20;
		}
		if triangle || held(K::KeyT) {
			b |= 0x10;
		}
		let axis = |neg: K, pos: K| (0x80 + (held(pos) as i32 - held(neg) as i32) * 0x7f).clamp(0, 0xff) as u8;
		(b, [axis(K::KeyE, K::KeyQ), axis(K::KeyX, K::KeyZ), axis(K::KeyA, K::KeyD), axis(K::KeyW, K::KeyS)])
	}

	/// The legacy XInput map of `prompts::map_xinput` (buttons only).
	fn legacy_xinput(buttons: u16, lt: bool, rt: bool) -> u16 {
		const MAP: [(u16, u16); 14] = [(0x1000, 0x40), (0x2000, 0x20), (0x4000, 0x80), (0x8000, 0x10), (0x100, 4), (0x200, 8), (0x10, 0x800), (0x20, 0x100), (0x40, 0x200), (0x80, 0x400), (1, 0x1000), (8, 0x2000), (2, 0x4000), (4, 0x8000)];
		let mut b = MAP.iter().filter(|(x, _)| buttons & x != 0).fold(0u16, |b, (_, m)| b | m);
		if lt {
			b |= 1;
		}
		if rt {
			b |= 2;
		}
		b
	}

	#[test]
	fn default_keyboard_is_todays_mapping() {
		let b = KbBinds::default();
		// Every listed key alone, every pair of the keys the old map knew, with and without the mouse buttons.
		let all: Vec<K> = KEYS.iter().map(|e| e.0).collect();
		for &k in &all {
			for (c, t) in [(false, false), (true, false), (false, true), (true, true)] {
				assert_eq!(b.pad(|x| x == k, c, t), legacy(|x| x == k, c, t), "{k:?}");
			}
		}
		let known: Vec<K> = DEFAULT_KEYS.iter().flatten().flatten().copied().collect();
		for &a in &known {
			for &z in &known {
				assert_eq!(b.pad(|x| x == a || x == z, false, false), legacy(|x| x == a || x == z, false, false), "{a:?} {z:?}");
			}
		}
		assert_eq!(b.pad(|_| true, true, true), legacy(|_| true, true, true));
		assert_eq!(b.pad(|_| false, false, false), ((0), [0x80; 4]));
	}

	#[test]
	fn default_controller_is_todays_mapping() {
		for buttons in 0..=0xffffu32 {
			for (lt, rt) in [(false, false), (true, false), (false, true), (true, true)] {
				let phys = buttons | (lt as u32) << 16 | (rt as u32) << 17;
				assert_eq!(pad_buttons(phys, &DEFAULT_PAD), legacy_xinput(buttons as u16, lt, rt), "{buttons:#x} {lt} {rt}");
			}
		}
	}

	#[test]
	fn defaults_are_distinct_and_listed() {
		let keys: Vec<K> = DEFAULT_KEYS.iter().flatten().flatten().copied().collect();
		for (i, k) in keys.iter().enumerate() {
			assert!(!keys[..i].contains(k), "{k:?} twice");
			assert!(key_file_name(*k).is_some(), "{k:?} not in KEYS");
		}
		let mut seen = DEFAULT_PAD.to_vec();
		seen.sort();
		assert_eq!(seen, (0..NP as u8).collect::<Vec<_>>(), "the controller default is a permutation");
		assert_eq!(KbBinds::default().to_text(), None);
		assert_eq!(pad_to_text(&DEFAULT_PAD), None);
		for e in KEYS {
			assert_eq!(key_from_name(e.1), Some(e.0));
		}
	}

	#[test]
	fn jump_on_k() {
		let mut b = KbBinds::default();
		b.assign(CROSS, [Some(K::KeyK), None, None]);
		assert_eq!(b.pad(|x| x == K::KeyK, false, false).0, 0x40, "K jumps");
		assert_eq!(b.pad(|x| x == K::Space, false, false).0, 0, "Space no longer does");
		assert_eq!(b.to_text().as_deref(), Some("cross:KeyK"));
		assert_eq!(KbBinds::from_text("cross:KeyK"), b);
		assert_eq!(b.shown(CROSS), "K");
	}

	#[test]
	fn a_taken_key_is_shared_and_nothing_else_moves() {
		// F (Square's key) onto Cross: Cross and Square both have F; Square keeps F, nothing else changed.
		let mut b = KbBinds::default();
		b.assign(CROSS, [Some(K::KeyF), None, None]);
		assert_eq!(b.0[CROSS], k1(K::KeyF));
		assert_eq!(b.0[SQUARE], k1(K::KeyF));
		for f in (0..NK).filter(|&f| f != CROSS) {
			assert_eq!(b.0[f], DEFAULT_KEYS[f], "function {f} untouched");
		}
		assert_eq!(b.pad(|x| x == K::KeyF, false, false).0, 0x40 | 0x80, "one key, both functions");
		assert_eq!(b.pad(|x| x == K::Space, false, false).0, 0, "Space drives nothing now");
		assert_eq!(b.shared(CROSS), vec![SQUARE]);
		assert_eq!(b.shared(SQUARE), vec![CROSS]);
		assert!(b.shared(CIRCLE).is_empty());
		assert_eq!(KbBinds::from_text(&b.to_text().unwrap()), b);
		// Enter (one of Start's three keys) onto Cross: Start keeps all three, Cross has Enter too.
		let mut b = KbBinds::default();
		b.assign(CROSS, [Some(K::Enter), None, None]);
		assert_eq!(b.0[CROSS], k1(K::Enter));
		assert_eq!(b.0[10], DEFAULT_KEYS[10]);
		assert_eq!(b.pad(|x| x == K::Enter, false, false).0, 0x40 | 0x800);
		// Start onto the left stick's W: Forward is unchanged, Start has W only (the other keys left it).
		let mut b = KbBinds::default();
		b.assign(10, [Some(K::KeyW), None, None]);
		assert_eq!(b.0[LSTICK], k1(K::KeyW));
		assert_eq!(b.0[10], k1(K::KeyW));
		// A chain of binds changes only the function bound each time; every function stays bound.
		let mut b = KbBinds::default();
		let ks: Vec<K> = KEYS.iter().map(|e| e.0).filter(|&k| bindable(k)).collect();
		for i in 0..400usize {
			let (f, k) = ((i * 7) % NK, ks[(i * 13) % ks.len()]);
			let before = b;
			b.assign(f, [Some(k), None, None]);
			assert_eq!(b.0[f], k1(k));
			for g in (0..NK).filter(|&g| g != f) {
				assert_eq!(b.0[g], before.0[g], "function {g} moved by a bind of {f} at {i}");
			}
			assert!((0..NK).all(|f| b.0[f][0].is_some()), "unbound after {i}");
		}
	}

	#[test]
	fn controller_buttons_may_be_shared_and_round_trip() {
		let mut m = DEFAULT_PAD;
		pad_assign(&mut m, CROSS, 1); // B onto Cross: Circle keeps B
		assert_eq!(m[CROSS], 1);
		assert_eq!(m[CIRCLE], 1);
		for f in (0..NP).filter(|&f| f != CROSS) {
			assert_eq!(m[f], DEFAULT_PAD[f], "function {f} untouched");
		}
		assert_eq!(pad_buttons(0x2000, &m), 0x40 | 0x20, "B is Cross and Circle");
		assert_eq!(pad_buttons(0x1000, &m), 0, "A drives nothing now");
		assert_eq!(pad_shared(&m, CROSS), vec![CIRCLE]);
		assert_eq!(pad_shared(&m, CIRCLE), vec![CROSS]);
		assert!(pad_shared(&m, SQUARE).is_empty());
		assert_eq!(pad_to_text(&m).as_deref(), Some("cross:b"));
		assert_eq!(pad_from_text("cross:b"), m);
		// Fire (Square) onto the right trigger, which is also the strafe aim (R2): both bits on one trigger.
		let mut m = DEFAULT_PAD;
		pad_assign(&mut m, SQUARE, 15);
		assert_eq!(pad_buttons(1 << 17, &m), 0x80 | 2);
		assert_eq!(pad_buttons(0x4000, &m), 0, "X drives nothing now");
		assert_eq!(pad_from_text(&pad_to_text(&m).unwrap()), m);
		// The order of the entries of a file does not matter.
		assert_eq!(pad_from_text("circle:a,cross:b"), pad_from_text("cross:b,circle:a"));
		// A chain of binds changes only the function bound each time.
		let mut m = DEFAULT_PAD;
		for i in 0..200usize {
			let (f, p) = ((i * 5) % NP, (i * 11) % NP);
			let before = m;
			pad_assign(&mut m, f, p);
			assert_eq!(m[f] as usize, p);
			for g in (0..NP).filter(|&g| g != f) {
				assert_eq!(m[g], before[g], "function {g} moved by a bind of {f} at {i}");
			}
		}
	}

	#[test]
	fn unreadable_text_is_skipped() {
		assert_eq!(KbBinds::from_text("garbage,cross:,cross:Nope,:KeyK,nofunc:KeyK,"), KbBinds::default());
		assert_eq!(pad_from_text("cross:zz,x:a,,"), DEFAULT_PAD);
		let b = KbBinds::from_text("Cross:keyk, forward:ArrowUp");
		assert_eq!(b.0[CROSS], k1(K::KeyK));
		assert_eq!(b.0[LSTICK], k1(K::ArrowUp));
		assert_eq!(b.0[DPAD], k1(K::ArrowUp), "the d-pad up keeps the Up arrow (shared with Forward)");
	}

	#[test]
	fn stick_keys_move_the_sticks() {
		let mut b = KbBinds::default();
		b.assign(LSTICK, [Some(K::KeyI), None, None]); // forward on I
		let (_, s) = b.pad(|x| x == K::KeyI, false, false);
		assert_eq!(s, [0x80, 0x80, 0x80, 0x01]);
		let (_, s) = b.pad(|x| x == K::KeyI || x == K::KeyS, false, false);
		assert_eq!(s[3], 0x80, "forward + back cancel");
	}

	#[test]
	fn prompt_names_follow_the_binding() {
		assert_eq!(prompt_name(CROSS), None);
		assert_eq!(stick_name(LSTICK), None);
		let mut b = KbBinds::default();
		b.assign(CROSS, [Some(K::KeyK), None, None]);
		b.assign(LSTICK, [Some(K::KeyI), None, None]);
		assert_eq!(b.shown(CROSS), "K");
		assert_eq!(b.shown(10), "Esc/Enter");
	}
}
