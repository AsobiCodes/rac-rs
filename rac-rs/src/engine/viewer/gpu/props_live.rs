//! [PC] (speed, exact) The props batch without its dead slots, for the passes that draw it again (the water bed and the planar
//! mirrors, `mirror.rs`). The batch holds one slot per moby of the pool; `vs_props` (viewer.rs) poses a live slot and collapses
//! every other slot (hidden, past the draw distance, sun-shadow-only, deferred-list overflow, prompt) to the one point
//! `HIDDEN_POS`, so its triangles have no area and no pixel. They still cost an index fetch and a vertex shader run each, and
//! on Hoven that is 1.9 M of the 2.2 M triangles a bed or mirror pass draws. A slot is live when its record (`PropInst::k[0]`) is
//! a prop or an actor without the prompt flag, the exact test of `vs_props_pose`; `draws_of` rebuilds the draws from the live
//! slots' runs (`Scene::slot_runs`) in the batch's own order, so the surviving triangles draw in the order they did before.

use super::*;

/// Does `vs_props_pose` pose this record's slot (true) or collapse it to `HIDDEN_POS` (false)? `k0` = `PropInst::k[0]`.
pub(super) fn live_kind(k0: u32) -> bool {
	matches!(k0 & 0xff, gpuskin::KIND_PROP | gpuskin::KIND_ACTOR) && k0 & gpuskin::FLAG_PROMPT == 0
}

/// The draws of the slots `take` accepts: their runs `(draw, first index, count)` in index order, runs that touch merged, each
/// cut taking its draw's texture, cull and instance word.
pub(super) fn draws_of(draws: &[Draw], runs: &[Vec<(u32, u32, u32)>], take: impl Fn(usize) -> bool) -> Vec<Draw> {
	let mut cut: Vec<(u32, u32, u32)> = runs.iter().enumerate().filter(|(s, _)| take(*s)).flat_map(|(_, r)| r.iter().copied()).filter(|r| r.2 > 0).collect();
	cut.sort_unstable();
	let mut out: Vec<Draw> = Vec::with_capacity(cut.len().min(draws.len() * 4));
	let mut last: Option<u32> = None;
	for (di, start, count) in cut {
		let Some(d) = draws.get(di as usize) else { continue };
		match out.last_mut() {
			Some(p) if last == Some(di) && p.start + p.count == start => p.count += count,
			_ => out.push(Draw { start, count, ..*d }),
		}
		last = Some(di);
	}
	out
}

/// The draws of the live slots (`live`) `take` accepts, for the normal passes (`additive` false: the slots without the additive draw
/// flag, `add`) or for the additive pass (`additive` true: the slots with it). The two selections of one frame share `live` and `add`,
/// so they split the live slots: every face of a live slot is in exactly one of them (`moby_add.rs` tests).
pub(super) fn split_draws(draws: &[Draw], runs: &[Vec<(u32, u32, u32)>], live: impl Fn(usize) -> bool, add: impl Fn(usize) -> bool, additive: bool, take: impl Fn(usize) -> bool) -> Vec<Draw> {
	draws_of(draws, runs, |s| take(s) && live(s) && add(s) == additive)
}

/// [probe] RAC_PROPS_LIVE=0 draws every slot again (the dead ones too), for A/B runs of one exe.
pub(super) fn enabled() -> bool {
	static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*S.get_or_init(|| std::env::var("RAC_PROPS_LIVE").map_or(true, |v| v != "0"))
}

/// [probe] RAC_COPY_STATS=1: the level pass's props draws and triangles, live against the whole batch (stderr, `copystat`).
pub(in crate::viewer) fn level_stats(all: &[Draw], live: &[Draw]) {
	static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	if *S.get_or_init(|| std::env::var("RAC_COPY_STATS").is_ok()) {
		let tris = |d: &[Draw]| d.iter().map(|d| d.count as u64 / 3).sum::<u64>();
		eprintln!("copystat level props draws {} of {} tris {} of {}", live.len(), all.len(), tris(live), tris(all));
	}
}

