//! ARI-6: Aridia (level 2) actor registry (REN-13 scenery clocks so far); every Aridia placement
//! whose class is neither here nor engine-wide (`levels::is_shared`) places as a static model at its authored
//! transform. Built props keep their original instance index (`Prop.index`), as on Novalis.

use crate::physics::V3;

/// Aridia-specific classes with a ported actor. REN-13 (decomp/notes/aridia_environment.md): the skeletal scenery clocks
/// (trackdrives 331 / 332, dump truck 662, liftcar 690, light 719, crane top 781, liftcar pole 789); ARI-22: elevator 651;
/// ARI-23: conveyors 652 / 653; ARI-24: rotating platforms 707 / 734, lift 792; ARI-32: red orb 1060; ARI-34: grass 766 / 767.
/// ARI-25: transport pod 713. ARI-10a: sand shark 580; ARI-12a: shark nest 668; ARI-14: Skid 788 (quest counters).
/// ARI-38: flying transports 1212 / 1213; ARI-35: white flower 1819 (aridia_props.rs). ARI-26: bulldozer 656, grab crane 675,
/// crane 732; ARI-27a: rocket launcher 733 and its rocket 1208 (aridia_machines.rs). ARI-13a: constructorbot 612 (bot.rs).
/// ARI-28: reward doors 735 / 736; ARI-16: the agent 786; ARI-30: the cave rock 762; ARI-42a: the Trespasser pad 615;
/// ARI-43: the Trespasser doors 743 / 744; ARI-44: the Swingshot targets 758 / 803.
/// Registry: one entry per line, sorted; add new classes in the owning level module.
pub(super) const ACTORS: &[u32] = &[
	331,
	332,
	580,
	612,
	615,
	651,
	652,
	653,
	656,
	662,
	668,
	675,
	690,
	707,
	713,
	719,
	732,
	733,
	734,
	735,
	736,
	743,
	744,
	758,
	762,
	766,
	767,
	781,
	786,
	788,
	789,
	792,
	803,
	1005,
	1060,
	1208,
	1212,
	1213,
	1324,
	1479,
	1819,
];

/// Whether `class` has an Aridia actor.
pub(super) fn is_actor(class: u32) -> bool {
	ACTORS.contains(&class)
}

