//! KER-0a: Kerwan (level 3) actor registry. Every Kerwan placement whose class is neither here nor engine-wide
//! (`levels::is_shared`) places as a static model at its authored transform. Built props keep their original instance
//! index (`Prop.index`), as on Novalis and Aridia.
//!
//! KER-25: the one-off NPC class 914 (decomp/notes/kerwan_flow.md "KER-25 port spec").
//!
//! KER-9: the hover ferry class 816 (decomp/notes/kerwan_environment.md "KER-9 port spec").
//!
//! KER-33: the two-station lift class 1012 (decomp/notes/kerwan_environment.md "KER-33 port spec").
//!
//! KER-23: Helga, the Swingshot seller class 890 (decomp/notes/kerwan_flow.md "KER-23 port spec").
//!
//! KER-24: the path shuttle platforms, classes 868 / 905 / 928 (decomp/notes/kerwan_environment.md "KER-24 port spec").
//!
//! KER-32: the rising gate class 997 (decomp/notes/kerwan_environment.md "KER-32 port spec").
//!
//! KER-22: the item-2 seller NPC class 909 (decomp/notes/kerwan_flow.md "KER-22 port spec").
//!
//! KER-8: the mine class 627 the class 578 layer drops (decomp/notes/kerwan_enemies.md "KER-8 port spec").

use super::Game;
// SHR-FP2: the EE-truncating f32 ops live in gameplay.rs; the kerwan submodules reach them through `use super::*`.
#[allow(unused_imports)]
use crate::gameplay::ee_add;

mod breakable;
pub(crate) mod cam1;
pub(crate) mod cam14;
mod cars;
mod conveyor;
mod debris;
mod destroy;
mod director;
mod dog;
mod ferry;
mod gate;
mod helga;
mod heli;
mod infobot;
mod layer;
mod lift;
mod mine;
mod npc909;
mod npc914;
mod ride;
mod shack_sign;
mod shuttles;
mod spawner;
mod walker;

pub(super) use breakable::*;
pub(super) use cars::*;
pub(super) use conveyor::*;
pub(super) use debris::*;
pub(super) use destroy::*;
pub(super) use director::*;
pub(super) use dog::*;
pub(super) use ferry::*;
pub(super) use gate::*;
pub(super) use helga::*;
pub(super) use heli::*;
pub(super) use infobot::*;
pub(super) use layer::*;
pub(super) use lift::*;
pub(super) use npc909::*;
pub(super) use npc914::*;
pub(super) use ride::*;
pub(super) use shack_sign::*;
pub(super) use shuttles::*;
pub(super) use spawner::*;
pub(super) use walker::*;

/// Kerwan-specific classes with a ported actor (KER-9 ferry, KER-25 NPC 914). Registry: one entry per line, sorted;
/// add new classes here.
pub(super) const ACTORS: &[u32] = &[
	// KER-20a: the traffic hover-cars (`CAR_CLASSES`).
	75,
	115,
	116,
	117,
	118,
	119,
	120,
	132,
	// KER-20b: the cars' exhaust puff (pool only, no placements).
	PUFF_CLASS,
	SPAWNER,
	DOG,
	WALKER,
	LAYER,
	// KER-86a: the house 604 and territory marker 605 (moby 884 / 885..904), the shared Novalis arms.
	super::novalis::HOUSE,
	super::novalis::MARKER,
	// KER-2a: the blarg helicopter.
	HELI,
	// KER-68a: the path platform (moby 1..3), the shared Novalis updater.
	super::novalis::PLATFORM,
	// KER-48a: the infobot (moby 908).
	INFOBOT,
	// KER-67a: the Swingshot pull target (moby 909 / 910), the Aridia updater.
	super::aridia::PULL_TARGET,
	795,
	// KER-67a: the Swingshot swing target (moby 914).
	super::aridia::SWING_TARGET,
	FERRY,
	// KER-10: the statue (a breakable) and the spinning shack sign.
	STATUE,
	// KER-80a: the class 822 ride (drives the 845 flyer 925).
	RIDE822,
	SHACK_SIGN,
	// KER-86a: the RC limiter 832 (moby 924), the shared Novalis arm.
	super::novalis::RC_LIMIT,
	// KER-3: the model-less flyer 925 (a `Kind::Jet` prop, `jets::KER_JET`), driven by class 822.
	super::jets::KER_JET,
	SHUTTLE_868,
	HELGA,
	// KER-27: the conveyor carrier (pool only, no placements); KER-26: its emitter.
	CARRIER,
	BELT_EMITTER,
	SHUTTLE_905,
	NPC909,
	NPC914,
	// KER-31: the placements that despawn on their first update.
	SELF_DESTROY_915,
	SELF_DESTROY_916,
	SELF_DESTROY_917,
	SHUTTLE_928,
	RISING_GATE,
	LIFT,
	// KER-47a: the three gold bolts (moby 963..965), the shared Novalis updater.
	super::novalis::GOLD_BOLT,
	// KER-81a: the class 822 ride's followers.
	FOLLOWER,
	// KER-71: the falling, fading debris piece (no placement; its spawner pool is KER-77's).
	DEBRIS_1221,
	DIRECTOR,
	// KER-87a: the sand mouse 1818 (moby 989), the shared Novalis arm; its bolt pool 1633 is admitted in
	// `LevelActors::is_prop_class`.
	super::novalis::MOUSE,
	// KER-29: the breakable prop.
	BREAKABLE,
];

