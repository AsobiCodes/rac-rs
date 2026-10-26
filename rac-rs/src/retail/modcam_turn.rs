//! The Modern camera's turn from one field's input (lane cam, 2026-10-23; player report: "to have a normal right stick view you
//! have to set the camera to inverted, with the mouse inverted really is inverted"). `modcam.rs` `step` calls these two pure fns.
//!
//! The view convention (modcam `rows`): yaw grows to the LEFT, pitch grows DOWN. The mouse turns the view the way it moves
//! (right = look right = yaw down, up = look up = pitch down; `look` is x right, y down). Retail's own follow camera turns the
//! other way for the stick (it orbits the hero: stick right moves the camera right, the view looks left), and modcam copied that,
//! so at Normal the stick was inverted against the mouse on both axes. Now, with the Modern camera, Normal means the same for
//! the physical right stick and the mouse (right = look right, up = look up) and Reversed flips both, per axis (`lr` for yaw,
//! `ud` for pitch, retail's two option bytes). The keyboard camera keys (Q/E turn, Z/X tilt: they feed the same right-stick bytes,
//! binds.rs) keep EXACTLY their old direction at both settings: the keys' share of the stick travels in `Input::keys`.

/// The right stick as the turn maths takes it, in `[-1, 1]` (PS2 bytes: right and down are high): `merged` = the pad bytes the game
/// sees (rx, ry; keyboard and controller merged by the larger deflection), `keys` = the keys' share (offset from 0x80, 0 = no key
/// held). An axis the keys drive (the merged byte is the keys' byte) keeps the old sign; an axis the controller drives gets the
/// opposite, the mouse's, sign.
pub fn stick_axes(merged: [u8; 2], keys: [i8; 2]) -> [f32; 2] {
	let axis = |i: usize| {
		let v = (merged[i] as f32 - 128.0) / 127.0;
		let from_keys = keys[i] != 0 && merged[i] as i16 - 128 == keys[i] as i16;
		if from_keys {
			v
		} else {
			-v
		}
	};
	[axis(0), axis(1)]
}

/// The per-field gains of the turn: the mouse (counts -> radians, `RAC_MODCAM_SENS`, the Mouse speed row's factor), the stick (yaw
/// rate, pitch step, the Camera speed row's factor) and retail's Reversed options (-1.0 Reversed, 1.0 Normal) per axis.
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
/// Smooth ramp). With the same factor for both and keys only, the numbers are those of the one-factor formula before the change.
pub fn turn(look: [i16; 2], s: [f32; 2], g: &Gains) -> (f32, f32) {
	let dyaw = (-(look[0] as f32) * g.mouse * g.sens * g.mouse_k + s[0] * g.rot * g.stick_k) * g.lr;
	let dpitch = (look[1] as f32 * g.mouse * g.sens * g.mouse_k - s[1] * g.tilt * g.stick_k) * g.ud;
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

	#[test]
	fn physical_stick_has_the_mouses_sign_on_both_axes() {
		let g = gains(1.0, 1.0, 1.0, 1.0);
		// Mouse right (x +) and up (y -) at Normal: yaw down = look right, pitch down = look up.
		let (my, mp) = turn([10, -10], [0.0, 0.0], &g);
		assert!(my < 0.0 && mp < 0.0);
		// The controller's right stick pushed right (rx high) and up (ry low): the same signs.
		let s = stick_axes([0xff, 0x01], [0, 0]);
		let (sy, sp) = turn([0, 0], s, &g);
		assert!(sy < 0.0 && sp < 0.0, "{sy} {sp}");
		// Left and down: the opposite signs, like the mouse moved left and down.
		let (my2, mp2) = turn([-10, 10], [0.0, 0.0], &g);
		let (sy2, sp2) = turn([0, 0], stick_axes([0x01, 0xff], [0, 0]), &g);
		assert!(my2 > 0.0 && mp2 > 0.0 && sy2 > 0.0 && sp2 > 0.0);
	}

	#[test]
	fn reversed_flips_stick_and_mouse_per_axis() {
		for (lr, ud) in [(-1.0f32, 1.0f32), (1.0, -1.0), (-1.0, -1.0)] {
			let (n, r) = (gains(1.0, 1.0, 1.0, 1.0), gains(1.0, 1.0, lr, ud));
			for (look, merged) in [([10, -10], [0xff, 0x01]), ([-7, 4], [0x20, 0xc0])] {
				let s = stick_axes(merged, [0, 0]);
				let (a, b) = (turn(look, [0.0; 2], &n), turn(look, [0.0; 2], &r));
				assert_eq!((b.0, b.1), (a.0 * lr, a.1 * ud), "mouse");
				let (a, b) = (turn([0, 0], s, &n), turn([0, 0], s, &r));
				assert_eq!((b.0, b.1), (a.0 * lr, a.1 * ud), "stick");
			}
		}
	}

	#[test]
	fn keys_keep_exactly_the_old_numbers_at_normal_and_reversed() {
		// Q (rx 0xff) turns the view left (yaw up), E right; Z (ry 0xff) tilts up (pitch down), X down: as before.
		for (lr, ud) in [(1.0f32, 1.0f32), (-1.0, -1.0), (-1.0, 1.0)] {
			for k in [1.0f32, 0.2, 2.0, 4.0] {
				let g = gains(k, k, lr, ud);
				for (merged, keys) in [([0xff, 0x80], [127i8, 0]), ([0x01, 0x80], [-127, 0]), ([0x80, 0xff], [0, 127]), ([0x80, 0x01], [0, -127]), ([0xff, 0x01], [127, -127])] {
					let s = stick_axes(merged, keys);
					let v = [(merged[0] as f32 - 128.0) / 127.0, (merged[1] as f32 - 128.0) / 127.0];
					assert_eq!(s, v, "a key axis is the old value");
					let look = [3, -5];
					assert_eq!(turn(look, s, &g), old_turn(look, v, &g, k), "keys {keys:?} k {k}");
				}
			}
		}
		let g = gains(1.0, 1.0, 1.0, 1.0);
		assert!(turn([0, 0], stick_axes([0xff, 0x80], [127, 0]), &g).0 > 0.0, "Q: yaw up = the view turns left");
		assert!(turn([0, 0], stick_axes([0x80, 0xff], [0, 127]), &g).1 < 0.0, "Z: pitch down = the view tilts up");
	}

	#[test]
	fn the_larger_deflection_decides_between_keys_and_controller() {
		// The controller pushed harder than the key: the merged byte is the controller's, so the controller's rule applies.
		assert_eq!(stick_axes([0xc0, 0x80], [10, 0])[0], -(0x40 as f32) / 127.0);
		// The key at full travel wins over a half pushed controller: the old sign.
		assert_eq!(stick_axes([0xff, 0x80], [127, 0])[0], 1.0);
		// One axis by keys, the other by the controller.
		let s = stick_axes([0xff, 0x20], [127, 0]);
		assert!(s[0] > 0.0 && s[1] > 0.0, "{s:?}");
		// No keys, centred: zero.
		assert_eq!(stick_axes([0x80, 0x80], [0, 0]), [0.0, 0.0]);
	}

	#[test]
	fn mouse_uses_the_mouse_factor_and_stick_the_camera_speed_factor() {
		let s = stick_axes([0xff, 0x01], [0, 0]);
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
