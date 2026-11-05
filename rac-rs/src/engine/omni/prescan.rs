//! Finds every game function a loaded mod can reach in ONE pass over the code, when a level has just loaded (events.rs `send`), so the first
//! `call_game` / `raycast` / `call_original` of the level costs nothing. Without it each of them scanned the ~3 MB of code on its first use
//! (tens of ms inside a mod call, which blew the mod's time budget).
//!
//! The wanted set: every spec of `rac1_funcs.txt` (call_game), the line test of raycast.rs and the hook points mods hold (call_original).
//! The pass looks at each function start (after a `jr ra`, `funcs::candidates`) once and tests all wanted specs there; specs that did not match
//! at a strict start get one more pass over every word (as `funcs::find_in` does), split over threads, but only after a level load (the
//! front-end scan skips it: a miss there stays with the lazy path). The answers seed the caches of
//! funcs.rs / raycast.rs / original.rs under the level word; the cost is logged once per level load. Never runs without a loaded mod.

use super::{funcs, hooks, original, raycast};
use ee_oracle::cpu::Cpu;
use ee_oracle::hs_reloc::Reloc;
use std::time::Instant;

/// What one pass found: per wanted spec the entry (None = not in the code), and how many needed the every-word pass.
#[derive(Debug, PartialEq)]
pub struct Found {
	pub entries: Vec<Option<u32>>,
	pub loose: usize,
}

/// The first function start in `words` (code from `base`) for each of `relocs`; same answers as `funcs::find_in` per spec.
pub fn scan(words: &[u32], base: u32, relocs: &[&Reloc], loose_pass: bool) -> Found {
	let end = base + 4 * words.len() as u32;
	let at = |a: u32| words.get(((a - base) / 4) as usize).copied().unwrap_or(0);
	let mut entries: Vec<Option<u32>> = vec![None; relocs.len()];
	let mut left = relocs.len();
	for a in funcs::candidates(words, base, true) {
		for (k, r) in relocs.iter().enumerate() {
			if entries[k].is_none() && a + r.len <= end && r.masked_hash_by(a, at) == r.mhash {
				entries[k] = Some(a);
				left -= 1;
			}
		}
		if left == 0 {
			break;
		}
	}
	let mut loose = 0;
	for (k, r) in relocs.iter().enumerate() {
		if entries[k].is_some() || !loose_pass {
			continue;
		}
		if let Some(a) = loose_scan(words, base, r) {
			entries[k] = Some(a);
			loose += 1;
		}
	}
	Found { entries, loose }
}

/// The first address (every word is a candidate) whose masked hash is the spec's; the words are cut into chunks, one thread each.
fn loose_scan(words: &[u32], base: u32, r: &Reloc) -> Option<u32> {
	let end = base + 4 * words.len() as u32;
	let at = |a: u32| words.get(((a - base) / 4) as usize).copied().unwrap_or(0);
	let threads = std::thread::available_parallelism().map_or(2, |n| n.get().min(8));
	let chunk = words.len().div_ceil(threads).max(1);
	let hits: Vec<Option<u32>> = std::thread::scope(|s| {
		let handles: Vec<_> = (0..threads)
			.map(|t| {
				s.spawn(move || {
					let (lo, hi) = (t * chunk, ((t + 1) * chunk).min(words.len()));
					(lo..hi).map(|i| base + 4 * i as u32).find(|&a| a + r.len <= end && r.masked_hash_by(a, at) == r.mhash)
				})
			})
			.collect();
		handles.into_iter().map(|h| h.join().ok().flatten()).collect()
	});
	hits.into_iter().flatten().min()
}

/// After a level loaded: resolves everything and seeds the caches. Returns the log line.
pub fn run(c: &Cpu, cache_missing: bool) -> String {
	let t0 = Instant::now();
	let level = super::native_host::level_word(c);
	let specs = funcs::all();
	let held: Vec<(usize, &'static Reloc)> = (0..hooks::MAX_HOOKS)
		.filter_map(|n| {
			let s = hooks::slot(n);
			(s.active.load(std::sync::atomic::Ordering::Relaxed)).then(|| s.reloc.get()).flatten().map(|r| (n, r))
		})
		.collect();
	let mut wanted: Vec<&Reloc> = specs.iter().map(|s| &s.reloc).collect();
	wanted.push(raycast::reloc());
	wanted.extend(held.iter().map(|&(_, r)| r));
	let found = scan(&funcs::code_words(c), funcs::CODE_LO, &wanted, cache_missing);
	// A function that is not there is remembered as missing only after a level load (the front end may still be loading its code).
	let keep = |e: Option<u32>| e.is_some() || cache_missing;
	for (s, e) in specs.iter().zip(&found.entries) {
		if keep(*e) {
			funcs::seed(s, *e, level);
		}
	}
	if keep(found.entries[specs.len()]) {
		raycast::seed(level, found.entries[specs.len()]);
	}
	for (i, &(n, _)) in held.iter().enumerate() {
		if keep(found.entries[specs.len() + 1 + i]) {
			original::seed(n, found.entries[specs.len() + 1 + i], level);
		}
	}
	let hit = found.entries.iter().filter(|e| e.is_some()).count();
	format!(
		"omni: prescan level word {level}: {} of {} game functions found ({} needed the slow pass) in {:.1} ms",
		hit,
		found.entries.len(),
		found.loose,
		t0.elapsed().as_secs_f64() * 1000.0
	)
}

#[cfg(test)]
mod tests {
	use super::*;

	const BASE: u32 = 0x10_0000;

	/// A tiny "function": `n` distinct words, then `jr ra` and its delay slot.
	fn func(seed: u32, n: usize) -> Vec<u32> {
		let mut v: Vec<u32> = (0..n as u32).map(|i| 0x2400_0000 | (seed << 8) | i).collect();
		v.extend([0x03e0_0008, 0]);
		v
	}

	fn spec(words: &[u32]) -> Reloc {
		let h = words.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &w| (h ^ w as u64).wrapping_mul(0x100_0000_01b3));
		Reloc::parse(&format!("reloc t entry 0x0 len 0x{:x}\nmhash {h:x}\n", 4 * words.len())).expect("spec")
	}

	#[test]
	fn one_pass_answers_like_find_in_for_every_spec() {
		let (a, b, c) = (func(1, 6), func(2, 9), func(3, 4));
		let words: Vec<u32> = [func(0, 2), a.clone(), b.clone(), vec![0; 2], c.clone()].concat();
		let (ra, rb, rc, rx) = (spec(&a), spec(&b), spec(&c), spec(&func(9, 5)));
		let f = scan(&words, BASE, &[&ra, &rb, &rc, &rx], true);
		let one = |r: &Reloc| funcs::find_in(&words, BASE, r);
		assert_eq!(f.entries, vec![one(&ra), one(&rb), one(&rc), None]);
		assert_eq!(f.entries[0], Some(BASE + 16));
		assert_eq!(f.entries[1], Some(BASE + 16 + 4 * a.len() as u32));
		assert_eq!(f.loose, 0);
	}

	#[test]
	fn a_function_not_after_a_jr_ra_is_found_by_the_slow_pass() {
		let a = func(1, 6);
		let words: Vec<u32> = [vec![7; 10], a.clone()].concat();
		let f = scan(&words, BASE, &[&spec(&a)], true);
		assert_eq!(scan(&words, BASE, &[&spec(&a)], false).entries, vec![None]);
		assert_eq!(f.entries, vec![Some(BASE + 40)]);
		assert_eq!(f.loose, 1);
	}
}
