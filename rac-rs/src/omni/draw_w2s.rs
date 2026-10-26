//! ABI 1.2 `world_to_screen`:a world point to the game's 512 x 448 2D space with the camera the renderer draws.
//!
//! The camera is read from the game's RAM on the worker thread (the machine in scope while a mod's callback runs): eye, basis rows and the tan
//! scalar the level overlay publishes (retail/camera.rs `view_in_ram`; with the modern camera on, rac-rs's hand writes the same words, so the mod
//! sees the picture's camera). The projection is the renderer's: vertical tangent = scalar * PROJ_Y_RATIO, horizontal = that times the window
//! aspect (the 3D view widens with the window; the 2D overlay does not). The window pixel is then put back into game space with the overlay's
//! layout (draw_map.rs), so drawing at the result lands on the point in every mode. It runs on the camera of the last tick, the picture shows the
//! camera between two ticks: at most one tick (20 ms) of lag on a moving camera.
//!
//! No 3D view (title, loading, a movie, before the first rendered frame): returns 0.

use super::cabi::Host;
use super::draw_map::Layout;
use super::host::cpu;
use crate::retail::disc::va;
use ee_oracle::cpu::Cpu;
use super::native_host::{level_word, loaded_level_of};
use std::ffi::c_void;

/// Nearest depth (m) that counts as in front of the camera.
const NEAR: f32 = 0.05;

/// The game mode word (PAL address; read through `disc::va`, so the USA disc works too) and its menu value (retail/present.rs `menu_presents`).
const MODE_WORD: usize = 0x15_F5A4;
const MENU_MODE: u32 = 3;

/// A point through a camera: (x, y in pixels of a `sw` x `sh` window, in front of the camera). The basis is `right = forward x up`.
pub fn project(eye: [f32; 3], fwd: [f32; 3], up: [f32; 3], tan_v: f32, sw: f32, sh: f32, p: [f32; 3]) -> ([f32; 2], bool) {
	let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
	let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
	let r = cross(fwd, up);
	let rl = dot(r, r).sqrt().max(1e-9);
	let r = [r[0] / rl, r[1] / rl, r[2] / rl];
	let u = cross(r, fwd);
	let d = [p[0] - eye[0], p[1] - eye[1], p[2] - eye[2]];
	let depth = dot(d, fwd);
	let front = depth > NEAR;
	let z = depth.max(NEAR);
	let (tan_h, tan_v) = (tan_v * sw / sh, tan_v);
	let (nx, ny) = (dot(d, r) / (z * tan_h), dot(d, u) / (z * tan_v));
	([(nx + 1.0) * 0.5 * sw, (1.0 - ny) * 0.5 * sh], front)
}

/// Game-space result of `project` for the layout: (x, y, in front).
pub fn to_game(layout: &Layout, win: ([f32; 2], bool)) -> ([f32; 2], bool) {
	(layout.to_game(win.0[0], win.0[1]), win.1)
}

