//! Level gameplay on top of the player controller: breakable crates, bolts with the magnet,
//! enemies, health and death. Numbers come from decomp/notes/veldin_mobies.md and hero_movement.md;
//! veldin_enemies.md; the ones the notes leave open are placeholders and marked [P].

use crate::gameplay::{self, Cuboid, MobyInstance};
use crate::inventory::{Inventory, BOMB_GLOVE};
use crate::physics::add;
use crate::physics::dot;
use crate::physics::length;
use crate::physics::scale;
use crate::physics::sub;
use crate::physics::MobySolid;
use crate::physics::V3;
use crate::player::Player;
use std::collections::{HashMap, HashSet};

mod camtrig;
pub(crate) mod currents;
mod foes;
mod jets;
mod jetdebris;
mod fragment;
pub use fragment::FRAGMENT_CLASSES;
pub mod hints;
mod mission;
pub mod vendor;
pub mod gadget_scene;
mod mouse;
pub use novalis::MOUSE;
mod novalis;
mod aridia;
pub(crate) mod kerwan;
pub use aridia::{PadPrim, PAD_FX};
mod bot;
pub(crate) mod shark;
mod aridia_props;
mod aridia_machines;
pub mod levels;
pub use levels::LevelActors;
pub mod pyro;
pub mod held;
mod decoys;
mod suck;
mod suckcomp;
mod tube;
pub mod transit;
pub mod breath_hud;
pub mod count_hud;
/// ARI-15: Skid's deferred relocation record (`Game.block_relocate`).
pub(crate) mod sched;
mod setup;
mod uidstate;
mod bombs;
pub mod devastator;
pub mod blaster;
pub mod visibomb;
pub mod morph;
mod shiprefl;
pub use bombs::{reticle_quads, reticle_quads_placed};
mod mine;
mod doom;
mod taunter;
mod swingshot;
mod pickups;
mod bolt;
mod reward;
mod crates;
mod hits;
mod grapple;
mod enemy;
pub use jetdebris::jet_debris_quad;
pub use novalis::TELE_RING_CLASS;
pub use foes::{BIRDBOT, BIRDBOT_FIRE, BIRDBOT_HIT_JOINTS};

/// AUD-11e: `Event::SoundLoop` owner ids from here up are the hero's eight loop slots (0x141568), not props.
pub const HERO_LOOP_OWNER: usize = usize::MAX - 8;
pub use visibomb::VISIBOMB_LOOP_OWNER;
pub const BOLT: u32 = 13; // 14, 15, 16: the 5, 20 and 50 bolt pieces
const BOLT_CRATE: u32 = 500;
const NANOTECH_CRATE: u32 = 501;
const AMMO_CRATE: u32 = 511;
pub const TOAD: u32 = 749;
pub const ROBOT: u32 = 1440;

/// Classes whose animation clock fires sound cues or a loop sound (`sound::events`, the viewer's voice requests): the
/// level parts below (Veldin toad, robot here; Novalis, Aridia in their modules). Registry: one entry per line, sorted;
/// add new classes in the owning level module.
pub const CUE_CLASSES: &[u32] = &levels::cat::<{ levels::total_len(CUE_PARTS) }>(CUE_PARTS);
pub(crate) const CUE_PARTS: &[&[u32]] = &[VELDIN_CUE_CLASSES, novalis::CUE_CLASSES, aridia::CUE_CLASSES];
/// Veldin's `CUE_CLASSES` part (one entry per line, sorted).
const VELDIN_CUE_CLASSES: &[u32] = &[
	TOAD, // 749
	ROBOT, // 1440
];
/// Classes whose class sounds the actors play (`sound::Events::class`): the engine-wide part (vendor, weapons, held rigs)
/// and the level parts. Registry: one entry per line, sorted; add new classes in the owning level module.
pub const SOUND_CLASSES: &[u32] = &levels::cat::<{ levels::total_len(SOUND_PARTS) }>(SOUND_PARTS);
pub(crate) const SOUND_PARTS: &[&[u32]] = &[novalis::SHARED_SOUND_CLASSES, novalis::SOUND_CLASSES, aridia::SOUND_CLASSES, kerwan::SOUND_CLASSES];

/// BRIDGE-3: classes whose joint world position the updaters read (`Game::joint_world`). Registry: one entry per line,
/// sorted; add new classes in the owning level module.
pub const JOINT_WORLD_CLASSES: &[u32] = &levels::cat::<{ levels::total_len(JOINT_WORLD_PARTS) }>(JOINT_WORLD_PARTS);
pub(crate) const JOINT_WORLD_PARTS: &[&[u32]] = &[novalis::JOINT_WORLD_CLASSES, aridia::JOINT_WORLD_CLASSES, kerwan::JOINT_WORLD_CLASSES];


