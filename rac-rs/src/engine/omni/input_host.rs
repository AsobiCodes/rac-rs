//! The C callbacks of the OmniLoader host for input: `key`, `pressed`, `mouse`, `window`, `pointer`, `take_input`, `capture_mouse`.
//! They answer from the snapshot of the field the worker runs (input.rs `current`); the loader calls them from a mod's callbacks
//! (tick, pad, hooks) on the worker thread, or from the main script at start-up (then the snapshot is neutral).

use super::cabi::Host;
use super::host::text;
use super::input;
use std::ffi::{c_char, c_void};

unsafe extern "C" fn key_cb(_user: *mut c_void, bind: *const c_char) -> i32 {
	i32::from(input::current().key(&text(bind)))
}

unsafe extern "C" fn pressed_cb(_user: *mut c_void, bind: *const c_char) -> i32 {
	i32::from(input::current().pressed(&text(bind)))
}

unsafe extern "C" fn mouse_cb(_user: *mut c_void, out: *mut f64) {
	let v = input::current().mouse();
	// SAFETY: `out` has the four doubles the DLL gave.
	unsafe { std::ptr::copy_nonoverlapping(v.as_ptr(), out, v.len()) };
}

unsafe extern "C" fn window_cb(_user: *mut c_void, out: *mut i32) {
	let v = input::current().window();
	// SAFETY: `out` has the three ints the DLL gave.
	unsafe { std::ptr::copy_nonoverlapping(v.as_ptr(), out, v.len()) };
}

unsafe extern "C" fn pointer_cb(_user: *mut c_void, out_xy: *mut f64, inside: *mut i32) {
	let s = input::current();
	// SAFETY: two doubles and one int, as the DLL gave.
	unsafe {
		*out_xy = s.pointer[0] as f64;
		*out_xy.add(1) = s.pointer[1] as f64;
		*inside = s.inside as i32;
	}
}

unsafe extern "C" fn take_input_cb(_user: *mut c_void, mask: u32) {
	input::take_input(mask);
}

unsafe extern "C" fn capture_mouse_cb(_user: *mut c_void, on: i32) {
	input::capture_mouse(on != 0);
}

/// Sets the seven input entries of the `OmniHost`.
pub fn fill(h: &mut Host) {
	h.key = Some(key_cb);
	h.pressed = Some(pressed_cb);
	h.mouse = Some(mouse_cb);
	h.window = Some(window_cb);
	h.pointer = Some(pointer_cb);
	h.take_input = Some(take_input_cb);
	h.capture_mouse = Some(capture_mouse_cb);
}
