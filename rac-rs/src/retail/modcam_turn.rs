//! The Modern camera's turn from one field's input (lane cam, 2026-10-23; player report: "to have a normal right stick view you
//! have to set the camera to inverted, with the mouse inverted really is inverted"). `modcam.rs` `step` calls `stick_axes` and
//! `turn`; the worker calls `camera_sticks` before the game reads the pad (v0.4.1 lane cam, 2026-10-27).
//!
//! The view convention (modcam `rows`): yaw grows to the LEFT, pitch grows DOWN. The mouse turns the view the way it moves
//! (right = look right = yaw down, up = look up = pitch down; `look` is x right, y down). Retail's own follow camera turns the
//! other way for the stick (it orbits the hero: stick right moves the camera right, the view looks left; stick up looks down),
//! and retail's free look (L1 + L2) is inverted on the vertical axis only. With the Modern camera, Normal means the same for the
//! physical right stick and the mouse (right = look right, up = look up) in EVERY phase, also the ones retail's own camera runs
//! (cutscene hand-back, slide, free look, the blend after them): `camera_sticks` flips the controller's share of the
//! stick before the game reads it, so retail's camera and modcam see the same bytes. Reversed flips the stick, per axis (retail's
//! two option bytes: modcam's `lr` / `ud`, retail's follow camera reads them itself), and with it the keyboard camera keys (Q/E
//! turn, Z/X tilt: they feed the same right-stick bytes, binds.rs). The mouse is not retail's: it always turns the view the way
//! it moves, whatever the option says. The keys' share of the stick travels in `Input::keys` and `camera_sticks` leaves an axis
//! they drive alone. With the Classic camera nothing is flipped (the strafe aim that borrows modcam then turns as retail's
//! follow camera does).

/// Flips one stick byte around the centre 0x80 (0x01 <-> 0xff; 0x00 -> 0xff, the PS2 range has no 0x100).
fn flip(b: u8) -> u8 {
	(256 - b as i32).clamp(1, 255) as u8
}

/// Whether retail's camera runs in this game mode (0x15f5a4), i.e. whether anything reads the right stick pad words 0x13ca40 / 0x13ca44
/// as a camera input. Every such reader is camera code (follow rig, free look, class handlers, the swing bias test) or the idle
/// counter FUN_002840e8 (zero test only), plus the pause map's zoom (boot FUN_00204f10 and the level overlays' copies: mode 3). The
/// camera main FUN_001e6910 is called from the mode 0 controller tail FUN_002841f0 (OV:108941) and from the mode 6 handler
/// FUN_0027c808 (OV:103557) only; modes 1 (load), 2 (scene), 3 (pause / map), 4 (dialog) and 5 (vendor) never reach it. So the
/// flip below is for modes 0 and 6; the pause map keeps the raw stick, like the camera-less v0.4.0 game.
pub fn camera_reads_stick(game_mode: u32) -> bool {
	matches!(game_mode, 0 | 6)
}

/// The right stick bytes [rx, ry] the game gets with the Modern camera on: `merged` = keyboard and controller merged by the larger
/// deflection, `keys` = the keys' share (offset from 0x80, 0 = no key held), `class4` = retail runs its free look (camera class 4),
/// `lr_reversed` = the Left/Right option is Reversed. An axis the keys drive (the merged byte is the keys' byte) is not touched.
/// A controller axis is flipped: vertical always (retail is inverted at Normal in every class); horizontal in the follow-type
/// classes (inverted at Normal), and in free look (natural at Normal, but it ignores the Left/Right option) only when Reversed,
/// so the option works there too.
pub fn camera_sticks(merged: [u8; 2], keys: [i8; 2], class4: bool, lr_reversed: bool) -> [u8; 2] {
	let from_keys = |i: usize| keys[i] != 0 && merged[i] as i16 - 128 == keys[i] as i16;
	let rx = if from_keys(0) || (class4 && !lr_reversed) { merged[0] } else { flip(merged[0]) };
	let ry = if from_keys(1) { merged[1] } else { flip(merged[1]) };
	[rx, ry]
}

