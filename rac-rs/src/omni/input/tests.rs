//! Unit tests of the mods' input (omni/input.rs): the key table against keys.h, the per-field snapshot, `take_input` masking and the
//! release of `capture_mouse`. Everything but `globals_follow_the_load` works on local values, so the tests do not race.

use super::*;
use winit::event::MouseButton;
use winit::keyboard::KeyCode as K;

#[test]
fn key_table_is_keys_h() {
	// keys.h: 107 keyboard names (106 with a winit key, IntlHash without), usage = USB HID usage, unique.
	assert_eq!(KEYS.len(), 106);
	let mut names: Vec<&str> = KEYS.iter().map(|k| k.1).collect();
	names.sort_unstable();
	names.dedup();
	assert_eq!(names.len(), 106);
	let mut usages: Vec<u8> = KEYS.iter().map(|k| k.2).collect();
	usages.sort_unstable();
	usages.dedup();
	assert_eq!(usages.len(), 106);
	// The bind name is the winit variant's name, except the Meta keys (winit: Super).
	for (code, name, _) in KEYS {
		let winit_name = format!("{code:?}");
		let want = name.replace("Meta", "Super");
		assert_eq!(winit_name, want, "bind {name}");
	}
	// Spot checks against the HID usage table (keys.h).
	for (code, want) in [
		(K::KeyA, 4),
		(K::KeyW, 26),
		(K::KeyZ, 29),
		(K::Digit1, 30),
		(K::Digit0, 39),
		(K::Enter, 40),
		(K::Escape, 41),
		(K::Backspace, 42),
		(K::Tab, 43),
		(K::Space, 44),
		(K::F1, 58),
		(K::F12, 69),
		(K::ArrowRight, 79),
		(K::ArrowUp, 82),
		(K::Numpad0, 98),
		(K::NumpadEqual, 103),
		(K::ControlLeft, 224),
		(K::ShiftLeft, 225),
		(K::SuperLeft, 227),
		(K::SuperRight, 231),
	] {
		assert_eq!(usage(code), Some(want), "{code:?}");
	}
	assert_eq!(usage(K::F13), None);
}

#[test]
fn bind_names_reach_the_snapshot() {
	let mut s = InputSnap::NEUTRAL;
	// KeyW (usage 26) held, KeyA (4) pressed only, IntlHash (50, no winit key) held.
	s.held[0] |= 1 << 26;
	s.pressed[0] |= 1 << 4;
	s.held[0] |= 1 << 50;
	s.held[3] |= 1 << (231 - 192);
	assert!(s.key("KeyW") && !s.pressed("KeyW"));
	assert!(s.pressed("KeyA") && !s.key("KeyA"));
	assert!(s.key("IntlHash") && s.key("MetaRight"));
	assert!(!s.key("KeyS") && !s.key("NotAKey") && !s.pressed(""));
	s.mouse_held = MOUSE_RIGHT | MOUSE_FORWARD;
	s.mouse_pressed = MOUSE_LEFT;
	assert!(s.key("MouseRight") && s.key("MouseForward") && !s.key("MouseLeft") && !s.key("MouseMiddle") && !s.key("MouseBack"));
	assert!(s.pressed("MouseLeft") && !s.pressed("MouseRight"));
	// Wheel: held for the tick it turned in, half a notch or more; up = y > 0, right = x > 0.
	s.wheel_y = 1.0;
	s.wheel_x = -0.4;
	assert!(s.key("WheelUp") && s.pressed("WheelUp") && !s.key("WheelDown") && !s.key("WheelLeft") && !s.key("WheelRight"));
	s.wheel_x = -0.5;
	assert!(s.key("WheelLeft"));
	assert_eq!(s.mouse(), [0.0, 0.0, -0.5, 1.0]);
	assert_eq!(s.window(), [0, 0, 0]);
	// The mouse bits are keys.h's: bit (bind - 0x200) of Left, Middle, Right, Back, Forward.
	assert_eq!([MOUSE_LEFT, MOUSE_MIDDLE, MOUSE_RIGHT, MOUSE_BACK, MOUSE_FORWARD], [1, 2, 4, 8, 16]);
	assert_eq!(mouse_bit(MouseButton::Right), Some(MOUSE_RIGHT));
	assert_eq!(mouse_bit(MouseButton::Back), Some(MOUSE_BACK));
	assert_eq!(mouse_bit(MouseButton::Other(7)), None);
}

