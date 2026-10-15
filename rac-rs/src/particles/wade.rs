//! Novalis hero wading effects (decomp/notes/particles.md "Water effects", hero_swim.md "Wading"): wake
//! sprites 0x2e (FUN_00286380 / 002864a0), ripple puffs 0x2d (FUN_00286098 / 002861f8), splash drops 0x23
//! (FUN_00283ec0 / 00283fd8, modes 0-3, NOV-20.2a), dust 0x42 (FUN_00288f60 / 002890c8) and bubbles 0x22 (FUN_002839f8 / 00283bf8). Tick units are the
//! retail per-tick ones (dt = 0.02). Each type draws with its own def's texture (DAT_001b2508/250c/2534/2538,
//! PAR-7). The hero's swim/slide/crouch call sites (PAR-7) come from `Player::water_fx` as `WaterFx` calls.

use super::*;

const B9_BUBBLE: u8 = 0x44;
const B9_RIPPLE: u8 = 0x22;
const B9_DROP: u8 = 0x12;
/// Effect type 0x23: drops draw with def 0x23's texture (`Level::particle_def_textures`).
const TEX_DROP: u32 = 0x23;
/// PAR-7: bubbles, ripples and wakes draw with defs 0x22 / 0x2d / 0x2e (*DAT_001b2508 / 2534 / 2538).
const TEX_BUBBLE: u32 = 0x22;
const TEX_RIPPLE: u32 = 0x2d;
const TEX_WAKE: u32 = 0x2e;


/// Type 0x22 bubble (alpha-over, 0x88a0a0a0).
#[derive(Clone, Debug)]
pub struct Bubble {
	pos: [f32; 3],
	size: f32,
	alpha: i32,
	rot: u8,
}


/// Type 0x23 splash drop (FUN_00283ec0 / FUN_00283fd8). Mode 0 free flight, 1 water-hit ripple, 2 solid-hit dust,
/// 3 falls to the hero water plane (fRam0013f640, read live) and ripples.
#[derive(Clone, Debug)]
pub struct Drop {
	pos: [f32; 3],
	size: f32,
	alpha: i32,
	rot: u8,
}


/// Type 0x2d ripple puff.
#[derive(Clone, Debug)]
pub struct Ripple {
	pos: [f32; 3],
	size: f32,
	alpha: i32,
	rot: u8,
}


/// Type 0x42 dust puff (FUN_00288f60 / FUN_002890c8), the mode-2 drop's solid hit.
#[derive(Clone, Debug)]
pub struct Dust {
	pub(super) pos: [f32; 3],
	pub(super) size: f32,
	pub(super) alpha: i32,
	pub(super) rot: u8,
	/// Def 0x42 frame: 1 when the `below(256)` draw is even, else 0.
	pub(super) frame: usize,
}


/// Type 0x2e wake sprite.
#[derive(Clone, Debug)]
pub struct Wake {
	pos: [f32; 3],
	size: f32,
	ang: f32,
	alpha: i32,
}






impl Particles {











	pub(super) fn wade_sprites(e: &Effect, out: &mut Vec<Sprite>) {
		let spr = |pos, size: f32, rot: u8, rgb: f32, alpha: i32, additive, b9, flat| Sprite {
			pos,
			size: size * SIZE_SCALE,
			rot: rot as f32 * TAU / 256.0,
			colour: [rgb / FULL, rgb / FULL, rgb / FULL, (alpha as f32 / FULL).clamp(0.0, 1.0)],
			tex: TEX_PUFF,
			additive,
			pool: Some(b9),
			flat,
		};
		match e {
			Effect::Bubble(b) => out.push(Sprite { tex: TEX_BUBBLE, ..spr(b.pos, b.size, b.rot, 160.0, b.alpha, false, B9_BUBBLE, false) }),
			Effect::Drop(d) => out.push(Sprite { tex: TEX_DROP, ..spr(d.pos, d.size, d.rot, 128.0, d.alpha, false, B9_DROP, false) }),
			Effect::Ripple(r) => out.push(Sprite { tex: TEX_RIPPLE, ..spr(r.pos, r.size, r.rot, 128.0, r.alpha, true, B9_RIPPLE, true) }),
			Effect::Wake(w) => out.push(Sprite { tex: TEX_WAKE, ..spr(w.pos, w.size, w.ang as i32 as u8, 128.0, w.alpha, true, B9_RIPPLE, true) }),
			_ => {}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::physics::Hit;

	fn wakes(p: &Particles) -> Vec<&Wake> {
		p.slots.iter().flatten().filter_map(|e| if let Effect::Wake(w) = e { Some(w) } else { None }).collect()
	}

	fn drops(p: &Particles) -> Vec<&Drop> {
		p.slots.iter().flatten().filter_map(|e| if let Effect::Drop(d) = e { Some(d) } else { None }).collect()
	}

	fn ripples(p: &Particles) -> Vec<&Ripple> {
		p.slots.iter().flatten().filter_map(|e| if let Effect::Ripple(r) = e { Some(r) } else { None }).collect()
	}

	fn dusts(p: &Particles) -> Vec<&Dust> {
		p.slots.iter().flatten().filter_map(|e| if let Effect::Dust(d) = e { Some(d) } else { None }).collect()
	}

	// NOV-20.2a test 1: life 3, then 0x80 -> 0x00 over 8 fade ticks, freed on the 9th; gravity each tick.
	// NOV-20.2a tests 2, 3: mode 1 water hit ripples at the hit point, a solid hit frees silently, no hit moves.
	// NOV-20.2a test 5: mode 2 solid hit spawns one dust (timer 4, size 0.25 * 210000) at z + 0.05.
	// NOV-20.2a test 6: depth 0.3 builds mode 1, 0.1 mode 3, both at water z + 0.1.
	// HERO-3b test 1: the drop direction is the 12-bit angle draw through the VU0 sine / cosine.
	// HERO-3b test 2: the angle quantum (q - 0x800) * pi / 2048.
	#[test]
	fn angle_quantum_values() {
		assert_eq!((3072 - 2048) as f32 * std::f32::consts::PI / 2048.0, 1.5707964);
		assert_eq!((0 - 2048) as f32 * std::f32::consts::PI / 2048.0, -3.1415927);
	}

	// NOV-20.2 Bubble constructor: six draws in order (max, rot, timer, angle, turn, wobble); a full pool draws none.
	// Water-height input: mode 3 reads the hero water z live; mode 0 never does.
}
