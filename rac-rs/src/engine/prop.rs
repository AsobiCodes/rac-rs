//! The game's moby instances as the renderer draws them (engine split stage E2 batch 4c, 2026-10-26: moved out of rac1/game.rs
//! and game/sched.rs, which re-export them): `Prop` = class, placement, visibility, animation clock, tint and light, with
//! the draw state the game keeps per moby (`Kind`: draw alpha and colour override of a game-driven moby; `Sched`: draw
//! distance, occlusion pair, scheduler flags). The game fills and moves them; the renderer reads them.

use crate::physics::V3;

#[derive(Clone)]
pub struct Prop {
	pub class_id: u32,
	pub pos: V3,
	pub rot: V3,
	/// Retail mode: the draw basis is mirrored in model Y (moby +0x34 bit 0x8000, retail.rs `basis_pose`); `rot` is the plain rotation.
	pub mirror: bool,
	/// [PC] Retail mode: the moby's draw position this tick (`retail/props_sync.rs`): its rank in retail's moby table walk, plus
	/// `DEFERRED_ORDER` when it is on the deferred list, which retail draws after that whole walk. The pool slot order is per
	/// class and says nothing about it. `viewer/gpu/tail_order.rs` reads it.
	pub order: u32,
	pub shown: bool,
	/// [PC] Retail mode: retail itself drew the moby this tick (`retail::Moby::shown`). `shown` is also set for the mobys only rac-rs's
	/// widescreen window keeps (retail culled them at its 4:3 side plane); those are not in retail's draw stream, so they are never
	/// the point other mobys are ordered against (`viewer/gpu/tail_order.rs`).
	pub retail_drawn: bool,
	/// [PC] Retail mode: not shown only because retail's draw culled it for the game camera (`retail::Moby::view_culled`): it still
	/// casts its sun shadow, and Culling Off draws it (viewer/gpu/props_set.rs). With Culling Off also when past its draw distance.
	pub cast: bool,
	/// Animation sequence and its retail clock (stepped once per game tick, anim.rs `PropAnim`).
	pub anim: crate::anim::PropAnim,
	/// Size multiplier, and a colour the model is multiplied by (the moby's RGB tint, 1.0 = neutral).
	pub scale: f32,
	pub tint: Option<[f32; 3]>,
	/// A placed moby's colour and light word (instance +0x64.. / +0x70); None for spawned ones, lit with
	/// gameplay::SPAWN_AMBIENT and word 0 (decomp/notes/lighting_fog.md "Moby lighting").
	pub light: Option<([u8; 3], u32)>,
	/// Scheduler state: update distance, override, group, class index and hit byte (game/sched.rs).
	pub sched: Sched,
	/// The placement index of a placed moby (the key of its UID, `raw_uid`); None for pooled and spawned ones.
	pub index: Option<u32>,
	pub kind: Kind,
}

impl Prop {
	pub fn is_retail(&self) -> bool {
		self.index.is_none() && matches!(self.kind, Kind::Retail { .. })
	}
	/// Retail mode: the moby's draw alpha +0x23, its +0x90 tail colour (None while flag +0x34 & 0x10 is clear) and its draw
	/// distance +0x32 (the header alpha is min(fade, 0x80) * +0x23 >> 7 with the fade from +0x32, moby_format.md MOBY-FADE). [C]
	pub fn set_retail_draw(&mut self, a: u8, o: Option<u32>, draw: i16) {
		if let Kind::Retail { alpha, ovr, .. } = &mut self.kind {
			(*alpha, *ovr) = (a, o);
			self.sched.draw = draw;
		}
	}
	/// Retail mode: the moby's flag +0x34 bit 0x200 (`retail::is_additive`): drawn with ALPHA_1 0x48, additive (viewer/gpu/moby_add.rs). [C]
	pub fn set_retail_additive(&mut self, on: bool) {
		if let Kind::Retail { add, .. } = &mut self.kind {
			*add = on;
		}
	}
	pub fn is_additive(&self) -> bool {
		matches!(self.kind, Kind::Retail { add: true, .. })
	}
	/// Retail mode: Some(tail colour) for a retail prop (channel = byte / 128, R = low byte; moby_format.md CLASS-tint), None
	/// for a rac-rs prop. The inner None = no colour override (lit as the other packets).
	pub fn retail_tint(&self) -> Option<Option<[f32; 3]>> {
		match self.kind {
			Kind::Retail { ovr, .. } => Some(ovr.map(|w| [0, 8, 16].map(|s| (w >> s & 0xff) as f32 / 128.0))),
			_ => None,
		}
	}

	/// The occlusion u16 (moby +0x36 / +0x37) of the actor.
	pub fn sched_occ(&self) -> u16 {
		self.sched.occ
	}
	/// Scheduler flags (actor +0x2c): 0x400 / 0x800 take the moby draw's deferred-list path.
	pub fn sched_flags(&self) -> u16 {
		self.sched.flags
	}
	/// The scheduler override byte (actor +0x31) as it stands.
	pub fn sched_over(&self) -> u8 {
		self.sched.over
	}
	pub fn draw_shown(&self) -> bool {
		self.shown
	}
	pub fn draw_dist(&self) -> i16 {
		self.sched.draw
	}
	pub fn moby_alpha(&self) -> u8 {
		match &self.kind {
			Kind::Retail { alpha, .. } => *alpha,
			Kind::Scenery => 0x80,
		}
	}

	/// A live actor slot (state byte +0x20 >= 0): shown, or one of the mission actors that run unseen.
	pub fn live(&self) -> bool {
		self.shown
	}
}

/// [PC] `Prop::order` bit: the moby is on retail's deferred draw list (drawn after the whole main moby walk).
pub const DEFERRED_ORDER: u32 = 1 << 31;

#[derive(Clone)]
pub enum Kind {
	/// REN-13: skeletal scenery with only the generic seq-0 clock; no position, collision or state.
	Scenery,
	/// A retail-mode pool prop (retail.rs moves it from the interpreter's moby table): draw alpha +0x23, colour +0x90.
	Retail { alpha: u8, ovr: Option<u32>, add: bool },
}

/// Per-actor scheduler state: update distance (actor +0x30), override (+0x31), group (+0x21, -1 = individual),
/// loaded class index (+0x22) and the ring reference (hit byte, 0xff = none).
#[derive(Clone, Copy, Debug)]
pub struct Sched {
	pub dist: u8,
	pub over: u8,
	pub rank: u32,
	/// Actor flags (+0x34) as set at creation: 0x2 = scheduler skips the actor, 0x1000 = in the 0x1abe00 registry.
	pub flags: u16,
	/// Draw distance in metres (actor +0x32; the draw clamps it to 500). Spawned actors keep 500 until their creator literals are mapped.
	pub draw: i16,
	/// Moby bytes +0x36 / +0x37 as one u16: the occlusion pair (low byte = bit mask, high byte = mask index); 0x7f80 always
	/// passes (every retail spawn writes it; REN-CULL-OCC-MOBY).
	pub occ: u16,
}

impl Default for Sched {
	/// Spawned actors (shots, pickups, bombs) mark distance 255 [I: weapons_framework.md creators].
	fn default() -> Self {
		Sched { dist: 255, over: 0,  rank: u32::MAX,  flags: 0,  draw: 500, occ: 0x7f80 }
	}
}