#[test]
fn neutral_is_the_default_and_the_replay_input() {
	assert_eq!(InputSnap::default(), InputSnap::NEUTRAL);
	let n = crate::retail::FieldIn::default().omni;
	assert!(!n.key("KeyW") && !n.pressed("MouseLeft") && n.window()[0] == 0 && !n.inside);
}

#[test]
fn pressed_means_down_this_field_not_the_last() {
	let mut l = Live::NEW;
	let id = |x: f32, y: f32| [x, y];
	// A tap shorter than a field: pressed, never held.
	l.key(26, true);
	l.key(26, false);
	let s = l.take(false, id);
	assert!(s.pressed("KeyW") && !s.key("KeyW"));
	assert!(!l.take(false, id).pressed("KeyW"));
	// A key held over two fields: pressed in the first only, held in both. An auto-repeat down while held is no new press.
	l.key(26, true);
	let a = l.take(false, id);
	l.key(26, true);
	let b = l.take(false, id);
	assert!(a.pressed("KeyW") && a.key("KeyW") && !b.pressed("KeyW") && b.key("KeyW"));
	// Released: not held, not pressed.
	l.key(26, false);
	let c = l.take(false, id);
	assert!(!c.key("KeyW") && !c.pressed("KeyW"));
	// Mouse buttons the same way, motion and wheel restart each field.
	l.button(MOUSE_LEFT, true);
	l.button(MOUSE_LEFT, false);
	l.wheel(0.0, 1.0);
	l.wheel(0.0, 1.0);
	l.dx = 3.0;
	l.dy = -2.0;
	let m = l.take(false, id);
	assert!(m.pressed("MouseLeft") && !m.key("MouseLeft"));
	assert_eq!(m.mouse(), [3.0, -2.0, 0.0, 2.0]);
	assert_eq!(l.take(false, id).mouse(), [0.0; 4]);
}

#[test]
fn losing_the_focus_lets_go_of_everything() {
	let mut l = Live::NEW;
	l.key(26, true);
	l.button(MOUSE_RIGHT, true);
	l.cursor = Some([0.5, 0.5]);
	l.focus(false);
	let s = l.take(false, |x, y| [x, y]);
	assert!(!s.key("KeyW") && !s.key("MouseRight") && s.window()[0] == 0);
	assert!(!s.inside, "an unfocused window has no pointer");
	// The key goes up while the window is away: no stuck key when it comes back.
	l.key(26, false);
	l.focus(true);
	assert!(!l.take(false, |x, y| [x, y]).key("KeyW"));
}

#[test]
fn pointer_is_over_the_picture_or_not() {
	let mut l = Live::NEW;
	// No cursor in the window: nothing.
	let s = l.take(false, |x, y| [x, y]);
	assert!(!s.inside);
	// The identity map: window = picture. Corners are inside, just outside is not (the coordinates are still given).
	for (c, inside) in [([0.0, 0.0], true), ([1.0, 1.0], true), ([0.5, 0.25], true), ([1.0, 0.5], true)] {
		l.cursor = Some(c);
		let s = l.take(false, |x, y| [x, y]);
		assert_eq!((s.pointer, s.inside), (c, inside));
	}
	// A wide window: the 4:3 picture is the middle; the map says the cursor at the window's left edge is left of it.
	l.cursor = Some([0.0, 0.5]);
	let s = l.take(true, |x, y| [(x - 0.5) * 1.5 + 0.5, y]);
	assert_eq!((s.pointer, s.inside, s.captured), ([-0.25, 0.5], false, true));
	l.cursor = Some([0.5, 1.5]);
	assert!(!l.take(false, |x, y| [x, y]).inside);
}

#[test]
fn take_input_masks_the_pad_keys() {
	use crate::retail::pad_from_keys;
	let neutral = pad_from_keys(|_| false, false, false);
	let w = |k: K| k == K::KeyW || k == K::Space;
	assert_ne!(pad_from_keys(gated(false, w), false, false), neutral, "WASD drives the pad when no mod took the keyboard");
	assert_eq!(pad_from_keys(gated(true, w), false, false), neutral, "a mod holds the keyboard: the same keys press nothing");
	// The mouse buttons are not the keyboard: Circle (left button) still reaches the pad.
	assert_ne!(pad_from_keys(gated(true, w), true, false), neutral);
}

