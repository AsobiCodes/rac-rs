//! Follow poses (ABI 1.6 `pose_follow`; the loader's `omni.pose.follow`): the HOST STORE and the callback. A mod asks that the moby `item`
//! be DRAWN where `source` is relative to the hero joint `from`, but relative to the hero joint `to` instead (optionally mirrored): the Dual Blasters
//! perk draws its left gun in the left hand exactly where the right gun sits in the right hand. PICTURE ONLY: nothing here writes game RAM, the
//! item's game position and everything the game does with it stay as they are. The renderer (viewer/follow.rs) turns the records into the item
//! prop's matrix every displayed frame, from the joint matrices it draws that frame (so the item never lags behind the body).
//!
//! Up to 8 records, one per item (calling again for the same item replaces the record). The auto-clear rules are the ones of pose ops (pose.rs):
//! level change, a movie / cutscene / vendor scene, the hero's death states, the hero cell changing, a new game or a loaded save, the mods being
//! replaced (`clear_all`); on top of them a record is dropped when its item or its source slot no longer holds a live moby of the same class
//! (the game reuses deleted slots). A mod that unloads removes its records through the loader (flag `REMOVE`).

use super::host::{cpu, state, HostState};
use super::{events, mobys, pose};
use std::ffi::c_void;
use std::sync::Mutex;

/// Records the store holds.
pub const MAX_RECORDS: usize = 8;
/// `pose_follow` flags (include/omniloader.h).
pub const FLAG_MIRROR: u32 = 1;
pub const FLAG_REMOVE: u32 = 2;
/// Return codes.
pub const ERR_BAD_JOINT: i32 = -3;
pub const ERR_NOT_A_MOBY: i32 = -7;
pub const ERR_CAP: i32 = -8;

const MASK: u32 = 0x01ff_ffff;

/// One follow pose for the renderer (`HeroParts.follow`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FollowRec {
	/// The moby that is drawn at the followed place (masked address).
	pub item: u32,
	/// The moby whose place relative to the `from` joint is copied.
	pub source: u32,
	pub from: u16,
	pub to: u16,
	/// Left/right mirror of the place (the item is then drawn left-handed).
	pub mirror: bool,
}

#[derive(Clone, Copy, Debug)]
struct Rec {
	rec: FollowRec,
	item_class: u16,
	source_class: u16,
}

/// The records and what the auto-clear rules remember.
#[derive(Default)]
pub struct Store {
	recs: Vec<Rec>,
	level: Option<i32>,
	hero: u32,
}

impl Store {
	pub const fn new() -> Store {
		Store { recs: Vec::new(), level: None, hero: 0 }
	}

	/// Adds or replaces the record of `rec.item`.
	pub fn set(&mut self, rec: FollowRec, item_class: u16, source_class: u16) -> Result<(), i32> {
		let new = Rec { rec, item_class, source_class };
		if let Some(r) = self.recs.iter_mut().find(|r| r.rec.item == rec.item) {
			*r = new;
		} else if self.recs.len() >= MAX_RECORDS {
			return Err(ERR_CAP);
		} else {
			self.recs.push(new);
		}
		Ok(())
	}

	/// Removes the record of `item`; false when there was none.
	pub fn remove(&mut self, item: u32) -> bool {
		let n = self.recs.len();
		self.recs.retain(|r| r.rec.item != item);
		self.recs.len() != n
	}

	pub fn len(&self) -> usize {
		self.recs.len()
	}

	pub fn is_empty(&self) -> bool {
		self.recs.is_empty()
	}

	pub fn clear_all(&mut self) {
		self.recs.clear();
	}

	/// The auto-clear rules, once per game field. `class_of(addr)` = the class of the live moby at `addr`, None for a slot that is not a live moby.
	pub fn run_rules(&mut self, level: i32, exit: bool, scene: bool, hero: u32, dead: bool, class_of: impl Fn(u32) -> Option<u16>) {
		let changed = self.level != Some(level) || exit || self.hero != hero;
		self.level = Some(level);
		self.hero = hero;
		if changed || scene || dead {
			self.recs.clear();
			return;
		}
		self.recs.retain(|r| class_of(r.rec.item) == Some(r.item_class) && class_of(r.rec.source) == Some(r.source_class));
	}

	/// The records for the renderer.
	pub fn records(&self) -> Vec<FollowRec> {
		self.recs.iter().map(|r| r.rec).collect()
	}
}

static STORE: Mutex<Store> = Mutex::new(Store::new());

fn lock() -> std::sync::MutexGuard<'static, Store> {
	STORE.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The `pose_follow` entry of the `OmniHost` (runs on the worker thread inside a mod's callback).
pub unsafe extern "C" fn pose_follow_cb(user: *mut c_void, item: u32, source: u32, from: u32, to: u32, flags: u32) -> i32 {
	let item = item & MASK;
	// A remove always succeeds (also for an item the auto-clear rules already dropped, and when no level is up).
	if flags & FLAG_REMOVE != 0 {
		lock().remove(item);
		return 0;
	}
	let Some(c) = cpu() else { return -1 };
	if events::live_level(c).is_none() {
		return -1;
	}
	let ram = &c.mem.ram;
	let hero = pose::hero_addr(ram);
	let Some(joints) = pose::hero_joints(ram, hero) else { return -2 };
	let source = source & MASK;
	if from >= joints || to >= joints {
		return ERR_BAD_JOINT;
	}
	let class_of = mobys::class_lookup(state(user), ram);
	let (Some(ic), Some(sc)) = (class_of(item), class_of(source)) else { return ERR_NOT_A_MOBY };
	if item == hero || source == hero || item == source {
		return ERR_NOT_A_MOBY;
	}
	let rec = FollowRec { item, source, from: from as u16, to: to as u16, mirror: flags & FLAG_MIRROR != 0 };
	match lock().set(rec, ic, sc) {
		Ok(()) => 0,
		Err(code) => code,
	}
}

/// Once per game field (`omni::tick`, after `pose::tick`): the auto-clear rules.
pub fn tick(ram: &[u8], st: &HostState, level: i32, exit: bool, mode: u32) {
	let mut s = lock();
	let (hero, dead, scene) = (pose::hero_addr(ram), pose::hero_dead(ram), pose::scene_mode(mode));
	if s.is_empty() {
		// Nothing to check, but the level and hero memory stay current so a record made later is not cleared as "changed".
		s.run_rules(level, exit, scene, hero, dead, |_| None);
		return;
	}
	s.run_rules(level, exit, scene, hero, dead, mobys::class_lookup(st, ram));
}

/// A save was loaded, a new game starts or the mods were replaced: everything goes at once.
pub fn clear_all() {
	lock().clear_all();
}

/// The records for `HeroParts.follow` (retail/snapshot.rs; `posable` = class 0 with 111 joints, checked there): empty in a scene or when the hero
/// cannot be posed.
pub fn snapshot_for(posable: bool, scene: bool) -> Vec<FollowRec> {
	let s = lock();
	if s.is_empty() || scene || !posable {
		return Vec::new();
	}
	s.records()
}

#[cfg(test)]
#[path = "pose_follow_tests.rs"]
mod tests;
