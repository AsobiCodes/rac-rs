//! Mod hooks layer OVER the built-in hands.
//!
//! A hook point is a relocatable hand (hooks.rs, oracle `hs_reloc`) installed at the entry of a retail function. `rec::Table::build`
//! puts the hand-written hands (rac-rs: aim.rs, PC features; oracle: speed hands) into one entry map first and the relocatable hands
//! after them with `insert`, so a mod hook on an entry that a built-in hand already holds REPLACES that hand: the hook's way down to
//! "the original" ran raw retail code and the built-in feature (e.g. the crosshair aim of `aim.rs blaster_spawn_any` on the Blaster
//! shot creator) was silently off for every call, even while the hook was inactive. The table is oracle code (dev, guarded), so the
//! layering is done here, one level up: whenever the hook does not take a call itself, `under` runs the built-in hand that the table
//! would have installed at that entry, with exactly the bookkeeping `Cpu::run_hand` does around it.
//!
//! Where `under` runs (bridge.rs `hook_n`):
//! - the slot is inactive (perk off, `uninstall_hook`) or the native mods are stale: the call goes straight to the built-in hand;
//! - the chain never reached the game (`omni_dispatch` failed before `original`): same;
//! - the hook's `original` (`call_retail` inside a dispatch, `original::call_between` outside one, e.g. `omni.fn` from `on_tick`):
//!   both enter the retail function at its entry through the dispatcher, which finds the hook's hand again; the hook's guard is held
//!   then, so the hand falls to `under` instead of to raw retail code. The built-in hand does its own re-entry guard (it calls the
//!   retail function again from inside), and that nested entry falls to retail as it does when the hand runs alone.
//!
//! The built-in hand is found by entry AND code hash (overlay addresses repeat per level; the same test `Table::build` makes) and cached
//! per entry. The cache is dropped at every table rebuild (`rebuilt`, called from the `ver::on_rebuild` callback, which `Table::build`
//! runs first). `RAC_OMNI_LAYER=0` switches the layering off (A/B against the old behaviour).
//!
//! Instruction counts: a hand with an `est` advances the count by it and returns to the caller (`run_hand`); `under` does the same, so
//! a built-in hand costs the same with or without a hook on its entry. Speed hands (`est` EXACT) count their own instructions.

use ee_oracle::cpu::Cpu;
use ee_oracle::hs::EXACT;
use ee_oracle::rec::Hand;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

type HandFn = fn(&mut Cpu) -> bool;

/// A built-in hand found under a hook entry.
#[derive(Clone, Copy)]
pub struct Under {
	f: HandFn,
	est: u64,
}

/// How `hook_n` treats a call (decided from the slot state; pure, so it is tested).
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Route {
	/// The mods' chain takes the call (the hook's guard is taken now).
	Dispatch,
	/// Not the chain's call (inactive slot, stale mods, a nested entry of the hook): the built-in hand under the entry, else retail.
	Under,
}

/// The route of one call into the hand of a hook slot.
pub fn route(active: bool, stale: bool, guard_free: bool) -> Route {
	if active && !stale && guard_free {
		Route::Dispatch
	} else {
		Route::Under
	}
}

/// Table rebuilds so far (every `Table::build` bumps it): cached answers of an older number are void.
static EPOCH: AtomicU64 = AtomicU64::new(1);

/// A table was (re)built: the code under the entries may be another level's.
pub fn rebuilt() {
	EPOCH.fetch_add(1, Ordering::Relaxed);
}

/// The number of table builds so far (the generation of the cached answers).
pub fn epoch() -> u64 {
	EPOCH.load(Ordering::Relaxed)
}

struct Cache {
	epoch: u64,
	items: Vec<(u32, Option<Under>)>,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache { epoch: 0, items: Vec::new() });

impl Cache {
	/// The answer for `entry` in generation `epoch`; `resolve` runs only when there is none yet.
	fn get(&mut self, epoch: u64, entry: u32, resolve: impl FnOnce() -> Option<Under>) -> Option<Under> {
		if self.epoch != epoch {
			self.epoch = epoch;
			self.items.clear();
		}
		if let Some(&(_, u)) = self.items.iter().find(|x| x.0 == entry) {
			return u;
		}
		let u = resolve();
		self.items.push((entry, u));
		u
	}
}