#[test]
fn holds_follow_the_mods_that_asked() {
	let mut h = Holds::NEW;
	h.loading = true;
	// Mod a takes the keyboard and loads; mod b takes the mouse and fails while loading: only a's hold stays.
	h.take_input(TAKE_KEYBOARD);
	h.on_log(0, "a", "loaded 1.0.0");
	h.take_input(TAKE_MOUSE);
	h.on_log(2, "b", "failed: script error");
	h.end_load();
	assert_eq!(h.taken(), TAKE_KEYBOARD);
	// Mod c takes both and loads; later a fails at run time: c keeps both, the keyboard stays with c.
	h.loading = true;
	h.take_input(TAKE_KEYBOARD | TAKE_MOUSE);
	h.on_log(0, "c", "loaded 2.1 (native)");
	h.end_load();
	assert_eq!(h.taken(), TAKE_KEYBOARD | TAKE_MOUSE);
	h.on_log(2, "a", "failed: budget exceeded");
	assert_eq!(h.taken(), TAKE_KEYBOARD | TAKE_MOUSE);
	h.on_log(2, "c", "failed: panic");
	assert_eq!(h.taken(), 0, "a failed mod gives the keyboard and the mouse back");
	// Info lines that are not `loaded`, errors that are not `failed:` and lines without a mod id change nothing.
	h.on_log(2, "a", "some other error");
	h.on_log(0, "a", "hello");
	h.on_log(2, "", "failed: x");
	assert_eq!((h.taken(), h.cap_on), (0, false));
}

#[test]
fn capture_is_released_when_the_asker_is_gone() {
	let mut h = Holds::NEW;
	h.loading = true;
	h.capture(true);
	h.on_log(0, "a", "loaded 1");
	h.end_load();
	assert!(h.cap_on);
	// Another mod failing at load or run time does not take a's capture; a failing does.
	h.on_log(2, "b", "failed: x");
	assert!(h.cap_on);
	h.on_log(2, "a", "failed: x");
	assert!(!h.cap_on);
	// Asked at run time (no owner): the mod can let go itself, or any failing mod lets go of it.
	h.capture(true);
	assert!(h.cap_on);
	h.capture(false);
	assert!(!h.cap_on);
	h.capture(true);
	h.on_log(2, "z", "failed: x");
	assert!(!h.cap_on);
	// The latest asking counts: a's yes, then b's no during load.
	let mut h = Holds::NEW;
	h.loading = true;
	h.capture(true);
	h.on_log(0, "a", "loaded 1");
	h.capture(false);
	h.on_log(0, "b", "loaded 1");
	assert!(!h.cap_on);
	// A load that ends without the loader's closing line keeps what was asked, owner unknown (so any failure frees the capture).
	let mut h = Holds::NEW;
	h.loading = true;
	h.take_input(TAKE_KEYBOARD);
	h.capture(true);
	h.end_load();
	assert_eq!((h.taken(), h.cap_on), (TAKE_KEYBOARD, true));
	h.on_log(2, "q", "failed: x");
	assert!(!h.cap_on);
}

/// The one test that touches the process-wide state: before the load nothing is active; after it the holds and the field snapshot
/// work; a failed load switches everything off again. It ends with the state it began with.
#[test]
fn globals_follow_the_load() {
	assert!(!keyboard_taken() && !mouse_owned() && !capture_wanted());
	assert_eq!(take(|x, y| [x, y]), InputSnap::NEUTRAL);
	set_field(&InputSnap { focused: true, ..InputSnap::NEUTRAL });
	assert!(current().window()[0] == 0, "nothing is published while no mod is loaded");
	begin_load();
	take_input(TAKE_KEYBOARD);
	note_log(0, "m", "loaded 1.0");
	capture_mouse(true);
	end_load(true);
	assert!(keyboard_taken() && capture_wanted() && mouse_owned());
	let mut s = InputSnap::NEUTRAL;
	s.focused = true;
	s.held[0] |= 1 << 26;
	set_field(&s);
	assert!(current().key("KeyW") && current().window() == [1, 0, 0]);
	note_log(2, "m", "failed: gone");
	assert!(!keyboard_taken() && !capture_wanted() && !mouse_owned());
	// A failed load: everything off, the state switches itself off.
	begin_load();
	take_input(TAKE_MOUSE);
	end_load(false);
	assert!(!mouse_owned());
	set_field(&InputSnap::NEUTRAL);
	assert_eq!(take(|x, y| [x, y]), InputSnap::NEUTRAL);
}
