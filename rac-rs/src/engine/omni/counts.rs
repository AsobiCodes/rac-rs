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
pub const NAMES: [&str; 6] = ["call_game refused", "relocate", "relocate (unmapped)", "raycast hit", "raycast miss", "raycast refused"];

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
