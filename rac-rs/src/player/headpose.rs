//! Hero limb pulse (HERO-16) and its trigger hooks (HERO-16b): a squash/stretch of the hero and its held items
//! driven by a counter that hero events start. decomp/notes/hero_movement.md.
use super::*;




/// A held item's matrix under the pulse (level mode 0): the first two axes scaled by `s7` (the wrench slot is
/// capped at 1.0) and the position moved to `point + (pos - point) * s`. `m` is the 3x4 row-major matrix whose
/// columns 0 and 1 are the item's x and y axes.
pub fn pulse_item(m: [f32; 12], s: f32, wrench_slot: bool, point: V3) -> [f32; 12] {
	let s7 = if wrench_slot { s.min(1.0) } else { s };
	let mut o = m;
	for r in 0..3 {
		o[4 * r] *= s7;
		o[4 * r + 1] *= s7;
		o[4 * r + 3] = point[r] + (m[4 * r + 3] - point[r]) * s;
	}
	o
}

/// One hero joint override block (FUN_00226b18, 0xb0 bytes in retail): springs a joint rotation and offset toward
/// targets the state code writes every tick; `active` is the retail "registered" state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointBlock {
	pub joint: u16,
	pub slot: u8,
	pub k: f32,
	pub c: f32,
	pub ang: [f32; 3],
	pub rate: [f32; 3],
	pub target: [f32; 3],
	pub off: [f32; 3],
	pub off_rate: [f32; 3],
	pub off_target: [f32; 3],
	pub scale: f32,
	/// The scale the last pass applied (record scale, BRIDGE-22): the block `scale` as read at the pass.
	pub rec_scale: f32,
	pub active: bool,
	/// Registration stamp (newest = largest), the record list order of the pose evaluator.
	pub reg: u32,
}

/// (joint, slot, k, c) of the 31 blocks at 0x17aa80 (initial data).
const BLOCK_INIT: [(u16, u8, f32, f32); 31] = [
	(9, 0, 0.0, 0.0), (10, 0, 0.0, 0.0), (11, 0, 0.0, 0.0), (4, 0, 0.0, 0.0),
	(22, 0, 0.1, 0.3), (23, 0, 0.1, 0.3), (7, 0, 0.1, 0.3), (8, 0, 0.1, 0.3),
	(13, 0, 0.0, 0.0), (12, 0, 0.0, 0.0), (6, 0, 0.0, 0.0), (0, 0, 0.0, 0.0),
	(21, 0, 0.12, 0.3), (25, 0, 0.015, 0.3), (26, 0, 0.015, 0.3), (27, 0, 0.013, 0.25),
	(28, 0, 0.017, 0.21), (24, 0, 0.03, 0.2), (0, 1, 0.0, 0.0), (1, 1, 0.0, 0.0),
	(2, 1, 0.04, 0.3), (3, 1, 0.04, 0.3), (4, 1, 0.04, 0.3), (5, 1, 0.04, 0.3),
	(4, 2, 0.02, 0.2), (5, 2, 0.02, 0.2), (6, 2, 0.02, 0.2),
	(2, 3, 0.02, 0.2), (1, 3, 0.02, 0.2), (0, 3, 0.02, 0.2), (3, 4, 0.02, 0.2),
];

impl JointBlock {
	pub fn new(joint: u16, slot: u8, k: f32, c: f32) -> JointBlock {
		JointBlock { joint, slot, k, c, ang: [0.0; 3], rate: [0.0; 3], target: [0.0; 3], off: [0.0; 3], off_rate: [0.0; 3], off_target: [0.0; 3], scale: 1.0, rec_scale: 1.0, active: false, reg: 0 }
	}
}

/// The 31 blocks in their initial state (FUN_00226ee8 resets the same fields).
pub fn joint_blocks() -> [JointBlock; 31] {
	BLOCK_INIT.map(|(j, s, k, c)| JointBlock::new(j, s, k, c))
}


/// The joint override quaternion of a block's angles `Qz(a2) Qy(a1) Qx(a0)` in the port's quaternion sign (the
/// retail record quaternion conjugated), shared by the hero blocks and the Plumber's head blocks.
pub fn block_quat(ang: [f32; 3]) -> [f32; 4] {
	let mul = |a: [f32; 4], b: [f32; 4]| {
		let ([ax, ay, az, aw], [bx, by, bz, bw]) = (a, b);
		[aw * bx + ax * bw + ay * bz - az * by, aw * by - ax * bz + ay * bw + az * bx, aw * bz + ax * by - ay * bx + az * bw, aw * bw - ax * bx - ay * by - az * bz]
	};
	let axis = |k: usize, t: f32| {
		let (s, c) = (t * 0.5).sin_cos();
		let mut q = [0.0, 0.0, 0.0, c];
		q[k] = s;
		q
	};
	mul(mul(axis(2, ang[2]), axis(1, ang[1])), axis(0, ang[0]))
}



