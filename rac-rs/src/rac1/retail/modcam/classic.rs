//! Strafe aim with Camera Classic (PC extra, user 2026-10-23): while Strafe aiming is On, RMB / gamepad R2 is the strafe aim with
//! either camera, in gameplay and as Ratchet in his own body (form 0) only; elsewhere R2 is retail's. With Classic the modern camera drives only while it is held: the press seeds it from retail's pose on screen
//! (`step`), letting go blends back to retail's camera as F7 does, and the viewer captures the mouse only meanwhile (it turns
//! the aim). Everything that asks "aiming?" (aim.rs, strafe.rs) reads `aim_yaw` / `aim_cam`, so it works the same with both.

use super::*;

/// Camera Classic with the strafe aim held in the last `step`.
static BORROW: AtomicBool = AtomicBool::new(false);
/// The hero was Ratchet in his own body (form 0) in the last `step`: only he aims, so only he gets the mouse grab.
static RATCHET: AtomicBool = AtomicBool::new(true);

/// Whether RMB / gamepad R2 is the strafe aim (not passed to the game) in gameplay as Ratchet (form 0, `prompts::r2_route`): with
/// the modern camera on, and with Camera Classic while Strafe aiming is On.
pub fn aim_button() -> bool {
	on() || (enabled() && crate::retail::pcext::strafe_on())
}

/// The strafe aim mode (RMB / pad R2) runs: asked for, Strafe aiming On, flat ground, and Ratchet in his own body (`form` 0): Clank,
/// Giant Clank and the Hologuise have no aim (strafe.rs / aim.rs test form 0 too).
pub(super) fn aim_mode(aim: bool, strafe_on: bool, flat: bool, form: u8) -> bool {
	aim && strafe_on && flat && form == 0
}

/// RMB held for the strafe aim with Camera Classic: the viewer captures the mouse while it is held, as Ratchet only (Clank, Giant
/// Clank and the Hologuise have no aim, so nothing would use the mouse).
pub fn aim_held() -> bool {
	enabled() && grab_wanted(crate::retail::pcext::strafe_on(), UI.lock().is_ok_and(|u| !u.on && u.aim), RATCHET.load(Ordering::Relaxed))
}

/// The mouse grab for the Classic strafe aim: Strafe aiming On, RMB held (the modern camera off) and the hero in form 0.
fn grab_wanted(strafe_on: bool, rmb: bool, ratchet: bool) -> bool {
	strafe_on && rmb && ratchet
}

/// Fields the press's view offset takes to fade out (the aim lift's blend, `AIM_BLEND`).
const LOOK_FADE: u32 = 12;

/// Where retail's classic camera looks at the press versus where the modern aim orbit looks (always at the pivot): the modern pose
/// is seeded from retail's EYE, so the eye does not jump, but retail's camera looks about 0.1 rad off the pivot (pitch), so writing
/// the pivot-looking rows at once tilted the whole view in one field (v0.5.0 known issue: the one-frame camera jump when a strafe
/// aim is pressed). The offset is taken on the first written pose (that pose then equals retail's view) and fades out smoothly.
#[derive(Clone, Copy, Default)]
pub(super) struct LookOff {
	/// Retail's (yaw, pitch) at the press, waiting for the first pose written.
	ret: Option<(f32, f32)>,
	/// The offset still to fade (yaw, pitch) and the fields faded so far.
	off: (f32, f32),
	n: u32,
}

impl LookOff {
	pub(super) const NONE: LookOff = LookOff { ret: None, off: (0.0, 0.0), n: LOOK_FADE };

	/// The press seeded the modern pose from retail's (yaw, pitch).
	pub(super) fn seed(&mut self, ret: (f32, f32)) {
		*self = LookOff { ret: Some(ret), off: (0.0, 0.0), n: 0 };
	}

	/// The seed of a new live pose: with the modern camera on (or a fresh level) there is no offset to keep.
	pub(super) fn on_seed(&mut self, modern: bool, ret: (f32, f32)) {
		if modern {
			self.clear();
		} else {
			self.seed(ret);
		}
	}

	/// Back to no offset (hand-back, a seed with the modern camera on, a re-seed).
	pub(super) fn clear(&mut self) {
		*self = LookOff { ret: None, off: (0.0, 0.0), n: LOOK_FADE };
	}

	/// The (yaw, pitch) the camera rows get for the pose (`yaw`, `pitch`) of this field: the pose itself once the offset is gone.
	pub(super) fn apply(&mut self, yaw: f32, pitch: f32) -> (f32, f32) {
		if let Some((ry, rp)) = self.ret.take() {
			self.off = (super::wrap(ry - yaw), rp - pitch);
			self.n = 0;
		}
		if self.n >= LOOK_FADE {
			return (yaw, pitch);
		}
		let t = self.n as f32 / LOOK_FADE as f32;
		let w = 1.0 - t * t * (3.0 - 2.0 * t);
		self.n += 1;
		(yaw + self.off.0 * w, pitch + self.off.1 * w)
	}
}

