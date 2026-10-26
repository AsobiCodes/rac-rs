//! The render side of the pose overrides' snapshots, debug dots and joint reads (omni/pose_capture.rs): once per frame, only while a mod asked for one.

use super::*;

impl App {
	/// Captures the base local rotations of the hero for the waiting `pose_snapshot` / `hold` requests (the game's pose: retail's joint records and the
	/// leg twist, never a mod's pose ops); while `pose_debug` is on, publishes the world position of every joint of the DRAWN pose; while a mod reads
	/// joints (`pose_read`), publishes the world frame of every joint of the GAME pose (no mod ops). The worker sees these one frame late.
	/// Three atomic loads when nothing is asked.
	pub(super) fn pose_service(&self) {
		use crate::omni::pose_capture as cap;
		let (snap, dots, reads) = (cap::pending(), cap::debug_on(), cap::reads_on());
		if !snap && !dots && !reads {
			return;
		}
		let rparts = self.retail.as_ref().and(self.retail_parts.as_ref());
		let (Some((_, a)), Some(q), Some(game)) = (self.hero_anim.as_ref(), rparts, self.game.as_ref()) else {
			if dots {
				cap::publish_joints(Vec::new());
			}
			if reads {
				cap::publish_world(Vec::new(), [0.0; 3]);
			}
			return;
		};
		let mut fx = q.hero_fx.clone();
		crate::retail::strafe::leg_fx(&mut fx, q.strafe);
		if snap {
			let parts = self.hero_rig.hero.local_held(a, &fx, None, None, &[None, None]);
			cap::service(&parts.iter().map(|p| p.0).collect::<Vec<_>>());
		}
		let (p, rows) = (&game.player, hero_rows(&game.player));
		let posed = |fx: &[anim::JointFx]| {
			let mut mats = self.hero_rig.hero.pose_held(a, fx, None, None, &[None, None]);
			crate::retail::strafe::twist_mats(&mut mats, a, q.strafe.twist);
			mats
		};
		if reads {
			let mats = posed(&fx);
			let world = (0..mats.len()).map(|j| {
				let w = a.joint_world(&mats, j);
				// the joint's axes (columns of the model-space 3x3) and origin, through the hero rows into the game world
				let axis = |k: usize| hero_to_world(&rows, [w[k], w[4 + k], w[8 + k]]);
				let o = hero_to_world(&rows, [w[3], w[7], w[11]]);
				let (x, y, z) = (axis(0), axis(1), axis(2));
				[x[0], x[1], x[2], y[0], y[1], y[2], z[0], z[1], z[2], p.pos[0] + o[0], p.pos[1] + o[1], p.pos[2] + o[2]]
			});
			cap::publish_world(world.collect(), p.pos);
		}
		if dots {
			fx.extend_from_slice(&q.pose_ops);
			let mats = posed(&fx);
			let world = (0..mats.len()).map(|j| {
				let w = a.joint_world(&mats, j);
				let v = hero_to_world(&rows, [w[3], w[7], w[11]]);
				[p.pos[0] + v[0], p.pos[1] + v[1], p.pos[2] + v[2]]
			});
			cap::publish_joints(world.collect());
		}
	}
}
