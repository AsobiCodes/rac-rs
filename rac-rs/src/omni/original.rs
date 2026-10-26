//! `call_original` between two fields (ABI 1.2 mods: `omni.fn(hookid)` called from an event or `on_frame`, not from inside a hook).
//!
//! Inside a hook dispatch `call_original` runs the retail function the hook sits at with the dispatch's own entry and return address
//! (bridge.rs `call_retail`). Outside one there is no dispatch to take them from, so the entry is found now: the hook point's relocation
//! spec (`hooks::slot(n).reloc`, the same masked hash the hand was installed with) is searched in the code loaded now, like `funcs::resolve`
//! does for a game function number. The call is then made like `native_host::do_call` makes one (`run_at`: registers saved and restored,
//! field timer held off), with the slot's guard TAKEN for its whole length so the hand at the entry falls through to the retail code instead
//! of dispatching the mods' chain again.
//!
//! Refused (and said once in the run log, the mod gets a zero): no machine in scope, the machine stopped, native mods stale, calls nested
//! too deep, an argument count other than the signature's, the hook busy (its dispatch is in progress further up), or the function is not in
//! the code loaded now (another level's overlay).

use super::binding::Sig;
use super::bridge::{self, Guard};
use super::cabi::Value;
use super::{funcs, hooks, native_host};
use ee_oracle::cpu::Cpu;
use ee_oracle::hs_reloc::Reloc;
use std::collections::HashMap;
use std::sync::Mutex;

/// Per hook slot: the entry found, or the level word at which the search failed (no new scan until the level changes).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Found {
	At(u32),
	Missing(i32),
}

static CACHE: Mutex<Option<HashMap<usize, Found>>> = Mutex::new(None);

/// Records the answer of the level-load prescan for hook slot `n` (prescan.rs).
pub(super) fn seed(n: usize, entry: Option<u32>, level_word: i32) {
	let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
	g.get_or_insert_with(HashMap::new).insert(n, entry.map_or(Found::Missing(level_word), Found::At));
}

/// Refusals that need no machine state: the machine state is passed in so the rules can be tested.
pub fn refusal(stopped: bool, stale: bool, given: usize, wanted: usize) -> Option<&'static str> {
	if stopped {
		Some("the game machine has stopped")
	} else if stale {
		Some("native mods are switched off (a second game load)")
	} else if given != wanted {
		Some("wrong number of arguments for the hook's signature")
	} else {
		None
	}
}

/// The entry of the hook point's function in `words` (code from `funcs::CODE_LO`), using the cache of slot `n`.
/// `still_there` tells whether a cached entry still matches the spec in the code now.
fn entry_in(n: usize, reloc: &Reloc, level_word: i32, words: impl FnOnce() -> Vec<u32>, still_there: impl Fn(u32) -> bool) -> Option<u32> {
	let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
	let cache = g.get_or_insert_with(HashMap::new);
	match cache.get(&n).copied() {
		Some(Found::At(e)) if still_there(e) => return Some(e),
		Some(Found::Missing(l)) if l == level_word => return None,
		_ => {}
	}
	match funcs::find_in(&words(), funcs::CODE_LO, reloc) {
		Some(e) => {
			cache.insert(n, Found::At(e));
			Some(e)
		}
		None => {
			cache.insert(n, Found::Missing(level_word));
			None
		}
	}
}

/// The retail function of hook `n` with `args`, called between fields. Err: the reason (see the module doc).
pub fn call_between(c: &mut Cpu, n: usize, sig: &Sig, args: &[Value]) -> Result<Value, String> {
	let s = hooks::slot(n);
	let name = s.id.get().map_or("?", String::as_str);
	if let Some(why) = refusal(c.stop.is_some(), native_host::stale(), args.len(), sig.args.len()) {
		return Err(why.to_string());
	}
	native_host::depth_check()?;
	let Some(reloc) = s.reloc.get() else { return Err("the hook point has no relocation spec".into()) };
	let Some(_guard) = Guard::take(&s.guard) else { return Err(format!("hook {name} is busy (its dispatch is in progress)")) };
	let level = native_host::level_word(c);
	let Some(entry) = entry_in(n, reloc, level, || funcs::code_words(c), |e| reloc.matches(&c.mem, e)) else {
		return Err(format!("the function of hook {name} is not in the code loaded now"));
	};
	native_host::run_at(c, entry, |c| bridge::put_args(c, sig, args), |c| bridge::read_ret(c, sig.ret)).map_err(|why| format!("the function of hook {name} {why}"))
}

/// The host's `call_original` outside a hook dispatch: the value, or zero with the reason said once.
pub fn between_value(c: &mut Cpu, n: usize, sig: &Sig, args: &[Value]) -> Value {
	match call_between(c, n, sig, args) {
		Ok(v) => v,
		Err(why) => {
			native_host::say_once(&format!("original-{n}-{why}"), &format!("call_original (hook {n}) refused: {why}"));
			Value::default()
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::cell::Cell;

	#[test]
	fn refusals_cover_a_stopped_machine_stale_mods_and_a_wrong_argument_count() {
		assert_eq!(refusal(false, false, 2, 2), None);
		assert!(refusal(true, false, 2, 2).unwrap().contains("stopped"));
		assert!(refusal(false, true, 2, 2).unwrap().contains("second game load"));
		assert!(refusal(false, false, 1, 2).unwrap().contains("number of arguments"));
		assert!(refusal(false, false, 0, 1).is_some());
		// A stopped machine is reported before the other reasons.
		assert!(refusal(true, true, 9, 1).unwrap().contains("stopped"));
	}

	#[test]
	fn a_missing_function_is_searched_once_per_level_and_a_found_one_is_kept_while_it_matches() {
		// Slots 250.. are unused by other tests (the cache is process-wide).
		let reloc = Reloc::parse("reloc t entry 0x100 len 0x10\nmhash 0x1\nmask 0x4 0xffff\n").unwrap();
		let scans = Cell::new(0);
		let words = || {
			scans.set(scans.get() + 1);
			vec![0u32; 8]
		};
		assert_eq!(entry_in(251, &reloc, 3, words, |_| true), None);
		assert_eq!(scans.get(), 1);
		// Same level: no new scan.
		assert_eq!(entry_in(251, &reloc, 3, || panic!("must not scan again"), |_| true), None);
		// Another level: scanned again.
		assert_eq!(entry_in(251, &reloc, 4, words, |_| true), None);
		assert_eq!(scans.get(), 2);
	}
}
