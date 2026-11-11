//! [PC] Fire, ground fire after explosions (engine/viewer/firefx_blast.rs): only the PLAYER's explosive weapons leave a ground fire
//! (user 2026-11-05: "sometimes enemies leave fire when they die and explode which shouldnt happen") and become simulated fire (user
//! 2026-11-07: "fire is only reserved for weapons explosions"). The explosion pool sprites (effect types 5 / 8, rac1/fire_look.rs
//! `FireKind::Blast`) cannot tell who exploded [C: the type-8 burst ctor, Novalis FUN_0027ebc8, keeps the velocity at slot +0x20, no
//! owner; its callers are the generic explosion FUN_00272ea8 (enemy deaths, the Glove of Doom agents, ...), the Devastator impact
//! FUN_002c54a0 and FUN_002f6360]. So the pack hands the engine where the player's explosive shots are (`fxpool::ShotZone`) and the
//! engine takes a blast sprite only when it is born near the place where one VANISHED a moment ago, never near one that still lives.
//! A shot "vanishes" from this list in the tick it explodes: most delete themselves then; the others drop out of the list by their
//! state (`of`). The Bomb Glove's own blast has no such sprite (bomb_blast.rs) but the bomb counts here too, so an enemy killed in its
//! blast merges into its patch. Facts: decomp/notes/particles.md "Who exploded".
//! Classes [C: decomp/notes/omni_weapons.md "Classes"; the same ids on every level, PAL and USA]: bomb 121, mine 74, R.Y.N.O. rocket 457,
//! Visibomb missile 172, Glove of Doom agent 186, Devastator rocket 153 (a gold rocket explodes at each bounce and lives on: every
//! bounce counts as a vanished shot), Drone Device drone 479, Decoy 203 / gold Decoy 1900, and every Suck Cannon victim while it is
//! flung. Not here: the Glove of Doom capsule 230 (it opens, no explosion), the Blaster shot 305, the Walloper / Taunter / Tesla Claw /
//! Morph-o-Ray (no explosion of their own: an enemy they kill dies in its own death explosion, retail), crates (not a weapon).

use crate::fxpool::ShotZone;

/// The bomb waiting in the Bomb Glove: class 121 with state byte +0x20 == 0 [C: retail/strafe.rs `turn_parts`]; it is no shot yet.
const BOMB_CLASS: u32 = 121;
/// The Devastator rocket: its bounce count is pvar +0x40 (i32) [C omni_weapons.md "Devastator rocket 153"].
const DEVASTATOR_CLASS: u32 = 153;
/// The Drone Device drone [C Novalis UpdateMoby_479 0x2e8be8]: it explodes (FUN_00272ea8) and enters state 4 in the same tick, then
/// fades out (alpha +0x23 -4 per tick) and deletes itself in state 5 about a second later. A drone counts while its state is < 4.
const DRONE_CLASS: u32 = 479;
const DRONE_GONE: u8 = 4;
/// The Decoy (203) and the gold Decoy (1900) [C Novalis 0x2d8b58 allocator, 0x2d9918 update]: health < 0 explodes it (FUN_00272ea8,
/// gold: also a blast hit) and deletes it (FUN_00263258) in the same tick.
const DECOY_CLASSES: [u32; 2] = [203, 1900];
/// A Suck Cannon victim [C Novalis 0x303dd0 fire, 0x304b90 flight, 0x304ad8 release]: its suck component state (u16 +0x68) is 5 or 6
/// while it flies after the shot; the flight ends in 0x304ad8(victim, comp, 1): the explosion 0x3040c8 (gold: on impact; every
/// victim: at the end of the flight), state 0 and the class's own end. The component is found per class: class record +0x2c ->
/// table +0x10 = the lookup `comp = pvar + K` (raw words `beq a0,zero; nop; lw v0,0x78(a0); bne v0,zero; addiu v0,v0,K`: Amoeboid
/// 0x2ef4f8 K 0xc0, Peckbot 0x2f1728 K 0x60). A class with another lookup (the Morph-o-Ray sheep 270) is not read [?].
const SUCK_FLUNG: [u16; 2] = [5, 6];
const SUCK_REACH: f32 = 3.0;
/// (class, reach in m): how far from where a shot vanished an explosion sprite may be born and still count as its explosion. Generous:
/// the shot was last seen up to a field before its blast (a rocket moves well under a metre per field) and a victim of the blast
/// explodes beside it. The engine opens a zone only when a shot VANISHES (a mine waiting in the glove, a laid mine, a Doom agent or a
/// Visibomb flying by never make a zone while they live). A mine or bomb that is put away or laid may open a short zone at the hand.
const SHOTS: &[(u32, f32)] = &[
	(BOMB_CLASS, 3.5),
	(74, 3.0),
	(457, 3.0),
	(172, 3.0),
	(186, 3.0),
	(DEVASTATOR_CLASS, 3.0),
	(DRONE_CLASS, 3.0),
	(DECOY_CLASSES[0], 3.0),
	(DECOY_CLASSES[1], 3.0),
];

