
/// Type 0x0f ember (spawn FUN_002804e8, update FUN_00280620). Colour runs `c1` -> `c0` as life ends.
/// With `trail` set (flag word nonzero) every odd global tick skips the drag and drops a trail ember.
#[derive(Clone, Debug)]
pub struct BlastEmber {
	pub pos: [f32; 3],
	pub size: f32,
	pub(super) colour: [f32; 4],
	pub(super) rot: u8,
	pub(super) additive: bool,
	/// The Sprite texture code (byte +2 of FUN_002804e8; `TEX_BLAST_EMBER` unless a caller overrides it, WPN-6.4e2).
	pub(super) tex: u32,
}


/// Type 0x40 water debris (ctor FUN_002886a8 OV:83852-83917, updater FUN_00288828 OV:83918-84077; WPN-6.4e2,
/// decomp/notes/weapons_framework.md): falls under `grav` per tick and, on going below `floor`, bounces into one
/// trail-less ember (big bounce 1 in 14) and is freed. Mode 0 point billboard, colour fixed at `c0`.
#[derive(Clone, Debug)]
pub struct WaterDebris {
	pub pos: [f32; 3],
	pub(super) size: f32,
	pub(super) rot: u8,
	pub(super) b9: u8,
	pub(super) c0: u32,
	pub(super) tex: u32,
	pub(super) additive: bool,
}


/// Type 0x0b spark (Novalis spawn FUN_0027f210 / update FUN_0027f498; Veldin FUN_0026a380 / FUN_0026a608, the
/// same but for phase B, see `veldin`). Phase A grows and drags with no gravity (children at 85% of life, up to
/// depth 2), phase B (flag bit 0) is a rising fade-out; `additive` is blend 0x48 (the others 0x44).
/// decomp/notes/effects_lighting_mapping.md §1, particles.md.
#[derive(Clone, Debug)]
pub struct BlastSpark {
	pub pos: [f32; 3],
	pub(super) colour: [f32; 4],
	pub(super) size: f32,
	pub(super) rot: u8,
	pub(super) additive: bool,
}


/// Type 8 flame (spawn FUN_0027ebc8, update FUN_0027ecb8): grows linearly from 0 to `size` over its life, steps
/// through the 10 frames of def 8, fades its alpha over the last sc(6) ticks.
#[derive(Clone, Debug)]
pub struct Flame {
	pub pos: [f32; 3],
	pub(super) size: f32,
	pub(super) colour: [f32; 4],
	pub(super) frame: usize,
	pub(super) rot: u8,
}


/// Type 4 smoke puff (spawn FUN_0027de50, update FUN_0027df68): size (b (life0 - life) / life0 + a) * 1000.
#[derive(Clone, Debug)]
pub struct Smoke {
	pub pos: [f32; 3],
	pub(super) size: f32,
	pub(super) colour: [f32; 4],
	pub(super) rot: u8,
	pub(super) additive: bool,
}


/// Type 0x16 lofting smoke (creator FUN_00281848, update 0x27c008; NOV-9.3): grows from 0.2 size by `step` a tick,
/// rises with buoyancy `buoy` (1.24 * dt2) added to vel.z, colour c1 -> c0 by remaining life.
#[derive(Clone, Debug)]
pub struct SmokeLoft {
	pub pos: [f32; 3],
	pub size: f32,
	pub(super) rot: u8,
	pub colour: [f32; 4],
}



/// Type 0x10 puff (ctor FUN_00280848, updater 0x280a00; NOV-15.2). Modes 0..2 fall with their own gravity and die
/// below `floor_z`; mode 1 spawns a mode-3 child where it lands; mode 3 is the child (slow fall, fast fade).
#[derive(Clone, Debug)]
pub struct Puff10 {
	pub pos: [f32; 3],
	pub size: f32,
	pub(super) colour: [f32; 4],
	pub(super) rot: u8,
	pub(super) tex: u8,
}


