//! ARI-6: the one per-level dispatch point for actor setup. Every placed moby whose class the level's entry does not
//! claim stays a static model at its authored transform (the viewer bakes it into the scenery), so an unported class
//! never crashes and never vanishes.
//!
//! Class code lives in each level's overlay (every class table row of the classes below is an overlay address, none
//! in the boot ELF), so "shared" means: the rac-rs implementation is the engine-wide one and every port level uses it.

use super::{vendor, AMMO_CRATE, BOLT_CRATE, NANOTECH_CRATE, SHIP};

/// The actor table a level builds from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LevelActors {
	/// Level 0: shared + Veldin + (as before ARI-6) every other ported class.
	Veldin,
	/// Level 1: shared + Novalis + (as before ARI-6) every other ported class.
	Novalis,
	/// Level 2: shared + the Aridia registry (`aridia::is_actor`).
	Aridia,
	/// Level 3: shared + the Kerwan registry (`kerwan::is_actor`, KER-0a).
	Kerwan,
	/// Levels 4..: shared only until their registry exists.
	Unported,
	/// No level (-1: `Game::new`, unit scenes): every ported class, as before ARI-6.
	All,
	/// Retail mode (retail.rs): the interpreter runs every moby; each class draws from a pool of inert props.
	Retail,
}

impl LevelActors {
	pub fn of(level: i32) -> LevelActors {
		if level >= 0 && crate::retail::active() {
			return LevelActors::Retail;
		}
		match level {
			0 => LevelActors::Veldin,
			1 => LevelActors::Novalis,
			2 => LevelActors::Aridia,
			3 => LevelActors::Kerwan,
			l if l < 0 => LevelActors::All,
			_ => LevelActors::Unported,
		}
	}

	/// The placed classes this level builds as live props (instance order = prop order).
	pub fn is_dynamic(self, class: u32) -> bool {
		match self {
			// Veldin and Novalis keep the full pre-ARI-6 union so their prop lists, tick order and RNG draws are
			// unchanged (e.g. Veldin's 45 placed 572s still build as foes).
			LevelActors::Veldin | LevelActors::Novalis | LevelActors::All => super::is_dynamic(class),
			// ARI-36 / ARI-37: the relocated Novalis sand mouse 1818, house 604, territory marker 605 and RC limiter 832.
			LevelActors::Aridia => is_shared(class) || super::aridia::is_actor(class) || super::aridia::RELOCATED_ACTORS.contains(&class),
			LevelActors::Kerwan => is_shared(class) || super::kerwan::is_actor(class),
			LevelActors::Unported => is_shared(class),
			LevelActors::Retail => false,
		}
	}

	/// Classes drawn as game props on this level: its live placed classes plus the engine-wide spawned pools. A
	/// placed class outside this set is baked into the scenery by the viewer.
	pub fn is_prop_class(self, class: u32) -> bool {
		match self {
			LevelActors::Veldin | LevelActors::Novalis | LevelActors::All => super::is_prop_class(class),
			LevelActors::Retail => class != 0,
			// ARI-36: the mouse's bolt pool 1633 draws on Aridia (a spawned pool, not a placed actor).
			LevelActors::Aridia if class == super::novalis::MOUSE_SHOT => true,
			// KER-87a: and on Kerwan, for the sand mouse (moby 989).
			LevelActors::Kerwan if class == super::novalis::MOUSE_SHOT => true,
			_ => self.is_dynamic(class) || (super::is_prop_class(class) && !super::is_dynamic(class)),
		}
	}

}

/// Engine-wide placed classes: the bolt pieces 13..16, the crates 500 / 501 / 502 / 505 / 511, the vendor 11 and the
/// landed ship 530. Their rac-rs builders read only the placement (pvars, slot, reward) and the class bounds.
pub fn is_shared(class: u32) -> bool {
	matches!(class, 13..=16 | BOLT_CRATE | NANOTECH_CRATE | 502 | 505 | AMMO_CRATE | SHIP) || class == vendor::VENDOR
}

