//! Tests that a level's plan and admission depend on that level alone (fix round 7): another level's main chains, still alive while
//! the leaving level's slots are being swapped in, change nothing about which chains this level replaces, stores as BC3 or drops.

use super::rebuild_tests::{counters, settle};
use super::tests::{rgba_job_n, run_level, tmp, ONE_LEVEL_AT_A_TIME};
use super::*;

/// 6 jobs of 16x16 with two levels: 1280 bytes each as RGBA8 (7680 together), 320 as BC3.
fn six_jobs(dir: &std::path::Path, first_key: u32) -> Vec<Job> {
	(first_key..first_key + 6).map(|k| rgba_job_n(dir, k, 16, 2)).collect()
}

/// Loads `jobs` as a level of `limit` bytes and keeps it alive (its main chains stay in the counters, as a level whose slots are not
/// swapped in yet does).
fn load_and_keep(level: u32, jobs: Vec<Job>, limit: usize) -> Arc<LevelRepl> {
	let r = LevelRepl::with_limit(level, jobs, true, limit);
	for h in r.spawn_workers(2) {
		h.join().unwrap();
	}
	assert_eq!(r.pending(), 0);
	r.release_plains(); // only the main chains are counted
	r
}

/// Level A holds unbound main chains while level B is planned and loaded under a tight limit: B's ready / BC3 / dropped sets and its
/// counts are the ones of B run alone (on the round-6 code A's chains counted in B's plan: B was planned over the budget and lost chains).
#[test]
fn a_level_is_planned_and_loaded_as_if_it_were_alone() {
	let _one = ONE_LEVEL_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
	let dir = tmp("lvl_alone");
	let before = counters();
	// B: 7680 bytes as RGBA8, limit 5000: some chains go to BC3, all of them fit.
	let limit = 5000;
	let alone = run_level(90, six_jobs(&dir, 10), limit, 2);
	assert_eq!(alone.1[0], 6, "the test level fits with BC3 chains: {alone:?}");
	assert!(!alone.2.is_empty() && alone.2.len() < 6, "the test needs a plan that stores some chains as BC3: {alone:?}");
	// A is alive with 7680 bytes of main chains (more than B's whole limit).
	let a = load_and_keep(91, six_jobs(&dir, 30), usize::MAX >> 4);
	assert_eq!(a.bytes.load(Relaxed), 6 * 1280);
	assert_eq!(MAINS.load(Relaxed), before[1] + 6 * 1280);
	for round in 0..3u32 {
		let with_a = run_level(92 + round, six_jobs(&dir, 10), limit, 1 + round as usize);
		assert_eq!(with_a, alone, "level B (round {round}) differs because level A is alive");
	}
	// The same through the plan of a level that stays alive: its own bytes stay within its own limit whatever A holds.
	let b = load_and_keep(95, six_jobs(&dir, 10), limit);
	assert!(b.bytes.load(Relaxed) <= limit);
	assert_eq!(MAINS.load(Relaxed), before[1] + 6 * 1280 + b.bytes.load(Relaxed));
	drop((a, b));
	settle(before);
	let _ = std::fs::remove_dir_all(&dir);
}

/// The counters return to their start once both levels are dropped, in either order, and each drop gives back exactly that level's bytes.
#[test]
fn the_budget_returns_to_its_start_after_both_levels_are_dropped() {
	let _one = ONE_LEVEL_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
	let dir = tmp("lvl_back");
	let before = counters();
	for a_first in [true, false] {
		let a = load_and_keep(96, six_jobs(&dir, 10), 5000);
		let b = load_and_keep(97, six_jobs(&dir, 30), 5000);
		let (ba, bb) = (a.bytes.load(Relaxed), b.bytes.load(Relaxed));
		assert!(ba > 0 && bb > 0);
		assert_eq!(MAINS.load(Relaxed), before[1] + ba + bb);
		let (first, second, kept) = if a_first { (a, b, bb) } else { (b, a, ba) };
		drop(first);
		settle([before[0] + kept, before[1] + kept, before[2], before[3]]);
		drop(second);
		settle(before);
	}
	let _ = std::fs::remove_dir_all(&dir);
}

/// N2: a scene dropped while its level is still loading stops the level's loader threads (each ends the key it is on): no more CPU copies
/// are made for it, and what was made goes back to the budget with the level. Without it the threads went on to the end of the queue.
#[test]
fn a_scene_dropped_during_the_load_stops_the_loader_threads() {
	let _one = ONE_LEVEL_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
	let dir = tmp("lvl_gone");
	let before = counters();
	const KEYS: u32 = 300;
	let r = LevelRepl::with_limit(99, (0..KEYS).map(|k| rgba_job_n(&dir, k, 128, 1)).collect(), true, usize::MAX >> 4);
	r.scene_dropped(); // before the workers start: the test does not race them
	let workers = r.spawn_workers(2);
	for h in workers {
		h.join().unwrap();
	}
	let done = r.done.load(Relaxed);
	assert_eq!(done, 0, "the loader threads went on after the scene was dropped: {done} of {KEYS} keys loaded");
	assert!(r.pending() > 0 && r.is_gone());
	assert_eq!(MAINS.load(Relaxed), before[1] + r.bytes.load(Relaxed));
	drop(r);
	settle(before);
	let _ = std::fs::remove_dir_all(&dir);
}