/// The game's RAM as the gate reads it (the machine; a sparse map in the tests).
pub(super) trait Ram {
	fn r8(&self, a: u32) -> u8;
	fn r16(&self, a: u32) -> u16;
	fn r32(&self, a: u32) -> u32;
}

impl Ram for ee_oracle::Machine {
	fn r8(&self, a: u32) -> u8 {
		self.rd8(a)
	}
	fn r16(&self, a: u32) -> u16 {
		self.rd16(a)
	}
	fn r32(&self, a: u32) -> u32 {
		self.rd32(a)
	}
}

/// A pointer word read from a moby or class record: Some(address) when it points into the 32 MB of main RAM, word aligned.
fn ptr(w: u32) -> Option<u32> {
	let a = w & 0x01ff_ffff;
	(a >= 0x0010_0000 && a % 4 == 0).then_some(a)
}

/// The suck component of the moby at `addr` when its class looks it up as `pvar + K` (see `SUCK_FLUNG`).
fn suck_comp(addr: u32, ram: &impl Ram) -> Option<u32> {
	let class = ptr(ram.r32(addr + 0x24))?;
	let table = ptr(ram.r32(class + 0x2c))?;
	let f = ptr(ram.r32(table + 0x10))?;
	let pvar_load = ram.r32(f + 8) == 0x8c82_0078 && ram.r32(f + 12) >> 16 == 0x1440;
	let add = ram.r32(f + 16);
	if !pvar_load || add >> 16 != 0x2442 {
		return None;
	}
	Some(ptr(ram.r32(addr + 0x78))? + (add & 0xffff))
}

