//! Counters of what native mods do through the host entries, for the run log (`omni: native calls at tick N: call_game fast_cos=12 ...`):
//! `call_game` per game function (by its name in `rac1_funcs.txt`, or the number when the port does not know it), `call_game refused`, and the
//! `relocate` answers. A mod that shows no count here did not use that entry; a count with the mod's own output is the proof the entry works.

use std::collections::BTreeMap;
use std::sync::Mutex;

static COUNTS: Mutex<BTreeMap<String, u64>> = Mutex::new(BTreeMap::new());

/// One more use of `key`.
pub fn bump(key: &str) {
	let mut g = COUNTS.lock().unwrap_or_else(|e| e.into_inner());
	*g.entry(key.to_string()).or_insert(0) += 1;
}

/// How often `key` was used so far.
pub fn get(key: &str) -> u64 {
	COUNTS.lock().unwrap_or_else(|e| e.into_inner()).get(key).copied().unwrap_or(0)
}

/// `key=count` for every key used, in key order; None when nothing was used.
pub fn line() -> Option<String> {
	let g = COUNTS.lock().unwrap_or_else(|e| e.into_inner());
	(!g.is_empty()).then(|| g.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" "))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn counts_per_key_and_line_lists_them() {
		bump("test_a");
		bump("test_a");
		bump("test_b");
		assert_eq!((get("test_a"), get("test_b"), get("test_none")), (2, 1, 0));
		let l = line().unwrap();
		assert!(l.contains("test_a=2") && l.contains("test_b=1"), "{l}");
	}
}
