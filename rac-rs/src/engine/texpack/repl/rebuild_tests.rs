//! Tests of the plain-chain rebuild threads and of the admission of main chains next to them (fix round 6: R1, R2).

use super::tests::{rgba_job_n, tmp, wait_plain, ONE_LEVEL_AT_A_TIME};
use super::*;

/// Spins (up to 20 s) until the budget counters are back at `before` (USED, MAINS, PLAIN, REBUILT): nothing of a dropped level is alive.
pub(super) fn settle(before: [usize; 4]) {
	for _ in 0..5000 {
		if [USED.load(Relaxed), MAINS.load(Relaxed), PLAIN.load(Relaxed), REBUILT.load(Relaxed)] == before {
			return;
		}
		std::thread::sleep(Duration::from_millis(4));
	}
	panic!("the counters did not go back: now {:?}, before {before:?}", [USED.load(Relaxed), MAINS.load(Relaxed), PLAIN.load(Relaxed), REBUILT.load(Relaxed)]);
}

pub(super) fn counters() -> [usize; 4] {
	[USED.load(Relaxed), MAINS.load(Relaxed), PLAIN.load(Relaxed), REBUILT.load(Relaxed)]
}

/// Bytes added to the global `REBUILT` for a test, given back when this drops (also when an assert fails first, so the other tests of the
/// process do not wait on a counter that stays high).
struct RebuiltGuard(usize);

impl RebuiltGuard {
	fn add(n: usize) -> Self {
		REBUILT.fetch_add(n, Relaxed);
		RebuiltGuard(n)
	}
}

impl Drop for RebuiltGuard {
	fn drop(&mut self) {
		REBUILT.fetch_sub(self.0, Relaxed);
	}
}

/// Spins (up to `ms`) until `cond` holds.
fn spin(ms: u64, cond: impl Fn() -> bool) -> bool {
	let t = Instant::now();
	while !cond() {
		if t.elapsed() > Duration::from_millis(ms) {
			return false;
		}
		std::thread::sleep(Duration::from_millis(4));
	}
	true
}

/// R1: a rebuilt plain chain is pinned (not shed) and force-charged to `USED`. It must not make `admit` refuse a main chain the plan said
/// fits: the decision is on `MAINS`. Here a pinned chain bigger than the slack is held before the last chain is admitted (what a rebuild
/// that starts while the level is still loading does); every key must end ready, whatever the timing.
#[test]
fn a_pinned_rebuilt_plain_chain_does_not_make_a_main_chain_that_fits_fail() {
	let _one = ONE_LEVEL_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
	let dir = tmp("r1");
	let before = counters();
	// 16x16, two levels: 1024 + 256 = 1280 bytes main each; the plain chain is 1364. The slack (100) is smaller than that chain.
	let mut jobs: Vec<Job> = (0..3u32).map(|k| rgba_job_n(&dir, k, 16, 2)).collect();
	let pinned = chain::plain_of(load_file(jobs[0].files[0].as_ref().unwrap()).unwrap()).unwrap();
	// A key with no file never builds anything: its entry only carries the pinned chain.
	jobs.push(Job { key: 99, fp: Fp { w: 1, h: 1, hash: 0 }, n: 1, files: [None, None, None, None] });
	let r = LevelRepl::with_limit(80, jobs, true, 3 * 1280 + 100);
	r.entry(99).unwrap().hold_rebuilt(pinned);
	for h in r.spawn_workers(1) {
		h.join().unwrap();
	}
	let ready: Vec<u32> = (0..3).filter(|k| matches!(&*r.entry(*k).unwrap().cpu.lock().unwrap(), Cpu::Ready(_))).collect();
	assert_eq!(ready, vec![0, 1, 2], "a chain that fits was refused because of a pinned plain chain");
	assert_eq!(r.stats.lock().unwrap().over_budget, 0);
	assert_eq!(MAINS.load(Relaxed), before[1] + 3 * 1280);
	drop(r);
	settle(before);
	let _ = std::fs::remove_dir_all(&dir);
}

