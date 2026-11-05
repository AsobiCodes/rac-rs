//! Novalis enemies (decomp/notes/novalis_enemies.md): peckbot 577 (FUN_002ef590), the amoeboid family
//! 572/865/866 (FUN_002ed5d0) and the birdbot 459 (FUN_002e6520) with its shot 722 (FUN_002f9df0).
//! Foes run on the fixed 50 Hz step (PAL dt 0.02): raw per-tick literals (the turn helper FUN_0026cbf0 and the
//! controller FUN_0026d540) are used as is, `dt * k` constants are u/s, scale(n) tick counts are n / 60 s.
//! The launch mover FUN_00270fb0/FUN_002710f0 keeps its own dt-based [P] conversion.

use super::Game;
use crate::physics::V3;

pub const BIRDBOT: u32 = 459;
pub(super) const AMOEBOID: u32 = 572;
pub(super) const AMOEBOID_M: u32 = 865;
pub(super) const AMOEBOID_S: u32 = 866;
pub(super) const PECKBOT: u32 = 577;
pub(super) const SHOT: u32 = 722;
/// Birdbot firing pose: sequence 4, frame 14 = 26 ticks in (frames are 4 ticks apart, numbered 1, 3, 5 ...).
pub const BIRDBOT_FIRE: (usize, f32) = (4, 26.0);

pub fn is_foe(class: u32) -> bool {
	matches!(class, BIRDBOT | AMOEBOID | AMOEBOID_M | AMOEBOID_S | PECKBOT)
}





















/// Birdbot joints with set-B primitives (class 459, decomp/notes/collision_and_gameplay.md "Moby collision
/// shapes"): sphere joint 5 r 1.0, sphere joint 10 r 0.35, capsule joints 13-14 r 0.35.
pub const BIRDBOT_HIT_JOINTS: [usize; 4] = [5, 10, 13, 14];













impl Game {

	/// Attach point 0 of each class in the firing pose (viewer, from the class's skeleton and sequences).
	/// NOV-2.5: each class's attach-point-0 joint, read live from its rig each tick.
	pub fn set_attach_joints(&mut self, m: std::collections::HashMap<u32, usize>) {
		self.attach0 = m;
	}

	pub fn set_muzzles(&mut self, m: std::collections::HashMap<u32, V3>) {
		self.muzzles = m;
	}

	/// Model-space positions of each class's hit joints (BIRDBOT_HIT_JOINTS) in the firing pose (viewer).
	pub fn set_hit_joints(&mut self, m: std::collections::HashMap<u32, Vec<V3>>) {
		self.hit_joints = m;
	}



	/// Per-class sequence timings (anim.rs `share_timing`): every prop of a class gets them for its clock (viewer::load).
	pub fn set_sequences(&mut self, seqs: std::collections::HashMap<u32, std::rc::Rc<Vec<crate::anim::Sequence>>>) {
		self.portrait_timing = seqs.get(&super::vendor::PORTRAIT_CLASS).cloned(); // UI-5a: the injected pack timings
		for p in self.props.iter_mut().chain(self.placed.iter_mut()) {
			p.anim.attach(seqs.get(&p.class_id).cloned());
			// REN-13: the retail initial clock of FUN_00263020 (rate 1.0, next 0); the first tick only moves next to 1.
			// ARI-26 / ARI-27a: the Aridia machinery and the rocket launcher run the same pre-update.
			if matches!(p.kind, super::Kind::Scenery | super::Kind::Retail { .. }) {
				p.anim.clock.r = 1.0;
				p.anim.clock.next = 0;
			}
		}
	}








}








































