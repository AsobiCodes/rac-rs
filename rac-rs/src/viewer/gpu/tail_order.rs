//! [PC] Issue #29 (Qwark's TV in Blackwater City): which mobys draw after the blended no-Z tail texels.
//!
//! Retail draws the mobys one after the other: a walk over its moby table in table order, with the deferred ones (moby +0x34 bit
//! 0x800, and the visible shadow casters) left out and drawn after the walk, again in table order. That draw position is
//! `Prop::order` (`rank`, plus `game::DEFERRED_ORDER` when deferred), so comparing two `order` values compares draw positions. A
//! moby's packet whose texel alpha is
//! below 0x60 (texture value below 0xc0) is blended and writes no Z (TEST AFAIL RGB_ONLY), so a moby drawn LATER passes the depth
//! test behind it and covers it. The TV's screen is such a packet (class 984, the tinted tail packet, texel alpha 0x24) and Qwark,
//! behind the screen quad, is drawn after it: retail shows him at full colour on a dark green screen. rac-rs draws every moby's
//! opaque texels first and every blended texel after all of them (`pass_world`), which put the screen over Qwark (28 % veil).
//!
//! The fix is a late round: the mobys drawn after the LAST moby with such a screen draw their opaque texels and then their
//! blended texels after the blended pass of the mobys before them (`draw/pass_tail.rs`), so they end up over it as in retail. One
//! split at the last such moby is exact for one TV and a close approximation for several. With no such moby on screen (almost
//! always) the set is empty and the frame draws as before.
//!
//! Where the round sits follows retail's own order: main walk, then the shadow volumes and the darken, then the deferred list. A
//! late moby of the main walk (`TAIL_MAIN`) is drawn right after the first round's moby pass, BEFORE the volumes (so the volumes
//! darken it as retail's, and the deferred hero and the deferred list still end up over it); a late moby of the deferred list
//! (`TAIL_DEFERRED`) is drawn after the deferred list's own draws. A late moby's metal faces and CPU-clipped faces go with it
//! (`clip_ends` / `metal_ends`: the draws of each group in `clip_batch` / the metal batch). The hero and the scene actors are
//! outside this order (never late, never the blender): the hero keeps his place in the marker batch.
//!
//! Only a moby retail itself drew (`Prop::retail_drawn`) can be that last moby. The widescreen window also shows mobys retail
//! culled at its 4:3 side plane; they are not in retail's stream, so taking one as the split point would push mobys that retail
//! draws before the screen (Blackwater: the vendor orb, class 1143, ranked before Ratchet's helmet) behind it.

use super::*;
use std::collections::HashMap;

/// `TailOrder::late` value: the slot draws in the first round.
pub(in crate::viewer) const TAIL_FIRST: u8 = 0;
/// A late slot of retail's main walk (drawn right after the first round's moby pass, before the shadow volumes).
pub(in crate::viewer) const TAIL_MAIN: u8 = 1;
/// A late slot of retail's deferred list (drawn after the deferred list's own draws).
pub(in crate::viewer) const TAIL_DEFERRED: u8 = 2;

/// The texture slots whose texels blend without a Z write, the class verdict memo and this frame's late slots. `Scene::tail`.
pub(in crate::viewer) struct TailOrder {
	/// Per texture slot: the texture holds a texel with PS2 alpha 1..0x5f (texture value 2..0xbf).
	low: Vec<bool>,
	/// (class, tint start) -> the class's tail faces (vertices from `start`, the tinted packets) use a `low` slot.
	memo: HashMap<(u32, usize), bool>,
	/// This frame, per props slot: `TAIL_FIRST`, `TAIL_MAIN` or `TAIL_DEFERRED`. Empty when no slot is late (the usual case).
	late: Vec<u8>,
	n_late: usize,
	/// This frame, per props slot: drawn in the additive pass (`moby_add.rs`), not in the opaque / blended ones. Empty when none.
	add: Vec<bool>,
	/// End (exclusive draw index) of the first-round, main-walk and deferred groups in `Scene::clip_batch.draws`; `usize::MAX` =
	/// everything is the first round (`group_range`). Set by `flush_clip`.
	pub(super) clip_ends: [usize; 3],
	/// The same for the metal batch's draws (`set_metal_props`).
	pub(super) metal_ends: [usize; 3],
}