/// The player's explosive shots among `mobys` (class, address, place) of this field. The id is the moby's address (a Devastator
/// rocket's changes at each bounce), the kind its class.
pub(super) fn of(mobys: impl Iterator<Item = (u32, u32, [f32; 3])>, ram: &impl Ram) -> Vec<ShotZone> {
	mobys
		.filter_map(|(class, addr, pos)| {
			let Some(&(_, reach)) = SHOTS.iter().find(|s| s.0 == class) else {
				let flung = suck_comp(addr, ram).is_some_and(|c| SUCK_FLUNG.contains(&ram.r16(c + 0x68)));
				return flung.then_some(ShotZone { id: addr, kind: class, pos, reach: SUCK_REACH });
			};
			let state = ram.r8(addr + 0x20);
			let gone = match class {
				BOMB_CLASS => state == 0,
				DRONE_CLASS => state >= DRONE_GONE,
				_ => false,
			};
			let id = match class {
				DEVASTATOR_CLASS => addr ^ (ptr(ram.r32(addr + 0x78)).map_or(0, |p| ram.r32(p + 0x40) & 7) << 25),
				_ => addr,
			};
			(!gone).then_some(ShotZone { id, kind: class, pos, reach })
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::collections::HashMap;

	#[derive(Default)]
	struct Fake(HashMap<u32, u8>);

	impl Fake {
		fn w8(&mut self, a: u32, v: u8) {
			self.0.insert(a, v);
		}
		fn w16(&mut self, a: u32, v: u16) {
			v.to_le_bytes().iter().enumerate().for_each(|(i, &b)| self.w8(a + i as u32, b));
		}
		fn w32(&mut self, a: u32, v: u32) {
			v.to_le_bytes().iter().enumerate().for_each(|(i, &b)| self.w8(a + i as u32, b));
		}
	}

	impl Ram for Fake {
		fn r8(&self, a: u32) -> u8 {
			self.0.get(&a).copied().unwrap_or(0)
		}
		fn r16(&self, a: u32) -> u16 {
			u16::from_le_bytes([self.r8(a), self.r8(a + 1)])
		}
		fn r32(&self, a: u32) -> u32 {
			u32::from_le_bytes([self.r8(a), self.r8(a + 1), self.r8(a + 2), self.r8(a + 3)])
		}
	}

	fn ids(z: &[ShotZone]) -> Vec<u32> {
		z.iter().map(|z| z.id).collect()
	}

	#[test]
	fn only_thrown_player_shots_are_zones() {
		let mut ram = Fake::default();
		ram.w8(0x1001_20, 1); // the thrown bomb flies (state 1); the bomb in the glove is state 0
		let mobys = [
			(121, 0x1000_00, [0.3, 0.0, 0.8]), // the bomb in the glove
			(121, 0x1001_00, [6.0, 0.0, 0.0]), // a thrown bomb
			(74, 0x1002_00, [0.5, 0.2, 0.7]),  // a mine (in the glove or laid: it only counts when it vanishes)
			(153, 0x1004_00, [0.5, 0.0, 1.0]), // a Devastator rocket just fired
			(457, 0x1005_00, [3.0, 3.0, 1.0]),
			(172, 0x1006_00, [4.0, 3.0, 1.0]),
			(186, 0x1007_00, [5.0, 3.0, 1.0]),
			(230, 0x1008_00, [6.0, 3.0, 1.0]), // the Doom capsule: no explosion
			(305, 0x1009_00, [7.0, 3.0, 1.0]), // a Blaster bolt
			(573, 0x100a_00, [8.0, 3.0, 1.0]), // an enemy
		];
		let z = of(mobys.into_iter(), &ram);
		assert_eq!(ids(&z), vec![0x1001_00, 0x1002_00, 0x1004_00, 0x1005_00, 0x1006_00, 0x1007_00]);
		assert!(z.iter().all(|z| z.reach >= 3.0) && z[1].kind == 74);
	}

	/// Lane rfire worker 12 (review finding 4): the Drone Device, the Decoy Glove (plain and gold) and the Suck Cannon's flung victims
	/// are player weapons too; each must drop out of the list in the tick its explosion is born.
	#[test]
	fn drones_decoys_and_flung_victims_vanish_when_they_explode() {
		let mut ram = Fake::default();
		ram.w8(0x2000_20, 3); // a drone circling its target
		ram.w8(0x2001_20, 4); // a drone that exploded this tick (fading out until state 5 deletes it)
		// A Peckbot-like class whose suck lookup is pvar + 0x60, and one with another lookup (no component read).
		let (class, table, f) = (0x30_0000, 0x30_1000, 0x30_2000);
		for (k, w) in [0x1080_0006u32, 0, 0x8c82_0078, 0x1440_0005, 0x2442_0060, 0x03e0_0008].iter().enumerate() {
			ram.w32(f + 4 * k as u32, *w);
		}
		ram.w32(table + 0x10, f);
		ram.w32(class + 0x2c, table);
		let (other, otable, of_) = (0x31_0000, 0x31_1000, 0x31_2000);
		ram.w32(of_ + 8, 0x8c82_0010);
		ram.w32(otable + 0x10, of_);
		ram.w32(other + 0x2c, otable);
		for (i, (cls, state)) in [(class, 6u16), (class, 3), (other, 6)].into_iter().enumerate() {
			let (m, pvar) = (0x2100_00 + 0x100 * i as u32, 0x40_0000 + 0x1000 * i as u32);
			ram.w32(m + 0x24, cls | 0x8000_0000);
			ram.w32(m + 0x78, pvar);
			ram.w16(pvar + 0x60 + 0x68, state);
		}
		let mobys = [
			(479, 0x2000_00, [1.0, 0.0, 0.0]),
			(479, 0x2001_00, [2.0, 0.0, 0.0]),
			(203, 0x2002_00, [3.0, 0.0, 0.0]),  // a Decoy
			(1900, 0x2003_00, [4.0, 0.0, 0.0]), // a gold Decoy
			(577, 0x2100_00, [5.0, 0.0, 0.0]),  // flung (component state 6)
			(577, 0x2101_00, [6.0, 0.0, 0.0]),  // captured, not flung (state 3)
			(270, 0x2102_00, [7.0, 0.0, 0.0]),  // another lookup: not read
		];
		let z = of(mobys.into_iter(), &ram);
		assert_eq!(ids(&z), vec![0x2000_00, 0x2002_00, 0x2003_00, 0x2100_00]);
		assert_eq!(z[3].kind, 577);
	}

	#[test]
	fn a_devastator_bounce_is_a_new_shot() {
		let mut ram = Fake::default();
		ram.w32(0x5000_78, 0x0050_1000);
		let before = of([(153, 0x5000_00, [0.0; 3])].into_iter(), &ram);
		ram.w32(0x50_1040, 1); // the gold rocket bounced (exploded) and flies on
		let after = of([(153, 0x5000_00, [0.0; 3])].into_iter(), &ram);
		assert_ne!(before[0].id, after[0].id, "the old id vanishes: a zone opens where it exploded");
		assert_eq!(before[0].id, 0x5000_00);
	}
}
