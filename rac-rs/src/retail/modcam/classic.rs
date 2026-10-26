//! Strafe aim with Camera Classic (PC extra, user 2026-10-23): while Strafe aiming is On, RMB / gamepad R2 is the strafe aim with
//! either camera. With Classic the modern camera drives only while it is held: the press seeds it from retail's pose on screen
//! (`step`), letting go blends back to retail's camera as F7 does, and the viewer captures the mouse only meanwhile (it turns
//! the aim). Everything that asks "aiming?" (aim.rs, strafe.rs) reads `aim_yaw` / `aim_cam`, so it works the same with both.

use super::*;

/// Camera Classic with the strafe aim held in the last `step`.
static BORROW: AtomicBool = AtomicBool::new(false);

/// Whether RMB / gamepad R2 is the strafe aim (not passed to the game): with the modern camera on, and with Camera Classic while
/// Strafe aiming is On.
pub fn aim_button() -> bool {
	on() || (enabled() && crate::retail::pcext::strafe_on())
}

/// RMB held for the strafe aim with Camera Classic: the viewer captures the mouse while it is held.
pub fn aim_held() -> bool {
	enabled() && crate::retail::pcext::strafe_on() && UI.lock().is_ok_and(|u| !u.on && u.aim)
}

/// The modern camera drives the follow camera: switched on, or borrowed for the strafe aim with Camera Classic.
pub(super) fn drives(st: &St) -> bool {
	st.active || BORROW.load(Ordering::Relaxed)
}

/// `step`'s verdict for this field: why retail's camera runs (None = the modern camera drives). Camera Classic borrows it while
/// the strafe aim is held on flat ground (the aim mode's own terms, `step`).
pub(super) fn why(c: &mut Cpu, lv: &Lv, rec: u32, st: &St) -> Option<String> {
	let borrow = !st.active && st.inp.aim && crate::retail::pcext::strafe_on() && Frame::carry(FLAT.a, hero_up(c, lv)).0.flat();
	let was = BORROW.swap(borrow, Ordering::Relaxed);
	if st.active || borrow {
		why_retail(c, lv, rec)
	} else if was {
		Some("strafe aim let go (Camera Classic)".into())
	} else {
		Some("toggled off (F7)".into())
	}
}
