//! Aridia props with their own break or flight updaters (decomp/notes/aridia_environment.md): ARI-35 white flowers
//! (class 1819, FUN_002f1890) and ARI-38 flying transports (1212 / 1213, FUN_002ebde8 / FUN_002ec748, BRIDGE-66).
//! The actor variants live in `aridia::Actor`; `Game::update_aridia` dispatches here.

use super::aridia::TRANSPORTS;
use super::Game;
use crate::physics::V3;


const GLOW_HALF: [f32; 2] = [f32::from_bits(0x3f66_6666), f32::from_bits(0x3f99_999a)];
const GLOW_BIAS: [f32; 2] = [f32::from_bits(0x3ee6_6666), f32::from_bits(0x3f11_eb85)];

thread_local! {
}




impl Game {





	/// ARI-38 / BRIDGE-66: the glow quads (FUN_002646e8 = `particles::glow`) the draw callbacks FUN_002ebc38 /
	/// FUN_002ec598 emit for the drawn transports whose colour low byte is above 0x80: (joint, word, half-size, bias).
	pub fn transport_glows(&self) -> Vec<(V3, u32, f32, f32)> {
		let mut out = Vec::new();
		let mut keys: Vec<_> = self.transport_glows.keys().copied().collect();
		keys.sort_unstable();
		for i in keys {
			let p = &self.props[i];
			let (joints, word) = &self.transport_glows[&i];
			if !p.shown || p.sched.over == 0 || word & 0xff <= 0x80 {
				continue;
			}
			let k = usize::from(p.class_id != TRANSPORTS[0]);
			out.extend(joints.iter().map(|&at| (at, *word, GLOW_HALF[k], GLOW_BIAS[k])));
		}
		out
	}
}