/// `RAC_OMNI_LAYER=0` (read once): no layering, the hook replaces the built-in hand as before.
fn layering_on() -> bool {
	static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*ON.get_or_init(|| !std::env::var("RAC_OMNI_LAYER").is_ok_and(|v| v == "0"))
}

/// The built-in hand `Table::build` would have installed at `entry`: a speed hand (`speed`, already matched by the caller) wins
/// over a hand-written one, as the relocatable hands overwrite the others in the table; a hand-written hand needs its entry, its end
/// inside the code area and the hash of its code (`hash_of(entry, end)` over the RAM now).
pub fn resolve_in(entry: u32, speed: Option<Under>, hands: &[&Hand], hash_of: impl Fn(u32, u32) -> u64) -> Option<Under> {
	if speed.is_some() {
		return speed;
	}
	let scale = est_scale();
	hands.iter().filter(|h| h.entry == entry && entry >= 0x10_0000 && h.end <= 0x40_0000).find(|h| hash_of(h.entry, h.end) == h.hash).map(|h| Under {
		f: h.f,
		est: if scale == 100 || h.est == EXACT { h.est } else { h.est * scale / 100 },
	})
}

/// ORACLE_EST_SCALE=P (percent, default 100): the timing probe of `Table::build`, kept identical.
fn est_scale() -> u64 {
	static S: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
	*S.get_or_init(|| std::env::var("ORACLE_EST_SCALE").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(100))
}

/// Looks the built-in hand under hook slot `n`'s entry up in the machine's RAM.
fn find(c: &Cpu, n: usize, entry: u32) -> Option<Under> {
	if !layering_on() || std::env::var("ORACLE_HAND").is_ok_and(|v| v == "0") {
		return None;
	}
	let hooked_len = super::hooks::slot(n).reloc.get().map(|r| r.len);
	let speed = if ee_oracle::rec::speed_hands_on() {
		ee_oracle::hs_hands::reloc_all().into_iter().find_map(|rh| {
			let r = (rh.reloc)();
			(Some(r.len) == hooked_len && r.matches(&c.mem, entry)).then_some(Under { f: rh.f, est: rh.est })
		})
	} else {
		None
	};
	let hands = ee_oracle::rec::hands();
	resolve_in(entry, speed, &hands, |lo, hi| ee_oracle::rec::hash(&c.mem, lo, hi))
}

/// Runs `u` as `Cpu::run_hand` runs a hand: false = the hand left the call to retail (state untouched).
pub fn run_under(c: &mut Cpu, u: Under) -> bool {
	if u.est == EXACT {
		return (u.f)(c);
	}
	let ra = c.g(31) as u32;
	if !(u.f)(c) {
		return false;
	}
	c.icount += u.est;
	c.mem.now = c.icount;
	c.pc = ra;
	c.npc = ra.wrapping_add(4);
	true
}