/// Whether `class` has a Kerwan actor.
pub(super) fn is_actor(class: u32) -> bool {
	ACTORS.contains(&class)
}

/// Kerwan's COL-14 static solids (`levels::static_solids`). One entry per line, sorted.
pub(super) const STATIC_SOLIDS: &[u32] = &[
	// KER-35a (decomp/notes/kerwan_environment.md): placed classes with no update fn whose collision block makes them
	// solid in retail (the moby's collision pointer is set from the class at placement). 1195 has no block, so it is absent.
	883,
	1162,
	1163,
	1164,
	1165,
	1166,
];

/// KER-0b: Kerwan's part of the placed-flag table (`sched::placed_flags`). Only values a checked port spec gives; the
/// research census (codex_sched_fields_2026-10-04) has no Kerwan rows yet. One entry per line, sorted; a class placed
/// on several levels lives in `sched::SHARED_PLACED_FLAGS`.
pub(super) const PLACED_FLAGS: &[(u32, u16)] = &[
	// KER-20a: the traffic hover-cars' initial +0x34 (`flags:N`): 0x4060 on the small cars, 0x4020 on class 795.
	(75, 0x4060),
	(115, 0x4060),
	(116, 0x4060),
	(117, 0x4060),
	(118, 0x4060),
	(119, 0x4060),
	(120, 0x4060),
	(132, 0x4060),
	// KER-20b: the exhaust puff's class flags (`flags:235`).
	(PUFF_CLASS, 0x40),
	// KER-0d (`flags:N`, placement loader FUN_0023d038 class header +0x44 rule): the lobbing spawner, header 0x5040
	// (registry 0x1000, one-frame 0x40) plus the placement mode bit 0x20; the registry bit is live before its state 0.
	(SPAWNER, 0x5060),
	// KER-4: the dog's class flags 0x4410 (header 0x4000, 0x10, 0x400) plus the placement mode bit 0x20; no registry bit
	// (its state 0 sets it).
	(DOG, 0x4430),
	// KER-1a: the walker's class flags 0x5410 (header 0x5000, 0x10, 0x400) plus the placement mode bit 0x20; the
	// registry bit 0x1000 is live before its state 0 sets it again.
	(WALKER, 0x5430),
	// KER-7: the mine layer (updater, registry 0x1000, header 0x4000, shadow byte 0x400, placement mode bits 0x20).
	(LAYER, 0x5420),
	// KER-2a: the helicopter (`flags:631`: class header 0x5400 with the registry bit 0x1000, placement mode bits 0x20).
	(HELI, 0x5420),
	// KER-48a: the infobot (`flags:750` = 0x10).
	(INFOBOT, 0x10),
	(795, 0x4020),
	// KER-9: the carry bit every ferry placement sets (+0x08 = 0x20), so `Game::carry` moves the hero standing on it.
	(FERRY, 0x20),
	// KER-10: the statue (`flags:817`), as the 1827 breakable.
	(STATUE, 0x4040),
	// KER-80a: the ride's class flags (`flags:822`).
	(RIDE822, 0x60),
	// KER-10: the shack sign (`flags:825`): 0x40 | the 0x20 carry bit, so `Game::carry` turns its rider.
	(SHACK_SIGN, 0x60),
	// KER-3: the flyer's class flags (`flags:845`): updater bits only, no skip bit until class 822 sets +0x34 |= 6.
	(super::jets::KER_JET, 0x25),
	// KER-24: the shuttles' initial flags (0x40 from the class header, 0x20 the carry bit of the placement mode bits).
	(SHUTTLE_868, 0x60),
	// KER-0d: the static solids 883 (no updater 0x2 + one-frame 0x40, scheduler-skipped but solid as Veldin 1964), 890
	// (0x400) and the model-less 899 (0x5).
	(883, 0x42),
	(890, 0x400),
	(899, 0x5),
	(SHUTTLE_905, 0x60),
	// KER-25 / KER-0d: the NPC's class flags (`flags:914`): header 0x4000 (the hit-target bit the writers' `actor_collect`
	// gate reads), 0x10 and 0x400; placement mode bits 0 (corrected 2026-10-07: was 0x4000 with the other bits
	// underived).
	(NPC914, 0x4410),
	// KER-31 (`flags:915,916,917`): class header 0x4040 plus the placement mode bit 0x20; no registry bit.
	(SELF_DESTROY_915, 0x4060),
	(SELF_DESTROY_916, 0x4060),
	(SELF_DESTROY_917, 0x4060),
	(SHUTTLE_928, 0x60),
	// KER-0d: the model-less 994 (0x5) and the one-frame 997 (0x40).
	(994, 0x5),
	(997, 0x40),
	// KER-33: the lift's carry bit (header mode bits 0x20 on both placements), so `Game::carry` moves its rider.
	(LIFT, 0x20),
	// KER-0d: the null-update static classes 1162-1166 and 1195 (no updater 0x2 + one-frame 0x40, as Veldin 1520 / 1964).
	(1162, 0x42),
	(1163, 0x42),
	(1164, 0x42),
	(1165, 0x42),
	(1166, 0x42),
	(1195, 0x42),
	// KER-81a: the followers' class flags (`flags:1210` read from the out-of-order class rows): 0x40 | the 0x20 carry
	// bit; the ride adds the 0x6 skip bits at runtime.
	(FOLLOWER, 0x60),
	// KER-71: the debris piece's class flags (`flags:1221`).
	(DEBRIS_1221, 0x40),
	// KER-21: the model-less help director (updater, no skip bit), as Aridia 1324 and Novalis 1341 [I].
	(DIRECTOR, 0x5),
	// KER-28: the scene-gated exhaust/smoke emitter (`flags:1548`); scheduler fidelity only, it has no Actor (its
	// emitters run viewer-side in `cutscene_emit`).
	(1548, 0x5),
	// KER-29: the breakable prop (`flags:1827`); no registry bit, the hit collector's shape arm is its victim path.
	(BREAKABLE, 0x4040),
];


