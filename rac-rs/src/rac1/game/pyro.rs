//! The Pyrocitor (gadget 16, held class 176, update FUN_002ccd88) and its flame effects: the held state machine,
//! the type 0xc flame segments (FUN_0027fa50 / FUN_0027fd20), their type 2 sub-bursts (FUN_0027d5b0), the type 0x19
//! embers (FUN_00281f10 / FUN_00282078), the firing light, the class 179 nozzle and the flame hit sweeps.
//! decomp/notes/weapons_framework.md "Pyrocitor", decomp/notes/particles.md "Pyrocitor flame",
//! decomp/notes/collision_and_gameplay.md "Hit classifier". Runs once per 50 Hz game tick with the authored
//! per-tick units (fRam0015ed6c = 1/60, fRam0015ed70 = 1/3600).

use super::Game;
use crate::particles::{pool_release, PoolSlot, Sprite, FX_TEX, PART_TEX_BASE};
use crate::physics::V3;
use std::f32::consts::TAU;

/// The held weapon's moby class and the nozzle moby class.
pub const PYRO_CLASS: u32 = 176;
pub const NOZZLE_CLASS: u32 = 179;
/// The flame effects live in the shared 0x800-slot effect pool (FUN_0027bdb8 -> FUN_00267248): their slots come
/// from `particles::pool_claim`, so capacity is shared with every other effect.
const POOL: usize = crate::particles::CAPACITY;
/// Raw effect size units: 210000 per world unit of half-diagonal; sprites take raw * sqrt2 / 210000.
const RAW: f32 = 210_000.0;
const SIZE_SCALE: f32 = std::f32::consts::SQRT_2 / RAW;

/// FUN_00220960: authored 60 Hz ticks to 50 Hz ticks.
pub(super) use crate::gameplay::pal_ticks as sc;










fn tint(c: [f32; 4]) -> [f32; 4] {
	[(c[0] / 128.0).min(2.0), (c[1] / 128.0).min(2.0), (c[2] / 128.0).min(2.0), (c[3] / 128.0).clamp(0.0, 1.0)]
}

fn toward(target: f32, step: f32, v: f32) -> f32 {
	v + (target - v).clamp(-step, step)
}

/// FUN_002703c8(target, accel2, accel, vmax, &pos, &v): the range smoother. Returns `target - pos` on the snap branch and
/// the new `v` otherwise (ARI-22: the Aridia elevator's voice stops on a 0.0 return).
pub(super) fn approach(target: f32, accel2: f32, accel: f32, vmax: f32, pos: &mut f32, v: &mut f32) -> f32 {
	let d = target - *pos;
	if *v * d < 0.0 || d == 0.0 {
		*v = toward(0.0, accel, *v);
		*pos += *v;
		return *v;
	}
	let stop = *v * *v / accel / 2.0;
	if d.abs() < stop {
		*v = toward(0.0, if stop < d.abs() + v.abs() { accel } else { 1.1 * accel }, *v);
	} else {
		let mut t = (2.0 * accel * d).abs().sqrt().min(vmax);
		if d < 0.0 {
			t = -t;
		}
		*v = toward(t, accel2, *v);
	}
	if d.abs() <= v.abs() {
		*pos = target;
		d
	} else {
		*pos += *v;
		*v
	}
}




/// Particle texture numbers of the flame's effect types (particle def tables 0xc, 0x18 and 0x19; viewer).
#[derive(Clone, Copy, Debug, Default)]
pub struct Frames {
}

