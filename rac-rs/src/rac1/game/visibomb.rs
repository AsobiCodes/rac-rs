//! WPN-6.3a: the held Visibomb (class 163) updater, the class-172 missile creator, the render snapshot / restore and
//! the cleanup (decomp/notes/weapons_framework.md "WPN-6.3a port spec"); WPN-6.3b: the class-172 flight updater
//! FUN_002cb6d8 and the aux 179 pose FUN_002cb138 ("WPN-6.3b port spec"), with the BRIDGE-48 draw props and the
//! BRIDGE-49 loop bend; WPN-6.3c: the impact / detonation `visibomb_detonate` and the burst; WPN-6.3d: the class-6
//! remote camera `VisCam` (create, actor-side update, camera frame, return) with the BRIDGE-56 roll; WPN-6.3e: the aux
//! 179 update FUN_002d0998 `update_visibomb_aux`.

use super::*;

/// The missile class and its aux class (BRIDGE-48 draw props).
pub const MISSILE: u32 = 172;
pub const AUX: u32 = 179;


/// AUD-11e / BRIDGE-49: the `Event::SoundLoop` / `Event::LoopBend` owner of the missile's loop voice (one missile).
pub const VISIBOMB_LOOP_OWNER: usize = HERO_LOOP_OWNER - 1;


/// The held Visibomb class.
pub const HELD: u32 = 163;


/// The fog words the render snapshot keeps (missile pvar +0x30..): near, far, the two GS fog coefficients and the colour.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RenderSave {
	pub fog_near: f32,
	pub fog_far: f32,
	pub fog_i0: f32,
	pub fog_i1: f32,
	pub fog_rgb: [u8; 3],
}



/// The render words the Visibomb writes (fog 0x15f444..0x15f454, clear colour, draw caps, 0x16a3f8, occlusion mode
/// 0x15f5e8). The fog words live in the viewer: it feeds `fog_seen` before each tick and applies `fog_write` after it.
#[derive(Clone, Debug, Default)]
pub struct RenderWords {
	/// Occlusion fallback mode for this tick's render (REN-CULL-OCC-M1); 0 again on the next tick.
	pub occ_mode: u8,
}

/// The class-172 missile (its pvar and actor words; one at a time, global 0x141330). `fins` are the joint-block
/// Y angles of slots 0 / 1 (blocks 0x1412b0 / 0x1412f0), `aux_k` / `aux_pos` / `aux_m` the aux 179 pvar +0xc, position and
/// basis, `bend` the loop voice bend of this tick.
#[derive(Clone, Debug)]
pub struct Missile172 {
	pub pos: V3,
	pub yaw: f32,
	pub pitch: f32,
	pub speed: f32,
	pub ang8: f32,
	pub angc: f32,
	pub life: i32,
	pub cam: bool,
	pub ctl: [f32; 2],
	pub bank: f32,
	pub desired: f32,
	pub clear: f32,
	pub contact: u8,
	pub saved: RenderSave,
	pub loop_cue: Option<u32>,
	pub fins: [f32; 2],
	pub aux_k: f32,
	pub aux_pos: V3,
	pub aux_m: [[f32; 3]; 3],
	pub bend: i32,
	/// WPN-6.3c: +0x34 bit 0 after the detonation (neither draw prop is drawn).
	pub hidden: bool,
	/// WPN-6.3e: the aux 179 rot y (+0x44), V phase (pvar +4), cumulative V offset (16-bit wrap), this tick's random
	/// scale factor and +0x34 bit 0.
	pub aux_spin: f32,
	pub aux_scroll: i32,
	pub aux_scroll_v: i32,
	pub aux_rand: f32,
	pub aux_hidden: bool,
}