/// Classes the game moves, hides or spawns; everything else is scenery.
pub fn is_dynamic(class: u32) -> bool {
	matches!(class, 13..=16 | BOLT_CRATE | NANOTECH_CRATE | 502 | 505 | AMMO_CRATE | TOAD | ROBOT | PLANT_RED | PLANT_GREEN | GRASS_A | GRASS_B | ORB | PTERODACTYL | SHIP | BARRIER | BRIDGE_PLATFORM | CLANK_TRIGGER) || novalis::is_actor(class) || foes::is_foe(class) || jets::is_jet(class) || class == jets::EMITTER
		|| matches!(class, 66 | 747 | 1137 | 1520) // REN-13 scenery clocks of the Veldin / Novalis union (VEL-18: 1137)
}

/// Classes drawn as game props: the placed dynamic ones plus the spawned bombs and ammo pickups.
pub fn is_prop_class(class: u32) -> bool {
	is_dynamic(class) || class == BOMB || class == BOMB_AMMO || class == PYRO_AMMO || class == foes::SHOT || class == mission::HEADER_SHIP || class == vendor::ORB || class == vendor::PROMPT_CLASS || class == vendor::PORTRAIT_CLASS || class == devastator::ROCKET || class == blaster::SHOT || class == jets::SHOT || class == visibomb::MISSILE || class == doom::CAPSULE || class == doom::AGENT || class == mine::MINE || class == morph::SHEEP || class == morph::DEBRIS
}


// Ambient mobies (decomp/notes/veldin_ambient.md).
const PLANT_RED: u32 = 1781;
const PLANT_GREEN: u32 = 1782;
/// Novalis swaying grass: same updater as the Veldin plants (novalis_environment.md "Grass 724 and 725").
const GRASS_A: u32 = 724;
const GRASS_B: u32 = 725;
const ORB: u32 = 1060;
const PTERODACTYL: u32 = 1564;
const SHIP: u32 = 530;
/// VEL-7.2: the Veldin hero-blocking barrier (class 1964), a static solid mesh placed at scale 4.
pub const BARRIER: u32 = 1964;
/// NOV-36: the Novalis hinged-bridge base blocks (class 780, Mobys 903/904, null update, 16-face collision block): a
/// static hero solid like the barrier (decomp/notes/novalis_environment.md "NOV-36 reopened").
pub const BRIDGE_PLATFORM: u32 = 780;
/// VEL-16: the Veldin crash-site Clank (scene controller, class 834), hidden while a scene plays.
pub const CLANK_TRIGGER: u32 = 834;


/// The Clank moby tint (0x80 = 1.0): a sine over fead0(100) = 83 ticks of the global frame counter, base + trunc(k s).
pub fn clank_tint(tick: u32) -> [f32; 3] {
	let s = ((tick % 83) as f32 / 83.0 * std::f32::consts::TAU - std::f32::consts::PI).sin();
	let c = |base: i32, k: f32| (base + (k * s) as i32) as f32 / 128.0;
	[c(0x44, 24.0), c(0xa2, 52.0), c(0x4c, 24.0)]
}
/// BRIDGE-21c: the Clank light's colour word: a sine over scale(120) = 100 ticks of the frame counter, R 0xd7 + 90 s (clamped to
/// 0xff), G 0x32 + 50 s, B 0x14 + 10 s, alpha 0x80.
pub fn clank_light_tint(frames: u32) -> u32 {
	let t = (frames % 100) as f32 / 100.0;
	let s = crate::camera::fast_sin((t + t) * std::f32::consts::PI - std::f32::consts::PI);
	let r = ((s * 90.0) as i32 + 0xd7).min(0xff);
	let (g, b) = ((s * 50.0) as i32 + 0x32, (s * 10.0) as i32 + 0x14);
	0x8000_0000 | (b as u32) << 16 | (g as u32) << 8 | r as u32
}
/// The light's tint as a colour scale (0x80 = 1.0).
pub fn clank_light_rgb(frames: u32) -> [f32; 3] {
	let w = clank_light_tint(frames);
	[w & 0xff, (w >> 8) & 0xff, (w >> 16) & 0xff].map(|c| c as f32 / 128.0)
}

const MAX_HEALTH: i32 = 4;


/// Death (death_respawn.md section 6): the hit state 0x16 runs fead0(0x1b) = 22 ticks, then 0x3d plays
/// seq 69 (blend fead0(12) = 10 ticks; 25 keyframes at 0.25 = 100 ticks) and its anim end calls
/// FUN_00231478: the 16-tick fade, then the reset (FUN_00210d98, FUN_00243cf8). Novalis never sets the
/// lava flag, so 0x7b/0x7c are not used.
const DEATH_HIT: f32 = 22.0 * 0.02;
const DEATH_BLEND: f32 = 10.0 * 0.02;
pub const DEATH_SEQ: usize = 69;
pub const DEATH_HIT_SEQ: usize = 16;
const DEATH_FADE: f32 = 16.0 * 0.02;


