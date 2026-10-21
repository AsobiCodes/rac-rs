//! The actor scheduler (FUN_002650e0) and the 64-slot hit ring (0x178500), decomp/notes/collision_and_gameplay.md
//! "Novalis collector and hit-ring refinement" and research/novalis_2026-10-04/handoff.md section 3.



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



/// Initial actor flags of placed classes on Veldin, Novalis, Aridia and Kerwan (KER-0b: spec-given entries only) (allocator 0x263020 + class +0x44 + animation rule),
/// from the research census (codex_sched_fields_2026-10-04). Runtime edits of the skip/hide bits are not modelled [P].
/// Registry: the per-level parts merged and sorted by class at compile time (binary search). One entry per line, sorted;
/// add new classes in the owning level module (a class placed on several levels may sit in either part or here, with the
/// same value).
static PLACED_FLAGS: [(u32, u16); super::levels::total_len(PLACED_FLAG_PARTS)] = super::levels::merge_by_class(PLACED_FLAG_PARTS);
pub(crate) const PLACED_FLAG_PARTS: &[&[(u32, u16)]] = &[SHARED_PLACED_FLAGS, VELDIN_PLACED_FLAGS, super::novalis::PLACED_FLAGS, super::aridia::PLACED_FLAGS, super::kerwan::PLACED_FLAGS];
/// Classes placed on more than one level (one entry per line, sorted).
const SHARED_PLACED_FLAGS: &[(u32, u16)] = &[
	(0, 0x4416),
	(11, 0x10),
	(13, 0x0),
	(500, 0x5060),
	(501, 0x5060),
	(505, 0x5060),
	(511, 0x5060),
	(604, 0x0),
	(605, 0x5),
	// KER-68a: the path platform (Novalis and Kerwan), placement mode bit 0x20.
	(726, 0x20),
	(737, 0x5),
	(760, 0x5),
	// KER-67a: the Swingshot swing target 803 (Aridia and Kerwan); its pull twin 758 is 0x0.
	(803, 0x40),
	(805, 0x5),
	(809, 0x5),
	(832, 0x5),
	(1060, 0x40),
	(1134, 0x0),
	(1818, 0x10),
];
/// Veldin's placed classes (one entry per line, sorted).
const VELDIN_PLACED_FLAGS: &[(u32, u16)] = &[
	(530, 0x10),
	(749, 0x5430),
	(834, 0x60),
	(1137, 0x0),
	(1413, 0x5),
	(1440, 0x5430),
	(1471, 0x5),
	(1520, 0x42),
	(1545, 0x5),
	(1564, 0x20),
	(1781, 0x0),
	(1782, 0x0),
	(1964, 0x42),
];

/// Initial flags for a placed class (0 for classes not placed on those two levels).
pub fn placed_flags(class: u32) -> u16 {
	PLACED_FLAGS.binary_search_by_key(&class, |e| e.0).map_or(0, |i| PLACED_FLAGS[i].1)
}

