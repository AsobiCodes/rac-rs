//! Player controller and third-person camera, with the numbers read from the decompiled hero code
//! (decomp/notes/hero_movement.md; SI units, the game stores them per 50 Hz tick). Marked [I] where
//! the note itself marks the meaning as inferred.

use crate::physics::add;
use crate::physics::length;
use crate::physics::scale;
use crate::physics::sub;
use crate::physics::MobySolid;
use crate::physics::World;
use crate::physics::V3;
mod edges;
mod headpose;
pub use headpose::pulse_item;
// PARKED HERO-18b: 0x21/0x7a and glide (8) are unavailable on current planets; see owner spec. State 0x20 (Walloper,
// reachable through the [PC] WPN-6.8 grant) is `State::Wallop` (player/wallop.rs, WPN-6.6a).
pub(crate) mod steering;
mod grind;
mod states;
mod wall;
pub(crate) mod wallop;
pub(crate) mod swing;

pub const HERO_POINT_Z: f32 = 0.7;
const RUN: f32 = 5.7;














/// Swim sequences (state table, hero_swim.md): 0x37 -> 58, 0x36 -> 101, 0x33 -> 59, 0x34 -> 60, 0x35 -> 61, slide -> 100.
pub mod swim_seq {
}

/// Crank sequences (hero state 0x3b, crank FUN_002e0598 state 3; novalis_environment.md "crank").
pub mod crank_seq {
}

/// ARI-46: the Swingshot pull states: 0x24 aim (hook flying), 0x25 reel toward the target, 0x26 orbit at a radius.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PullKind {
	Aim,
	Reel,
	Orbit,
}

/// Water states: 0x37 surface idle, 0x36 surface swim, 0x33 swim, 0x34 tread, 0x35 fast swim, 0x6a drowning.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Swim {
	Idle,
	Surface,
	Under,
	Tread,
	Fast,
	Drown,
}

/// What the water probe found under the hero: water z (face id 0 hit), floor z (solid ray), depth = water - floor.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Water {
	pub z: f32,
	pub floor: Option<f32>,
	pub depth: f32,
}