pub const EYELID_JOINTS: [u16; 7] = [15, 16, 17, 18, 19, 20, 21];
pub const EYELID_SCALE: [[f32; 3]; 7] = [[1.0; 3], [1.0; 3], [1.0, 1.0, 0.91], [1.0, 1.0, 0.91], [1.0; 3], [1.0; 3], [0.98, 1.0, 1.0]];
pub const EYELID_OFF: [[f32; 3]; 7] = [
	[210.8818, 85.62652, 652.4599], [210.8818, -85.62652, 652.4599], [782.6689, 518.0051, -520.8357], [782.6689, -513.7591, -520.8357],
	[985.0588, 564.7104, 78.54995], [985.0588, -560.4645, 78.54995], [312.7845, 2.122972, 486.1605],
];
pub const EYELID_W: [f32; 12] = [0.0, 0.25, 0.5, 0.75, 1.0, 1.0, 1.0, 0.8, 0.6, 0.4, 0.2, 0.0];
pub const CLANK_JOINTS: [u16; 4] = [2, 3, 4, 5];
/// Clank blink quaternions in the Rust (conjugate) component convention.
pub const CLANK_Q: [[f32; 4]; 4] = [
	[-0.14938, 0.04963, -0.31572, 0.93568], [-0.16208, 0.05551, -0.31456, 0.93361], [-0.00056, -0.00288, -0.43953, 0.89819], [0.0009, 0.00443, -0.43679, 0.89951],
];
pub const CLANK_OFF: [[f32; 3]; 4] = [[1808.79, 426.97, 1522.16], [1821.16, 410.78, 2293.52], [1808.79, -426.97, 1522.16], [1821.16, -410.78, 2293.52]];
pub const CLANK_W: [f32; 22] = [0.0, 0.2, 0.4, 0.6, 0.7, 0.8, 0.9, 1.0, 1.0, 1.0, 1.0, 1.0, 0.9, 0.8, 0.7, 0.6, 0.5, 0.4, 0.3, 0.2, 0.1, 0.0];



impl Player {


	/// BRIDGE-22: the joint override records of the active blocks of `slot` in block order (slot 0 = hero, 1 = Clank).
	/// HERO-20: every record is keyed by a class attach slot; `map` (the posed model's `Animated::slot_map`) gives the
	/// skeleton joint it drives, and a record whose slot has no joint is dropped.
	pub fn hero_joint_fx(&self, slot: u8, map: &[Option<usize>]) -> Vec<crate::anim::JointFx> {
		let joint = |k: u16| map.get(k as usize).copied().flatten();
		let mut v: Vec<crate::anim::JointFx> = self
			.blocks
			.iter()
			.filter(|b| b.active && b.slot == slot)
			.filter_map(|b| Some(crate::anim::JointFx { joint: joint(b.joint)?, rot: block_quat(b.ang), scale: [b.rec_scale; 3], off: b.off, blend: None, reg: b.reg }))
			.collect();
		if slot == 0 && self.blink != 0 {
			for i in 0..7 {
				if let (Some(reg), Some(j)) = (self.eyelid[i], joint(EYELID_JOINTS[i])) {
					v.push(crate::anim::JointFx { joint: j, rot: [0.0, 0.0, 0.0, 1.0], scale: EYELID_SCALE[i], off: EYELID_OFF[i], blend: Some(EYELID_W[self.blink as usize]), reg });
				}
			}
		}
		if slot == 1 && self.clank_blink != 0 {
			for i in 0..4 {
				if let (Some(reg), Some(j)) = (self.clank_rec[i], joint(CLANK_JOINTS[i])) {
					v.push(crate::anim::JointFx { joint: j, rot: CLANK_Q[i], scale: [1.0; 3], off: CLANK_OFF[i].map(|c| c * self.clank_scale), blend: Some(CLANK_W[self.clank_blink as usize]), reg });
				}
			}
		}
		v.sort_by(|a, b| b.reg.cmp(&a.reg));
		v
	}








	/// HERO-24: the draw-pass view of the two hold layers (weight = the glove layer weight).
	pub fn hold_poses(&self) -> [Option<crate::anim::LayerPose>; 2] {
		[0, 1].map(|i| self.hold[i].map(|h| crate::anim::LayerPose { weight: self.glove_w[i].unwrap_or(0.0), ..h.l.pose() }))
	}













}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn item_rows_and_position() {
		let m = [1.0, 0.0, 0.0, 2.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.7];
		let hp = [0.0, 0.0, 0.7];
		let a = pulse_item(m, 1.5, true, hp);
		assert_eq!((a[0], a[5], a[10]), (1.0, 1.0, 1.0));
		assert!((a[3] - 3.0).abs() < 1e-6 && (a[11] - 2.2).abs() < 1e-6);
		let b = pulse_item(m, 1.5, false, hp);
		assert_eq!((b[0], b[5], b[10]), (1.5, 1.5, 1.0));
	}

	/// HERO-20: the identity attach-slot map (slot k drives joint k).
	fn ident_map() -> Vec<Option<usize>> {
		(0..32).map(Some).collect()
	}

	fn near(a: f32, b: f32) -> bool {
		(a - b).abs() < 1e-6
	}

	fn floor(z: f32) -> Vec<([V3; 3], u8)> {
		let h = 50.0;
		vec![([[-h, -h, z], [h, -h, z], [h, h, z]], 0x2), ([[-h, -h, z], [h, h, z], [-h, h, z]], 0x2)]
	}

	fn wall(x: f32) -> Vec<([V3; 3], u8)> {
		vec![([[x, -10.0, 10.0], [x, 10.0, 60.0], [x, 10.0, 10.0]], 0x2), ([[x, -10.0, 10.0], [x, -10.0, 60.0], [x, 10.0, 60.0]], 0x2)]
	}

	/// A floor plane through (0, 0, z) with unit normal (-sin a, 0, cos a).
	fn tilted(z: f32, a: f32) -> Vec<([V3; 3], u8)> {
		let h = 20.0;
		let t = a.tan();
		let v = |x: f32, y: f32| [x, y, z + t * x];
		vec![([v(-h, -h), v(h, -h), v(h, h)], 0x2), ([v(-h, -h), v(h, h), v(-h, h)], 0x2)]
	}

}
