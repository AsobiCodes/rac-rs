//! [PC] The eye of a smoothed frame on the arc round the hero (frame smoothing, display only: nothing here is written to game RAM).
//!
//! In game view the camera follows the hero, so between two fields it turns round him. `Retail::camera` used to move the eye on the
//! straight chord between the two fields' eyes; for a boom that turned by `t` radians that chord runs `cos(t / 2)` of the boom from the
//! hero at its middle, and past 120 degrees it came nearer than half the boom (viewer/interp.rs had to cut there: one field drawn
//! unblended, the hitch of a fast mouse turn-around at Mouse speed 200 %). The eye now follows the hero's blended position plus
//! the boom turned about the vertical axis (azimuth by the shortest way, horizontal length and height blended linearly), the same
//! way the camera's yaw is blended, so the eye stays at the boom's distance all through the turn. For a boom that does not
//! turn it is the straight blend, exactly.
use crate::fmath::Fm;

type V3 = [f32; 3];

/// The shortest horizontal boom (eye minus hero, metres) the arc is used for; a nearer eye (straight above the hero) has no
/// azimuth to speak of and keeps the straight blend. Also the minimum boom a turn is judged on (the azimuth of a near-zero vector is noise).
pub const MIN_BOOM: f32 = 0.5;

/// The widest azimuth turn of the boom (radians, about 160 degrees) in one field that is blended on the arc. Near 180 degrees the
/// way round is ambiguous (the two ends nearly face each other), so a turn beyond it is a cut (`viewer/interp.rs` `cam_cut`).
pub const MAX_TURN: f32 = 2.8;

/// A boom turn (radians) under this is no turn: the straight blend is used, which is exact (the arc would rebuild the eye with
/// cos/sin, a rounding step away from the straight blend). At a 20 m boom the arc and the chord differ by less than 1e-7 m here.
pub const ZERO_TURN: f32 = 1e-4;

fn boom(eye: V3, hero: V3) -> Option<(f32, f32, f32)> {
	let (x, y, z) = (eye[0] - hero[0], eye[1] - hero[1], eye[2] - hero[2]);
	let h = x.fhypot(y);
	(h.is_finite() && z.is_finite() && h > MIN_BOOM).then(|| (y.fatan2(x), h, z))
}

/// The signed turn (shortest way, -pi..pi) of the boom from the field before (`ea` round `ha`) to the newest (`eb` round `hb`), or
/// `None` when either boom is shorter than `MIN_BOOM` horizontally (or not finite).
pub fn boom_turn(ea: V3, ha: V3, eb: V3, hb: V3) -> Option<f32> {
	use std::f32::consts::{PI, TAU};
	let (a, b) = (boom(ea, ha)?, boom(eb, hb)?);
	let d = (b.0 - a.0).rem_euclid(TAU);
	Some(if d > PI { d - TAU } else { d })
}

/// Whether the camera of this field orbits the hero: the eye moved further than the hero did. A camera that stands still or follows
/// a path while the hero moves past it is not orbiting; the straight blend is exact for it (the arc would bend its eye off its path).
pub fn orbits(ea: V3, ha: V3, eb: V3, hb: V3) -> bool {
	let dist = |a: V3, b: V3| (0..3).map(|i| (a[i] - b[i]) * (a[i] - b[i])).sum::<f32>().sqrt();
	dist(ea, eb) > dist(ha, hb)
}

/// The turn (-pi..pi, the shortest way) of a field's boom that the arc blends, or `None` when the straight blend is used: the
/// camera does not orbit the hero (`orbits`), a boom is too short, the turn is under `ZERO_TURN` (the straight blend is exact then,
/// to the last bit) or past `MAX_TURN` (a cut).
fn arc_turn(ea: V3, ha: V3, eb: V3, hb: V3) -> Option<f32> {
	boom_turn(ea, ha, eb, hb).filter(|d| d.abs() <= MAX_TURN && d.abs() >= ZERO_TURN && orbits(ea, ha, eb, hb))
}