/// Ratchet sequences for the movement states.
pub mod seq {
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum JumpKind {
	/// State 7.
	Stand,
	/// State 9.
	Run,
	/// State 0xb, with the stick-direction variant.
	High(u8),
	/// State 0x1c, off a ledge.
	PullUp,
	/// State 0xe, the double jump out of a stand or run jump (decomp/notes/hero_double_jump.md).
	Double,
	/// State 0x11, the wall jump (hero_air_states.md section 4; detector in player/wall.rs).
	Wall,
	/// ARI-48 state 0x69 (decomp/notes/aridia_gadgets.md "ARI-48 port spec"): the escape out of Aridia mud, `base` =
	/// mud depth + 1.7.
	Mud { base: f32 },
}











/// Which retail ground state `State::Ground` is in (HERO-15c.g): state 0, 2 / 0x73, 3.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Gnd {
	Stand,
	Walk,
	Skid,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum State {
	/// Idle, walking, running and the stop skid (retail states 0 / 2 (0x73 wading) / 3).
	Ground { id: Gnd },
	/// State 4; `turn` is the last turn rate (rad/s) for the animation.
	Crouch { t: f32, turn: f32 },
	/// State 1: L1/L2 (pad mask 5) held on the ground; the camera goes to free-look (class 4) while
	/// it lasts. `crouch` = mask 10 also held (crouch pose seq 0xd, else idle) (hero_crouch_ledge.md). `fp`: retail
	/// state 0x1e instead (WPN-6.1e2: the Blaster weapon-ready look, group 0, camera mode 0x51).
	Look { t: f32, crouch: bool, fp: bool },
	/// `heading` is the direction a high jump travels; `landed` counts the landing.
	Jump { kind: JumpKind, t: f32, z0: f32, heading: f32, landed: Option<u32> },
	/// State 6.
	Fall { t: f32, long: bool },
	/// ARI-47a: Swingshot rope swing, retail state 0x2c group 0xe (decomp/notes/aridia_gadgets.md "ARI-47a port spec").
	Swing,
	/// ARI-47b: swing release fall, retail state 0x2d group 2; `long` = seq 0xb set (aridia_gadgets.md "ARI-47b port spec").
	SwingFall { t: f32, long: bool },
	/// ARI-46: Swingshot pull states 0x24 / 0x25 / 0x26, group 0xd (aridia_gadgets.md "ARI-46 port spec"); `t` is the
	/// seconds in the state.
	Pull { kind: PullKind, t: f32 },
	Grab { t: f32 },
	Hang { t: f32 },
	Shimmy { left: bool, t: f32, stop: bool },
	/// State 0x77.
	Tumble { t: f32 },
	/// State 0x79 (HERO-8c): sliding on a slippery face (surface id 8 / 0xc); `t` is the seconds in the state.
	Slip { t: f32 },
	/// Water states 0x33-0x37 and 0x6a (hero_swim.md); `t` is the seconds in the state.
	Swim { mode: Swim, t: f32 },
	/// Water slide, state 0x31 (surface id 4).
	Slide { t: f32 },
	/// Lava hurt, state 0x3c (group 4): launched by the lava ground ray (hero_surfaces.md).
	/// `landed` counts the ticks since the touchdown (retail 0x13f768, as the jump states; CAM-5c).
	Hurt { t: f32, landed: Option<u32> },
	/// Hit reaction, state 0x16 (group 7): knocked back by an attack (hero_movement.md HERO-HURT).
	Hit { t: f32 },
	/// Swim hit, states 0x75 (`top`, group 0x12) / 0x76 (group 0x11) (hero_movement.md HERO-HURT).
	SwimHit { top: bool },
	/// Walloper attack, state 0x20 (group 6; weapons_framework.md WPN-6.6a, player/wallop.rs).
	Wallop,
	/// KER-61a: rail grind, state 0x74 (group 0x1a; decomp/notes/kerwan_gadgets.md, player/grind.rs).
	Grind,
	/// Visibomb remote-control hold, state 0x1d (group 9, BRIDGE-46): stowed, no turn, no walk, no button exit;
	/// `t` is the seconds in the state.
	Remote { t: f32 },
	/// ARI-48 states 0x68 / 0x7b (`death`, entered with health 0; group 0x19): sinking into Aridia mud
	/// (decomp/notes/aridia_gadgets.md "ARI-48 port spec"); `t` is the seconds in the state.
	Mud { t: f32, death: bool },
}




/// The hero throw layer (retail layer slot 0 at 0x140058; HERO-10.1): sequence 66 (44 when crouched) blended over
/// the locomotion. `mode` 0 = playing, 1 = fading after the anim end (0.084 per tick), 2 = fading after the
/// stand-still conversion (0.3 per tick); freed at weight 0.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ThrowLayer {
	pub seq: usize,
	pub weight: f32,
	pub mode: u8,
	/// Layer clock (+0x24): starts at 0, 1/9 per tick (blend sc(11)).
	pub t: f32,
	/// Two-frame clock (FUN_0024eda0): the target frame (+0x21), the clock rate (+0x2c) and speed (+0x28).
	pub frame: usize,
	pub rate: f32,
	pub speed: f32,
	/// The FROM pose the layer fades out of (retail +0x22 / +0x20): the hero's sequence and frame at creation, then the
	/// previous target after each frame crossing.
	pub from_seq: usize,
	pub from_frame: usize,
}

impl ThrowLayer {

	/// The draw-pass view of the layer (BRIDGE-20).
	pub fn pose(&self) -> crate::anim::LayerPose {
		crate::anim::LayerPose { seq: self.seq, frame: self.frame, from_seq: self.from_seq, from_frame: self.from_frame, t: self.t, weight: self.weight }
	}

}

/// HERO-24 (decomp/notes/hero_movement.md "HERO-24 port spec"): one weapon hold layer (FUN_0022e128 slot 0 / 1,
/// jtab 12 / 13, frames from hold class 1 / 2 below seq 23). `l` carries the layer clock (from / to seq and frame,
/// t, rate, speed); `fading` = the own fade from the previous hero seq, `preset` = holding hold seq 0 for a preset
/// hero seq, `back` = blending back out of the preset.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct HoldLayer {
	pub l: ThrowLayer,
	pub fading: bool,
	pub preset: bool,
	pub back: bool,
}






/// The wrench in flight (wrench moby state byte 10 outbound, 11 returning).
#[derive(Clone, Copy)]
pub struct Thrown {
	pub pos: V3,
	/// Accumulated spin angle (rad) for drawing.
	pub spin: f32,
	/// The afterimage ring (HERO-AFTERIMAGE-15): the last 8 (position, spin) pairs, written before each flight step; `head` is
	/// the next slot and `filled` the entries stored (capped at 8).
	trail: [(V3, f32); 8],
	head: u8,
	filled: u8,
}

impl Thrown {
	/// The two translucent wrench copies (alpha byte, lag in updates): (0x30, 3) and (0x17, 5). A copy shows the entry written
	/// `lag - 1` steps before the latest and stays hidden until the ring holds that many entries [I]. Returns (position, spin,
	/// alpha byte) per copy.
	pub fn ghosts(&self) -> [Option<(V3, f32, u8)>; 2] {
		[(0x30u8, 3u8), (0x17, 5)].map(|(a, lag)| (self.filled >= lag).then(|| {
			let (p, spin) = self.trail[(self.head as usize + 8 - lag as usize) % 8];
			(p, spin, a)
		}))
	}
}