/// ARI-36 / ARI-37: Novalis classes placed on Aridia that build as their shared actors (house 604, territory marker 605,
/// RC limiter 832, sand mouse 1818; `LevelActors::is_dynamic`; ARI-33 gold bolt 1134, same updater as Novalis, its
/// four placements differ only in data). One entry per line, sorted.
pub(super) const RELOCATED_ACTORS: &[u32] = &[
	super::novalis::HOUSE, // 604
	super::novalis::MARKER, // 605
	super::novalis::RC_LIMIT, // 832
	super::novalis::GOLD_BOLT, // 1134
	super::novalis::MOUSE, // 1818
];
/// Aridia's part of `game::CUE_CLASSES` (the enemies' sequence-trigger cues: shark 580, constructorbot 612, nest 668; ARI-26 bulldozer 656, grab crane 675, crane 732; REN-13 liftcar pole 789). One
/// entry per line, sorted; add new classes in the owning level module.
pub(super) const CUE_CLASSES: &[u32] = &[
	580,
	612,
	656,
	668,
	675,
	732,
	789,
];
/// Aridia's part of `game::SOUND_CLASSES`: ARI-22 the elevator's loop (class sound 0); ARI-24 the liftcar pole's travel
/// loops (cs 0 / 2); ARI-25 the transport pod's one-shots (cs 0..2); ARI-26 the crane's direct cues 0 / 1; ARI-27a the
/// rocket launcher's cues 0..4; ARI-30 the cave rock's break cue 0; ARI-38 the transports' loop cue 1 and destroy cue 0;
/// ARI-35 the flower's break cue 0; ARI-42a the Trespasser pad's one-shots (cs 0 / 1 / 3); ARI-43 the Trespasser door 743's
/// start / stop (cs 0 / 1). One entry per line, sorted; add new classes in the owning level module.
pub(super) const SOUND_CLASSES: &[u32] = &[
	PAD, // 615
	ELEVATOR, // 651
	POD, // 713
	CRANE, // 732
	LAUNCHER, // 733
	SLIDERS[0], // 743
	CAVE_ROCK, // 762
	LIFT_POLE, // 789
	TRANSPORTS[0], // 1212
	TRANSPORTS[1], // 1213
	FLOWER, // 1819
];
/// Aridia's part of `game::JOINT_WORLD_CLASSES` (BRIDGE-3): ARI-13c the constructorbot 612 (joint 0, the arc and
/// sparks); ARI-42a the Trespasser pad 615 (joint 0, the hero align); ARI-27a the rocket launcher 733 (muzzle joints 0 / 1); ARI-16 the agent 786; the liftcar pole 789; the
/// flying transports 1212 / 1213. One entry per line, sorted; add new classes in the owning level module.
pub(super) const JOINT_WORLD_CLASSES: &[u32] = &[
	BOT, // 612
	PAD, // 615
	LAUNCHER, // 733
	AGENT, // 786
	LIFT_POLE, // 789
	TRANSPORTS[0], // 1212
	TRANSPORTS[1], // 1213
];
/// Aridia's part of `game::JOINT_ROWS_CLASSES` (ARI-24 liftcar pole 789, ARI-16 agent 786). One entry per line, sorted.
pub(super) const JOINT_ROWS_CLASSES: &[u32] = &[
	AGENT, // 786
	LIFT_POLE, // 789
];
/// Aridia's COL-14 static solids (`levels::static_solids`). One entry per line, sorted.
pub(super) const STATIC_SOLIDS: &[u32] = &[
	706,
	720,
	727,
	728,
	1056,
];
/// Aridia's part of the placed-flag table (`sched::placed_flags`): the class header rule (w44 low half, +0x40 -> 0x10,
/// a one-frame sequence without a loop sound -> 0x40) plus the placement mode bits. One entry per line, sorted; a class placed on several
/// levels lives in `sched::SHARED_PLACED_FLAGS`.
pub(super) const PLACED_FLAGS: &[(u32, u16)] = &[
	(331, 0x4000),
	(332, 0x4002),
	(580, 0x5420), // (corrected 2026-10-06: was 0x5020; allocator 0x263020 ORs 0x400 when class byte +0xf != 0)
	(612, 0x4420), // (corrected 2026-10-06: was 0x4020; same allocator rule, 1 shadow sphere + 10 capsules)
	(615, 0x40),
	(651, 0x60),
	(652, 0x20),
	(653, 0x20),
	(656, 0x10),
	(662, 0x10),
	(668, 0x5020),
	(690, 0x40),
	(707, 0x60),
	(713, 0x60),
	(719, 0x52),
	(733, 0x10),
	(734, 0x60),
	(735, 0x60),
	(736, 0x60),
	(743, 0x60),
	(744, 0x60),
	(762, 0x4040),
	(781, 0x42),
	(789, 0x0),
	(792, 0x60),
	(1005, 0x40),
	(1212, 0x5070),
	(1213, 0x5070),
	(1324, 0x5),
	(1819, 0x4040),
];
/// Aridia's part of `fragment::FRAGMENT_CLASSES` (spawn order): ARI-12b the nest death pieces, ARI-35 / ARI-38 the
/// flower piece / remains and the six transport debris classes, ARI-13b the constructorbot's five. One entry per line.
pub(super) const FRAGMENT_CLASSES: &[u32] = &[
	super::fragment::NEST_FRAGMENTS[0],
	super::fragment::NEST_FRAGMENTS[1],
	FLOWER_PIECE,
	FLOWER_REMAINS,
	TRANSPORT_PIECES[0][0],
	TRANSPORT_PIECES[0][1],
	TRANSPORT_PIECES[0][2],
	TRANSPORT_PIECES[1][0],
	TRANSPORT_PIECES[1][1],
	TRANSPORT_PIECES[1][2],
	super::bot::BOT_FRAGMENTS[0],
	super::bot::BOT_FRAGMENTS[1],
	super::bot::BOT_FRAGMENTS[2],
	super::bot::BOT_FRAGMENTS[3],
	super::bot::BOT_FRAGMENTS[4],
];

