//! KER-64a / KER-64b: Kerwan camera class 14, a hero-follow camera entered through a cuboid. The camera sits a boom
//! length behind the hero along the record's forward (row 0 of its euler rotation) and a boom height above, follows a
//! sprung point toward the hero and looks at that point plus 1.5 up; it leaves once the hero exits the cuboid
//! (decomp/notes/kerwan_gadgets.md "KER-64a port spec", "KER-64b port spec"). The camera-record driver
//! (`CameraRig::drive`, decomp/notes/camera.md "SHR-CAMDRVa port spec") runs `exit` -> `select_record` with
//! `class14_pred` -> `setup` on a switch -> `update` each camera frame.

use super::super::aridia::set_len;
use crate::camera::{CamBlendRegs, CamRec};
use crate::gameplay::Cuboid;
use crate::particles::euler_m;
use crate::physics::{vadd, vadd1, vcross, vmul, vsub, vsub1, V3};

/// The hero's down vector (Kerwan 0x13f5e0; its only writer stores (0, 0, -1)).
const DOWN: V3 = [0.0, 0.0, -1.0];
/// Hero camera mode of the jump family and the hero states the predicate and the z hold test.
const MODE_JUMP: u8 = 0x50;
const STATE_WALL_JUMP: u8 = 0x11;

/// The class-14 camera state: the camera block (rows, eye, aim) and the rig fields the class reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Class14Cam {
	/// Rig +0x00..+0x08: the sprung follow point.
	pub s: V3,
	/// Rig +0x10..+0x18: its per-axis spring velocities.
	pub vel: V3,
	/// Rig +0x20 / +0x24 / +0x28: spring gain, damping and limit (always 0; the limit branch never runs).
	pub gain: f32,
	pub damp: f32,
	pub limit: f32,
	/// Rig +0xb8 / +0xbc: a second sprung hero height and its velocity (written only).
	pub z2: f32,
	pub z2_vel: f32,
	/// Rig +0xc4: the look height above the follow point.
	pub look_h: f32,
	/// Rig +0xd0 / +0xe0: the record's boom length and height.
	pub boom_len: f32,
	pub boom_h: f32,
	/// Rig +0xf0: the record's forward (row 0 of its euler rotation).
	pub fwd: V3,
	/// Rig +0x124: the height the follow point springs toward.
	pub z_target: f32,
	/// Camera +0x30: the eye.
	pub pos: V3,
	/// Camera +0x00 / +0x10 / +0x20: forward, left, up.
	pub rows: [V3; 3],
	/// Camera +0x40: the aim (row 0).
	pub aim: V3,
}


/// 0x1f8888: the reversed cross product, `b x a`.
fn cross_rev(a: V3, b: V3) -> V3 {
	vcross(b, a)
}

/// 0x1e4aa8 with limit 0: one linear spring step of `cur` toward `target`. The velocity gains `k * e` against `d * v`, is
/// clamped to the error's magnitude, and is added to `cur`.
pub(crate) fn lin_spring(cur: &mut f32, target: f32, k: f32, d: f32, vel: &mut f32) {
	let e = vsub1(target, *cur);
	*vel = vadd1(*vel, vsub1(vmul(k, e), vmul(d, *vel)));
	if vel.abs() > e.abs() {
		*vel = e.abs().copysign(*vel);
	}
	*cur = vadd1(*cur, *vel);
}

impl Class14Cam {
	/// KER-64a FUN_002eb2a8: entering class 14. Look height 1.5, the record's boom, the follow point at the hero with zero
	/// velocity (gain 0.01, damping 0.2, no limit), the z target and second height at the hero's z, the forward from the
	/// record rotation; then places the camera, clears the record's request and sets the blend countdown to a raw 120.
	pub(crate) fn setup(&mut self, rec: &mut CamRec, hero: V3, regs: &mut CamBlendRegs) {
		self.look_h = f32::from_bits(0x3fc0_0000);
		self.z2 = hero[2];
		self.z2_vel = 0.0;
		self.boom_len = rec.boom_len;
		self.boom_h = rec.boom_h;
		self.limit = 0.0;
		self.gain = f32::from_bits(0x3c23_d70a);
		self.damp = f32::from_bits(0x3e4c_cccd);
		self.vel = [0.0; 3];
		self.s = hero;
		self.z_target = hero[2];
		self.fwd = euler_m(rec.rot)[0];
		self.place(hero);
		rec.request = 0;
		regs.count = 120;
	}

	/// KER-64a FUN_002eb110: the eye at hero + up * boom height - boom length * forward, aimed at the point 1.5 above the
	/// hero (the aim normalised three times in a row), left = -unit(down x forward), up = forward x left.
	pub(crate) fn place(&mut self, hero: V3) {
		let a = set_len(DOWN, -self.boom_h);
		let b = set_len(self.fwd, -self.boom_len);
		self.pos = vadd(vadd(hero, a), b);
		let look = vadd(hero, set_len(DOWN, -self.look_h));
		let d = vsub(look, self.pos);
		let first = set_len(d, 1.0);
		let second = set_len(first, 1.0);
		self.aim = set_len(second, 1.0);
		self.rows[0] = self.aim;
		self.rows[1] = set_len(cross_rev(self.rows[0], DOWN), -1.0);
		self.rows[2] = cross_rev(self.rows[1], self.rows[0]);
	}

	/// KER-64b FUN_002eb6e0: one camera frame. The z target follows the hero's z unless the hero is in a jump-family mode
	/// without wall jumping; the follow point springs toward (hero x, hero y, z target); the eye sits at the follow point
	/// plus the boom; the second height springs toward the hero's z; the camera aims at the follow point 1.5 up.
	/// `up` is the driver's smoothed world up (0x166eb0, `CameraRig.up_s`, SHR-CAMDRVa1).
	pub(crate) fn update(&mut self, hero: V3, hero_mode: u8, hero_state: u8, up: V3) {
		if !(hero_mode == MODE_JUMP && hero_state != STATE_WALL_JUMP) {
			self.z_target = hero[2];
		}
		let a = set_len(DOWN, -self.boom_h);
		let b = set_len(self.fwd, -self.boom_len);
		let target = [hero[0], hero[1], self.z_target];
		for j in 0..3 {
			lin_spring(&mut self.s[j], target[j], self.gain, self.damp, &mut self.vel[j]);
		}
		self.pos = vadd(vadd(self.s, a), b);
		// 0x2eb420 is an empty callback.
		lin_spring(&mut self.z2, hero[2], f32::from_bits(0x3ba3_d70a), f32::from_bits(0x3e4c_cccd), &mut self.z2_vel);
		let t = vadd(set_len(up, self.look_h), self.s);
		self.rows[0] = set_len(vsub(t, self.pos), 1.0);
		self.aim = self.rows[0];
		self.rows[1] = set_len(cross_rev(self.rows[0], up), 1.0);
		self.rows[2] = cross_rev(self.rows[1], self.rows[0]);
	}

	/// KER-64b FUN_002eb718: the pre-scan exit test. A record with a cuboid that no longer holds the hero requests 3.
	pub(crate) fn exit(rec: &mut CamRec, hero: V3, cuboids: &[Cuboid]) {
		if let Some(c) = rec.cuboid {
			if !cuboids.get(c).is_some_and(|cb| cb.contains(hero)) {
				rec.request = 3;
			}
		}
	}
}

