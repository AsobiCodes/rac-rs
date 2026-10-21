//! Novalis mechanisms (decomp/notes/novalis_environment.md, owner of every class here): proximity
//! doors, the bolt cranks and what they drive, pistons, the path platform, collapsing rocks, fans,
//! lily pads, teleporters, gold bolts and the breakable props. The game's authored timers are scale(n) PAL ticks;
//! here they are seconds of PAL_DT (`ticks`). Solid actors block the hero with their class collision blocks (Game::solids).

use super::Game;

pub(super) const CRANK: u32 = 280;
/// NOV-14.3c the sand mouse's bolt (game/mouse.rs).
pub(super) const MOUSE_SHOT: u32 = 1633;
const CAVE_DOOR: u32 = 641;
const GATE: u32 = 665;
const LILY: u32 = 695;
pub(super) const SMOKE_EMITTER: u32 = 700;
const BRIDGE_PIECE_1: u32 = 709;
const BRIDGE_PIECE_3: u32 = 711;
pub(super) const BRIDGE: u32 = 746;
use super::mission::{GOLD_CASE, GOLD_DISPLAY_FIRST, GOLD_DISPLAY_LAST, INFOBOT, MISSION_NPC, MISSION_NPC_2, PLUMBER};
use super::vendor::VENDOR;
const ROCK: u32 = 701;
const PISTON: u32 = 703;
const ROCK_FORMATION: u32 = 704;
const FAN: u32 = 705;
const PISTON_2: u32 = 715;
pub(in crate::game) const PLATFORM: u32 = 726;
const CAVE_WALL: u32 = 729;
const MUSHROOM_POT: u32 = 754;
const DOOR_LEFT: u32 = 768;
const DOOR_RIGHT: u32 = 769;
const PIPE: u32 = 778;
/// NOV-9.3b: the smoking replacement the broken pipe leaves (FUN_002ff3c0 creator, FUN_002ff458 updater).
const PIPE_CHILD: u32 = 779;
const SHROOM_UNDERSIDE: u32 = 1042;
pub(super) const GOLD_BOLT: u32 = 1134;
const TELEPORTER: u32 = 1135;
const TALL_SHROOM: u32 = 1813;
/// NOV-14.2 (decomp/notes/novalis_checklist.md "NOV-14.2"): the mouse's house, the bolt-crate territory markers and
/// the RC Visibomb range limiter.
pub(super) const HOUSE: u32 = 604;
pub(super) const MARKER: u32 = 605;
pub(super) const RC_LIMIT: u32 = 832;
/// NOV-15.1: the environment UV scroller (FUN_0030eb38) and the pulsing strips (FUN_002fe488).
pub(super) const ENV_SCROLL: u32 = 1848;
pub(super) const PULSE_STRIP: u32 = 761;
/// NOV-14.5 (B): the draw-only strip surfaces (FUN_002f5a58 / 2f5ab0 / 3095c8 updaters).
pub(super) const STRIP_676: u32 = 676;
pub(super) const STRIP_678: u32 = 678;
pub(super) const STRIP_1225: u32 = 1225;
/// NOV-14.3 the sand mouse (game/mouse.rs).
pub const MOUSE: u32 = 1818;