/// ARI-22: the elevator class.
pub(super) const ELEVATOR: u32 = 651;
pub(crate) const LIFT_POLE: u32 = 789;
/// ARI-25: the transport pod (FUN_002dd5b8) and its two ring models.
pub(crate) const POD: u32 = 713;
pub(crate) const POD_RINGS: [u32; 2] = [1057, 1058];
/// ARI-13a: the constructorbot (FUN_002d7078).
pub(crate) const BOT: u32 = 612;
/// ARI-16: the agent (FUN_002e06f0) and the prop model he holds at joint 2.
pub(crate) const AGENT: u32 = 786;
pub(crate) const AGENT_PROP: u32 = 56;
/// ARI-35: the white flower (FUN_002f1890), its moving piece and its static remains.
pub(crate) const FLOWER: u32 = 1819;
pub(crate) const FLOWER_PIECE: u32 = 1821;
pub(crate) const FLOWER_REMAINS: u32 = 1820;
/// ARI-38: the flying transports (FUN_002ebde8 / FUN_002ec748) and their three debris classes each, in spawn order.
pub(crate) const TRANSPORTS: [u32; 2] = [1212, 1213];
pub(crate) const TRANSPORT_PIECES: [[u32; 3]; 2] = [[1522, 1525, 1936], [1526, 1528, 1529]];
pub(crate) const CRANE: u32 = 732;
/// ARI-27a: the rocket launcher (FUN_002ded08) and its rocket (creator FUN_002ebb58, update FUN_002eb850).
pub(crate) const LAUNCHER: u32 = 733;
/// ARI-30: the cave rock (FUN_002e0050), broken by a bomb blast; its story byte (set once broken).
pub(super) const CAVE_ROCK: u32 = 762;
/// ARI-44: the Swingshot targets (updater FUN_002dfd80): the pull target 758 and the swing target 803.
pub(super) const PULL_TARGET: u32 = 758;
pub(super) const SWING_TARGET: u32 = 803;


/// ARI-42a: the Trespasser pad (FUN_002d8400; placement 563).
pub(super) const PAD: u32 = 615;
/// ARI-43: the Trespasser doors (FUN_002df8f8 / FUN_002dfa68, one machine with sign +2 / -2; placements 627 / 628).
pub(super) const SLIDERS: [u32; 2] = [743, 744];



























/// FUN_00210098 / FUN_002100e8: `v` rescaled to length `l` with the VU0 rsqrt (Q = l / sqrt(x^2 + y^2 + z^2), one
/// truncation), a zero vector stays zero.
pub(super) fn set_len(v: V3, l: f32) -> V3 {
	use crate::physics::{vadd1, vmul, vrsqrt};
	let q = vrsqrt(l, vadd1(vadd1(vmul(v[0], v[0]), vmul(v[1], v[1])), vmul(v[2], v[2])));
	[vmul(v[0], q), vmul(v[1], q), vmul(v[2], q)]
}









/// ARI-42c: one puzzle HUD strip (FUN_0020bb88: a textured 4-vertex triangle strip): `tex` the fx texture record, `xy`
/// retail pixel offsets from the screen centre (x right, y down) in strip order v0..v3, `uv` texels, `rgba` the GS
/// colour (R, G, B, A bytes low first) and `clamp` the CLAMP_1 clamp (else repeat).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PadPrim {
	pub tex: u16,
	pub xy: [[f32; 2]; 4],
	pub uv: [[f32; 2]; 4],
	pub rgba: u32,
	pub clamp: bool,
}

/// ARI-42c: the fx texture records of the puzzle HUD (beam 7, arcs 16, backdrop 104, slot icon 105).
pub const PAD_FX: [u16; 4] = [7, 16, 104, 105];