pub struct Player {
	/// Feet position.
	pub pos: V3,
	pub vel: V3,
	/// Facing, radians around Z.
	pub yaw: f32,
	pub state: State,
	/// HERO-15: the hero joint override blocks, the head look-at target (0x1415c4), the hurt timer (0x13f510), the hero
	/// animation id (+0x53, mirrored by the viewer) and the yaw at the end of the last pose pass.
	pub blocks: [headpose::JointBlock; 31],
	/// AUD-11e.2: last water surface height seen (0x13f640).
	under_wz: f32,
	/// AUD-11g: footstep cues made this tick: (class, water, foot), drained by the viewer.
	pub foot_cues: Vec<(u8, bool, u8)>,
	/// Idle look scheduler (retail 0x140360 counter, 0x140354/58 pitch/yaw targets, 0x140364/68 gains, HERO-15c).
	pub blink_count: i32,
	/// Eyelid blink (retail 0x140340 counter 0 = idle, 0x140344 deadline, 0x140348 range 0 / 0x68 set by the state).
	pub blink: u8,
	/// Registration order stamp of each eyelid record (None = unregistered).
	pub eyelid: [Option<u32>; 7],
	pub clank_blink: u8,
	pub clank_rec: [Option<u32>; 4],
	/// Clank backpack scale used for its blink offsets.
	pub clank_scale: f32,
	/// Glove layer object weights (retail layer objects of FUN_0022e128, slot 0 and slot 1): None = no object (HERO-15c-rest-c).
	pub glove_w: [Option<f32>; 2],
	/// HERO-24: the hold layer clocks of the two glove layer objects (None with no object).
	pub hold: [Option<HoldLayer>; 2],
	/// HERO-2: the downhill push of the FUN_002163b0 move model exists on Veldin only.
	pub slope_push_on: bool,
	/// Current wrench swing: (combo index 0..2, seconds in, unused).
	pub swing: Option<(usize, f32, bool)>,
	/// The pulse scale of this tick, set after `update` (consumed by the viewer pose).
	pub pulse_scale: Option<f32>,
	/// BRIDGE-21: the back objects are hidden (retail 0x141628); set on Veldin by class 834 (VEL-14), reset by the hero memset at level load/respawn.
	pub clank_hidden: bool,
	/// The throw layer (HERO-10.1), advanced with the fire clock; drawn by the hero animator.
	pub throw_layer: Option<ThrowLayer>,
	/// WPN-6.2e / BRIDGE-41: weapon layer slot 1 (0x14005c, jtab 13), live only with equip_mode 2 (the Devastator).
	pub weapon_layer1: Option<ThrowLayer>,
	/// The thrown wrench while it is out of the hand.
	pub wrench: Option<Thrown>,
	/// KER-60: the level's grind rails (list 0x15f6ec), shared with `Game.grind_rails`.
	pub grind_rails: std::rc::Rc<Vec<crate::gameplay::GrindRail>>,
	/// ARI-47a / BRIDGE-75: the hero body tilt (x roll 0x13f3e0, y pitch 0x13f3e4) and its spring rates (0x13f3f0 / 0x13f3f4).
	pub body_tilt: [f32; 2],
	/// BRIDGE-40: the hero afterimage trail record 0x1409c0 (freed by every state change).
	pub wallop_trail: Option<wallop::Trail>,
	/// Water probe of the last update (hero_swim.md): water z, floor z, depth.
	pub water: Option<Water>,
	/// NOV-12b: the look-session countdown 0x15f668 (reloaded to 50 ticks on every entry to state 1, -1 after a finished
	/// session) and the finished-session count 0x15f66c (`Game::look_session_tick`); both survive a death.
	pub look_cd: i32,
	pub look_count: i32,
	pub(super) push_hit: Option<V3>, // NOV-16.2
	/// Item slot 0 applied id (retail 0x141424).
	pub item_applied: u8,
	/// Frames per tick of every Ratchet sequence's frames (set by the game from the hero anim data).
	pub frame_speeds: std::rc::Rc<Vec<Vec<f32>>>,
	/// Each hero sequence's raw per-frame speeds (`Sequence.frames[i].speed`), for the throw layer's clock end flag.
	pub layer_rates: std::rc::Rc<Vec<Vec<f32>>>,
	/// HERO-23: the hero was re-placed (hero_replace FUN_00236448); the viewer writes the camera water flag
	/// (OV:29238) and clears it.
	pub replaced: bool,
}


