//! Mods may not patch recompiled game code (`omni.write` / `write_mem` over a function the recompiled crates run).
//!
//! The recompiled Rust functions (oracle `rec`) are chosen when the table is built, by hashing each set's code against RAM; a write into such a
//! function is never seen by the compiled code (the mod "does nothing" while the interpreter-only paths DO see it), and at the next table build
//! the whole set no longer matches, so it falls back to the interpreter for the rest of the session (a large slowdown). A hook point
//! (`rac1.*`, OmniLoader hooks) is the supported way to change what a game function does. MODDING.md lists `omni.write` as game memory below 32 MB and
//! does not name code; the refusal is documented in ARCHITECTURE.md (omni row) and the proposed MODDING.md line, and every refusal is in the run log (`refused`).
//! So `write_mem` refuses a write that overlaps a function of a set the table uses now, and says so once in the run log. The raw
//! `ram_ptr` of native mods cannot be checked: code written through it is unsupported the same way (documented in native_host.rs).
//!
//! The spans are taken from the registered sets of the boot code and the levels (`ee_oracle::rec::level_sets`, read only) the way `Table::build`
//! decides: a set counts when every function of it still hashes to its stored value. The front-end sets (title and menus, boot image and frontbin)
//! are left out on purpose: the front end was interpreted before it was compiled, so a mod's write there always worked, and it still does:
//! `write_cb` (host.rs) lets it through and `Cpu::rec_front_written` rebuilds the table, which then runs the patched front end interpreted.
//! The spans are cached per table build (layer.rs epoch) and found by binary search; a write
//! outside 0x100000..0x400000 (the heap, the stack, the overlay's data beyond) costs two comparisons.

use ee_oracle::cpu::Cpu;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

/// The code range the tables compile (`rec::Table::build`: `lo`, `hi`).
const CODE_LO: u32 = 0x10_0000;
const CODE_HI: u32 = 0x40_0000;

struct Spans {
	epoch: u64,
	/// `(entry, the largest end of every span up to this entry)`, sorted by entry (`used_spans`).
	spans: Vec<(u32, u32)>,
}

static CACHE: Mutex<Option<Spans>> = Mutex::new(None);

/// Every function in the sets that match RAM as `(entry, reach)`, sorted by entry; a set is `(entry, end, hash)` per function and `hash` hashes a
/// range of RAM. The functions of one crate overlap (a function's range runs a few words into the next) and a regenerated disc may nest one in
/// another, so `reach` is the largest end over this entry and every entry before it, not the function's own end: `overlaps` stays exact.
pub(super) fn used_spans(sets: &[Vec<(u32, u32, u64)>], hash: impl Fn(u32, u32) -> u64) -> Vec<(u32, u32)> {
	let mut out = Vec::new();
	for set in sets {
		let used = !set.iter().any(|&(e, en, h)| en > CODE_HI || e < CODE_LO || hash(e, en) != h);
		if used {
			out.extend(set.iter().map(|&(e, en, _)| (e, en)));
		}
	}
	out.sort_unstable();
	let mut reach = 0;
	for s in &mut out {
		reach = reach.max(s.1);
		s.1 = reach;
	}
	out
}

/// Whether `addr..addr+len` overlaps one of the functions `used_spans` made `spans` of (overlapping or nested ones included).
pub(super) fn overlaps(spans: &[(u32, u32)], addr: u32, len: u32) -> bool {
	let end = addr.saturating_add(len);
	let k = spans.partition_point(|&(e, _)| e < end);
	k.checked_sub(1).and_then(|i| spans.get(i)).is_some_and(|&(_, en)| en > addr)
}

/// The run-log line for a refused write: once per address, for the first few addresses of a run. The host call carries no mod id; the loader
/// stops the mod that made it with its own `[mod id]` line right after (a refused write raises its "memory access" error).
pub(super) fn refused(addr: u32, len: u32) {
	static SAID: AtomicU32 = AtomicU32::new(0);
	log_refused(&SAID, addr, len, super::native_host::say_once);
}

/// At most this many refusal lines per run.
const REFUSED_LINES: u32 = 8;

/// `refused` with its counter and its once-per-key log (`native_host::say_once`) given. The cap is checked first, so the once-per-key set
/// holds at most `REFUSED_LINES` entries from here.
fn log_refused(said: &AtomicU32, addr: u32, len: u32, say_once: impl Fn(&str, &str) -> bool) {
	let key = format!("code_write {addr:x}");
	if said.load(Ordering::Relaxed) < REFUSED_LINES && say_once(&key, &format!("a mod wrote {len} byte(s) over game code at 0x{addr:x} (omni.write / write_mem); refused: patches are not seen by the recompiled functions, use a hook point; the loader names the mod in the next [mod id] line")) {
		said.fetch_add(1, Ordering::Relaxed);
	}
}