// Bomb Glove (decomp/notes/glove_prereq_pool_blast.md; glove_prereq_leftovers.md sections D, F, G).
pub const BOMB: u32 = 121;
pub const BOMB_AMMO: u32 = 226;
/// Pyrocitor ammo pickup class (vendor.md; class table 0x20ba00).
pub const PYRO_AMMO: u32 = 214;




#[derive(Clone)]
pub struct Prop {
	pub class_id: u32,
	pub pos: V3,
	pub rot: V3,
	/// Retail mode: the draw basis is mirrored in model Y (moby +0x34 bit 0x8000, retail.rs `basis_pose`); `rot` is the plain rotation.
	pub mirror: bool,
	pub shown: bool,
	/// Animation sequence and its retail clock (stepped once per game tick, anim.rs `PropAnim`).
	pub anim: crate::anim::PropAnim,
	/// Size multiplier, and a colour the model is multiplied by (the moby's RGB tint, 1.0 = neutral).
	pub scale: f32,
	pub tint: Option<[f32; 3]>,
	/// A placed moby's colour and light word (instance +0x64.. / +0x70); None for spawned ones, lit with
	/// gameplay::SPAWN_AMBIENT and word 0 (decomp/notes/lighting_fog.md "Moby lighting").
	pub light: Option<([u8; 3], u32)>,
	/// Scheduler state: update distance, override, group, class index and hit byte (game/sched.rs).
	sched: sched::Sched,
	/// The placement index of a placed moby (the key of its UID, `raw_uid`); None for pooled and spawned ones.
	index: Option<u32>,
	kind: Kind,
}

impl Prop {
	/// An inert prop of a retail-mode pool (retail.rs moves and shows it from the interpreter's moby table).
	pub fn retail(class_id: u32, level: i32) -> Prop {
		Prop { class_id, pos: [0.0; 3], rot: [0.0; 3], mirror: false, shown: false, anim: crate::anim::PropAnim::new(0), scale: 1.0, tint: None, light: None, index: None, sched: sched::spawned(level, class_id),  kind: Kind::Retail { alpha: 0x80, ovr: None } }
	}
	pub fn is_retail(&self) -> bool {
		self.index.is_none() && matches!(self.kind, Kind::Retail { .. })
	}
	/// Retail mode: the moby's draw alpha +0x23, its +0x90 tail colour (None while flag +0x34 & 0x10 is clear) and its draw
	/// distance +0x32 (the header alpha is min(fade, 0x80) * +0x23 >> 7 with the fade from +0x32, moby_format.md MOBY-FADE). [C]
	pub fn set_retail_draw(&mut self, a: u8, o: Option<u32>, draw: i16) {
		if let Kind::Retail { alpha, ovr } = &mut self.kind {
			(*alpha, *ovr) = (a, o);
			self.sched.draw = draw;
		}
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
	fn live(&self) -> bool {
		self.shown
	}
}



/// BRIDGE-24: a sound request resolved at its request point (FUN_002a0cd0): the voice was reserved (a free
/// slot, volume at least 0x20) and the def's random pitch range was drawn. `event` indexes `Game.events` (the
/// frame's queue, drained by the viewer); `random` is the drawn shift (0 when the def's range endpoints are equal).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoundRequest {
	pub kind: SoundSource,
	pub index: usize,
	pub random: i32,
}

/// Which queue a `SoundRequest.index` points into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundSource {
	/// `Game.events`.
	Event,
	/// `Player.foot_cues`.
	Foot,
}






#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Glint {pub pos: V3,pub life:i16,pub fade:i16,pub owner:i16,pub angle:f32,pub size:f32}


#[derive(Clone)]
enum Kind {
	/// REN-13: skeletal scenery with only the generic seq-0 clock; no position, collision or state.
	Scenery,
	/// A retail-mode pool prop (retail.rs moves it from the interpreter's moby table): draw alpha +0x23, colour +0x90.
	Retail { alpha: u8, ovr: Option<u32> },
}



