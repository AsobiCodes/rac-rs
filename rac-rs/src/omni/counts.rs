//! Counters of what native mods do through the host entries, for the run log (`omni: native calls at tick N: call_game fast_cos=12 ...`):
//! `call_game` per game function (by its name in `rac1_funcs.txt`), `call_game refused`, the `relocate` answers and the line tests. A mod that
//! shows no count here did not use that entry; a count with the mod's own output is the proof the entry works.
//!
//! A count is one atomic add (no lock, no allocation) on the hot path; the text is built only in `line()`. The per-function counts live in the
//! function's spec (`funcs::FuncSpec::calls`).

use std::sync::atomic::{AtomicU64, Ordering};

/// What is counted besides the per-function `call_game` counts.
#[derive(Clone, Copy)]
pub enum Counter {
	CallGameRefused,
	Relocate,
	RelocateUnmapped,
	RaycastHit,
	RaycastMiss,
	RaycastRefused,
}

/// The names in the run-log line, in `Counter` order.
const NAMES: [&str; 6] = ["call_game refused", "relocate", "relocate (unmapped)", "raycast hit", "raycast miss", "raycast refused"];

static COUNTS: [AtomicU64; NAMES.len()] = [const { AtomicU64::new(0) }; NAMES.len()];

/// One more use of `c`.
pub fn bump(c: Counter) {
	COUNTS[c as usize].fetch_add(1, Ordering::Relaxed);
}

/// How often `c` was used so far.
pub fn get(c: Counter) -> u64 {
	COUNTS[c as usize].load(Ordering::Relaxed)
}

/// `key=count` for every key used, in key order; None when nothing was used.
pub fn line() -> Option<String> {
	let mut all: Vec<(String, u64)> = NAMES.iter().zip(&COUNTS).map(|(k, n)| (k.to_string(), n.load(Ordering::Relaxed))).collect();
	all.extend(super::funcs::all().iter().map(|f| (format!("call_game {}", f.name), f.calls.load(Ordering::Relaxed))));
	all.retain(|(_, n)| *n > 0);
	all.sort();
	(!all.is_empty()).then(|| all.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" "))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn counts_per_key_and_line_lists_them() {
		let before = (get(Counter::RaycastHit), get(Counter::RaycastMiss));
		bump(Counter::RaycastHit);
		bump(Counter::RaycastHit);
		bump(Counter::RaycastMiss);
		assert_eq!((get(Counter::RaycastHit) - before.0, get(Counter::RaycastMiss) - before.1), (2, 1));
		let l = line().unwrap();
		assert!(l.contains("raycast hit=") && l.contains("raycast miss="), "{l}");
		// Every counter has its name.
		assert_eq!(NAMES[Counter::RaycastRefused as usize], "raycast refused");
		assert_eq!(NAMES[Counter::CallGameRefused as usize], "call_game refused");
		assert_eq!((NAMES[Counter::Relocate as usize], NAMES[Counter::RelocateUnmapped as usize]), ("relocate", "relocate (unmapped)"));
	}

	#[test]
	fn a_function_call_is_counted_in_its_spec_and_named_in_the_line() {
		let spec = &super::super::funcs::all()[0];
		spec.calls.fetch_add(1, Ordering::Relaxed);
		let l = line().unwrap();
		assert!(l.contains(&format!("call_game {}=", spec.name)), "{l}");
	}
}
