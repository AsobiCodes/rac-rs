//! Blaster effect types (decomp/notes/weapons_framework.md WPN-6.1a / 6.1b): the type-0x1b fading spark (ctor
//! FUN_00282698, update FUN_00282778) and the type-0x48 beam sprite (ctor FUN_00289d08, update FUN_00289e90), and the Glove of Doom capsule glow type 0x20 (WPN-6.5e2) and flash5 type 5 (WPN-6.5e3).

use super::*;

/// Type 0x1b fading spark: drifts by `vel` and fades its alpha from `a0` to 0 over `life` ticks.
#[derive(Clone, Debug)]
pub struct Fade27 {
	pub pos: [f32; 3],
	pub size: f32,
	/// Colour word (+4); its alpha byte is rewritten by the update.
	pub rgba: u32,
	pub rot: u8,
}


/// Type 0x48 beam sprite: placed by its owner every tick, spinning by `spin`; it frees itself only at size <= 0.
/// `shrink`, `mode` and `extra` are ctor writes, write-only in retail (no reader: updater FUN_00289e90, draw FUN_0027c190
/// mode 0); the rand(30) draws that pick `extra` stay for the RNG order (WPN-6.1e4).
#[derive(Clone, Debug)]
pub struct Beam48 {
	pub pos: [f32; 3],
	pub size: f32,
	pub rgba: u32,
	/// +8.
	pub rot: u8,
	/// +0x38, added to `rot` every tick.
	pub spin: i8,
	/// +0x3c.
	pub shrink: f32,
	/// +0x30, the creator's mode argument.
	pub mode: i32,
	/// +0x34: -40, -1 or 0.
	pub extra: i32,
}


impl Particles {



	/// Live (type-0x1b sparks, type-0x48 beams, type-0x1a trails).
	#[cfg(test)]
	pub fn blaster_fx_counts(&self) -> (usize, usize, usize) {
		let n = |f: fn(&Effect) -> bool| self.slots.iter().flatten().filter(|e| f(e)).count();
		(n(|e| matches!(e, Effect::Fade27(_))), n(|e| matches!(e, Effect::Beam48(_))), n(|e| matches!(e, Effect::Trail(_))))
	}

}



/// WPN-6.5e2: type 0x20 capsule glow (ctor FUN_002836a0, update FUN_002837c0): rides its owner, an alpha triangle up to
/// 0.6 and back, the colour fading to orange, its size following the owner's absolute scale (BRIDGE-53).
#[derive(Clone, Debug)]
pub struct Glow20 {
	pub pos: [f32; 3],
	pub size: f32,
	pub rgba: u32,
	/// +8 = trunc(rot * 255).
	pub rot_byte: u8,
}


/// WPN-6.5e3: type 5 "flash5" (ctor FUN_0027e068, update FUN_0027e168): a still sprite growing by `rate` per tick, its
/// alpha fading over the last pal_ticks(6) ticks.
#[derive(Clone, Debug)]
pub struct Flash5 {
	pub pos: [f32; 3],
	pub size: f32,
	pub rgba: u32,
}