/// Whether a write of `len` bytes at `addr` (a real address) would change a recompiled function of the table in use.
pub(super) fn is_code(c: &Cpu, addr: u32, len: u32) -> bool {
	if len == 0 || addr.saturating_add(len) <= CODE_LO || addr >= CODE_HI {
		return false;
	}
	let epoch = super::layer::epoch();
	let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
	if g.as_ref().is_none_or(|s| s.epoch != epoch) {
		let sets: Vec<Vec<(u32, u32, u64)>> = ee_oracle::rec::level_sets().iter().map(|s| s.iter().map(|f| (f.entry, f.end, f.hash)).collect()).collect();
		*g = Some(Spans { epoch, spans: used_spans(&sets, |lo, hi| ee_oracle::rec::hash(&c.mem, lo, hi)) });
	}
	g.as_ref().is_some_and(|s| overlaps(&s.spans, addr, len))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_write_over_a_compiled_function_is_code_and_one_beside_it_is_not() {
		let spans = [(0x200000, 0x200040), (0x200100, 0x200180)];
		assert!(overlaps(&spans, 0x200000, 4));
		assert!(overlaps(&spans, 0x20003c, 4));
		assert!(overlaps(&spans, 0x1fffff, 2));
		assert!(overlaps(&spans, 0x200170, 8));
		assert!(!overlaps(&spans, 0x200040, 4));
		assert!(!overlaps(&spans, 0x1ffffc, 4));
		assert!(!overlaps(&spans, 0x200080, 0x80));
		assert!(!overlaps(&[], 0x200000, 4));
	}

	#[test]
	fn a_span_nested_in_or_overlapping_another_does_not_hide_the_outer_one() {
		// Outer 0x200000..0x200100 with a function nested in it, one overlapping its end, and a later disjoint one.
		let sets = vec![vec![(0x200000, 0x200100, 1), (0x200010, 0x200020, 2), (0x2000e0, 0x200140, 3), (0x200300, 0x200310, 4)]];
		let spans = used_spans(&sets, |lo, _| match lo {
			0x200000 => 1,
			0x200010 => 2,
			0x2000e0 => 3,
			_ => 4,
		});
		// A write after the nested function but inside the outer one (the last-span test alone would miss it) and one in the overlap tail.
		assert!(overlaps(&spans, 0x2000a0, 4));
		assert!(overlaps(&spans, 0x200130, 4));
		assert!(overlaps(&spans, 0x2000fc, 8));
		// Gaps stay free: between the overlapped group and the next function, and before the first.
		assert!(!overlaps(&spans, 0x200140, 4));
		assert!(!overlaps(&spans, 0x2002fc, 4));
		assert!(!overlaps(&spans, 0x1ffff0, 0x10));
		assert!(overlaps(&spans, 0x200304, 4));
	}

	#[test]
	fn a_refused_write_is_logged_once_per_address_and_for_eight_addresses() {
		use std::cell::RefCell;
		use std::collections::HashSet;
		let said = AtomicU32::new(0);
		let (keys, lines) = (RefCell::new(HashSet::new()), RefCell::new(Vec::new()));
		// The same once-per-key rule as `native_host::say_once`, with its own state.
		let say = |k: &str, m: &str| {
			let first = keys.borrow_mut().insert(k.to_string());
			if first {
				lines.borrow_mut().push(m.to_string());
			}
			first
		};
		// 9 distinct addresses, the first one written again 5 times in between: 8 lines, one per address, none for the 9th.
		for i in 0..9u32 {
			log_refused(&said, 0x200000 + i * 4, 4, &say);
			log_refused(&said, 0x200000, 4, &say);
		}
		let lines: Vec<String> = lines.borrow().clone();
		assert_eq!(lines.len(), 8);
		assert_eq!(said.load(Ordering::Relaxed), 8);
		assert!(lines[0].contains("0x200000") && lines[7].contains("0x20001c"));
		assert!(!lines.iter().any(|l| l.contains("0x200020")));
		// The cap checks first: a 10th address does not even enter the once-per-key set.
		log_refused(&said, 0x300000, 4, &say);
		assert!(!keys.borrow().contains("code_write 300000"));
	}

	#[test]
	fn only_the_sets_that_still_match_ram_count() {
		let sets = vec![vec![(0x200000, 0x200040, 7), (0x200100, 0x200180, 9)], vec![(0x300000, 0x300020, 5)]];
		// The first set matches, the second does not (another level's code in RAM).
		let hash = |lo: u32, _hi: u32| match lo {
			0x200000 => 7,
			0x200100 => 9,
			_ => 0,
		};
		assert_eq!(used_spans(&sets, hash), vec![(0x200000, 0x200040), (0x200100, 0x200180)]);
		// One changed function takes its whole set out (the table would drop it too).
		let patched = |lo: u32, _hi: u32| if lo == 0x200100 { 1 } else { 7 };
		assert!(used_spans(&sets, patched).is_empty());
	}
}