/// Things the viewer turns into sound and particles (drained with `std::mem::take`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
	/// NOV-13.5: a streamed voice cue (`0x1516ec`), id 10000.. = TOC +0x1a0 + 8 * (id - 10000) (the viewer plays it).
	Voice(u16),
	JetDown { class: u32, pos: V3, vel: V3, scale: f32 },
	/// The bomb left the glove at this point (whoosh).
	Throw(V3),
	/// A mine/bomb first hit the water face going down (NOV-20.2 / NOV-20.5): `Water::splash` at the hit point,
	/// answered inside the tick by `emit_events`.
	Splash(V3),
	/// ARI-10b3: the launch mover entering a water face (FUN_002e0510): `Water::ring(pos, scale)`, answered with the
	/// splashes by `emit_events`.
	Ring(V3, f32),
	/// Fire with no ammo (click).
	Empty,
	/// The glove was brought out.
	Draw,
	/// Ammo picked up here.
	Pickup(V3),
	/// A bolt reached the hero here (FUN_002a6518: two sparkle particles).
	BoltCollect(V3),
	/// A bomb went off; `big` = more than 17 ticks after its apex.
	Blast { pos: V3, big: bool, gold: bool },
	/// WPN-5.1c: a Suck Cannon victim exploded (`gold` = the weapon's gold byte); no sound in retail.
	SuckBlast { pos: V3, gold: u8 },
	/// Wrench sounds (decomp/notes/sound.md "Wrench and hero sounds"): quick-throw release (hero cs27),
	/// catch (hero cs5), and an impact as class 71 cs0/cs1 (hit a moby) or cs2 (wall).
	WrenchThrow(V3),
	WrenchCatch,
	WrenchImpact { pos: V3, cs: usize },
	/// Hyper-Strike seq 43 triggers: hero cs53 (0) at T 21 and cs27 (1) at T 24.
	SlamCue(usize),
	/// A combo sequence sound trigger: hero class sound 27, 30 or 54.
	Swing(usize),
	/// Hyper-Strike wrench hits at T 27: class 71 cs2 here, plus FUN_002a79d0 sparks if `ground`.
	/// `sound` false: only the burst (the AUD-3e melee impact plays its own chooser sound).
	SlamHit { pos: V3, ground: bool, sound: bool },
	/// Health death (states 0x7b / 0x7c): hero cs9 (bank 29). The pit tumble has none.
	Death,
	/// A nanotech orb was picked up here (class 501 cs1, bank 181).
	Nanotech(V3),
	/// Class sound `cs` of moby class `class` at `pos` (FUN_002a0f48).
	Sound { class: u32, cs: usize, pos: V3 },
	/// ARI-27b: class sound `cs` started with flags 0x10 (FUN_0028c980): a fixed `level` in 1/1024 instead of the distance
	/// falloff, no start gate, no occlusion; positional pan, Doppler and the underwater term as `Sound`.
	SoundLevel { class: u32, cs: usize, pos: V3, level: i32 },
	/// BRIDGE-50 (WPN-6.1e1): the held weapon's class sound (the Blaster empty click); the viewer plays it like `Sound`
	/// and reports whether its voice still plays (`Game::set_held_voice_alive`).
	HeldSound { class: u32, cs: usize, pos: V3 },
	/// WPN-6.9a: stop the voice of the last `HeldSound` (the Taunter cut, FUN_002cc4a8 STOP).
	HeldStop,
	/// Camera shake (DAT_001671e0 amount, DAT_001671e8 duration).
	Shake { amount: f32, secs: f32 },
	/// A breakable prop broke here (its debris burst).
	Broke { class: u32, pos: V3 },
	/// NOV-5.8a: a global level cue FUN_002a10a0(def, 0, 0): the level sound-map def, played flat (no position or owner).
	GlobalCue(u8),
	/// BRIDGE-86: an actor's music request FUN_00253138(track, cue) (KER-80a ride start); the viewer forwards it to
	/// `music::Player::request` in the same frame.
	Music { track: i16, cue: u8 },
	/// PAR-BRIDGE-B crate break in the tick its hit was consumed (FUN_002d0360): `hero_fx` only for class 505, `limited`
	/// within 2 ticks of the last unlimited break, `sound` unless within 2 ticks of the last played break sound.
	CrateBreak { class: u32, pos: V3, rot_z: f32, hero_fx: bool, limited: bool, sound: bool },
	/// PAR-BRIDGE-B robot death burst in the tick its dying timer ends.
	RobotDeath { pos: V3, rot_z: f32 },
	/// NOV-6.6 collapsing rock camera cue (FUN_002f8930) for this tick: `CameraRig::rock_cue(row0)`.
	RockCue { row0: V3 },
	/// NOV-6.3 looping class sound owned by prop `owner` (FUN_002a0f48 flags 4): `on` starts it unless that owner's voice
	/// still plays, `!on` stops it.
	SoundLoop { class: u32, cs: usize, pos: V3, owner: usize, on: bool },
	/// BRIDGE-49 (FUN_002a12b8): this tick's bend value of loop owner `owner`'s voice (the voice slot +0x14, IOP bend term).
	LoopBend { owner: usize, bend: i32 },
}



/// Hero state kept across a level change (Game::carry_out).
#[derive(Clone)]
pub struct Carry {
	pub bolts: u32,
	pub gold_bolts: u32,
	pub gold_slots: [[u8; 4]; 20],
	pub skill_points: [u8; 32],
	pub inv: Inventory,
	pub moves: u32,
	/// Unlocks, story flags, NPC records and mission bytes (save.rs `Story`).
	pub story: crate::save::Story,
}

/// A class's joint positions: (the prop's clock state, joints) -> model-space positions of those joints.
pub type JointRig = std::rc::Rc<dyn Fn(&crate::anim::PropAnim, &[usize]) -> Option<Vec<V3>>>;

