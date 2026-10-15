//! NOV-14.3 sand mouse (class 1818, FUN_0030d870, the Sonic Summoner pet) and NOV-14.3c its bolt (class 1633,
//! spawner FUN_0030c1c8, updater FUN_0030c2d8): decomp/notes/novalis_checklist.md "NOV-14.3" / "NOV-14.3c".
//! Particle effects (shadow, dust, the type 0x35 burst) are NOV-14.3b: their shared-RNG draws are taken here in
//! retail order, the effects are not spawned.

use super::foes::self;
use super::novalis::MOUSE;
use super::novalis::MOUSE_SHOT;
use super::{Event, Game, Kind};
use crate::gameplay::self;
use crate::gameplay::PAL_DT;
use crate::gameplay::PAL_DT2;
use crate::particles::Rng;
use crate::physics::add;
use crate::physics::length;
use crate::physics::scale;
use crate::physics::sub;
use crate::physics::MobySolid;
use crate::physics::Scene;
use crate::physics::World;
use crate::physics::V3;
use std::f32::consts::{PI, TAU};



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













