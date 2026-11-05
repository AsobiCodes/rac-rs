//! [PC] The page hosts' watchdog (v0.5 hardening). A modern page host (title, pause, shop, Galactic Map) that is "up" owns the mouse and
//! hides the retail picture behind its page. If no page is finished (`end`) for `SILENT_MS` and the last draw list is empty, the screen
//! is blank and every click is dead (the shop whose rows never read, a page the builder cannot make). `host::watchdog`, once per
//! display frame, then gives such a host up (`trip`): its hide switch goes off, so the classic screen and the classic mouse are back.
//! A tripped host stays down until its frame step asks it down once (the screen was left, the style was switched): a frame step that
//! asks it up in every frame (the shop's "hold") would otherwise bring the blank page straight back. Each host keeps one `Watch`.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// How long a host may be up without finishing a page before an empty one counts as stuck (ms).
pub const SILENT_MS: u64 = 1500;

/// The rule: up, silent for `SILENT_MS` (the time since the last finished page, or since the host came up) and nothing in the last draw
/// list.
pub fn stuck(up: bool, silent_ms: u64, last_empty: bool) -> bool {
	up && silent_ms >= SILENT_MS && last_empty
}

/// One host's watch: when it last finished a page, and whether the watchdog gave it up.
pub struct Watch {
	ended: AtomicU64,
	tripped: AtomicBool,
}

impl Watch {
	pub const fn new() -> Watch {
		Watch { ended: AtomicU64::new(0), tripped: AtomicBool::new(false) }
	}

	/// A page was finished, or the host came up, at `now` (the host's own ms clock).
	pub fn stamp(&self, now: u64) {
		self.ended.store(now, Ordering::Relaxed);
	}

	/// The host's `set_up(on)`: whether the host may be up now. Asking it down clears a trip; a tripped host refuses to come up.
	pub fn allow(&self, on: bool) -> bool {
		if !on {
			self.tripped.store(false, Ordering::Relaxed);
		}
		on && !self.tripped.load(Ordering::Relaxed)
	}

	/// The watchdog's question for a host that is `up` with `last_empty` as its last draw list, at `now`.
	pub fn silent(&self, up: bool, now: u64, last_empty: bool) -> bool {
		stuck(up, now.saturating_sub(self.ended.load(Ordering::Relaxed)), last_empty)
	}

	/// The watchdog gave the host up (call after the host went down: `set_up(false)` clears a trip).
	pub fn trip(&self) {
		self.tripped.store(true, Ordering::Relaxed);
	}

	pub fn tripped(&self) -> bool {
		self.tripped.load(Ordering::Relaxed)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Only an up host with an empty last list that finished nothing for the whole time is stuck.
	#[test]
	fn stuck_needs_up_silence_and_an_empty_list() {
		assert!(stuck(true, SILENT_MS, true));
		assert!(!stuck(true, SILENT_MS - 1, true), "not long enough");
		assert!(!stuck(true, 10_000, false), "a picture is on screen (a repeated last page)");
		assert!(!stuck(false, 10_000, true), "not up: nothing to give up");
	}

	/// A trip keeps the host down against "hold" frames until something asks it down once.
	#[test]
	fn a_tripped_host_stays_down_until_asked_down() {
		let w = Watch::new();
		assert!(w.allow(true));
		w.stamp(100);
		assert!(!w.silent(true, 100 + SILENT_MS - 1, true));
		assert!(w.silent(true, 100 + SILENT_MS, true));
		assert!(!w.silent(true, 100 + SILENT_MS, false));
		w.trip();
		assert!(w.tripped() && !w.allow(true), "hold frames do not bring it back");
		assert!(!w.allow(false) && !w.tripped(), "asked down once: the trip is over");
		assert!(w.allow(true));
	}

	/// A finished page restarts the silence.
	#[test]
	fn a_finished_page_restarts_the_clock() {
		let w = Watch::new();
		w.stamp(0);
		w.stamp(1400);
		assert!(!w.silent(true, 2000, true));
		assert!(w.silent(true, 1400 + SILENT_MS, true));
	}
}