/// R1: no plain chain is rebuilt while the level's loader is still admitting main chains (`pending() > 0`), so memory stays near the
/// budget; it is rebuilt as soon as the loader is done.
#[test]
fn a_plain_rebuild_waits_until_the_level_is_loaded() {
	let _one = ONE_LEVEL_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
	let dir = tmp("r1_wait");
	let before = counters();
	// 16x16, n = 1: main 1024 bytes (256 as BC3), plain 1364. The limit holds the BC3 main chain only: no plain chain is held by the loader.
	let r = LevelRepl::with_limit(81, vec![rgba_job_n(&dir, 9, 16, 1)], true, 300);
	let e = r.entry(9).unwrap();
	r.queue_plain(9);
	std::thread::sleep(Duration::from_millis(150));
	assert!(e.plain.lock().unwrap().is_none(), "rebuilt while the level was still loading");
	assert_eq!(r.pending(), 1);
	for h in r.spawn_workers(1) {
		h.join().unwrap();
	}
	let p = wait_plain(e);
	assert_eq!(p.fmt, dds::Fmt::Rgba8);
	assert!(e.pinned.load(Relaxed), "a rebuilt chain is pinned until its upload");
	e.release_plain();
	drop(r);
	settle(before);
	let _ = std::fs::remove_dir_all(&dir);
}

/// R2: a level whose scene is dropped while both rebuild threads wait for the rebuilt chains in memory to be uploaded ends: the threads
/// exit, the budget comes back (main chains and the pinned chain), and a later level's rebuild is not stalled.
#[test]
fn a_dropped_level_ends_both_rebuild_threads_and_returns_its_budget() {
	let _one = ONE_LEVEL_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
	let dir = tmp("r2");
	let before = counters();
	let jobs: Vec<Job> = (0..3u32).map(|k| rgba_job_n(&dir, k, 16, 2)).collect();
	let pinned = chain::plain_of(load_file(jobs[0].files[0].as_ref().unwrap()).unwrap()).unwrap();
	let r = LevelRepl::with_limit(82, jobs, true, 3 * 1280);
	for h in r.spawn_workers(2) {
		h.join().unwrap();
	}
	assert_eq!(r.pending(), 0);
	let weak = Arc::downgrade(&r);
	// Rebuilt chains already wait for their upload (a lot of them): both threads pop a key and wait, the third key stays queued.
	r.entry(0).unwrap().hold_rebuilt(pinned);
	let rebuilt = RebuiltGuard::add(REBUILD_AHEAD + 1);
	for k in 0..3 {
		r.queue_plain(k);
	}
	std::thread::sleep(Duration::from_millis(80));
	assert!(r.entry(1).unwrap().plain.lock().unwrap().is_none() && r.entry(2).unwrap().plain.lock().unwrap().is_none());
	// The scene goes away: nobody calls `release_plains`. The level must still end.
	drop(r);
	let mut gone = false;
	for _ in 0..5000 {
		if weak.strong_count() == 0 {
			gone = true;
			break;
		}
		std::thread::sleep(Duration::from_millis(4));
	}
	assert!(gone, "the rebuild threads keep the dropped level alive");
	drop(rebuilt);
	settle(before);
	// A later level's rebuild is not stalled.
	let r2 = LevelRepl::with_limit(83, vec![rgba_job_n(&dir, 9, 16, 1)], true, 300);
	for h in r2.spawn_workers(1) {
		h.join().unwrap();
	}
	let e = r2.entry(9).unwrap();
	assert!(e.plain_for_bind(&r2).unwrap().is_none());
	let _ = wait_plain(e);
	e.release_plain();
	drop(r2);
	settle(before);
	let _ = std::fs::remove_dir_all(&dir);
}