impl Default for TailOrder {
	fn default() -> TailOrder {
		TailOrder { low: Vec::new(), memo: HashMap::new(), late: Vec::new(), n_late: 0, add: Vec::new(), clip_ends: [usize::MAX; 3], metal_ends: [usize::MAX; 3] }
	}
}

impl TailOrder {
	/// Built once per scene from the original textures (a textures mod changes the images, not retail's draw order).
	pub(super) fn new(slots: &HashMap<u32, usize>, n: usize, textures: &HashMap<u32, crate::level::Texture>) -> TailOrder {
		let mut low = vec![false; n];
		for (&key, &slot) in slots {
			if let Some(t) = textures.get(&(key & crate::mobies::KEY_MASK)) {
				low[slot] = t.rgba.iter().any(|c| c[3] != 0 && c[3] < 0xc0);
			}
		}
		TailOrder { low, ..Default::default() }
	}

	/// `TAIL_FIRST`, `TAIL_MAIN` or `TAIL_DEFERRED` of props slot `slot`.
	pub(in crate::viewer) fn group(&self, slot: usize) -> u8 {
		self.late.get(slot).copied().unwrap_or(TAIL_FIRST)
	}

	pub(super) fn is_late(&self, slot: usize) -> bool {
		self.group(slot) != TAIL_FIRST
	}

	/// Does props slot `slot` draw in the additive pass (`moby_add.rs`)?
	pub(super) fn in_add(&self, slot: usize) -> bool {
		self.add.get(slot).copied().unwrap_or(false)
	}

	/// Is any slot additive this frame?
	pub(super) fn any_add(&self) -> bool {
		self.add.iter().any(|&a| a)
	}

	/// The draws of group `g` (0 first round, 1 main walk, 2 deferred) among `draws`, whose groups end at `ends` (in that order).
	pub(in crate::viewer) fn group_slice<'a>(ends: &[usize; 3], g: usize, draws: &'a [Draw]) -> &'a [Draw] {
		let lo = if g == 0 { 0 } else { ends[g - 1].min(draws.len()) };
		&draws[lo..ends[g].min(draws.len())]
	}

	/// Is any slot late this frame?
	pub(super) fn any(&self) -> bool {
		self.n_late > 0
	}
}

/// The largest draw position among the slots retail drew that have a blended no-Z tail (`(order, retail drew it, has such a
/// tail)` per shown slot); None = there is none.
pub(super) fn last_blender(slots: impl Iterator<Item = (u32, bool, bool)>) -> Option<u32> {
	slots.filter(|s| s.1 && s.2).map(|s| s.0).max()
}

/// Marks in `out` (one entry per slot, cleared first; left empty when none) the shown props with the additive draw flag (`Prop::is_additive`).
pub(super) fn mark_add(out: &mut Vec<bool>, props: &[Prop], shown: &[bool]) {
	out.clear();
	out.extend(props.iter().zip(shown).map(|(p, &s)| s && p.is_additive()));
	if !out.contains(&true) {
		out.clear();
	}
}

/// Marks in `out` (one entry per slot, cleared first) the shown slots drawn after draw position `above` (`TAIL_DEFERRED` when the
/// slot is on retail's deferred list, else `TAIL_MAIN`); gives how many.
pub(super) fn mark_late(out: &mut Vec<u8>, rank: impl Fn(usize) -> u32, shown: &[bool], above: u32) -> usize {
	out.clear();
	out.extend(shown.iter().enumerate().map(|(s, &sh)| match rank(s) {
		r if !sh || r <= above => TAIL_FIRST,
		r if r & crate::game::DEFERRED_ORDER != 0 => TAIL_DEFERRED,
		_ => TAIL_MAIN,
	}));
	out.iter().filter(|&&g| g != TAIL_FIRST).count()
}