/// The eye `t` (0..1) of the way from the field before to the newest, on the arc round the hero, and the boom's turn `d` the view
/// yaw has to agree with (`same_side`); `None` (use the straight blend) when the camera does not orbit the hero, the boom is too
/// short, it does not turn, or it turns by more than `MAX_TURN`.
pub fn arc(ea: V3, ha: V3, eb: V3, hb: V3, t: f32) -> Option<(V3, f32)> {
	let d = arc_turn(ea, ha, eb, hb)?;
	let (a, b) = (boom(ea, ha)?, boom(eb, hb)?);
	let l = |x: f32, y: f32| x + (y - x) * t;
	let (az, h, z) = (a.0 + d * t, l(a.1, b.1), l(a.2, b.2));
	let hero = [l(ha[0], hb[0]), l(ha[1], hb[1]), l(ha[2], hb[2])];
	Some(([hero[0] + h * az.fcos(), hero[1] + h * az.fsin(), hero[2] + z], d))
}

/// The eye of `arc` alone.
pub fn arc_eye(ea: V3, ha: V3, eb: V3, hb: V3, t: f32) -> Option<V3> {
	arc(ea, ha, eb, hb, t).map(|(e, _)| e)
}

/// The view yaw turn `dy` (any wrap) taken on the side of the boom's turn `d`: the two are wrapped to -pi..pi on their own, and a
/// look point that lags the hero (the Modern camera's pivot) makes `dy` differ from `d` by a little; near a half turn that little
/// can push one of them over pi, the eye then goes round the hero one way and the view the other. `dy` is shifted by whole turns
/// to lie within pi of `d`; when it already does it is returned as it is.
pub fn same_side(dy: f32, d: f32) -> f32 {
	use std::f32::consts::{PI, TAU};
	if dy - d > PI {
		dy - TAU
	} else if dy - d < -PI {
		dy + TAU
	} else {
		dy
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn eye(az: f32) -> V3 {
		[5.3 * az.fcos(), 5.3 * az.fsin(), 2.8]
	}
	fn dist(a: V3, b: V3) -> f32 {
		(0..3).map(|i| (a[i] - b[i]) * (a[i] - b[i])).sum::<f32>().sqrt()
	}

	/// A turn-around of 2.5 rad keeps the eye 5.3 m (horizontally) from the hero all the way; the straight chord came to 1.7 m.
	#[test]
	fn the_eye_keeps_the_boom_length_through_a_turn_around() {
		let h = [0.0; 3];
		for i in 0..=10 {
			let e = arc_eye(eye(0.0), h, eye(2.5), h, i as f32 / 10.0).unwrap();
			assert!((e[0].fhypot(e[1]) - 5.3).abs() < 1e-3, "{e:?}");
			assert!((e[2] - 2.8).abs() < 1e-4);
		}
		let mid = arc_eye(eye(0.0), h, eye(2.5), h, 0.5).unwrap();
		assert!((mid[1].fatan2(mid[0]) - 1.25).abs() < 1e-4, "halfway round");
		let chord = [(eye(0.0)[0] + eye(2.5)[0]) * 0.5, (eye(0.0)[1] + eye(2.5)[1]) * 0.5];
		assert!(chord[0].fhypot(chord[1]) < 2.0, "the straight chord passes near the hero");
	}

	/// A boom that does not turn is the straight blend, exactly (no arc: `None`), and the ends of a turn are the two eyes.
	#[test]
	fn no_turn_is_the_straight_blend_and_the_ends_are_exact() {
		let (ha, hb) = ([0.0, 0.0, 0.0], [1.0, 0.5, 0.2]);
		let (ea, eb) = ([5.0, 0.0, 2.0], [6.0 + 0.5, 0.5 + 0.0, 2.2 + 0.4]);
		for t in [0.0f32, 0.25, 0.5, 1.0] {
			assert_eq!(arc_eye(ea, ha, eb, hb, t), None, "t {t}: a boom that does not turn takes the straight blend");
		}
		// A turn under ZERO_TURN (rounding noise of a following eye) too.
		assert_eq!(arc_eye(eye(0.0), [0.0; 3], eye(ZERO_TURN * 0.5), [0.0; 3], 0.5), None);
		let end = arc_eye(eye(0.0), [0.0; 3], eye(1.0), [0.0; 3], 1.0).unwrap();
		assert!(dist(end, eye(1.0)) < 1e-4);
		let start = arc_eye(eye(0.0), [0.0; 3], eye(1.0), [0.0; 3], 0.0).unwrap();
		assert!(dist(start, eye(0.0)) < 1e-4);
	}

	/// A camera that stands still (or follows a path) while the hero moves past it is not orbiting: no arc, the straight blend is exact.
	#[test]
	fn a_camera_the_hero_passes_does_not_arc() {
		let (ea, eb) = ([5.0, 0.0, 3.0], [5.0, 0.0, 3.0]);
		let (ha, hb) = ([3.0, -2.0, 0.0], [3.0, 2.0, 0.0]);
		assert!(!orbits(ea, ha, eb, hb));
		assert_eq!(arc_eye(ea, ha, eb, hb, 0.5), None, "the boom turns 90 degrees round a hero running by, the eye does not move");
		// The same turn by an eye that moves (more than the hero) does arc.
		assert!(orbits(eye(0.0), ha, eye(1.5), ha));
		assert!(arc_eye(eye(0.0), ha, eye(1.5), ha, 0.5).is_some());
		// A follow camera: eye and hero move alike, the boom stands: nothing to arc.
		assert!(!orbits([0.0, 5.0, 2.0], [0.0, 0.0, 0.0], [0.0, 6.0, 2.0], [0.0, 1.0, 0.0]));
	}

	/// The view yaw turn is taken on the side of the boom turn: a pivot that lags the hero must not send the view round the other way.
	#[test]
	fn the_view_turns_on_the_side_of_the_boom() {
		use std::f32::consts::{PI, TAU};
		assert_eq!(same_side(1.0, 0.9), 1.0, "already within pi: untouched");
		assert_eq!(same_side(-2.0, -2.1), -2.0);
		// Boom turn +2.75, the yaw (pivot lag) came out at +3.2 and wrapped to -3.08: shift back to +3.2.
		let d = 2.75f32;
		let dy = 3.2 - TAU;
		assert!((same_side(dy, d) - 3.2).abs() < 1e-5, "{}", same_side(dy, d));
		// And the mirror case: boom turn -2.75, yaw -3.2 wrapped to +3.08.
		assert!((same_side(TAU - 3.2, -d) + 3.2).abs() < 1e-5, "{}", same_side(TAU - 3.2, -d));
		// The result is always within pi of d.
		for i in -20..=20 {
			let dy = i as f32 * 0.3;
			assert!((same_side(dy, d) - d).abs() <= PI + 1e-5, "dy {dy}");
		}
	}

	/// Short booms and turns past `MAX_TURN` have no arc; the turn goes the shortest way round (also across +-pi).
	#[test]
	fn short_booms_and_near_half_turns_have_no_arc() {
		let h = [0.0; 3];
		assert_eq!(boom_turn([0.3, 0.0, 5.0], h, eye(1.0), h), None, "eye above the hero");
		assert_eq!(boom_turn(eye(0.0), h, [0.0, 0.2, 0.1], h), None);
		assert!(arc_eye(eye(0.0), h, eye(2.9), h, 0.5).is_none(), "166 degrees");
		assert!(arc_eye(eye(0.0), h, eye(std::f32::consts::PI), h, 0.5).is_none());
		let d = boom_turn(eye(3.0), h, eye(-3.0), h).unwrap();
		assert!((d - (std::f32::consts::TAU - 6.0)).abs() < 1e-4, "across pi the short way: {d}");
		assert!(arc_eye(eye(f32::NAN), h, eye(1.0), h, 0.5).is_none());
	}
}