/// N1: the rebuilt chain of a key that no slot of the finished scene build uses (its slots were in a prop pass that was thrown away) is
/// pinned and never uploaded. It must not keep the other rebuilds of the level waiting for the whole visit: `unwant` gives its bytes back,
/// the rebuilds that are queued or waiting for such keys end without building, and the others go on.
#[test]
fn rebuilt_chains_nobody_uploads_do_not_stall_the_other_rebuilds() {
	let _one = ONE_LEVEL_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
	let dir = tmp("n1");
	let before = counters();
	// 16x16, n = 1: main 1024 bytes, plain 1364. The limit holds the four main chains and no plain chain: every plain chain is rebuilt.
	let r = LevelRepl::with_limit(84, (0..4u32).map(|k| rgba_job_n(&dir, k, 16, 1)).collect(), true, 4 * 1024 + 100);
	for h in r.spawn_workers(2) {
		h.join().unwrap();
	}
	assert_eq!(r.pending(), 0);
	let e = |k: u32| r.entry(k).unwrap();
	let held = |k: u32| e(k).plain.lock().unwrap().is_some();
	// Rebuilt chains already wait for their upload up to the limit of the rebuilds: the orphan of key 0 is the one that goes over it.
	let rebuilt = RebuiltGuard::add(REBUILD_AHEAD);
	r.queue_plain(0);
	assert!(spin(3000, || held(0)), "the first rebuild is allowed");
	assert!(e(0).pinned.load(Relaxed) && REBUILT.load(Relaxed) > REBUILD_AHEAD);
	// The slots of keys 1, 2, 3 ask: both threads wait on the counter, the third key stays queued.
	for k in 1..4 {
		r.queue_plain(k);
	}
	std::thread::sleep(Duration::from_millis(80));
	assert!(!held(1) && !held(2) && !held(3), "rebuilt over the limit");
	// The scene build ends: only key 1 has a kept slot.
	r.unwant(2);
	r.unwant(3);
	r.unwant(0);
	assert!(spin(3000, || held(1)), "a rebuild is stalled by the pinned chain of a key no slot uses");
	assert!(spin(3000, || (2..4).all(|k| e(k).pstate.load(Relaxed) == P_NONE)), "the rebuilds of unwanted keys did not end");
	assert!(!held(0) && !held(2) && !held(3), "a chain was held for a key no slot uses");
	assert_eq!(REBUILT.load(Relaxed), REBUILD_AHEAD + e(1).plain.lock().unwrap().as_ref().unwrap().bytes());
	// A later slot of an unwanted key asks again: the flag goes and the chain is rebuilt (after the one waiting for its upload is gone).
	r.queue_plain(2);
	e(1).release_plain();
	assert!(spin(3000, || held(2)), "a key asked for again is not rebuilt");
	e(2).release_plain();
	drop(rebuilt);
	drop(r);
	settle(before);
	let _ = std::fs::remove_dir_all(&dir);
}

/// N1: a chain whose rebuild finishes after `unwant` is not held (nobody would upload it), and the key can be asked for again.
#[test]
fn a_chain_finished_after_unwant_is_not_held() {
	let _one = ONE_LEVEL_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
	let dir = tmp("n1b");
	let before = counters();
	let jobs = vec![rgba_job_n(&dir, 7, 16, 1)];
	let p = chain::plain_of(load_file(jobs[0].files[0].as_ref().unwrap()).unwrap()).unwrap();
	let r = LevelRepl::with_limit(85, jobs, true, 300);
	let e = r.entry(7).unwrap();
	e.pstate.store(P_QUEUED, Relaxed);
	r.unwant(7);
	e.hold_rebuilt(p);
	assert!(e.plain.lock().unwrap().is_none());
	assert_eq!(counters(), before, "an unwanted chain was charged");
	assert_eq!(e.pstate.load(Relaxed), P_NONE, "the key cannot be asked for again");
	drop(r);
	settle(before);
	let _ = std::fs::remove_dir_all(&dir);
}