impl Scene {
	/// This frame's late slots, from the shown flags `set_props` decided: a shown moby with an active tail (`PROP_TINTS`: the
	/// moby has the tail colour) that retail drew and whose class has tail faces on a blended texture, and every shown moby drawn
	/// after the last of those (deferred list after the main walk). Runs in `Gpu::set_props` after `props_vis` is set.
	pub(super) fn update_tail_order(&mut self, props: &[Prop], shown: &[bool]) {
		let mut t = std::mem::take(&mut self.tail);
		t.n_late = 0;
		mark_add(&mut t.add, props, shown);
		let found = if t.low.iter().any(|&b| b) {
			PROP_TINTS.with(|tints| {
				let tints = tints.borrow();
				let blenders = props.iter().enumerate().filter(|(s, _)| shown.get(*s).copied().unwrap_or(false)).map(|(s, p)| {
					let blends = tints.get(s).copied().flatten().is_some_and(|(_, start)| t.class_blends(self, p.class_id as u32, start));
					(p.order, p.retail_drawn, blends)
				});
				last_blender(blenders)
			})
		} else {
			None
		};
		match found {
			Some(above) => t.n_late = mark_late(&mut t.late, |s| props[s].order, &shown[..props.len().min(shown.len())], above),
			None => t.late.clear(),
		}
		self.tail = t;
	}

	/// The live draws of the late slots, per group: `[main walk, deferred list]` (both empty when there are none).
	pub(in crate::viewer) fn props_tail_draws(&self, shown: bool) -> [Vec<Draw>; 2] {
		if shown && self.tail.any() {
			[TAIL_MAIN, TAIL_DEFERRED].map(|g| self.props_live_draws(|s| self.tail.group(s) == g))
		} else {
			[Vec::new(), Vec::new()]
		}
	}

	/// The CPU-clipped faces' draws of group `g` (`TailOrder::clip_ends`).
	pub(in crate::viewer) fn clip_group(&self, g: usize) -> &[Draw] {
		TailOrder::group_slice(&self.tail.clip_ends, g, &self.clip_batch.draws)
	}
}

