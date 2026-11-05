//! [PC] Fire, ground fire after explosions (engine/viewer/firefx_blast.rs): the Bomb Glove's thrown bombs that went off this field.
//! A bomb's blast is spark mobys (class 122), glow rings and a light (decomp/notes/bomb_glove.md "3. Explosion"), no blast-type pool
//! sprite (the effect types 5 / 8 of the R.Y.N.O. and Devastator explosions), so the watch of the effects pool never saw it (user
//! 2026-11-04: "bomb glove doesnt leave fire"). The pack hands these to the engine as `fxpool::BlastPoint`s.
//! [C] The bomb is class 121; its update (Veldin FUN_002ad4d8, Novalis the same code) sets the state byte +0x20 to 2 when it goes
//! off on dry ground or in shallow water (a deep-water blast deletes it at once), keeps it in state 2 while the blast grows (+0xbc
//! counts the ticks, 15) and then deletes it; its place +0x10 is the blast's. Class ids and the moby layout are the same on PAL and USA.

use crate::fxpool::BlastPoint;

/// The thrown bomb's class.
const BOMB_CLASS: u32 = 121;
/// Its state byte (+0x20) while the blast lasts.
const BLASTED: u8 = 2;
/// The size handed to the ground fire (firefx_blast.rs `land`: 3 = a fire bed of the default radius). The normal bomb's blast
/// reaches 2.5 m; the gold one (5.5 m) gets the same bed: its level byte is a core address not checked on USA.
const SIZE: f32 = 3.0;

/// The bombs in state 2 among `mobys` (class, address, place); `rd8` reads a byte of the game's RAM. The id is the moby's address,
/// the same for every field of one blast.
pub(super) fn of(mobys: impl Iterator<Item = (u32, u32, [f32; 3])>, rd8: impl Fn(u32) -> u8) -> Vec<BlastPoint> {
	mobys.filter(|&(class, addr, _)| class == BOMB_CLASS && rd8(addr + 0x20) == BLASTED).map(|(_, id, pos)| BlastPoint { id, pos, size: SIZE }).collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn only_bombs_in_their_blast_state_are_blasts() {
		// Moby states by address: a bomb in flight (1), a bomb going off (2), a crate (class 500) whose state byte is 2.
		let state = |a: u32| match a {
			0x1000_20 => 1,
			0x1001_20 => 2,
			0x1002_20 => 2,
			_ => 0,
		};
		let mobys = [(121, 0x1000_00, [0.0; 3]), (121, 0x1001_00, [1.0, 2.0, 3.0]), (500, 0x1002_00, [5.0; 3])];
		assert_eq!(of(mobys.into_iter(), state), vec![BlastPoint { id: 0x1001_00, pos: [1.0, 2.0, 3.0], size: SIZE }]);
	}
}