#[derive(Clone, Copy, Debug)]
enum Fx {
	/// Type 0xc: `len` +0x30 the range still to travel, colours +4 / +0x34 / +0x38, raw size +0xc, life +10 and
	/// its start +0x3d, rotation +8 and spin +0x3e, flags +0x3f (1 big, 4 from the weapon).
	Seg { pos: V3, vel: V3, len: f32, colour: [f32; 4], c34: [f32; 4], c38: [f32; 4], size: f32, life: i32, life0: i32, rot: u8, spin: i8, flags: u8, tex: u8 },
	/// Type 2: phase 1 moves along `a`, phase 2 blends `a` into `b`, phase 3 drifts along `b`. `b9` is byte +9 (near/far
	/// fade) and `additive` the GS alpha mode (BRIDGE-66: the transport exhaust overrides both).
	Sub { pos: V3, a: V3, b: V3, c28: [f32; 4], c2c: [f32; 4], colour: [f32; 4], size: f32, life: i32, lives: [i32; 3], phase: u8, sizes: [f32; 2], rot: u8, tex: u8, b9: u8, additive: bool },
	/// Type 0x19: colour channels 1.0 = 0x80. `grav` is the velocity w lane: 0 takes the built-in fall step, else it is
	/// added to the z speed each tick (KER-61b grind sparks).
	Ember { pos: V3, vel: V3, rgb: [f32; 3], alpha: f32, size: f32, rot: f32, life: i32, tex: u8, grav: f32 },
}

/// The flame effect pool.
pub(super) struct Flames {
	/// Effect contents by shared-pool index.
	slots: Vec<Option<Fx>>,
	/// The pool claim behind each live slot.
	claims: Vec<Option<PoolSlot>>,
}

impl Default for Flames {
	fn default() -> Self {
		Flames { slots: vec![None; POOL], claims: vec![None; POOL], }
	}
}

impl Drop for Flames {
	fn drop(&mut self) {
		self.clear();
	}
}

impl Flames {

	/// FUN_00267398.
	fn free(&mut self, i: usize) {
		self.slots[i] = None;
		if let Some(c) = self.claims[i].take() {
			pool_release(c);
		}
	}

	pub fn clear(&mut self) {
		for i in 0..self.slots.len() {
			if self.claims[i].is_some() {
				self.free(i);
			}
		}
	}







	/// The live effects as pool sprites (byte +9 = ftoi(4) + 0x40 = 0x44 for all three types).
	pub fn sprites(&self, out: &mut Vec<Sprite>) {
		let tex = |n: u8| FX_TEX + PART_TEX_BASE + n as u32;
		for fx in self.slots.iter().flatten() {
			out.push(match *fx {
				Fx::Seg { pos, colour, size, rot, flags, tex: t, .. } => Sprite { pos, size: size * SIZE_SCALE, rot: rot as f32 * TAU / 256.0, colour: tint(colour), tex: tex(t), additive: flags & 1 != 0, pool: Some(0x44), flat: false },
				// FUN_0027d5b0: the tracked hits pass the def-0xc texture with the additive flag 0x10000 (param 9 = 0x1000c) [I];
				// the ARI-10c shark puffs pass -1 (def 0x17, alpha-over); the ARI-38 exhaust also sets byte +9 = 0x68.
				Fx::Sub { pos, colour, size, rot, tex: t, b9, additive, .. } => Sprite { pos, size: size * SIZE_SCALE, rot: rot as f32 * TAU / 256.0, colour: tint(colour), tex: tex(t), additive, pool: Some(b9), flat: false },
				Fx::Ember { pos, rgb, alpha, size, rot, tex: t, .. } => Sprite { pos, size: size * SIZE_SCALE, rot: rot * TAU / 256.0, colour: [rgb[0], rgb[1], rgb[2], alpha.clamp(0.0, 1.0)], tex: tex(t), additive: true, pool: Some(0x44), flat: false },
			});
		}
	}


}

/// The class 179 nozzle (FUN_002d08f8 / FUN_002d0998): V scroll accumulator, scale factor and Y spin.
#[derive(Clone, Copy, Debug)]
pub struct Nozzle {
	pub scale: f32,
	pub spin: f32,
	pub shown: bool,
	/// The V offset FUN_00263928 has added to the class's texture coordinates so far (4096 = 1.0, 16-bit wrap).
	pub v: i16,
}

