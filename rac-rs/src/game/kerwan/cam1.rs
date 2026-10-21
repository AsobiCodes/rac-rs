//! KER-63a / KER-63b: Kerwan camera class 1, a fixed-position camera entered through a cuboid that turns toward the hero
//! with an angle spring and leaves once the hero exits the cuboid (decomp/notes/kerwan_gadgets.md "KER-63a port spec",
//! "KER-63b port spec"). The camera-record driver (`CameraRig::drive`, decomp/notes/camera.md "SHR-CAMDRVa port
//! spec") runs `exit` -> `select_record` with `class1_pred` -> `setup` on a switch -> `update` ->
//! `CameraRig::publish_rows(pos, rows)` each camera frame.

use super::super::aridia::set_len;
use super::super::bolt::{wrap_add, wrap_sub};
use super::super::suckcomp::qmul;
use crate::camera::{fast_cos, fast_sin, retail_asin, CamRec};
use crate::gameplay::Cuboid;
use crate::particles::euler_m;
use crate::physics::{cross, dot, sub, vadd1, vmul, vsub1, V3};

/// pi/2 as the class uses it (0x3fc90fdb).
const HALF_PI: f32 = f32::from_bits(0x3fc9_0fdb);

/// The class-1 camera state (camera block rows / position / aim and its rig). `vel` is the angle-spring velocity, which
/// setup leaves alone, so one instance must outlive each class-1 session.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Class1Cam {
	/// Rig +0x5c: the height above the hero the camera looks at.
	pub look_h: f32,
	/// Rig +0x04 / +0x08 / +0x0c: the angle-spring gain, damping and velocity limit (0 = no limit).
	pub gain: f32,
	pub damp: f32,
	pub limit: f32,
	/// Rig +0x00: the angle-spring velocity.
	pub vel: f32,
	/// Camera +0x40: the forward vector the rotation starts from (row 0 after each update).
	pub aim: V3,
	/// Camera +0x30: the fixed eye (never moved by update).
	pub pos: V3,
	/// Camera +0x00 / +0x10 / +0x20: forward, left, up.
	pub rows: [V3; 3],
}

/// The hero (or eye) against record cuboid `idx`; a record with no cuboid has no inside.
fn in_cuboid(cuboids: &[Cuboid], idx: Option<usize>, p: V3) -> bool {
	idx.and_then(|k| cuboids.get(k)).is_some_and(|c| c.contains(p))
}


/// KER-63b FUN_001e4b90 with current angle 0: the angle spring. The error `phi` drives the velocity by `gain` against
/// `damp`; the velocity is clamped to +-`limit` when a limit is set, then to the error's magnitude. Returns the wrapped
/// step.
fn class1_spring(phi: f32, gain: f32, damp: f32, limit: f32, vel: &mut f32) -> f32 {
	let e = wrap_sub(phi, 0.0);
	let mut v = vadd1(*vel, vsub1(vmul(gain, e), vmul(damp, *vel)));
	if limit != 0.0 {
		v = v.clamp(-limit.abs(), limit.abs());
	}
	let m = e.abs();
	v = v.clamp(-m, m);
	*vel = v;
	wrap_add(0.0, v)
}

/// KER-63b FUN_0024e290 (length flag 1): unit `from` turned toward unit `to` by the fraction `s` of the angle between them,
/// about their unit cross axis, as `q * from * conj(q)`. No early-out: parallel vectors give a zero axis and a zero turn.
fn rotate_toward(s: f32, from: V3, to: V3) -> V3 {
	let ax = set_len(cross(from, to), 1.0);
	let c = dot(from, to);
	let h = vmul(vmul(HALF_PI - retail_asin(c), 0.5), s);
	let (sn, cs) = (fast_sin(h), fast_cos(h));
	let q = [vmul(ax[0], sn), vmul(ax[1], sn), vmul(ax[2], sn), cs];
	let t = qmul(q, [from[0], from[1], from[2], 0.0]);
	let r = qmul(t, [-q[0], -q[1], -q[2], q[3]]);
	[r[0], r[1], r[2]]
}

impl Class1Cam {
	/// KER-63a FUN_002e8270 -> FUN_002e7f58: entering class 1. Sets the look height (1.5, then the record's), the spring
	/// gain 0.02 / damping 0.1 / no limit, the eye to the record position and the rows to the record rotation, aims along
	/// row 0 and clears the record's request. The spring velocity keeps its last session value.
	pub(crate) fn setup(&mut self, rec: &mut CamRec) {
		self.look_h = f32::from_bits(0x3fc0_0000);
		self.gain = f32::from_bits(0x3ca3_d70a);
		self.damp = f32::from_bits(0x3dcc_cccd);
		self.limit = 0.0;
		self.look_h = rec.look;
		self.pos = rec.pos;
		self.rows = euler_m(rec.rot);
		self.aim = self.rows[0];
		rec.request = 0;
	}

	/// KER-63b FUN_002e8060: one camera frame. Turns row 0 toward the point `look_h` above the hero by the spring's
	/// fraction of the angle, then rebuilds left = unit(up x forward) and up = forward x left from the view's `up`.
	pub(crate) fn update(&mut self, hero: V3, up: V3) {
		let t = [hero[0], hero[1], hero[2] + self.look_h];
		let dir = set_len(sub(t, self.pos), 1.0);
		let phi = HALF_PI - retail_asin(dot(dir, self.rows[0]));
		let s = class1_spring(phi, self.gain, self.damp, self.limit, &mut self.vel);
		let r0 = rotate_toward(s, self.aim, dir);
		self.aim = set_len(r0, 1.0);
		self.rows[0] = self.aim;
		self.rows[1] = set_len(cross(up, self.rows[0]), 1.0);
		self.rows[2] = cross(self.rows[0], self.rows[1]);
	}

	/// KER-63b FUN_002e82b0: the pre-scan exit test. A mode-4 record whose cuboid no longer holds the hero requests 3,
	/// which lets an equal or lower priority record displace it.
	pub(crate) fn exit(rec: &mut CamRec, hero: V3, cuboids: &[Cuboid]) {
		if rec.mode == 4 && !in_cuboid(cuboids, rec.cuboid, hero) {
			rec.request = 3;
		}
	}
}