/// The right stick as the turn maths takes it, in `[-1, 1]` (PS2 bytes: right and down are high), read per axis as retail's own
/// camera reads it (the pad reader, decomp/notes/camera.md: `d = |byte - 0x7f|`, 0 up to 47, else `(d - 48) / 76` up to 1): a
/// controller turns the view from the game's own dead zone on, smoothly from 0 (the pad bytes are PCSX2's, prompts.rs `stick`),
/// and the keys' bytes 0x01 / 0xff give exactly -1 / 1 as before. The bytes are the ones the game sees: `camera_sticks` has already
/// given the controller the mouse's sign (Modern) or left retail's (Classic strafe aim).
pub fn stick_axes(merged: [u8; 2]) -> [f32; 2] {
	merged.map(|b| {
		let d = (b as i32 - 0x7f).abs();
		let v = if d <= 0x2f { 0.0 } else { ((d - 0x30) as f32 / 0x4c as f32).min(1.0) };
		if b < 0x7f { -v } else { v }
	})
}

/// The per-field gains of the turn: the mouse (counts -> radians, `RAC_MODCAM_SENS`, the Mouse speed row's factor), the stick (yaw
/// rate, pitch step, the Camera speed row's factor) and retail's Reversed options (-1.0 Reversed, 1.0 Normal) per axis, which
/// reverse the stick (and the keys) only.
pub struct Gains {
	pub mouse: f32,
	pub sens: f32,
	pub mouse_k: f32,
	pub rot: f32,
	pub tilt: f32,
	pub stick_k: f32,
	pub lr: f32,
	pub ud: f32,
}