pub struct Input {
	pub jump: bool,
	pub attack: bool,
}




impl Player {
	pub fn new(pos: V3, yaw: f32) -> Player {
		Player { pos,    vel: [0.0; 3],   yaw,   state: State::Ground { id: Gnd::Stand },        swing: None,                  pulse_scale: None,   throw_layer: None,  weapon_layer1: None,          replaced: false,       wrench: None,                     grind_rails: Default::default(),            body_tilt: [0.0; 2],                  wallop_trail: None,             water: None,                look_cd: 0, look_count: 0,     push_hit: None,            frame_speeds: Default::default(), layer_rates: Default::default(),                    blocks: headpose::joint_blocks(),                    slope_push_on: false,                                                      under_wz: 0.0,     foot_cues: Vec::new(),   item_applied: 0,            blink_count: 0, blink: 0,   eyelid: [None; 7],   clank_blink: 0, clank_rec: [None; 4],   clank_scale: 1.0,              glove_w: [None; 2], hold: [None; 2],        clank_hidden: false }
	}

	/// A fresh hero at `pos`/`yaw` (respawn, teleporter re-place, save resume) that keeps the class frame-speed and
	/// layer-rate tables of `self`, which only the level's sequence setter fills.
	pub fn respawned(&self, pos: V3, yaw: f32) -> Player {
		let mut p = Player::new(pos, yaw);
		p.frame_speeds = self.frame_speeds.clone();
		p.layer_rates = self.layer_rates.clone();
		p.grind_rails = self.grind_rails.clone();
		// NOV-12b: the look countdown and count are retail globals and persist across a death.
		p.look_cd = self.look_cd;
		p.look_count = self.look_count;
		p.replaced = true;
		p
	}













































	/// HERO-23: the sticky water surface height (retail 0x13f640), last seen by the water ray.
	pub fn water_level(&self) -> f32 {
		self.under_wz
	}

	/// In group 0x11 (0x33/0x34/0x35): the breath drains.
	pub fn underwater(&self) -> bool {
		matches!(self.state, State::Swim { mode: Swim::Under | Swim::Tread | Swim::Fast, .. })
	}


	/// Drowning (state 0x6a).
	pub fn drowning(&self) -> bool {
		matches!(self.state, State::Swim { mode: Swim::Drown, .. })
	}








}

/// Wraps to -pi..pi.
fn angle(a: f32) -> f32 {
	let t = std::f32::consts::TAU;
	(a + std::f32::consts::PI).rem_euclid(t) - std::f32::consts::PI
}

fn turn(yaw: &mut f32, want: f32, max: f32) {
	*yaw = angle(*yaw + angle(want - *yaw).clamp(-max, max));
}

/// Orbit camera behind the player: yaw/pitch from the mouse, distance pulled in by walls.
/// Type 0 follow camera: 4.64 behind, 2.0 up, aimed 1.5 above the feet [I].
pub struct Orbit {
	pub yaw: f32,
	pub pitch: f32,
	pub distance: f32,
}

pub const CAMERA_DISTANCE: f32 = 4.64;
pub const CAMERA_PITCH: f32 = -0.25;

impl Orbit {
	/// Swings the camera behind the hero while he runs (the game's follow behaviour, simplified).
	pub fn follow(&mut self, hero_yaw: f32, speed: f32, dt: f32) {
		if speed > 1.0 {
			turn(&mut self.yaw, hero_yaw, 1.2 * dt * (speed / RUN));
		}
	}

	/// (eye, look yaw, look pitch) for the viewer's camera.
	pub fn eye(&self, world: &World, target: V3) -> (V3, f32, f32) {
		let focus = add(target, [0.0, 0.0, 1.5]);
		let dir = [self.yaw.cos() * self.pitch.cos(), self.yaw.sin() * self.pitch.cos(), self.pitch.sin()];
		// Shorten the boom until a small sphere at its end is clear of geometry.
		let mut d = self.distance;
		while d > 0.6 {
			let eye = sub(focus, scale(dir, d));
			let (c, _) = world.push_sphere(eye, 0.25, 2.0);
			if length(sub(c, eye)) < 1e-4 {
				break;
			}
			d -= 0.25;
		}
		(sub(focus, scale(dir, d)), self.yaw, self.pitch)
	}
}

#[cfg(test)]
mod swim_tests;

#[cfg(test)]
mod pad_tests;


#[cfg(test)]
mod retail_ground_tests;

#[cfg(test)]
mod states_tests;

#[cfg(test)]
mod ground_id_tests;

#[cfg(test)]
mod hero_cue_tests;