impl Scene {
	/// The level pass's props draws: (all live, live without the late slots, the late slots' live draws); the last two only with
	/// shadow volumes this frame (the deferred-list slots leave the opaque draws for the redraw after the volumes).
	pub(in crate::viewer) fn props_level_draws(&self, shown: bool, volumes: bool) -> (Vec<Draw>, Vec<Draw>, Vec<Draw>) {
		// [PC] #29 (tail_order.rs): the slots ranked after a blended no-Z tail leave these draws for the second round (`props_tail_draws`).
		let early = |s: usize| !self.tail.is_late(s);
		let live = if shown { self.props_live_draws(early) } else { Vec::new() };
		level_stats(&self.props.draws, &live);
		if !volumes {
			return (live, Vec::new(), Vec::new());
		}
		let late = |s: usize| self.props_late.contains(&s);
		let (main, tail) = (self.props_live_draws(|s| !late(s) && early(s)), self.props_live_draws(|s| late(s) && early(s)));
		(live, main, tail)
	}

	/// Is slot `s` posed by `vs_props` this frame? Slots the records do not cover (a pool growth not yet followed by `set_props`)
	/// count as live.
	pub(super) fn prop_live(&self, s: usize) -> bool {
		self.props_inst.get(s).is_none_or(|i| live_kind(i.k[0]))
	}

	/// The props batch's draws without its dead slots, with `take` further limiting the live slots (the bed's still / moving split).
	/// All of the batch's draws when the records and the runs disagree in length. [PC] #29 (moby_add.rs): the additive slots have their
	/// own pass (`props_add_draws`), they are not in these draws.
	pub(in crate::viewer) fn props_live_draws(&self, take: impl Fn(usize) -> bool) -> Vec<Draw> {
		let every = !enabled() || self.props_inst.len() != self.slot_runs.len();
		split_draws(&self.props.draws, &self.slot_runs, |s| every || self.prop_live(s), |s| self.tail.in_add(s), false, take)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn d(texture: usize, start: u32, count: u32) -> Draw {
		Draw { inst: 7, cull: 1, texture, start, count }
	}

	#[test]
	fn only_posed_kinds_are_live() {
		use gpuskin::{FLAG_PROMPT, KIND_ACTOR, KIND_CAST, KIND_DEFERRED, KIND_HIDDEN, KIND_PROP};
		assert!(live_kind(KIND_PROP) && live_kind(KIND_ACTOR));
		assert!(!live_kind(KIND_HIDDEN) && !live_kind(KIND_CAST) && !live_kind(KIND_DEFERRED));
		assert!(!live_kind(KIND_PROP | FLAG_PROMPT) && !live_kind(KIND_ACTOR | FLAG_PROMPT));
		assert!(live_kind(KIND_PROP | 0x200));
	}

	/// Two draws (texture 5: indices 0..9, texture 6: 9..15) over three slots; dropping the middle slot splits draw 0 in two and
	/// merges draw 1's two runs (slot 1 holds none of it), in the batch's order, with draw 0's fields.
	#[test]
	fn dead_slots_leave_gaps_and_neighbours_merge() {
		let draws = [d(5, 0, 9), d(6, 9, 6)];
		let runs = vec![vec![(0, 0, 3), (1, 9, 3)], vec![(0, 3, 3), (1, 12, 0)], vec![(0, 6, 3), (1, 12, 3)]];
		let all = draws_of(&draws, &runs, |_| true);
		assert_eq!(all.iter().map(|x| (x.texture, x.start, x.count)).collect::<Vec<_>>(), vec![(5, 0, 9), (6, 9, 6)]);
		let two = draws_of(&draws, &runs, |s| s != 1);
		assert_eq!(two.iter().map(|x| (x.texture, x.start, x.count)).collect::<Vec<_>>(), vec![(5, 0, 3), (5, 6, 3), (6, 9, 6)]);
		assert!(two.iter().all(|x| x.inst == 7 && x.cull == 1));
		assert!(draws_of(&draws, &runs, |_| false).is_empty());
	}

	/// The faces drawn are exactly the live slots' faces, whatever the merge did.
	#[test]
	fn the_union_of_the_runs_is_unchanged() {
		let draws = [d(1, 0, 12), d(2, 12, 6)];
		let runs = vec![vec![(0, 0, 4), (1, 12, 2)], vec![(0, 4, 4), (1, 14, 2)], vec![(0, 8, 4), (1, 16, 2)]];
		for mask in 0..8u32 {
			let live = |s: usize| mask >> s & 1 == 1;
			let got: std::collections::BTreeSet<u32> = draws_of(&draws, &runs, live).iter().flat_map(|x| x.start..x.start + x.count).collect();
			let want: std::collections::BTreeSet<u32> = runs.iter().enumerate().filter(|(s, _)| live(*s)).flat_map(|(_, r)| r.iter().flat_map(|&(_, a, n)| a..a + n)).collect();
			assert_eq!(got, want, "mask {mask}");
		}
	}
}