/// BRIDGE-61: a class's joint axes: (the prop's clock state, joint) -> the model-space rows (x, y, z axis) of that joint.
pub type JointRowsRig = std::rc::Rc<dyn Fn(&crate::anim::PropAnim, usize) -> Option<[V3; 3]>>;
/// ARI-24: classes whose joint axes the game reads (the Aridia liftcar pole 789 drives the lift's yaw). Registry: one entry
/// per line, sorted; add new classes in the owning level module.
pub const JOINT_ROWS_CLASSES: &[u32] = &levels::cat::<{ levels::total_len(JOINT_ROWS_PARTS) }>(JOINT_ROWS_PARTS);
pub(crate) const JOINT_ROWS_PARTS: &[&[u32]] = &[aridia::JOINT_ROWS_CLASSES];
/// ARI-25: the transport pod's ring models (drawn as world instances, `Game::pod_rings`).
pub const POD_RING_CLASSES: [u32; 2] = [aridia::POD_RINGS[0], aridia::POD_RINGS[1]];
/// ARI-16: the agent's held prop (drawn as a world instance, `Game::agent_prop`).
pub const AGENT_PROP_CLASS: u32 = aridia::AGENT_PROP;

/// NOV-14.2 (decomp/notes/novalis_checklist.md): `rocket` the RC Visibomb position (written by the WPN-6 port, None until
/// then), `kill` the range kill request it consumes, `alpha` the static overlay alpha, `active` the remote-camera flag
/// (0x15f30c) that gates the overlay, `tiles` this tick's static tile offsets.
#[derive(Clone, Default)]
pub struct RcLink {
	pub alpha: Option<u8>,
	pub tiles: Vec<[u8; 2]>,
}



/// NOV-14.2b marker prompt widget (HUD slot 7, element 0x753a; update 0x24e990): `a` the frame ramp, `b` the icon
/// ramp, 0..=8 each.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct MarkerHud {
	pub a: u8,
	pub b: u8,
}


