//! [PC] Value texts known only at run time (lane nups2): a Choice row's value can show a text the engine found out while running
//! (the Upscaler row's FidelityFX value: "FSR 3.1" / "FSR 4.1", whatever the PC's runtime offers) in place of its static name
//! (model_edit.rs `text_of`). A row id can be watched (`watch`): the first time the menu asks for one of its texts, `seen` says
//! so once, so the engine can find the text out then (the upscaler probes its runtime once, right after the window's device is
//! made, and names its values again when the menu shows the row or the player picks it). Game-neutral: ids are the rows' own (`UiRow::id`); a game whose rows nobody labels sees no change.
//! A value can also be withdrawn (`offer`, lane nfsr1: an upscaler this PC cannot run): the menu skips it when stepping and a
//! row holding it reads as its default (model.rs `value`, model_edit.rs `valid` / `step_row`, screen.rs).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// (row id, value, text) of every value with a run-time text.
static LABELS: Mutex<Vec<(&'static str, i32, String)>> = Mutex::new(Vec::new());
/// (row id, asked since the last `seen`) of every watched row.
static WATCHED: Mutex<Vec<(&'static str, bool)>> = Mutex::new(Vec::new());
/// (row id, value) of every withdrawn value, and whether there is one (the menu asks `offered` per row and frame: no lock while
/// nothing is withdrawn).
static WITHDRAWN: Mutex<Vec<(&'static str, i32)>> = Mutex::new(Vec::new());
static ANY_WITHDRAWN: AtomicBool = AtomicBool::new(false);

/// Value `v` of row `id` is offered from now on (`on`) or withdrawn (not offered: skipped, read as the row's default).
pub fn offer(id: &'static str, v: i32, on: bool) {
	let mut w = WITHDRAWN.lock().unwrap_or_else(|e| e.into_inner());
	w.retain(|e| !(e.0 == id && e.1 == v));
	if !on {
		w.push((id, v));
	}
	ANY_WITHDRAWN.store(!w.is_empty(), Ordering::Relaxed);
}

/// Whether value `v` of row `id` is offered (every value is, until `offer` withdraws it).
pub fn offered(id: &str, v: i32) -> bool {
	!ANY_WITHDRAWN.load(Ordering::Relaxed) || !WITHDRAWN.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|e| e.0 == id && e.1 == v)
}

/// Value `v` of row `id` shows `text` from now on.
pub fn set(id: &'static str, v: i32, text: String) {
	let mut l = LABELS.lock().unwrap_or_else(|e| e.into_inner());
	match l.iter_mut().find(|e| e.0 == id && e.1 == v) {
		Some(e) => e.2 = text,
		None => l.push((id, v, text)),
	}
}

/// Value `v` of row `id` shows its static name again.
pub fn clear(id: &str, v: i32) {
	LABELS.lock().unwrap_or_else(|e| e.into_inner()).retain(|e| !(e.0 == id && e.1 == v));
}

/// The run-time text of value `v` of row `id` (None: the row's static name). Marks a watched row as asked for.
pub fn get(id: &str, v: i32) -> Option<String> {
	if let Some(w) = WATCHED.lock().unwrap_or_else(|e| e.into_inner()).iter_mut().find(|w| w.0 == id) {
		w.1 = true;
	}
	LABELS.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|e| e.0 == id && e.1 == v).map(|e| e.2.clone())
}

/// Watch row `id` (`seen`).
pub fn watch(id: &'static str) {
	let mut w = WATCHED.lock().unwrap_or_else(|e| e.into_inner());
	if !w.iter().any(|e| e.0 == id) {
		w.push((id, false));
	}
}

/// Whether the menu asked for a text of watched row `id` since the last call (cleared by this call).
pub fn seen(id: &str) -> bool {
	WATCHED.lock().unwrap_or_else(|e| e.into_inner()).iter_mut().find(|w| w.0 == id).is_some_and(|w| std::mem::take(&mut w.1))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn texts_and_watching() {
		assert_eq!(get("live test row", 2), None);
		set("live test row", 2, "FSR 3.1".into());
		assert_eq!(get("live test row", 2).as_deref(), Some("FSR 3.1"));
		assert_eq!(get("live test row", 1), None);
		set("live test row", 2, "FSR 4.1".into());
		assert_eq!(get("live test row", 2).as_deref(), Some("FSR 4.1"));
		set("live test row", 1, "DLAA".into());
		clear("live test row", 2);
		assert_eq!(get("live test row", 2), None, "cleared: the static name again");
		assert_eq!(get("live test row", 1).as_deref(), Some("DLAA"), "only that value");
		clear("live test row", 2);
		assert!(!seen("live watched row"), "not watched");
		watch("live watched row");
		assert!(!seen("live watched row"), "not asked yet");
		let _ = get("live watched row", 0);
		assert!(seen("live watched row"));
		assert!(!seen("live watched row"), "cleared by the first call");
	}

	#[test]
	fn values_can_be_withdrawn_and_offered_again() {
		assert!(offered("live offer row", 1), "every value is offered at first");
		offer("live offer row", 1, false);
		assert!(!offered("live offer row", 1));
		assert!(offered("live offer row", 2) && offered("live other row", 1), "only that value of that row");
		offer("live offer row", 1, false);
		offer("live offer row", 1, true);
		assert!(offered("live offer row", 1), "offered again (twice withdrawn = one entry)");
	}
}