/// The field's turn (yaw, pitch) in radians from the mouse counts `look` and the stick `s` (`stick_axes`, after the deadzone and the
/// Smooth ramp). The mouse stays direct; retail's Reversed options reverse the stick's share only. With the same factor for both
/// and keys only, the numbers are those of the one-factor formula before the change.
pub fn turn(look: [i16; 2], s: [f32; 2], g: &Gains) -> (f32, f32) {
	let dyaw = -(look[0] as f32) * g.mouse * g.sens * g.mouse_k + s[0] * g.rot * g.stick_k * g.lr;
	let dpitch = look[1] as f32 * g.mouse * g.sens * g.mouse_k - s[1] * g.tilt * g.stick_k * g.ud;
	(dyaw, dpitch)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn gains(mouse_k: f32, stick_k: f32, lr: f32, ud: f32) -> Gains {
		Gains { mouse: 0.003, sens: 1.0, mouse_k, rot: 0.03, tilt: 0.02, stick_k, lr, ud }
	}

	/// The formula before the change (one factor `k` for mouse and stick), for the keys' exact numbers.
	fn old_turn(look: [i16; 2], s: [f32; 2], g: &Gains, k: f32) -> (f32, f32) {
		let dyaw = (-(look[0] as f32) * g.mouse * g.sens * k + s[0] * g.rot * k) * g.lr;
		let dpitch = (look[1] as f32 * g.mouse * g.sens * k - s[1] * g.tilt * k) * g.ud;
		(dyaw, dpitch)
	}

	/// What the turn maths gets for a controller stick (no keys) in a follow-type class with the Modern camera on.
	fn pad_axes(merged: [u8; 2]) -> [f32; 2] {
		stick_axes(camera_sticks(merged, [0, 0], false, false))
	}

	#[test]
	fn physical_stick_has_the_mouses_sign_on_both_axes() {
		let g = gains(1.0, 1.0, 1.0, 1.0);
		// Mouse right (x +) and up (y -) at Normal: yaw down = look right, pitch down = look up.
		let (my, mp) = turn([10, -10], [0.0, 0.0], &g);
		assert!(my < 0.0 && mp < 0.0);
		// The controller's right stick pushed right (rx high) and up (ry low): the same signs.
		let (sy, sp) = turn([0, 0], pad_axes([0xff, 0x01]), &g);
		assert!(sy < 0.0 && sp < 0.0, "{sy} {sp}");
		// Left and down: the opposite signs, like the mouse moved left and down.
		let (my2, mp2) = turn([-10, 10], [0.0, 0.0], &g);
		let (sy2, sp2) = turn([0, 0], pad_axes([0x01, 0xff]), &g);
		assert!(my2 > 0.0 && mp2 > 0.0 && sy2 > 0.0 && sp2 > 0.0);
	}

	#[test]
	fn the_game_gets_the_flipped_bytes_with_the_modern_camera_only_on_the_controller_axes() {
		// Follow-type class: both axes flip (stick up 0x00 -> 0xff, right 0xff -> 0x01).
		assert_eq!(camera_sticks([0xff, 0x00], [0, 0], false, false), [0x01, 0xff]);
		assert_eq!(camera_sticks([0x20, 0xe0], [0, 0], false, false), [0xe0, 0x20]);
		assert_eq!(camera_sticks([0x80, 0x80], [0, 0], false, false), [0x80, 0x80], "centre stays");
		// The left/right option does not matter for the follow-type classes (retail's follow camera reads it itself).
		assert_eq!(camera_sticks([0xff, 0x00], [0, 0], false, true), [0x01, 0xff]);
		// The PS2 range has no 0x100: full travel left / up maps to 0xff, and the flip is its own inverse otherwise.
		assert_eq!(flip(0x00), 0xff);
		assert_eq!(flip(0x80), 0x80);
		for b in 1..=255u8 {
			assert_eq!(flip(flip(b)), b);
		}
	}

	#[test]
	fn only_the_modes_that_run_the_camera_main_flip_the_stick() {
		// Mode 0 (gameplay) and 6 (takeoff / landing) run camera main; 3 (pause, the map zooms on the right stick Y) never does.
		assert!(camera_reads_stick(0) && camera_reads_stick(6));
		for mode in [1, 2, 3, 4, 5, 7] {
			assert!(!camera_reads_stick(mode), "mode {mode}");
		}
	}

	#[test]
	fn free_look_is_flipped_vertically_and_horizontally_only_when_reversed() {
		// Class 4 at Normal: retail's free look turns right on stick right already, so rx stays; ry flips (retail is inverted).
		assert_eq!(camera_sticks([0xff, 0x00], [0, 0], true, false), [0xff, 0xff]);
		// Class 4 with Left/Right Reversed: retail's free look ignores the option, so rx flips to honour it.
		assert_eq!(camera_sticks([0xff, 0x00], [0, 0], true, true), [0x01, 0xff]);
	}

	/// Reversed flips the stick (and the keyboard keys, which are the stick) per axis; the mouse always turns the view the way
	/// it moves, whatever the option says (the option is retail's camera one, and the mouse is not retail's).
	#[test]
	fn reversed_flips_the_stick_and_leaves_the_mouse_alone() {
		for (lr, ud) in [(-1.0f32, 1.0f32), (1.0, -1.0), (-1.0, -1.0)] {
			let (n, r) = (gains(1.0, 1.0, 1.0, 1.0), gains(1.0, 1.0, lr, ud));
			for (look, merged) in [([10, -10], [0xff, 0x01]), ([-7, 4], [0x20, 0xc0])] {
				assert_eq!(turn(look, [0.0; 2], &r), turn(look, [0.0; 2], &n), "the mouse is direct at Reversed");
				let s = pad_axes(merged);
				let (a, b) = (turn([0, 0], s, &n), turn([0, 0], s, &r));
				assert_eq!((b.0, b.1), (a.0 * lr, a.1 * ud), "the stick reverses");
				let k = [0.7f32, -0.4];
				let (a, b) = (turn([0, 0], k, &n), turn([0, 0], k, &r));
				assert_eq!((b.0, b.1), (a.0 * lr, a.1 * ud), "the keys follow the stick");
			}
		}
		// Mouse right and up with both axes Reversed still look right and up.
		let (yaw, pitch) = turn([10, -10], [0.0; 2], &gains(1.0, 1.0, -1.0, -1.0));
		assert!(yaw < 0.0 && pitch < 0.0, "{yaw} {pitch}");
	}

	#[test]
	fn keys_keep_exactly_the_old_numbers_at_normal_and_reversed() {
		// Q (rx 0xff) turns the view left (yaw up), E right; Z (ry 0xff) tilts up (pitch down), X down: as before.
		for (lr, ud) in [(1.0f32, 1.0f32), (-1.0, -1.0), (-1.0, 1.0)] {
			for k in [1.0f32, 0.2, 2.0, 4.0] {
				let g = gains(k, k, lr, ud);
				for (merged, keys) in [([0xff, 0x80], [127i8, 0]), ([0x01, 0x80], [-127, 0]), ([0x80, 0xff], [0, 127]), ([0x80, 0x01], [0, -127]), ([0xff, 0x01], [127, -127])] {
					for (class4, lr_rev) in [(false, false), (true, false), (true, true), (false, true)] {
						let bytes = camera_sticks(merged, keys, class4, lr_rev);
						assert_eq!(bytes, merged, "a key axis is not touched");
						let s = stick_axes(bytes);
						let v = [(merged[0] as f32 - 128.0) / 127.0, (merged[1] as f32 - 128.0) / 127.0];
						assert_eq!(s, v, "a key axis is the old value");
						// The keys are the stick: the same numbers the pre-change formula gave the stick's share (the mouse is not in it).
						assert_eq!(turn([0, 0], s, &g), old_turn([0, 0], v, &g, k), "keys {keys:?} k {k}");
					}
				}
			}
		}
		let g = gains(1.0, 1.0, 1.0, 1.0);
		assert!(turn([0, 0], stick_axes(camera_sticks([0xff, 0x80], [127, 0], false, false)), &g).0 > 0.0, "Q: yaw up = the view turns left");
		assert!(turn([0, 0], stick_axes(camera_sticks([0x80, 0xff], [0, 127], false, false)), &g).1 < 0.0, "Z: pitch down = the view tilts up");
	}

	#[test]
	fn the_larger_deflection_decides_between_keys_and_controller() {
		// The controller pushed harder than the key: the merged byte is the controller's, so it is flipped.
		assert_eq!(camera_sticks([0xc0, 0x80], [10, 0], false, false)[0], 0x40);
		// The key at full travel wins over a half pushed controller: its byte, untouched.
		assert_eq!(camera_sticks([0xff, 0x80], [127, 0], false, false)[0], 0xff);
		// One axis by keys, the other by the controller.
		assert_eq!(camera_sticks([0xff, 0x20], [127, 0], false, false), [0xff, 0xe0]);
		// No keys, centred: zero.
		assert_eq!(pad_axes([0x80, 0x80]), [0.0, 0.0]);
	}

	/// v0.4.0 read the bytes straight and cut below 0.2: with the rescaled pad bytes the view turned only from about 41 % of the
	/// throw, with a jump. Now: nothing inside the game's dead zone, a small turn just past it, full from 124.
	#[test]
	fn the_controller_turns_the_view_from_the_games_own_dead_zone_on() {
		assert_eq!(stick_axes([0x7f + 47, 0x7f - 47]), [0.0, 0.0]);
		let s = stick_axes([0x7f + 49, 0x7f - 49]);
		assert!(s[0] > 0.0 && s[0] < 0.03 && s[1] < 0.0 && s[1] > -0.03, "{s:?}");
		assert_eq!(stick_axes([0x7f + 124, 0x7f - 124]), [1.0, -1.0]);
		assert_eq!(stick_axes([0x80, 0x7f]), [0.0, 0.0]);
	}

	#[test]
	fn mouse_uses_the_mouse_factor_and_stick_the_camera_speed_factor() {
		let s = pad_axes([0xff, 0x01]);
		let base = turn([10, 10], s, &gains(1.0, 1.0, 1.0, 1.0));
		let m_only = turn([10, 10], [0.0; 2], &gains(2.0, 1.0, 1.0, 1.0));
		let m_base = turn([10, 10], [0.0; 2], &gains(1.0, 1.0, 1.0, 1.0));
		assert_eq!(m_only, (m_base.0 * 2.0, m_base.1 * 2.0), "mouse factor doubles the mouse");
		let st = turn([0, 0], s, &gains(1.0, 1.0, 1.0, 1.0));
		assert_eq!(turn([0, 0], s, &gains(5.0, 1.0, 1.0, 1.0)), st, "mouse factor leaves the stick alone");
		assert_eq!(turn([0, 0], s, &gains(1.0, 2.0, 1.0, 1.0)), (st.0 * 2.0, st.1 * 2.0), "stick factor doubles the stick");
		assert_eq!(turn([10, 10], [0.0; 2], &gains(1.0, 7.0, 1.0, 1.0)), turn([10, 10], [0.0; 2], &gains(1.0, 1.0, 1.0, 1.0)), "stick factor leaves the mouse alone");
		assert!((base.0 - (m_base.0 + st.0)).abs() < 1e-9 && (base.1 - (m_base.1 + st.1)).abs() < 1e-9, "the two add");
	}
}
