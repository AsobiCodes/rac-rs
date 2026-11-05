//! NOV-14.3 sand mouse (class 1818, FUN_0030d870, the Sonic Summoner pet) and NOV-14.3c its bolt (class 1633,
//! spawner FUN_0030c1c8, updater FUN_0030c2d8): decomp/notes/novalis_checklist.md "NOV-14.3" / "NOV-14.3c".
//! Particle effects (shadow, dust, the type 0x35 burst) are NOV-14.3b: their shared-RNG draws are taken here in
//! retail order, the effects are not spawned.

use std::f32::consts::PI;



/// FUN_00221b28: `a + b` wrapped into [-pi, pi) by one subtract / add, both tests on the sum.
pub(super) fn wrap_angle(a: f32, b: f32) -> f32 {
	let x = a + b;
	let mut r = x;
	if x >= PI {
		r = (x - PI) - PI;
	}
	if x < -PI {
		r = x + PI + PI;
	}
	r
}













