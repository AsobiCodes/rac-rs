//! KER-71: the class 1221 debris piece (decomp/notes/kerwan_environment.md "KER-71 port spec"), updater
//! FUN_002ddca0: it falls under gravity, spins on all three axes, fades with its remaining life and despawns when the
//! life runs out. No placement and no spawner yet: KER-77 owns the allocator that seeds it.

use crate::physics::V3;

/// KER-71: the debris class (0x4c5).
pub(in crate::game) const DEBRIS_1221: u32 = 1221;

/// KER-71: the piece's pvar block: per-tick velocity (+0x0, z = +0x8), per-tick spin rates x/y/z (+0x10/+0x14/+0x18),
/// the life it started with (+0x20) and the life left (+0x24); `alpha` is the moby alpha byte +0x23.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(in crate::game) struct Debris1221 {
	pub(in crate::game) vel: V3,
	pub(in crate::game) spin: V3,
	pub(in crate::game) max: i32,
	pub(in crate::game) life: i32,
	pub(in crate::game) alpha: u8,
}

impl Debris1221 {
	/// KER-77's seeding of a claimed piece. The alpha byte is rewritten by every update before the draw.
	pub(in crate::game) fn seed(vel: V3, spin: V3, life: i32, max: i32) -> Debris1221 {
		Debris1221 { vel, spin, max, life, alpha: 0x80 }
	}
}