/// The call into hook slot `n`'s entry is not the chain's: runs the built-in hand under the entry (true = it did the call), false =
/// the retail code runs.
pub fn under(n: usize, c: &mut Cpu) -> bool {
	let entry = c.pc;
	let epoch = epoch();
	let found = CACHE.lock().unwrap_or_else(|e| e.into_inner()).get(epoch, entry, || find(c, n, entry));
	found.is_some_and(|u| run_under(c, u))
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::cell::Cell;
	use std::sync::atomic::AtomicU32;

	fn yes(_: &mut Cpu) -> bool {
		true
	}
	fn no(_: &mut Cpu) -> bool {
		false
	}
	static RAN: AtomicU32 = AtomicU32::new(0);
	fn counts(c: &mut Cpu) -> bool {
		RAN.fetch_add(1, Ordering::SeqCst);
		// A hand that calls retail clobbers ra (r31), as `Cpu::call` does.
		c.s(31, 0x10);
		true
	}
	fn hand(entry: u32, hash: u64, est: u64, f: HandFn) -> Hand {
		Hand { entry, end: entry + 0x40, hash, est, f }
	}

	#[test]
	fn a_call_is_the_chains_only_for_an_active_fresh_free_slot() {
		assert_eq!(route(true, false, true), Route::Dispatch);
		// Inactive slot (perk off / uninstall_hook), stale native mods, a nested entry (guard held): the built-in hand under the entry.
		assert_eq!(route(false, false, true), Route::Under);
		assert_eq!(route(true, true, true), Route::Under);
		assert_eq!(route(true, false, false), Route::Under);
		assert_eq!(route(false, true, false), Route::Under);
	}

	#[test]
	fn the_built_in_hand_is_found_by_entry_and_code_hash_only() {
		let (a, b) = (hand(0x2c04b0, 0x11, 0, yes), hand(0x2bc560, 0x22, 0, no));
		let hands = [&a, &b];
		// RAM now holds the code of hash 0x11 at the Eudora entry.
		let h = |lo: u32, _hi: u32| if lo == 0x2c04b0 { 0x11 } else { 0x99 };
		assert!(resolve_in(0x2c04b0, None, &hands, h).is_some());
		// Another level's copy at the same address (hash differs): no hand, retail runs.
		assert!(resolve_in(0x2c04b0, None, &hands, |_, _| 0x99).is_none());
		// An entry no hand sits on.
		assert!(resolve_in(0x2d0000, None, &hands, h).is_none());
		assert!(resolve_in(0x2c04b0, None, &[], h).is_none());
	}

	#[test]
	fn a_speed_hand_wins_over_a_hand_written_one_like_in_the_table() {
		let a = hand(0x2c04b0, 0x11, 0, yes);
		let sp = Under { f: no, est: EXACT };
		let u = resolve_in(0x2c04b0, Some(sp), &[&a], |_, _| 0x11).unwrap();
		assert_eq!(u.est, EXACT);
		assert!(!(u.f)(&mut Cpu::new(ee_oracle::mem::Mem::new())));
	}

	#[test]
	fn the_answer_is_cached_per_entry_and_dropped_when_the_table_is_rebuilt() {
		let mut cache = Cache { epoch: 0, items: Vec::new() };
		let n = Cell::new(0);
		let mk = || {
			n.set(n.get() + 1);
			Some(Under { f: yes, est: 0 })
		};
		assert!(cache.get(5, 0x100, mk).is_some());
		assert!(cache.get(5, 0x100, || panic!("cached")).is_some());
		// "No hand here" is cached too (hot hooks on entries without a hand must not rescan).
		assert!(cache.get(5, 0x200, || None).is_none());
		assert!(cache.get(5, 0x200, || panic!("cached")).is_none());
		// A table rebuild (another generation): asked again.
		assert!(cache.get(6, 0x100, mk).is_some());
		assert_eq!(n.get(), 2);
	}

	#[test]
	fn a_hand_with_est_returns_to_the_caller_and_counts_like_run_hand() {
		let mut c = Cpu::new(ee_oracle::mem::Mem::new());
		c.s(31, 0x123450);
		c.icount = 1000;
		let before = RAN.load(Ordering::SeqCst);
		assert!(run_under(&mut c, Under { f: counts, est: 37 }));
		assert_eq!(RAN.load(Ordering::SeqCst), before + 1);
		assert_eq!((c.icount, c.pc, c.npc), (1037, 0x123450, 0x123454));
		// A hand that declines leaves the state alone and the call to retail.
		let mut d = Cpu::new(ee_oracle::mem::Mem::new());
		d.s(31, 0x2000);
		d.icount = 7;
		assert!(!run_under(&mut d, Under { f: no, est: 5 }));
		assert_eq!((d.icount, d.g(31)), (7, 0x2000));
	}

	#[test]
	fn a_speed_hand_sets_its_own_pc_and_count() {
		let mut c = Cpu::new(ee_oracle::mem::Mem::new());
		c.icount = 50;
		c.pc = 0x2c04b0;
		assert!(run_under(&mut c, Under { f: yes, est: EXACT }));
		// Nothing is added around an exact hand.
		assert_eq!((c.icount, c.pc), (50, 0x2c04b0));
	}
}
