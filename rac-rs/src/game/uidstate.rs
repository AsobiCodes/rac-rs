//! The UID tables of a level run (decomp/notes/savegame.md "UID checkpoint", death_respawn.md): the pending and
//! committed byte tables, the session and persistent bitmaps, the 64-record actor-state registry and the
//! checkpoint copy (FUN_0029a4f0 clear, FUN_0029a540 commit, FUN_0029a6f8 death restore, FUN_0029a9d0 registrar).

use super::*;

/// Table size: UIDs 0..0x7ff (the scan stops at 0x7fe).
pub(super) const UIDS: usize = 0x7ff;


#[derive(Clone)]
pub(super) struct UidState {
	/// Checkpoint block +0x454 (0x1bba84): what survives a death.
	committed: Vec<u8>,
	/// 0x1ba550: the session bitmap, cleared on a fresh level entry, never rolled back (corrected 2026-10-07: was
	/// 0x1ba8d0, which has no reader or writer; destroy_reward 0x245b78 sets 0x1ba550, KER-2a).
	session: [u8; 256],
	/// 0x14c190 + level * 0x100: the saved level bitmap, never rolled back.
	persistent: [u8; 256],
}

impl UidState {

	pub(super) fn new(persistent: [u8; 256], cache: [u8; 16]) -> UidState {
		UidState {  committed: vec![0; UIDS], session: [0; 256], persistent,     }
	}



	pub(super) fn committed(&self, uid: usize) -> bool {
		self.committed.get(uid).is_some_and(|&b| b != 0)
	}


	pub(super) fn session(&self, uid: usize) -> bool {
		uid < 0x800 && self.session[uid >> 3] >> (uid & 7) & 1 != 0
	}

	pub(super) fn persistent(&self, uid: usize) -> bool {
		uid < 0x800 && self.persistent[uid >> 3] >> (uid & 7) & 1 != 0
	}


	pub(super) fn persistent_bits(&self) -> &[u8; 256] {
		&self.persistent
	}

	/// The setters' pair of writes: the session and the saved level bit.
	pub(super) fn mark(&mut self, uid: usize) {
		if uid < 0x800 {
			self.session[uid >> 3] |= 1 << (uid & 7);
			self.persistent[uid >> 3] |= 1 << (uid & 7);
		}
	}






}

impl Game {
	/// The raw UID (0..0x800) of placement `idx` of `class`, if it has one.
	pub(super) fn raw_uid(&self, class: u32, idx: u32) -> Option<usize> {
		self.uid_map.iter().find(|m| m.0 == class && m.1 == idx).map(|m| m.2)
	}


	/// An updater-specific setter (2b97e0, 2f7058, 2f89b0, 2f96b0, 2fa130, 308458): the session and saved level
	/// bits, and the port's per-placement set.
	pub(super) fn mark_saved(&mut self, class: u32, idx: u32) {
		mark(&mut self.saved, &mut self.uids, &self.uid_map, class, idx);
	}














	/// ARI-8: the uid half of the loader's predicate (OV:45498-45511; the phase half is `MobyInstance::in_phase`) for prop
	/// `k`, with no actor-kind exclusion: false for a pooled / spawned prop, a uid outside the bitmap or flags 0; else the
	/// committed selector, or (no phase bits) flag 8's session bit / flag 4's saved level bit.
	pub fn load_skipped(&self, k: usize) -> bool {
		let Some(idx) = self.props[k].index else { return false };
		let Some(uid) = self.raw_uid(self.props[k].class_id, idx) else { return false };
		let flags = self.place_flags.get(&idx).copied().unwrap_or(0);
		if flags == 0 {
			return false;
		}
		self.uids.committed(uid) || (flags & 3 == 0 && if flags & 8 != 0 { self.uids.session(uid) } else { flags & 4 != 0 && self.uids.persistent(uid) })
	}
}



/// Disjoint-field form of `mark_saved`, for loops that hold a borrow of `props`.
pub(super) fn mark(saved: &mut HashSet<(u32, u32)>, uids: &mut UidState, map: &[(u32, u32, usize)], class: u32, idx: u32) {
	saved.insert((class, idx));
	if let Some(m) = map.iter().find(|m| m.0 == class && m.1 == idx) {
		uids.mark(m.2);
	}
}