pub struct Game {
 pub glints: [Glint;16],
 pub debris_draws: Vec<(V3, V3)>,
 /// NOV-15.1b: the overlay strip tables, the per-strip scroll words and wave phase, the 1848 pass tables, the
 /// first-761-update rewrite flag and whether each draw callback registered this tick.
 pub strips: Vec<crate::strips::StripRec>,
 pub env_passes: Vec<crate::strips::EnvPass>,
 /// NOV-14.5 (B): the class 676 / 678 / 1225 strip-surface groups.
 pub strip_groups: [crate::strips::StripGroup; 3],
 /// NOV-26: the strip draws of the last tick whose actor pass ran, drawn on every display frame until the next one.
 pub strip_out: Vec<crate::particles::WorldStrip>,
	jet_joint: HashMap<u32, V3>,
	seq_spheres: HashMap<u32, (Vec<[f32; 4]>, f32)>,
	/// The held weapon moby's hidden bit (+0x34 bit 0); no hero-hide caller is ported, so false (WPN-1).
	pub held_hidden: bool,
	pub props: Vec<Prop>,
	pub player: Player,
	pub bolts: u32,
	pub health: i32,
	/// NOV-9.1: class bounding spheres in world units (MobyClass.bsphere) for the classes the game reads them of
	/// (`FRAGMENT_CLASSES`); set by the viewer at load.
	pub class_spheres: HashMap<u32, [f32; 4]>,
	/// WPN-6.6b1: the class header +0x46 type (`MobyClass.kind`) of the loaded classes of kind 5 (enemy) or 9 (crate);
	/// set by the viewer at load, read by the Walloper spark burst.
	pub class_kinds: HashMap<u32, u16>,
	/// BRIDGE-53: the class header +0x24 scale (`MobyClass.scale * 1024`) of the loaded classes; set by the viewer at load.
	pub class_scales: HashMap<u32, f32>,
	/// Hero invulnerability in ticks (retail 0x13f510, an int counter, SCH-5).
	pub invulnerable: u32,
	/// Seconds until respawn after dying.
	pub dead: Option<f32>,
	/// The props as placed, for the reset on death (FUN_00241528 re-creates every moby).
	placed: Vec<Prop>,
	/// Global per-frame counter DAT_0015f5ac: 0 at level init, +1 at the end of init (+1 here at the start of `ambient`, so 1 on the first frame), then +1 per
	/// game frame, cutscene frames included (veldin_ambient.md VEL-5 spec 4).
	pub frames: u32,
	/// WPN-3.2: the held bomb's tint counter (pvar+0x6a), counting ticks while the bomb sits in the glove.
	bomb_hold: i16,
	/// Bits of `MOVE_*` for the moves performed so far (the tutor tips are suppressed once set).
	pub moves_done: u32,
	/// Messages for the console (no HUD text yet).
	pub log: Vec<String>,
	/// Gadgets and ammo, and their state at level start (restored on death).
	pub inv: Inventory,
	/// Pickup popup texts waiting for the HUD (give_item).
	pub popups: Vec<(u32, u32)>,
	inv_start: Inventory,
	/// The weapon's lock-on target point, while the glove is out.
	pub lock: Option<V3>,
	/// REN-2: the Veldin ship reflection tables (level overlay), set by the viewer.
	ship_mesh: Option<shiprefl::ShipMesh>,
	/// The camera position the scheduler measures from (0x1671c0); the hero's when unset.
	pub camera: Option<V3>,
	pub events: Vec<Event>,
	/// BRIDGE-24: requests resolved at the request point this frame (cleared with `events` by the viewer).
	pub sound_requests: Vec<SoundRequest>,
	/// SCH-6b: the class table's loaded index per class (rank of the effect mobys); empty without a level.
	pub class_ranks: HashMap<u32, u32>,
	/// ARI-6: raw class-table row per class (`gameplay::class_rows`), the spawned-actor rank on levels past Novalis.
	pub class_rows: HashMap<u32, u32>,
	pub spawned: Vec<usize>,
	/// Saved bits as (class, uid): kept across deaths (the level's bitfields 0x14c190 / 0x1ba8d0).
	saved: HashSet<(u32, u32)>,
	cuboids: Vec<Cuboid>,
	/// KER-5: each cuboid record's forward basis rows (+0x00 / +0x10 / +0x20), `gameplay::cuboid_basis`; set_cuboid_basis.
	cuboid_basis: Vec<[V3; 3]>,
	/// ARI-25: the level's gameplay paths (`gameplay::polygons` points), set_paths.
	paths: Vec<Vec<V3>>,
	/// KER-1a: the w lane of every path vertex (`Polygon.marks`), set_path_w; the walker's state 0 writes its segment
	/// lengths into it.
	path_w: Vec<Vec<f32>>,
	/// KER-60: the level's grind rails (gameplay +0x74, list 0x15f6ec), set_grind_rails; the same `Rc` as `Player.grind_rails`.
	pub grind_rails: std::rc::Rc<Vec<gameplay::GrindRail>>,
	/// ARI-38 / BRIDGE-66: each transport's glow joints and colour word recorded by its tick (FUN_002ebd20 / FUN_002ec680),
	/// drawn by the render (`Game::transport_glows`).
	transport_glows: HashMap<usize, (Vec<V3>, u32)>,
	/// NOV-3.4: the level's raw moby group lists (`gameplay::group_lists`) and the built runtime lists (prop indices,
	/// blob order, uninstantiated members dropped; empty = no list).
	group_src: Vec<Option<Vec<u16>>>,
	group_lists: Vec<Vec<usize>>,
	/// ARI-8 (FUN_00243000 OV:45665-45681): the loader's reference relocation, built once in `with_story`: authored instance
	/// index -> prop index for every prop placement the loader allocated (`loaded_ref`).
	pub load_refs: HashMap<u32, usize>,
	/// ARI-8: authored indices of prop placements the loader omitted by its uid test (`load_skipped`).
	pub load_dropped: HashSet<u32>,
	/// ARI-22 / BRIDGE-59: the level's gameplay cylinders (centre +0x30, axes rows +0, inverse rows +0x40), set by the viewer.
	cylinders: Vec<(V3, [V3; 3], [V3; 3])>,
	/// Ratchet's sequence lengths in seconds (drowning 95, crank entry 68), from the viewer.
	hero_lens: Vec<f32>,
	/// Seconds of the death states before the fade (0 for a fade-only death).
	death_len: f32,
	/// Interact (Triangle) pressed since the last update.
	pub interact: bool,
	/// NOV-10.5a: the hero mesh is hidden by a teleporter beam (FUN_00308e20).
	pub hero_hidden: bool,
	/// REN-3: this tick's bomb landing markers (predicted impact point and normal), drawn by the viewer.
	pub reticles: Vec<Reticle>,
	/// WPN-6.4b4: the Mine Glove landing markers queued by the mine updaters of this frame.
	pub mine_reticles: Vec<Reticle>,
	/// WPN-6.5e4: the Glove of Doom landing markers queued by the capsule updaters of this frame.
	pub doom_reticles: Vec<Reticle>,
	/// BRIDGE-39: this tick's screen-sprite requests (queue 0x169440, at most 3, none in scene mode), filled by the held
	/// weapon updaters; the draw consumer FUN_0020fb40 is not mapped.
	pub sprite_queue: Vec<ScreenSprite>,
	pub gold_bolts: u32,
	/// Gold bolts collected per level (retail 0x14bec0 + level*4 + n), saved by save.rs.
	pub gold_slots: [[u8; 4]; 20],
	/// Skill point bytes (retail 0x13d408..); [0] = Take Aim.
	pub skill_points: [u8; 32],
	/// Pause-menu button code entry: count (retail 0x160378) and the values entered (0x1ba8a8).
	pub code_n: usize,
	pub code_buf: [u8; 20],
	/// Set when the original calls FUN_00260fe0(0,-1) (autosave); the viewer clears it.
	pub save_request: bool,
	/// (class, placement index, UID bit) of this level's placements that can hold a persistent UID bit (save::uid_map).
	uid_map: Vec<(u32, u32, usize)>,
	/// The UID tables of this level run (pending/committed, session and saved level bits, registry, checkpoint copy).
	uids: uidstate::UidState,
	/// Placement index -> record +8 flag word (the loader's skip rules read bits 4 and 8).
	place_flags: HashMap<u32, u32>,
	/// Checkpoint commits since level entry (SAV-1c): the viewer captures hero light, reverb and music track when it changes.
	pub ckpt_serial: u32,
	/// The Pyrocinex held weapon's sequence timings (props use their own `PropAnim` clock).
	pyro_seqs: Option<std::rc::Rc<Vec<crate::anim::Sequence>>>,
	/// Per-class attach point 0 (model space) at the birdbot's firing frame, set_muzzles.
	muzzles: HashMap<u32, [f32; 3]>,
	hit_joints: HashMap<u32, Vec<[f32; 3]>>,
	/// Per-class joint-position sources (set_joint_rigs).
	rigs: HashMap<u32, JointRig>,
	/// BRIDGE-61: per-class joint-axis sources (set_joint_rows_rigs).
	joint_rows: HashMap<u32, JointRowsRig>,
	/// NOV-2.5: per-class attach-point-0 joint (`Animated.attach0`), posed live through `rigs` (FUN_00264140).
	attach0: HashMap<u32, usize>,
	/// BRIDGE-3: per-class joint of each attach point (`Animated.attach_points`); `joint_world`'s argument is a point.
	attach_points: HashMap<u32, Vec<Option<usize>>>,
	/// Class collision blocks (set_colliders) for the hero push-out.
	colliders: HashMap<u32, crate::mobies::Collision>,
	/// The held Pyrocitor (game/pyro.rs) while equipped, its flame pool, texture frames and the posed muzzle.
	pyro: Option<pyro::Pyro>,
	flames: pyro::Flames,
	pyro_frames: pyro::Frames,
	/// BRIDGE-38 (WPN-6.0): the generic held-weapon rig, per-class sequence timings, and its muzzle posed by the viewer.
	held: Option<held::HeldRig>,
	held_seqs: HashMap<u32, std::rc::Rc<Vec<crate::anim::Sequence>>>,
	held_muzzle: Option<V3>,
	morph_beam: morph::MorphBeam,
	/// WPN-6.9c: the Taunter sound-wave rings 0x1db7c0 and their flags.
	taunt_rings: taunter::Rings,
	/// The held Suck Cannon (game/suck.rs) while equipped, and the weapon matrix the viewer posed last frame.
	suck: Option<suck::SuckState>,
	suck_seqs: Option<std::rc::Rc<Vec<crate::anim::Sequence>>>,
	pub tube_draw: Vec<crate::particles::WorldQuad>,
	/// ARI-41: the global screen fade word 0x15f3fc (0 clear, 1 black), folded into `Game::fade`.
	pub screen_fade: f32,
	/// The text an NPC or the ship pad shows this frame (text id).
	pub prompt: Option<u32>,
	/// NOV-14.2: the RC Visibomb link the range limiter 832 reads and writes.
	pub rc: RcLink,
	/// WPN-6.3a: the live Visibomb missile (global 0x141330, held pvar +4; one at a time).
	pub visibomb: Option<visibomb::Missile172>,
	/// WPN-6.3a: the render words the Visibomb snapshot / restore write, for the viewer.
	pub vis_render: visibomb::RenderWords,
	/// NOV-14.2: this level's packed marker crate counts (save B3008, retail 0x14bf10 + level * 16).
	pub crate_nibbles: [u8; 16],
	/// NOV-14.2b: the marker prompt widget's ramps (HUD slot 7).
	pub marker_hud: MarkerHud,
	/// ARI-15b / BRIDGE-69: Skid's counter widgets, HUD slot 5 (sharks) and slot 7 (nests).
	pub count_hud: [count_hud::CountSlot; 2],
	/// HERO-3a / BRIDGE-14: the breath meter (HUD slot 4), ticked once per game tick after the hero update.
	pub breath_hud: breath_hud::BreathHud,
	/// NOV-30f: the shop's HUD slots 0 (ammo counter) and 2 (bolt counter), ticked with the breath meter.
	pub shop_hud: vendor::ShopHud,
	/// NOV-14.3: the sand mouse's attach points 1 and 2 as joints (viewer, `set_mouse_joints`), posed live via `rigs`.
	mouse_joint: [Option<usize>; 2],
	/// The ship pad asked for travel (DAT_0015f610).
	pub depart: bool,
	/// Persistent story state (level_transition.md "Unlock persistence"): carried across levels.
	pub story: crate::save::Story,
	/// The level this game runs (set by `arrive`); -1 before.
	pub level: i32,
	pad: Option<mission::Pad>,
	pub cam_cut: Option<camtrig::CamCut>,
	/// BRIDGE-71: this tick's d-pad new-press bits (up 0x1000, right 0x2000, down 0x4000, left 0x8000) from the viewer's
	/// key presses; cleared at the end of the actor pass.
	pub pad_press: u16,
	pub pad_hud: Vec<aridia::PadPrim>,
	/// The vendor's shop screen while open (vendor.rs); the world is frozen.
	pub shop: Option<vendor::Shop>,
	/// NOV-13.5: the gadget-buy cinematic after the shop closes (the world stays frozen while it runs).
	pub gadget: Option<gadget_scene::GadgetRun>,
	/// UI-5a / BRIDGE-37: the voice language offset L = `Settings.lang` (written by the viewer each frame), the portrait voice's
	/// end flag (retail 0x15172a == 3), the stop request (FUN_00279790) and the class-12 pack timings (None: the portrait never spawns).
	pub voice_lang: u8,
	pub voice_ended: bool,
	pub portrait_stop: bool,
	pub portrait_timing: Option<std::rc::Rc<Vec<crate::anim::Sequence>>>,
	/// Vendor and orb pose effects (vendor.rs): blends, joint spins, glow scroll.
	pub vendor_fx: vendor::VendorFx,
}

