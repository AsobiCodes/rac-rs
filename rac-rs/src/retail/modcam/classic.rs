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
	fn the_mouse_grab_follows_the_body_form() {
		assert!(grab_wanted(true, true, true), "Ratchet, Strafe On, RMB held: grabbed");
		assert!(!grab_wanted(true, true, false), "Clank / Giant Clank / Hologuise: nothing aims, no grab");
		assert!(!grab_wanted(false, true, true) && !grab_wanted(true, false, true));
	}
}