/// Novalis's part of `game::CUE_CLASSES` (fighter, bomber, Birdbot, Amoeboids, Peckbot, house, fan, Mayor; REN-13
/// scenery clock: inner support 747). Registry: one entry per line, sorted; add new classes in the owning level module.
pub(super) const CUE_CLASSES: &[u32] = &[
	459,
	572,
	577,
	604,
	660,
	688,
	705,
	747,
	790,
	865,
	866,
];
/// Novalis's part of `game::SOUND_CLASSES` (one entry per line, sorted; add new classes in the owning level module).
pub(super) const SOUND_CLASSES: &[u32] = &[
	CRANK, // 280
	GATE, // 665
	686,
	ROCK, // 701
	PISTON, // 703
	ROCK_FORMATION, // 704
	PISTON_2, // 715
	super::foes::SHOT, // 722
	PLATFORM, // 726
	CAVE_WALL, // 729
	BRIDGE, // 746
	MUSHROOM_POT, // 754
	DOOR_LEFT, // 768
	DOOR_RIGHT, // 769
	PIPE, // 778
	PIPE_CHILD, // 779
	crate::water::DROP_CLASS, // 787
	GOLD_BOLT, // 1134
	TELEPORTER, // 1135
	1510,
	TALL_SHROOM, // 1813
	MOUSE, // 1818
];
/// The engine-wide part of `game::SOUND_CLASSES`: the vendor, the weapons and (WPN-6.9d4) the held rig classes whose
/// sequence triggers and direct cues play (180 Walloper and 190 Mine Glove have no public class const), the held Suck
/// Cannon 849. One entry per line, sorted.
pub(super) const SHARED_SOUND_CLASSES: &[u32] = &[
	VENDOR, // 11
	super::devastator::HELD, // 157
	super::visibomb::HELD, // 163
	super::blaster::HELD, // 168
	super::visibomb::MISSILE, // 172
	super::taunter::HELD, // 175
	super::pyro::PYRO_CLASS, // 176
	180,
	super::morph::HELD, // 185
	190,
	super::doom::HELD, // 229
	super::morph::SHEEP, // 270
	849,
	1007, // KER-69: the camera marker's free-look start cue
];
/// Novalis's part of `game::JOINT_WORLD_CLASSES` (BRIDGE-3: fighter 660, bomber 688, gold bolt). One entry per line,
/// sorted; add new classes in the owning level module.
pub(super) const JOINT_WORLD_CLASSES: &[u32] = &[
	super::jets::FIGHTER, // 660
	super::jets::BOMBER, // 688
	GOLD_BOLT, // 1134
];
/// Novalis's COL-14 static solids (`levels::static_solids`: the 742 door frames). One entry per line, sorted.
pub(super) const STATIC_SOLIDS: &[u32] = &[
	742,
];
/// Novalis's part of the placed-flag table (`sched::placed_flags`). One entry per line, sorted; a class placed on several
/// levels lives in `sched::SHARED_PLACED_FLAGS`.
pub(super) const PLACED_FLAGS: &[(u32, u16)] = &[
	(27, 0x5),
	(66, 0x0),
	(280, 0x40),
	(304, 0x40),
	(310, 0x42),
	(459, 0x4430),
	(502, 0x5060),
	(572, 0x5430),
	(577, 0x5430),
	(613, 0x5),
	(641, 0x40),
	(660, 0x4020),
	(665, 0x40),
	(666, 0x20),
	(676, 0x5),
	(678, 0x5),
	(679, 0x5),
	(687, 0x42),
	(688, 0x4020),
	(695, 0x40),
	(700, 0x5),
	(701, 0x60),
	(703, 0x60),
	(704, 0x4040),
	(705, 0x0),
	(709, 0x4040),
	(710, 0x4040),
	(711, 0x4040),
	(715, 0x60),
	(724, 0x0),
	(725, 0x0),
	(729, 0x4040),
	(730, 0x0),
	(742, 0x42),
	(746, 0x60),
	(747, 0x0),
	(750, 0x10),
	(751, 0x5),
	(754, 0x4000),
	(761, 0x5),
	(768, 0x40),
	(769, 0x40),
	(774, 0x400),
	(778, 0x4040),
	(780, 0x42),
	(790, 0x0),
	(815, 0x40),
	(865, 0x5430),
	(866, 0x5430),
	(1042, 0x4040),
	(1135, 0x50),
	(1225, 0x5),
	(1341, 0x5),
	(1456, 0x40),
	(1457, 0x40),
	(1458, 0x40),
	(1459, 0x40),
	(1460, 0x40),
	(1461, 0x40),
	(1462, 0x40),
	(1463, 0x40),
	(1464, 0x40),
	(1465, 0x40),
	(1504, 0x5),
	(1546, 0x5),
	(1813, 0x4040),
	(1848, 0x5),
];

pub(super) fn is_actor(class: u32) -> bool {
	matches!(
		class,
		CRANK | CAVE_DOOR | GATE | LILY | ROCK | PISTON | ROCK_FORMATION | FAN | PISTON_2 | PLATFORM | CAVE_WALL | MUSHROOM_POT | DOOR_LEFT | DOOR_RIGHT | PIPE | SHROOM_UNDERSIDE | GOLD_BOLT | TELEPORTER | TALL_SHROOM | BRIDGE | BRIDGE_PIECE_1..=BRIDGE_PIECE_3 | MISSION_NPC | MISSION_NPC_2 | PLUMBER | INFOBOT | VENDOR | GOLD_CASE | GOLD_DISPLAY_FIRST..=GOLD_DISPLAY_LAST | HOUSE | MARKER | RC_LIMIT | ENV_SCROLL | PULSE_STRIP | STRIP_676 | STRIP_678 | STRIP_1225 | MOUSE | MOUSE_SHOT | 696..=698 | SMOKE_EMITTER | PIPE_CHILD
	)
}











/// Rings: class 315, yaw step 2pi/3, (z offset -0.4, base pitch -20 deg, spread 2.4, bias -1.2, pitch swing -60 deg).
pub const TELE_RING_CLASS: u32 = 315;





















impl Game {













	/// NOV-15.1b: loads the class-761 strip records and the class-1848 pass tables from the level overlay image
	/// (empty when the image does not hold them).
	pub fn set_strip_tables(&mut self, ov: &[u8]) {
		self.strips = crate::strips::parse_strips(ov).unwrap_or_default();
		self.env_passes = crate::strips::parse_env_passes(ov).unwrap_or_default();
		self.strip_groups = [crate::strips::TABLE_676, crate::strips::TABLE_678, crate::strips::TABLE_1225].map(|(base, n)| crate::strips::StripGroup::new(crate::strips::parse_records(ov, base, n).unwrap_or_default()));
	}


























}