/// `p` in the game's 2D space with the camera of the machine `c` and the last rendered window; None when there is no 3D view.
pub fn screen_of(c: &Cpu, p: [f32; 3]) -> Option<([f32; 2], bool)> {
	let level = loaded_level_of(level_word(c));
	if level < 0 || !p.iter().all(|v| v.is_finite()) {
		return None;
	}
	// Game mode 3 = pause menu / galactic map / post-game page: the world is a frozen, blurred picture drawn with another view, so no 3D view.
	let mode_at = va(MODE_WORD);
	let mode = c.mem.ram.get(mode_at..mode_at + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
	if mode == MENU_MODE {
		return None;
	}
	let (sw, sh, layout) = super::draw::view()?;
	let (eye, fwd, up, scalar) = crate::retail::camera::view_in_ram(&c.mem.ram, level as u32)?;
	let tan_v = scalar * crate::viewer::PROJ_Y_RATIO;
	Some(to_game(&layout, project(eye, fwd, up, tan_v, sw, sh, p)))
}

unsafe extern "C" fn world_to_screen_cb(_user: *mut c_void, xyz: *const f32, out_xy: *mut f32, in_front: *mut i32) -> i32 {
	if xyz.is_null() || out_xy.is_null() || in_front.is_null() {
		return 0;
	}
	let Some(c) = cpu() else { return 0 };
	// SAFETY: the DLL gives three floats at `xyz`.
	let p = unsafe { [*xyz, *xyz.add(1), *xyz.add(2)] };
	let Some((g, front)) = screen_of(c, p) else { return 0 };
	// SAFETY: two floats and one int, as the DLL gave.
	unsafe {
		*out_xy = g[0];
		*out_xy.add(1) = g[1];
		*in_front = front as i32;
	}
	1
}

/// `world_to_screen` and the icon list entries (the icon table is draw_art.rs).
pub fn fill(h: &mut Host) {
	h.world_to_screen = Some(world_to_screen_cb);
	h.icon_count = Some(super::draw_art::icon_count_cb);
	h.icon_name = Some(super::draw_art::icon_name_cb);
}

#[cfg(test)]
mod tests {
	use super::*;

	// Eye at the origin looking along +x with z up: right is -y, up is +z.
	const EYE: [f32; 3] = [0.0, 0.0, 0.0];
	const FWD: [f32; 3] = [1.0, 0.0, 0.0];
	const UP: [f32; 3] = [0.0, 0.0, 1.0];

	fn close(a: f32, b: f32) -> bool {
		(a - b).abs() < 0.01
	}

	#[test]
	fn straight_ahead_is_the_window_centre() {
		let (p, front) = project(EYE, FWD, UP, 0.75, 1280.0, 720.0, [10.0, 0.0, 0.0]);
		assert!(front && close(p[0], 640.0) && close(p[1], 360.0), "{p:?}");
	}

	#[test]
	fn right_and_up_move_the_right_way() {
		// tan_v 0.5 on a 4:3 window: horizontal tangent 0.6667. A point 10 ahead and 3.3333 to the right (-y) is at ndc x 0.5.
		let (p, _) = project(EYE, FWD, UP, 0.5, 800.0, 600.0, [10.0, -10.0 * 0.5 * (4.0 / 3.0) * 0.5, 0.0]);
		assert!(close(p[0], 600.0), "{p:?}");
		// 10 ahead, 2.5 up = ndc y 0.5: a quarter of the height above the middle.
		let (p, _) = project(EYE, FWD, UP, 0.5, 800.0, 600.0, [10.0, 0.0, 2.5]);
		assert!(close(p[1], 150.0), "{p:?}");
	}

	#[test]
	fn wider_windows_see_more_at_the_sides() {
		// The same point, 10 ahead and 5 to the right: nearer the centre on a 21:9 window than on 4:3 (horizontal FOV grows with the aspect).
		let (a, _) = project(EYE, FWD, UP, 0.5, 800.0, 600.0, [10.0, -5.0, 0.0]);
		let (b, _) = project(EYE, FWD, UP, 0.5, 2520.0, 1080.0, [10.0, -5.0, 0.0]);
		assert!((a[0] - 400.0) / 400.0 > (b[0] - 1260.0) / 1260.0);
	}

	#[test]
	fn behind_the_camera_is_not_in_front() {
		let (_, front) = project(EYE, FWD, UP, 0.75, 1280.0, 720.0, [-5.0, 0.0, 0.0]);
		assert!(!front);
		let (_, front) = project(EYE, FWD, UP, 0.75, 1280.0, 720.0, [0.01, 0.0, 0.0]);
		assert!(!front);
	}

	#[test]
	fn a_pitched_camera_with_roll_basis() {
		// Looking 45 degrees down: a point straight ahead along the view direction is the centre.
		let s = std::f32::consts::FRAC_1_SQRT_2;
		let (p, front) = project([0.0, 0.0, 10.0], [s, 0.0, -s], UP, 0.75, 1000.0, 500.0, [10.0, 0.0, 0.0]);
		assert!(front && close(p[0], 500.0) && close(p[1], 250.0), "{p:?}");
	}

	#[test]
	fn game_space_goes_through_the_overlay_layout() {
		// At 16:9 the picture centre is the window centre: the centre pixel is game (256, 224).
		let l = Layout::plain(1920.0, 1080.0);
		let (g, _) = to_game(&l, project(EYE, FWD, UP, 0.75, 1920.0, 1080.0, [10.0, 0.0, 0.0]));
		assert!(close(g[0], 256.0) && close(g[1], 224.0), "{g:?}");
		// A point at the right edge of the 3D view is beyond game x 512 on a wide window (the 3D view is wider than the 4:3 picture).
		let (g, _) = to_game(&l, project(EYE, FWD, UP, 0.75, 1920.0, 1080.0, [10.0, -10.0 * 0.75 * (16.0 / 9.0), 0.0]));
		assert!(g[0] > 512.0, "{g:?}");
	}

	#[test]
	fn nothing_to_project_without_a_machine_or_a_view() {
		// No machine in scope: 0, and the output stays untouched.
		let (xyz, mut out, mut front) = ([1.0f32, 2.0, 3.0], [-1.0f32; 2], -1i32);
		// SAFETY: valid pointers.
		let r = unsafe { world_to_screen_cb(std::ptr::null_mut(), xyz.as_ptr(), out.as_mut_ptr(), &mut front) };
		assert_eq!(r, 0);
		assert_eq!(out, [-1.0, -1.0]);
	}
}
