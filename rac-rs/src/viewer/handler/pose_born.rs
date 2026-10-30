//! [PC] perf (no retail counterpart): producer-side change tracking of the prop poses. Every pose `set_props` reads gets a
//! token ("born" stamp) from the code that made it: a pose that is, bit for bit, one an earlier frame drew keeps that pose's
//! token, a pose made (skinned or blended) this frame gets a new one. `gpu/pose_reuse.rs` then neither copies nor uploads a
//! pose whose token and place are last frame's. Tokens are unique for the process (a counter), 0 = no token (always written).
//! Since lane nprops the GPU bytes rest on this contract in more places: the direct upload re-converts gap and grown entries
//! from the current sources, and the props clipper / prompt read the current source, not a mirror (a lying token = a wrong
//! picture; RAC_PROPS_SET_CHECK=1 counts it).
//!
//! Key poses (`frame_props.rs`, by `PoseKey`): moved over from the last frame (no game tick), or skinned from the same class
//! and, bit for bit, the same joint matrices as the last frame's pose of that key (skinning is a pure function of both,
//! `anim::skin_all`; the same rule as `fx_skin.rs`), keep the last frame's token; the second case also takes the last frame's
//! pose (`Smooth::posed`) instead of skinning again.

use super::*;
use crate::viewer::interp::FxMap;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(1);

/// A token no earlier pose had.
pub(in crate::viewer) fn new_token() -> u64 {
	NEXT.fetch_add(1, Ordering::Relaxed)
}

/// This frame's tokens of `posed` (by key) and `fx_posed` (by prop slot).
#[derive(Default)]
pub(in crate::viewer) struct PoseBorn {
	pub keys: FxMap<PoseKey, u64>,
	pub fx: FxMap<usize, u64>,
}

impl PoseBorn {
	/// The token of the pose `set_props` takes for prop `pi`: `fx_posed`'s when it has one (`in_fx`), else its key's
	/// (`slot_pose` picks the same way); 0 when the maker gave none.
	pub(in crate::viewer) fn of(&self, in_fx: bool, pi: usize, key: impl FnOnce() -> PoseKey) -> u64 {
		let t = if in_fx { self.fx.get(&pi) } else { self.keys.get(&key()) };
		t.copied().unwrap_or(0)
	}
}

/// What a key's last pose was made from (class address, joint matrices; None = moved over without them) and its token.
struct KeySkin {
	class: usize,
	mats: Option<Vec<[f32; 12]>>,
	token: u64,
}

/// The last frame's key poses' sources, by key.
#[derive(Default)]
pub(in crate::viewer) struct KeySkins {
	kept: FxMap<PoseKey, KeySkin>,
}

/// How a key's pose is had this frame.
pub(in crate::viewer) enum KeyJob<'a> {
	/// Moved over from the last frame (no game tick: `frame_props.rs`).
	Moved(Posed),
	/// The last frame's pose again (same class, same matrices).
	Kept(Posed),
	/// Skinned now.
	Skin(&'a mobies::MobyClass, Vec<[f32; 12]>),
}

impl KeySkins {
	pub(in crate::viewer) fn forget(&mut self) {
		self.kept.clear();
	}

	/// The job of key `key` (not moved over) posed from `class` with `mats`: the last frame's pose (`last`, `Smooth::posed`)
	/// when it came from the same class and, bit for bit, the same matrices; else a skinning.
	pub(in crate::viewer) fn job<'a>(&self, key: PoseKey, class: &'a mobies::MobyClass, mats: Vec<[f32; 12]>, last: &HashMap<PoseKey, Posed>) -> KeyJob<'a> {
		let id = class as *const mobies::MobyClass as usize;
		match (self.kept.get(&key), last.get(&key)) {
			(Some(k), Some(p)) if k.class == id && k.mats.as_deref().is_some_and(|m| super::fx_skin::same_mats(m, &mats)) => KeyJob::Kept(p.clone()),
			_ => KeyJob::Skin(class, mats),
		}
	}

	/// After the skinning: remembers this frame's keys (`jobs`, with the class address of each) and puts their tokens into
	/// `born`. Keys not posed this frame are forgotten.
	pub(in crate::viewer) fn settle(&mut self, jobs: Vec<(PoseKey, usize, KeyJob)>, born: &mut FxMap<PoseKey, u64>) {
		let mut old = std::mem::take(&mut self.kept);
		born.clear();
		for (key, class, job) in jobs {
			let k = match (job, old.remove(&key)) {
				(KeyJob::Skin(_, mats), _) => KeySkin { class, mats: Some(mats), token: new_token() },
				(KeyJob::Kept(_), Some(k)) => k,
				(KeyJob::Moved(_), Some(k)) if k.class == class => k,
				(KeyJob::Moved(_) | KeyJob::Kept(_), _) => KeySkin { class, mats: None, token: new_token() },
			};
			born.insert(key, k.token);
			self.kept.insert(key, k);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn key(n: i32) -> PoseKey {
		(1, 0, 0, n, n + 1, 0)
	}

	#[test]
	fn tokens_are_new_each_time() {
		let a = new_token();
		assert!(a != 0 && new_token() > a);
	}

	#[test]
	fn settle_keeps_tokens_of_kept_and_moved_and_renews_skins() {
		let mut s = KeySkins::default();
		let mut born = FxMap::default();
		s.kept.insert(key(1), KeySkin { class: 7, mats: Some(vec![[1.0; 12]]), token: 11 });
		s.kept.insert(key(2), KeySkin { class: 7, mats: None, token: 22 });
		s.kept.insert(key(3), KeySkin { class: 7, mats: Some(vec![[1.0; 12]]), token: 33 });
		s.kept.insert(key(9), KeySkin { class: 7, mats: None, token: 99 });
		let jobs = vec![
			(key(1), 7, KeyJob::Kept(Default::default())),
			(key(2), 7, KeyJob::Moved(Default::default())),
			(key(3), 8, KeyJob::Moved(Default::default())),
			(key(4), 7, KeyJob::Moved(Default::default())),
		];
		s.settle(jobs, &mut born);
		assert_eq!(born[&key(1)], 11);
		assert_eq!(born[&key(2)], 22);
		let old = [0, 11, 22, 33, 99];
		assert!(!old.contains(&born[&key(3)]), "another class: a new token");
		assert!(!old.contains(&born[&key(4)]) && born[&key(4)] != born[&key(3)], "no source known: a new token");
		assert!(!s.kept.contains_key(&key(9)), "keys not posed are forgotten");
		assert!(s.kept[&key(3)].mats.is_none());
	}

	#[test]
	fn of_picks_fx_first_like_slot_pose() {
		let mut b = PoseBorn::default();
		b.keys.insert(key(1), 5);
		b.fx.insert(3, 6);
		assert_eq!(b.of(true, 3, || key(1)), 6);
		assert_eq!(b.of(false, 3, || key(1)), 5);
		assert_eq!(b.of(true, 4, || key(1)), 0, "an fx pose without a token");
		assert_eq!(b.of(false, 4, || key(2)), 0);
	}
}