impl Game {




	/// PAR-5: the trail owner snapshot, `Some(pos)` while a prop is shown (retail owner state byte >= 0).
	/// BRIDGE-33 / REN-MOBY-DRAWN: the moby draw's verdict per prop slot becomes each live actor's scheduler override (retail
	/// actor +0x31: 1 when drawn, 0 when culled), which the next `schedule` reads. Non-live slots keep theirs.
	pub fn set_draw_verdict(&mut self, drawn: &[bool]) {
		for (p, &d) in self.props.iter_mut().zip(drawn) {
			if p.live() {
				p.sched.over = d as u8;
			}
		}
	}


















}









impl Game {
	/// Has the gold upgrade byte 0x13e520 + item (save record A30) been bought?
	pub fn gold(&self, item: u8) -> bool {
		self.story.gold_upgrades.get(item as usize).is_some_and(|&b| b != 0)
	}

	/// [PC] debug grant key (WPN-6.8, user decision 2026-10-06; not a retail feature): plain press grants the seven
	/// weapons with full ammo and quick-select slots; `toggle_gold` instead flips the gold upgrade of those with a gold cap.
	pub fn debug_grant_weapons(&mut self, toggle_gold: bool) {
		if !toggle_gold {
			self.inv.debug_grant_weapons();
			return;
		}
		for id in crate::inventory::DEBUG_WEAPONS {
			if crate::inventory::GOLD_CAP[id as usize] > 0 {
				let b = &mut self.story.gold_upgrades[id as usize];
				*b = if *b != 0 { 0 } else { 1 };
			}
		}
	}
}