/// Kerwan's part of `fragment::FRAGMENT_PARTS` (KER-20c car pieces, KER-5 spawner pieces, KER-4 dog pieces). One
/// entry per line, sorted.
pub(super) const FRAGMENT_CLASSES: &[u32] = &[
	1591,
	1592,
	1593,
	1705,
	1706,
	1707,
	1708,
	1709,
	1710,
	1711,
	1712,
	1713,
	1714,
	1715,
	1716,
	1717,
	1718,
	1719,
	1720,
	1721,
	1722,
	1724,
	1726,
	1727,
	1733,
	1734,
	1735,
	1742,
	1743,
	// KER-1e: the walker's three death pieces (WALKER_PIECES).
	1744,
	1745,
	1746,
	// KER-29: the breakable's static remains and its piece.
	1828,
	1829,
	// KER-10: the statue's three fragments (STATUE_PIECES).
	1831,
	1832,
	1833,
	1917,
];
/// Kerwan's part of `game::SOUND_CLASSES` (KER-20c: the cars' destroy cue 2; KER-5: the spawner's death cue 1). One
/// entry per line, sorted.
pub(super) const SOUND_CLASSES: &[u32] = &[
	CAR_CLASSES[0], // 75
	CAR_CLASSES[1], // 115
	CAR_CLASSES[2], // 116
	CAR_CLASSES[3], // 117
	CAR_CLASSES[4], // 118
	CAR_CLASSES[5], // 119
	CAR_CLASSES[6], // 120
	CAR_CLASSES[7], // 132
	SPAWNER, // 455
	LOB, // 545
	WALKER, // 574
	HELI, // 631: KER-2e, the knock cue 1
	CAR_CLASSES[8], // 795
	STATUE, // 817: KER-10, the break cue 0
	CARRIER, // 898: KER-26, the emitter's loop cue 0 of the carrier class
	BREAKABLE, // 1827: KER-29, the break cue 0
];
/// Kerwan's part of `game::JOINT_WORLD_CLASSES` (BRIDGE-3; KER-20b: the cars' exhaust joints; KER-5: the spawner's
/// launch joint 0). One entry per line, sorted.
pub(super) const JOINT_WORLD_CLASSES: &[u32] = &[
	CAR_CLASSES[0], // 75
	CAR_CLASSES[1], // 115
	CAR_CLASSES[2], // 116
	CAR_CLASSES[3], // 117
	CAR_CLASSES[4], // 118
	CAR_CLASSES[5], // 119
	CAR_CLASSES[6], // 120
	CAR_CLASSES[7], // 132
	SPAWNER,
	// KER-91: the helicopter's gun barrels (attach points 2 / 3) the aim marker fires from.
	HELI,
	// KER-80a: the ride's joint 0 (its state-0 distance).
	RIDE822,
	// KER-81a: the followers' joint 0 (their offset) and joint 1 (the next link's anchor).
	FOLLOWER,
];




