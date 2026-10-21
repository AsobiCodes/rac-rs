//! The Taunter (item 14, held class 175), decomp/notes/weapons_framework.md "WPN-6.9 Taunter": the held updater
//! FUN_002cc4a8 (WPN-6.9a: taunt cue cycle, hold repeat, release stop), the alert scan FUN_002cc160 head (WPN-6.9b:
//! the crate knock and the enemy taunt marks; its mine tail is `taunt_mines`, WPN-6.4f2) and the sound-wave rings
//! FUN_002cc930 with their draw callback FUN_002ccb00 (WPN-6.9c). No ammo, no fire dispatch case, no stance call.

use super::*;
use crate::particles::WorldQuad;
use crate::physics::{cross, normalize};

/// The held class (item row 14 word 4).
pub const HELD: u32 = 175;

const RING_START: f32 = 10.0;
const RING_END: f32 = 0.3;
/// The alpha byte trunc(20 * t) per timer with the truncating `t = timer * (1/24)` (nearest rounding differs at 12).
const RING_ALPHA: [u8; 24] = [0, 0, 1, 2, 3, 4, 4, 5, 6, 7, 8, 9, 9, 10, 11, 12, 13, 14, 14, 15, 16, 17, 18, 19];
/// Colour word 0x007f5050 / 128.
const RING_RGB: [f32; 3] = [0.625, 0.625, 0.992_187_5];
/// Quad corners 0x1db780 (y, z) in the ring frame.
const RING_CORNERS: [[f32; 2]; 4] = [[-1.0, 1.0], [-1.0, -1.0], [1.0, 1.0], [1.0, -1.0]];


/// The ten rings 0x1db7c0 {radius, timer, active}, the spawn countdown, life and the flags 0x161484 (idle) and
/// 0x16147c (reset), with the joint-0 origin and wave direction of the last update.
#[derive(Default)]
pub(super) struct Rings {
	slot: [(f32, i16, bool); 10],
	inv_life: f32,
	origin: V3,
	dir: V3,
}


impl Game {




	/// WPN-6.9c: FUN_002ccb00, the ring quads drawn while the Taunter rig exists: each ring sits `radius` along the
	/// wave direction, shrinking from 10.0 to 0.3 and fading by `RING_ALPHA`, additive on fx texture 8.
	/// [I] up = (0,0,1) (gravity 0x13f5e0 is (0,0,-1) on Veldin/Novalis).
	pub fn taunt_ring_quads(&self) -> Vec<WorldQuad> {
		let r = &self.taunt_rings;
		if self.held.as_ref().is_none_or(|h| h.class != HELD) || length(r.dir) == 0.0 {
			return Vec::new();
		}
		let r1 = normalize(cross([0.0, 0.0, 1.0], r.dir));
		let r2 = cross(r.dir, r1);
		r.slot
			.iter()
			.filter(|s| s.2)
			.map(|&(radius, timer, _)| {
				let c = add(r.origin, scale(r.dir, radius));
				let t = (timer as f32 * r.inv_life).min(1.0);
				let s = RING_START + (RING_END - RING_START) * t;
				let a = RING_ALPHA.get(timer.max(0) as usize).copied().unwrap_or(20) as f32 / 128.0;
				let v = RING_CORNERS.map(|[y, z]| add(c, scale(add(scale(r1, y), scale(r2, z)), s)));
				WorldQuad {
					pos: [v[0], v[1], v[3], v[2]],
					uv: [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
					colour: [[RING_RGB[0], RING_RGB[1], RING_RGB[2], a]; 4],
					tex: 8,
					additive: true,
				}
			})
			.collect()
	}
}