/// `bomb_launch` for the throw range `range` (WPN-3.2): the vertical speed cap is g (R / v) / 2.
/// A bomb landing marker (FUN_002acdb8 result): the predicted impact point and the hit normal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reticle {
	pub pos: V3,
	pub normal: V3,
}

/// BRIDGE-39 screen-sprite request (FUN_0020fa60): scale (x 40 px), angle, ABGR colour, fx texture, and the world
/// position it is projected from (None = the screen centre).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenSprite {
	pub scale: f32,
	pub angle: f32,
	pub abgr: u32,
	pub tex: u8,
	pub at: Option<V3>,
}













/// The moby matrix (Z * Y * X) columns, local -> world.
fn moby_axes(rot: V3) -> [V3; 3] {
	let ((sx, cx), (sy, cy), (sz, cz)) = (rot[0].sin_cos(), rot[1].sin_cos(), rot[2].sin_cos());
	[[cz * cy, sz * cy, -sy], [cz * sy * sx - sz * cx, sz * sy * sx + cz * cx, cy * sx], [cz * sy * cx + sz * sx, sz * sy * cx - cz * sx, cy * cx]]
}



#[cfg(test)]
mod clank_light_tests {
	#[test]
	fn clank_light_tint_cycle() {
		let w = super::clank_light_tint;
		assert_eq!(w(0), 0x801432d7);
		assert_eq!(w(25), 0x800a007d);
		assert_eq!(w(50), 0x801432d7);
		assert_eq!(w(75), 0x801e64ff);
		assert_eq!(w(100), w(0));
	}
}

#[cfg(test)]
mod angle_diff_tests {
	use std::f32::consts::PI;

}