/// The held private block and moby state of class 176.
#[derive(Clone, Debug)]
pub(super) struct Pyro {
	/// The weapon moby's animation clock (anim.rs `PropAnim`, stepped once per game tick): current sequence +0x53,
	/// +0x52 = 0xff while fading from a snapshot.
	anim: crate::anim::PropAnim,
	nozzle: Option<Nozzle>,
	/// Loop voice wanted (+0x4a).
	looping: bool,
}

impl Pyro {
	fn new() -> Pyro {
		Pyro {  anim: crate::anim::PropAnim::new(0), nozzle: None,  looping: false,    }
	}


}

impl Game {
	/// Retail mode (retail.rs `sync`): retail's held moby (0x1403e0) of this field drives the viewer's held paths in
	/// place of the game's own weapon updates: `class` is the held moby's class (+0xa6) and `clock` writes its animation
	/// clock (+0x50..+0x5f) into the rig. The Pyrocitor (176) and Suck Cannon (849) take their own rigs, a class with a
	/// generic rig (`held::rig_classes`, timings set by the viewer) the generic one; anything else (the wrench 71, the
	/// Bomb Glove 192, no held moby) clears them, so the viewer draws the wrench or the glove as before. Returns whether
	/// a rig took the held moby (the mirror then leaves it out of the prop pools).
	pub fn mirror_held(&mut self, class: Option<u32>, clock: impl Fn(&mut crate::anim::PropAnim)) -> bool {
		let c = class.unwrap_or(0);
		let pyro = c == PYRO_CLASS && self.pyro_seqs.is_some();
		let suck = c == 849 && self.suck_seqs.is_some();
		let generic = !pyro && !suck && self.held_seqs.contains_key(&c) && super::held::rig_classes().contains(&c);
		if pyro {
			let seqs = self.pyro_seqs.clone();
			let p = self.pyro.get_or_insert_with(|| {
				let mut p = Pyro::new();
				p.anim.attach(seqs);
				p
			});
			clock(&mut p.anim);
		} else {
			self.pyro = None;
		}
		if suck {
			let seqs = self.suck_seqs.clone();
			let s = self.suck.get_or_insert_with(|| {
				let mut s = super::suck::SuckState::new();
				s.anim.attach(seqs);
				s
			});
			clock(&mut s.anim);
		} else {
			self.suck = None;
		}
		if generic {
			if self.held.as_ref().is_none_or(|h| h.class != c) {
				let mut anim = crate::anim::PropAnim::new(0);
				anim.attach(self.held_seqs.get(&c).cloned());
				self.held = Some(super::held::HeldRig { class: c, anim,          swing: Default::default() });
			}
			if let Some(h) = &mut self.held {
				clock(&mut h.anim);
			}
		} else {
			self.held = None;
		}
		pyro || suck || generic
	}
}

impl Game {
	/// Viewer-facing: the weapon's animation clock while the Pyrocitor is held.
	pub fn pyro_pose(&self) -> Option<&crate::anim::PropAnim> {
		self.pyro.as_ref().map(|p| &p.anim)
	}

	/// The nozzle while it exists.
	pub fn pyro_nozzle(&self) -> Option<Nozzle> {
		self.pyro.as_ref().and_then(|p| p.nozzle)
	}


	/// Whether the firing loop (class 176 sound 1, restarted when it ends) should be playing.
	pub fn pyro_loop(&self) -> bool {
		self.pyro.as_ref().is_some_and(|p| p.looping)
	}

	/// The flame effects as sprites.
	pub fn pyro_sprites(&self, out: &mut Vec<Sprite>) {
		self.flames.sprites(out);
	}


	/// The weapon's sequence timings (anim.rs `share_timing`) for its anim clock.
	pub fn set_pyro_timing(&mut self, seqs: std::rc::Rc<Vec<crate::anim::Sequence>>) {
		self.pyro_seqs = Some(seqs);
		if let Some(p) = &mut self.pyro {
			p.anim.attach(self.pyro_seqs.clone());
		}
	}

	/// Particle def frames for the flame (viewer::load).
	pub fn set_pyro_frames(&mut self, f: Frames) {
		self.pyro_frames = f;
	}









}

