//! The Shoulder lock-on search's view of the crosshair (v0.4.2, lane aim; child of `aim.rs`): the full crosshair cast (`aim_point`,
//! chunked line tests along the ray) is the expensive part of `strafe_lock`, and its search only needs a direction good to a few
//! degrees (the cone is 20 degrees). So with the Shoulder camera the cast runs every third field of a held Blaster and the fields
//! between reuse its DISTANCE along the ray: the point is rebuilt on this field's own ray (this field's eye and forward), so the
//! search direction follows the reticle exactly and only the depth can be up to `LOCK_AGE` fields old (a parallax error of a
//! fraction of a degree). Deterministic: keyed on the camera's field counter, no clock. A shot hook still asks `aim_point` itself
//! (a fresh cast, cached per field), so shots and their icons never use a reused distance.
use super::*;

/// Reach of the strafe lock-on (lane nlock, 2026-11-03, user: "the lock-on locks enemies which are farther than the Blaster can
/// shoot"): only targets the held weapon reaches are picked, at retail's own reach of that weapon's target search. The strafe lock
/// runs for the Blaster only (aim.rs `blaster_search_any`; the other weapons' searches are answered with 0 or the aim point).
///
/// | weapon (item) | reach (units) | source |
/// |---------------|---------------|--------|
/// | Blaster (15)  | 20.0          | [C] retail `blaster_aim_search` (Eudora 0x2a6a50, Novalis 0x2c9c40, Veldin 0x2b4050): inline `d <= 20.0`, cone 0.157 rad (decomp/notes/omni_weapons.md "Target search"; aim.rs `LockOn::RETAIL`). The shot itself flies 40 u/s x dt 0.02 x scale(35) = 29 ticks = 23.2 units (gold: +10 ticks = 31.2; weapons_framework.md "Projectile moves40*dt", "scaled35 + goldByte*10"), so 20 is inside the shot's travel. |
pub(super) const BLASTER_REACH: f32 = 20.0;

/// The strafe lock-on's search numbers at Lock-on strength `pct` (pcext/lockstr.rs): the wide lock-on's cone (20 degrees) scaled
/// by `pct` / 100 (100 = the cone unchanged, bit for bit), the Blaster's reach; None at 0 (no lock, as Lock-on Off).
pub(super) fn strafe_numbers(pct: u8) -> Option<LockOn> {
	let wide = LockOn::wide();
	match pct {
		0 => None,
		100.. => Some(LockOn { range: BLASTER_REACH, ..wide }),
		p => Some(LockOn { range: BLASTER_REACH, cone: wide.cone * p as f32 / 100.0, ..wide }),
	}
}

/// Fields the last cast's distance may be reused.
const LOCK_AGE: u32 = 2;

/// (field, distance along the ray from its start `a` to the aim point) of the last full cast.
static LAST: Mutex<Option<(u32, f32)>> = Mutex::new(None);

/// `aim_point` made a full cast in `field`: the aim point is `dist` from the ray start.
pub(super) fn note(field: u32, dist: f32) {
	if let Ok(mut l) = LAST.lock() {
		*l = Some((field, dist));
	}
}

/// The distance of the last cast if it was made 1..=`LOCK_AGE` fields before `field` (the same field, a cast too old or one from
/// the future, i.e. a counter reset, is not reused).
fn reusable(last: Option<(u32, f32)>, field: u32) -> Option<f32> {
	last.filter(|&(f, _)| (1..=LOCK_AGE).contains(&field.wrapping_sub(f))).map(|(_, dist)| dist)
}

/// The point under the crosshair for `strafe_lock`'s search direction: the last cast's distance on this field's ray when it is
/// young enough, else `aim_point` (a fresh cast, or this field's cached one).
pub(super) fn point(c: &mut Cpu, cam: &modcam::AimCam) -> Option<[f32; 3]> {
	let last = LAST.lock().ok().and_then(|l| *l);
	let Some(dist) = reusable(last, cam.field) else { return aim_point(c).map(|a| a.pos) };
	Some(at(ray_start(cam), cam.fwd, dist))
}

/// Where `aim_point` starts its ray: the camera's pivot projected on the view axis (the eye itself when the pivot is behind it).
fn ray_start(cam: &modcam::AimCam) -> [f32; 3] {
	let t0 = dot([cam.pivot[0] - cam.eye[0], cam.pivot[1] - cam.eye[1], cam.pivot[2] - cam.eye[2]], cam.fwd).max(0.0);
	at(cam.eye, cam.fwd, t0)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn cam(field: u32, eye: [f32; 3], fwd: [f32; 3]) -> modcam::AimCam {
		modcam::AimCam { eye, fwd, pivot: [eye[0] + fwd[0] * 3.0, eye[1] + fwd[1] * 3.0, eye[2] + fwd[2] * 3.0], field, shoulder: true }
	}

	#[test]
	fn strength_scales_the_cone_and_the_reach_is_the_blasters() {
		let wide = LockOn::wide();
		assert!(strafe_numbers(0).is_none(), "0 % = no lock");
		let full = strafe_numbers(100).unwrap();
		assert_eq!((full.range, full.cone.to_bits(), full.snap.to_bits(), full.nearest), (20.0, wide.cone.to_bits(), wide.snap.to_bits(), wide.nearest), "100 % = the wide cone bit for bit");
		assert_eq!(strafe_numbers(50).unwrap().cone, wide.cone * 0.5);
		assert!(strafe_numbers(10).unwrap().cone < strafe_numbers(90).unwrap().cone);
		assert_eq!(strafe_numbers(30).unwrap().range, BLASTER_REACH);
		assert_eq!(BLASTER_REACH, LockOn::RETAIL.range, "retail's own Blaster search reach");
		if std::env::var_os("RAC_LOCKON_RANGE").is_none() {
			assert_eq!(LockOn::wide().range, BLASTER_REACH, "the crosshair-off wide search reaches no further than the Blaster");
		}
	}

	#[test]
	fn the_ray_start_is_the_pivot_on_the_view_axis() {
		let c = cam(1, [1.0, 2.0, 3.0], [1.0, 0.0, 0.0]);
		assert_eq!(ray_start(&c), [4.0, 2.0, 3.0]);
		// A pivot behind the eye starts the ray at the eye.
		let behind = modcam::AimCam { pivot: [-5.0, 2.0, 3.0], ..c };
		assert_eq!(ray_start(&behind), [1.0, 2.0, 3.0]);
	}

	#[test]
	fn a_reused_distance_follows_this_fields_ray() {
		// A cast 2 fields ago found the point 10 m along the ray; the camera has turned since: the point is on the NEW ray.
		let dist = reusable(Some((100, 10.0)), 102).expect("young enough");
		let turned = cam(102, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
		assert_eq!(at(ray_start(&turned), turned.fwd, dist), [0.0, 13.0, 0.0]);
	}

	#[test]
	fn only_a_cast_one_or_two_fields_old_is_reused() {
		let last = Some((100, 7.5));
		assert_eq!((reusable(last, 101), reusable(last, 102)), (Some(7.5), Some(7.5)));
		// The same field (aim_point answers from its own cache), too old, from the future, none yet.
		assert_eq!((reusable(last, 100), reusable(last, 103), reusable(last, 99), reusable(None, 5)), (None, None, None, None));
		assert_eq!(reusable(Some((u32::MAX, 1.0)), 1), Some(1.0));
	}
}