impl TailOrder {
	/// Does class `class` (tinted from vertex `start` on) have a face on a blended texture among its tail vertices? Memoised.
	fn class_blends(&mut self, scene: &Scene, class: u32, start: usize) -> bool {
		if let Some(&v) = self.memo.get(&(class, start)) {
			return v;
		}
		let Some(cf) = scene.class_faces.get(&class) else { return false }; // a class not built yet: asked again next frame
		let v = cf.groups.iter().any(|(key, idx)| {
			let slot = scene.tex_slots.get(&(key & crate::mobies::SLOT_MASK)).copied();
			slot.is_some_and(|s| self.low.get(s).copied().unwrap_or(false)) && idx.iter().any(|&v| v as usize >= start)
		});
		self.memo.insert((class, start), v);
		v
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn last_blender_is_the_largest_rank_with_a_blended_tail() {
		let slots = [(5u32, true, false), (9, true, true), (7, true, true), (12, true, false)];
		assert_eq!(last_blender(slots.into_iter()), Some(9));
		assert_eq!(last_blender([(3u32, true, false), (4, true, false)].into_iter()), None);
		assert_eq!(last_blender(std::iter::empty()), None);
	}

	/// A moby only the widescreen window shows is not in retail's stream: it is never the blender (Blackwater, the vendor orb
	/// ranked 852 was taken for it and pushed Ratchet's helmet behind the screen), though it may be late like any other.
	#[test]
	fn a_wide_only_moby_is_never_the_blender() {
		let slots = [(852u32, false, true), (700, true, true), (900, true, false)];
		assert_eq!(last_blender(slots.into_iter()), Some(700));
		assert_eq!(last_blender([(852u32, false, true)].into_iter()), None);
		let shown = [true, true, true];
		let mut out = Vec::new();
		assert_eq!(mark_late(&mut out, |s| [852u32, 700, 900][s], &shown, 700), 2);
		assert_eq!(out, vec![TAIL_MAIN, TAIL_FIRST, TAIL_MAIN]);
	}

	/// The deferred list is drawn after the whole table walk: a deferred slot of a LOWER table rank than a main-walk blender is
	/// late, a main-walk slot is never late for a deferred blender, and a deferred blender makes only later deferred slots late.
	#[test]
	fn deferred_slots_come_after_the_whole_walk() {
		let d = crate::game::DEFERRED_ORDER;
		// slot 0: main walk rank 40 (the blender), slot 1: deferred rank 3, slot 2: main rank 41, slot 3: deferred rank 90
		let order = [40u32, 3 | d, 41, 90 | d];
		let shown = [true; 4];
		let mut out = Vec::new();
		let above = last_blender([(order[0], true, true), (order[2], true, false)].into_iter()).unwrap();
		assert_eq!(mark_late(&mut out, |s| order[s], &shown, above), 3);
		assert_eq!(out, vec![TAIL_FIRST, TAIL_DEFERRED, TAIL_MAIN, TAIL_DEFERRED]);
		// the blender is the deferred rank-3 slot: the main-walk slots (even rank 41) draw before it, only deferred rank 90 is after
		let above = last_blender([(order[0], true, false), (order[1], true, true)].into_iter()).unwrap();
		assert_eq!(mark_late(&mut out, |s| order[s], &shown, above), 1);
		assert_eq!(out, vec![TAIL_FIRST, TAIL_FIRST, TAIL_FIRST, TAIL_DEFERRED]);
	}

	/// Pool slot order is not table order: slot 0 is ranked last, slot 2 first. Only the shown slots after rank 9 are late.
	#[test]
	fn late_slots_are_the_shown_ones_ranked_after() {
		let rank = [20u32, 9, 3, 10, 40];
		let shown = [true, true, true, true, false];
		let mut out = vec![TAIL_DEFERRED; 9];
		let n = mark_late(&mut out, |s| rank[s], &shown, 9);
		assert_eq!((n, out), (2, vec![TAIL_MAIN, TAIL_FIRST, TAIL_FIRST, TAIL_MAIN, TAIL_FIRST]));
		assert_eq!(mark_late(&mut Vec::new(), |s| rank[s], &shown, 20), 0);
	}

	/// The groups of a draw list are consecutive ranges; the unset marker (`usize::MAX`) makes everything the first round.
	#[test]
	fn group_slices_split_the_draw_list() {
		let d = |n: u32| Draw { inst: 0, cull: 0, texture: 0, start: n, count: 3 };
		let draws = [d(0), d(3), d(6), d(9), d(12)];
		let starts = |g: usize, ends: [usize; 3]| TailOrder::group_slice(&ends, g, &draws).iter().map(|x| x.start).collect::<Vec<_>>();
		assert_eq!(starts(0, [2, 3, 5]), vec![0, 3]);
		assert_eq!(starts(1, [2, 3, 5]), vec![6]);
		assert_eq!(starts(2, [2, 3, 5]), vec![9, 12]);
		assert_eq!(starts(0, [usize::MAX; 3]), vec![0, 3, 6, 9, 12]);
		assert!(starts(1, [usize::MAX; 3]).is_empty() && starts(2, [usize::MAX; 3]).is_empty());
		assert!(TailOrder::group_slice(&[0, 0, 0], 2, &[]).is_empty());
	}

	/// The texture test: alpha 0 (cut-out) and opaque texels never make a texture blend; 0x24 * 2 does.
	#[test]
	fn only_translucent_texels_count() {
		let tex = |a: &[u8]| crate::level::Texture { w: a.len(), h: 1, rgba: a.iter().map(|&x| [1, 2, 3, x]).collect() };
		let textures: HashMap<u32, crate::level::Texture> = [(1, tex(&[0, 255, 128])), (2, tex(&[255, 72])), (3, tex(&[0xbf, 255])), (4, tex(&[0xc0]))].into_iter().collect();
		let slots: HashMap<u32, usize> = (1..=4).map(|k| (k, k as usize - 1)).collect();
		let t = TailOrder::new(&slots, 4, &textures);
		assert_eq!(t.low, vec![true, true, true, false]);
	}
}