/// Whether a seed keeps no look offset: the modern camera drives, or the orbit frame is tilted (retail's world yaw / pitch are
/// not angles of that frame, so their difference to the orbit's would be meaningless).
pub(super) fn no_offset(modern: bool, flat: bool) -> bool {
	modern || !flat
}

/// The modern camera drives the follow camera: switched on, or borrowed for the strafe aim with Camera Classic.
pub(super) fn drives(st: &St) -> bool {
	st.active || BORROW.load(Ordering::Relaxed)
}

/// `step`'s verdict for this field: why retail's camera runs (None = the modern camera drives). Camera Classic borrows it while
/// the strafe aim is held on flat ground (the aim mode's own terms, `step`).
pub(super) fn why(c: &mut Cpu, lv: &Lv, rec: u32, st: &St) -> Option<String> {
	let form = c.mem.rd8(BODY_FORM);
	RATCHET.store(form == 0, Ordering::Relaxed);
	let borrow = !st.active && aim_mode(st.inp.aim, crate::retail::pcext::strafe_on(), Frame::carry(FLAT.a, hero_up(c, lv)).0.flat(), form);
	let was = BORROW.swap(borrow, Ordering::Relaxed);
	if st.active || borrow {
		why_retail(c, lv, rec)
	} else if was {
		Some("strafe aim let go (Camera Classic)".into())
	} else {
		Some("toggled off (F7)".into())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn only_ratchets_own_body_aims() {
		assert!(aim_mode(true, true, true, 0), "form 0: as before");
		for form in 1..=3 {
			assert!(!aim_mode(true, true, true, form), "form {form} (Clank / Giant Clank / Hologuise): no aim mode, no raised camera");
		}
		assert!(!aim_mode(false, true, true, 0) && !aim_mode(true, false, true, 0) && !aim_mode(true, true, false, 0));
	}

	#[test]
	fn the_press_view_starts_at_retails_and_fades_to_the_orbit() {
		let mut l = LookOff::NONE;
		assert_eq!(l.apply(1.0, 0.2), (1.0, 0.2), "no seed: the pose as it is");
		l.seed((1.0, 0.13));
		let (y0, p0) = l.apply(1.0, 0.24);
		assert!((y0 - 1.0).abs() < 1e-6 && (p0 - 0.13).abs() < 1e-6, "the first pose written is retail's view: no jump");
		let mut last = p0;
		for _ in 1..LOOK_FADE {
			let (_, p) = l.apply(1.0, 0.24);
			assert!(p >= last - 1e-6 && p <= 0.24 + 1e-6, "monotone towards the orbit's pitch");
			last = p;
		}
		assert_eq!(l.apply(1.0, 0.24), (1.0, 0.24), "faded out");
		l.seed((2.0, 0.0));
		l.clear();
		assert_eq!(l.apply(1.0, 0.24), (1.0, 0.24), "cleared");
	}

	#[test]
	fn a_repress_in_the_blend_starts_at_the_view_on_screen() {
		// What `step` passes on a seed: the blended pose shown (yaw, pitch), not retail's rows. The first pose written equals it.
		let mut l = LookOff::NONE;
		l.on_seed(false, (0.5, 0.20));
		let (y, p) = l.apply(0.5, 0.24);
		assert!((y - 0.5).abs() < 1e-6 && (p - 0.20).abs() < 1e-6, "first written pose = the shown view");
		l.on_seed(true, (0.5, 0.20));
		assert_eq!(l.apply(0.5, 0.24), (0.5, 0.24), "modern camera on: nothing computed, no offset");
	}

	#[test]
	fn no_look_offset_off_flat_ground() {
		assert!(no_offset(false, false) && no_offset(true, false) && no_offset(true, true));
		assert!(!no_offset(false, true));
		let mut l = LookOff::NONE;
		l.on_seed(no_offset(false, false), (2.0, 0.5));
		assert_eq!(l.apply(0.5, 0.24), (0.5, 0.24), "tilted frame: the pose as it is");
	}

	#[test]
	fn the_mouse_grab_follows_the_body_form() {
		assert!(grab_wanted(true, true, true), "Ratchet, Strafe On, RMB held: grabbed");
		assert!(!grab_wanted(true, true, false), "Clank / Giant Clank / Hologuise: nothing aims, no grab");
		assert!(!grab_wanted(false, true, true) && !grab_wanted(true, false, true));
	}
}