/// SCH-6b spawn rows (class, rank Veldin, rank Novalis, dist, override, draw distance); u32::MAX = not in that level's table. Every row
/// is the creator's literal writes of +0x30 / +0x31 / +0x32 (SCH-6b); 806 and 1143 write only some of them.
const NO_ROW: u32 = u32::MAX;
const SPAWN_ROWS: &[(u32, u32, u32, u8, u8, i16)] = &[
	(13, 8, 9, 64, 1, 64), (14, 9, 10, 64, 1, 64), (15, 10, 11, 64, 1, 64), (16, 11, 12, 64, 1, 64),
	(74, 13, 16, 255, 1, 127), (121, 16, 18, 255, 1, 255), (153, 19, 21, 255, 1, 255), (172, 25, 28, 255, 1, 255), (179, 30, 33, 255, 1, 255), (226, 47, 50, 255, 1, 255), (270, 50, 53, 127, 1, 127), (305, 51, 56, 127, 1, 127), (428, 61, 70, 64, 1, 32), (686, NO_ROW, 110, 255, 1, 255),
	(696, NO_ROW, 114, 255, 1, 255), (697, NO_ROW, 115, 255, 1, 255), (698, NO_ROW, 116, 255, 1, 255),
	(722, NO_ROW, 126, 127, 1, 127), (779, NO_ROW, 146, 32, 1, 255), (806, 88, 152, 64, 0, 0),
	(1143, 95, 165, 0, 0, 64), (1510, NO_ROW, 189, 255, 1, 255), (1633, NO_ROW, 191, 255, 1, 127),
	(1736, NO_ROW, 192, 255, 1, 128), (1737, NO_ROW, 193, 255, 1, 128), (1738, NO_ROW, 194, 255, 1, 128), (1770, NO_ROW, 201, 255, 1, 128),
	(1815, NO_ROW, 204, 255, 1, 128), (1816, NO_ROW, 205, 0, 1, 255), (1817, NO_ROW, 206, 255, 1, 128),
	// ARI-35 / ARI-38 (Aridia only; rank from the level's class rows): the shared fragment creator FUN_00265148 writes
	// 255 / 1 / 0x80, the static remains creator FUN_00265490 0 / 1 / 255.
	(1821, NO_ROW, NO_ROW, 255, 1, 128), (1820, NO_ROW, NO_ROW, 0, 1, 255),
	(1522, NO_ROW, NO_ROW, 255, 1, 128), (1525, NO_ROW, NO_ROW, 255, 1, 128), (1936, NO_ROW, NO_ROW, 255, 1, 128),
	(1526, NO_ROW, NO_ROW, 255, 1, 128), (1528, NO_ROW, NO_ROW, 255, 1, 128), (1529, NO_ROW, NO_ROW, 255, 1, 128),
	// ARI-27a: the launcher rocket creator FUN_002ebb58 writes 255 / 1 / 255.
	(1208, NO_ROW, NO_ROW, 255, 1, 255),
	// KER-5 (Kerwan only): the lob creator FUN_002c3f08 writes +0x31 = 1 (dist / draw from the parent, then the caller
	// 255); the shared fragment creator 0x251c48 writes 255 / 1 / 0x80 for the spawner's death pieces.
	(545, NO_ROW, NO_ROW, 255, 1, 0),
	(1733, NO_ROW, NO_ROW, 255, 1, 128), (1734, NO_ROW, NO_ROW, 255, 1, 128), (1735, NO_ROW, NO_ROW, 255, 1, 128),
	// KER-27 (Kerwan only): the conveyor emitter's carrier creator FUN_002daac8 writes 0x80 / 1 / 0x40.
	(898, NO_ROW, NO_ROW, 0x80, 1, 0x40),
	// KER-1j (Kerwan only): the walker shot creator FUN_002d3bb8 writes 0xff / 1 / 0x7e.
	(833, NO_ROW, NO_ROW, 0xff, 1, 0x7e),
];
/// Scheduler state of a spawned (creator-allocated) actor: class-table row as rank and the creator's literal +0x30/+0x31
/// (spawn_classes.tsv). Classes without a literal write, or absent from the level's table, keep the defaults [P].
/// ARI-6: the rank columns are Veldin's and Novalis's class-table rows (level 0 and no level read Veldin's); any other
/// level gets rank u32::MAX here and its own row from the level data through `spawned_in` / `Game::set_class_rows`.
pub fn spawned(level: i32, class: u32) -> Sched {
	let mut s = Sched::default();
	if let Some(e) = SPAWN_ROWS.iter().find(|e| e.0 == class) {
		let rank = match level {
			1 => e.2,
			l if l <= 0 => e.1,
			_ => u32::MAX,
		};
		(s.rank, s.dist, s.over, s.draw) = (rank, e.3, e.4, e.5);
		if class == 1143 {
			// SCH-6: the vendor orb also sets the skip flag (the parent vendor drives it).
			s.flags = 2;
		}
	}
	s
}

/// Whether a spawned class has a creator row (and so a class-table rank) in `spawned`.
pub fn has_spawn_row(class: u32) -> bool {
	SPAWN_ROWS.iter().any(|e| e.0 == class)
}







thread_local! {
	static SHARED: std::cell::RefCell<Shared> = std::cell::RefCell::new(Shared {  });
}

/// BRIDGE-15: the one physical slot allocator of the level, shared by `Game` (spawned Props) and `Particles` (effect
/// mobys), plus the Game's tick (`Game.frames`) once a Game has run: every claim and release stamps with it.
pub struct Shared {
}

/// Runs `f` on the level's shared slot allocator.
pub fn with_shared<R>(f: impl FnOnce(&mut Shared) -> R) -> R {
	SHARED.with(|s| f(&mut s.borrow_mut()))
}

/// Level load (app.rs): `d` is the actor header's D; the allocator starts empty (first state 0xff, stamp 0).
pub fn reset_slots(d: usize) {
	with_shared(|s| *s = Shared {  });
}



