/// COL-14 (decomp/notes/collision_and_gameplay.md): placed null-update classes with a collision block that the loader files
/// in the moby grid and nothing ever un-files (+0x94 = class +0x10 for life); `with_story` adds them as collision-only
/// props after every pool. Each level module owns its `STATIC_SOLIDS` slice (Novalis 742 door frames; Aridia 706, 720,
/// 727, 728, 1056; Kerwan none mapped yet).
pub fn static_solids(level: i32) -> &'static [u32] {
	match LevelActors::of(level) {
		LevelActors::Novalis => super::novalis::STATIC_SOLIDS,
		LevelActors::Aridia => super::aridia::STATIC_SOLIDS,
		LevelActors::Kerwan => super::kerwan::STATIC_SOLIDS,
		_ => &[],
	}
}

/// Registry plumbing. A class registry that several levels add to is split into per-level slices, each owned by its level
/// module (one entry per line, sorted; add new classes in the owning level module), so lanes adding different classes
/// touch different lines. The engine-wide registry is the compile-time concatenation of its `*_PARTS` (`cat`) or, for a
/// binary-searched `(class, value)` table, their merge sorted by class (`merge_by_class`).
pub(crate) const fn total_len<T>(parts: &[&[T]]) -> usize {
	let (mut n, mut p) = (0, 0);
	while p < parts.len() {
		n += parts[p].len();
		p += 1;
	}
	n
}

/// The slices of `parts` one after another, in order; `N` = `total_len(parts)`.
pub(crate) const fn cat<const N: usize>(parts: &[&[u32]]) -> [u32; N] {
	let mut out = [0; N];
	let (mut n, mut p) = (0, 0);
	while p < parts.len() {
		let mut i = 0;
		while i < parts[p].len() {
			out[n] = parts[p][i];
			n += 1;
			i += 1;
		}
		p += 1;
	}
	assert!(n == N);
	out
}

/// The `(class, value)` slices of `parts` merged into one table sorted by class (stable insertion sort, for
/// `binary_search_by_key`); `N` = `total_len(parts)`. A class in two slices must carry the same value (test below).
pub(crate) const fn merge_by_class<const N: usize>(parts: &[&[(u32, u16)]]) -> [(u32, u16); N] {
	let mut out = [(0, 0); N];
	let (mut n, mut p) = (0, 0);
	while p < parts.len() {
		let mut i = 0;
		while i < parts[p].len() {
			out[n] = parts[p][i];
			n += 1;
			i += 1;
		}
		p += 1;
	}
	assert!(n == N);
	let mut i = 1;
	while i < N {
		let mut j = i;
		while j > 0 && out[j - 1].0 > out[j].0 {
			let t = out[j - 1];
			out[j - 1] = out[j];
			out[j] = t;
			j -= 1;
		}
		i += 1;
	}
	out
}

impl super::Game {
	/// ARI-6: the level's raw class-table rows (`gameplay::class_rows`, the spawned-actor rank). On a level past
	/// Novalis the spawned pool props built before the level data was at hand take their rank from it; Veldin and
	/// Novalis ranks are the `sched::spawned` columns already (equal to these rows, disc test below).
	pub fn set_class_rows(&mut self, rows: std::collections::HashMap<u32, u32>) {
		if self.level >= 2 {
			for p in self.props.iter_mut().filter(|p| p.index.is_none()) {
				if super::sched::has_spawn_row(p.class_id) {
					p.sched.rank = rows.get(&p.class_id).copied().unwrap_or(u32::MAX);
				}
			}
		}
		self.class_rows = rows;
	}

	/// The level overlay's data tables the actors read (REN-2 ship mesh in Veldin's, NOV-15.1b strip tables in
	/// Novalis's); none mapped for the other levels.
	pub fn load_overlay_tables(&mut self, level: i32, ov: &[u8]) {
		match LevelActors::of(level) {
			LevelActors::Veldin => self.set_ship_mesh(ov),
			LevelActors::Novalis => self.set_strip_tables(ov),
			_ => {}
		}
	}
}

