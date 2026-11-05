//! [PC] Achievements (user 2026-11-02: "add achievements both in game and synchroed to the launcher", the PS3 trophy list). The
//! game-neutral half: a game pack hands over its list and one detector (`Pack::achievements`, default none: R&C2 has none); this
//! module keeps the unlock state in `achievements.json` next to `pc_options.txt`, queues the pop-ups and draws them and the list.
//!
//! - `store.rs`: the entries, the debounce, `achievements.json` (safe read of a missing or broken file: the broken one is kept).
//! - `writer.rs`: the small thread that writes the file (atomic: unique tmp, fsync, rename), off the hub lock.
//! - `toast.rs`: the pop-up top right (icon in its grade ring, name, about 5 s, queued) and the procedural grade cups. Drawn in the
//!   HUD pass from the wall clock, never from game fields, so frame smoothing cannot make it flicker between stepped and drawn frames.
//! - `screen.rs`: the list (opened from the pause menu with F6) and its hint line.
//! - `icons.rs` / `badge.rs`: the icons a pack hands over (`AchPack::icons`) as one atlas for the GPU (`Quad.tex == ICON_TEX`), and
//!   the badge (icon circle + grade ring) the pop-up and the list draw; locked = grey, locked hidden = the hidden icon.
//!
//! The detector reads retail RAM only (`AchPack::read`, once per field from the worker, no RAM writes: RAM hashes unchanged).
mod badge;
pub mod icons;
mod screen;
mod store;
mod toast;
mod view;
mod writer;
#[cfg(test)]
mod icon_tests;
#[cfg(test)]
mod tests;

pub use icons::{atlas, IconSet, ICON_TEX};
pub use screen::{close, draw, is_open, open, ui_key, ui_pad};
pub use view::{cursor, focus_on, hosted, list_view, move_focus, set_hosted, window, ListView, RowView};
pub use toast::hold as hold_toast;
pub use store::{iso_to_unix, unix_to_iso, Entry, Loaded, Store};

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grade {
	Bronze,
	Silver,
	Gold,
	Platinum,
}

impl Grade {
	pub fn key(self) -> &'static str {
		match self {
			Grade::Bronze => "bronze",
			Grade::Silver => "silver",
			Grade::Gold => "gold",
			Grade::Platinum => "platinum",
		}
	}
	pub fn label(self) -> &'static str {
		match self {
			Grade::Bronze => "BRONZE",
			Grade::Silver => "SILVER",
			Grade::Gold => "GOLD",
			Grade::Platinum => "PLATINUM",
		}
	}
	pub fn colour(self) -> [f32; 3] {
		match self {
			Grade::Bronze => [0.80, 0.50, 0.20],
			Grade::Silver => [0.78, 0.80, 0.86],
			Grade::Gold => [1.00, 0.80, 0.15],
			Grade::Platinum => [0.62, 0.86, 1.00],
		}
	}
}

/// One achievement of a game's list.
#[derive(Clone, Copy, Debug)]
pub struct Def {
	pub id: &'static str,
	pub name: &'static str,
	pub desc: &'static str,
	pub grade: Grade,
	/// Name and description stay secret in the list until it is unlocked.
	pub hidden: bool,
	/// Progress goal (shown as n / max); 0 = no counter.
	pub max: u32,
	/// Unlocks by itself when every other entry of the list is unlocked (the game does not detect it).
	pub platinum: bool,
}

/// What the detector saw this field for one `Def` (same index): the condition holds now, and the counter if it has one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reading {
	pub met: bool,
	pub progress: u32,
	/// Some(n): `progress` counts from zero again whenever the game starts a new cycle (R&C1: New Game+), n is the cycle number.
	/// The store then adds the total of the finished cycles (`Entry::carry`) so the counter is a lifetime total, and an older cycle
	/// (an earlier save loaded) never lowers or inflates it. None = a plain counter.
	pub epoch: Option<u32>,
}

/// A game's list and detector (`Pack::achievements`).
pub struct AchPack {
	/// The `game` key of achievements.json ("rac1").
	pub game: &'static str,
	pub defs: &'static [Def],
	/// Game RAM (address = index) to one `Reading` per def, or None when the game is not in real play (title, loading garbage).
	pub read: fn(&[u8]) -> Option<Vec<Reading>>,
	/// One picture per def (`icons.rs`), None = the pop-up and the list draw a procedural cup instead.
	pub icons: Option<&'static IconSet>,
}

pub(crate) struct Hub {
	pub pack: &'static AchPack,
	pub store: Store,
	pub path: PathBuf,
	/// Unlocked indices waiting for their pop-up.
	pub queue: VecDeque<usize>,
	pub last_save: Option<Instant>,
	pub dirty: bool,
	/// Never written this session: the file exists but could not be read at init (the list in memory starts empty, pop-ups still show),
	/// or the session was cheated at init (read-only, `load_into`; review R22 m1).
	pub unreadable: bool,
}

static HUB: Mutex<Option<Hub>> = Mutex::new(None);

pub(crate) fn hub() -> MutexGuard<'static, Option<Hub>> {
	HUB.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_unix() -> u64 {
	std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Starts the hub for the running game's pack with `path` as the file (first call wins; no pack = nothing). Loads the file.
pub fn init(path: PathBuf) {
	let Some(p) = crate::pack::get().achievements else { return };
	if load_into(p, &HUB, path, true) {
		icons::prewarm();
	}
}

fn lock_hub(hub: &Mutex<Option<Hub>>) -> MutexGuard<'_, Option<Hub>> {
	hub.lock().unwrap_or_else(|e| e.into_inner())
}

/// Loads `path` into an empty `hub` (first call wins); true = loaded now. `write` false = read-only (a session cheated before its
/// first field, review R22 m1): the file is only read, never made, rewritten or replaced, a broken one is not copied aside, and the
/// hub writes nothing this session (before lane nach such a session never loaded the hub and so never touched the disk either).
fn load_into(p: &'static AchPack, hub: &Mutex<Option<Hub>>, path: PathBuf, write: bool) -> bool {
	if lock_hub(hub).is_some() {
		return false;
	}
	// All file work happens before the hub lock is taken: the folder (made once here), the read, and the existence check.
	if write {
		if let Some(dir) = path.parent() {
			let _ = std::fs::create_dir_all(dir);
		}
		writer::remove_stale_tmp(&path, std::process::id());
	}
	let mut store = Store::new(p);
	// Only "no file" starts a new one; a file that is there but cannot be read (after a few tries) is never written over.
	let loaded = if write { store.load(&path) } else { store.peek(&path) };
	let mut h = lock_hub(hub);
	if h.is_some() {
		return false;
	}
	let unreadable = !write || loaded == Loaded::Unreadable;
	let hb = h.insert(Hub { pack: p, store, path, queue: VecDeque::new(), last_save: None, dirty: write && loaded == Loaded::New, unreadable });
	if write {
		hb.flush(true); // only builds the text and hands it to the writer thread: no file I/O under the lock
	}
	true
}

/// This game has a list (the hint and the F6 key exist only then).
pub fn available() -> bool {
	crate::pack::get().achievements.is_some()
}

/// The session's cheat latch (fix round M2 / m6): once the game says its cheating tools were on (R&C1: Debug mode, the debug keys,
/// the dev tools), nothing unlocks and no counter moves for the rest of the session, whatever happens to the switch afterwards.
/// Whatever is already in the file stays. `owns_f6`: the debug keys are on right now, so F6 is theirs (infinite ammo), not the list's.
pub(crate) struct Latch {
	cheated: AtomicBool,
	owns_f6: AtomicBool,
}

impl Latch {
	pub(crate) const fn new() -> Latch {
		Latch { cheated: AtomicBool::new(false), owns_f6: AtomicBool::new(false) }
	}
	/// One field's look at the switches: `on` latches, `owns_f6` follows the switch live.
	pub(crate) fn note(&self, on: bool, owns_f6: bool) {
		if on {
			self.cheated.store(true, Ordering::Relaxed);
		}
		self.owns_f6.store(owns_f6, Ordering::Relaxed);
	}
	pub(crate) fn cheated(&self) -> bool {
		self.cheated.load(Ordering::Relaxed)
	}
	pub(crate) fn owns_f6(&self) -> bool {
		self.owns_f6.load(Ordering::Relaxed)
	}
}

static CHEATS: Latch = Latch::new();

/// The game's call every field in real play, before `field`: are cheating tools on now, and do they own F6 now.
pub fn note_cheats(on: bool, owns_f6: bool) {
	CHEATS.note(on, owns_f6);
}

/// Cheating tools were on at some point of this session: achievements are off until the game is restarted.
pub fn cheated() -> bool {
	CHEATS.cheated()
}

/// The debug keys are on right now: F6 is theirs (`screen::ui_key` leaves it alone).
pub(crate) fn owns_f6() -> bool {
	CHEATS.owns_f6()
}

/// The worker's call after a field in real play: reads the game's RAM, unlocks what is met, queues pop-ups. Nothing unlocks once
/// cheated, but the file is loaded all the same: the list shows what was earned before (lane nach 2026-11-03: a session cheated from
/// its first field, Play.cmd's debug keys, never loaded it, and the opened list drew nothing at all, `screen::list` / the modern page).
/// A session cheated at that first load reads the file only: nothing on disk changes (review R22 m1, `load_into`).
pub fn field(ram: &[u8]) {
	let Some(p) = crate::pack::get().achievements else { return };
	if field_in(p, &HUB, || crate::pcopts::player_dir().join("achievements.json"), cheated(), ram) {
		icons::prewarm();
	}
}

/// `field` on `hub` with the file `path` (the test seam); true = the hub was loaded in this call.
fn field_in(p: &'static AchPack, hub: &Mutex<Option<Hub>>, path: impl FnOnce() -> PathBuf, cheated: bool, ram: &[u8]) -> bool {
	let empty = lock_hub(hub).is_none();
	let loaded = empty && load_into(p, hub, path(), !cheated);
	if cheated {
		return loaded;
	}
	if let Some(r) = (p.read)(ram) {
		if let Some(h) = lock_hub(hub).as_mut() {
			h.observe(&r, now_unix());
		}
	}
	loaded
}

/// How long a quit waits in all for the worker to let go of the hub and for the writer thread to put the file on disk.
const FLUSH_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// The quit path (`quit::quit_app`, also Alt+F4): hands the unsaved unlocks and counters to the writer thread NOW, skipping the 10 s
/// debounce (waiting for the worker if it holds the hub), then waits until the writer has them on disk (`writer::wait_idle`); at most
/// `FLUSH_WAIT` in all. A write that failed (the last one before the quit, or this one: a launcher or a virus scanner holding the file
/// a moment) is handed over again, at most `QUIT_RETRIES` times. False = the hub or the writer stayed busy, or the last write failed;
/// true = on disk, or nothing to flush (no hub: a game without a list, or before the first field).
pub fn flush_now() -> bool {
	flush_retrying(&HUB, FLUSH_WAIT)
}

/// How often the quit writes again after a failed write, `QUIT_RETRY` apart (a few hundred ms at most, so a quit never hangs on a
/// folder that stays unwritable).
const QUIT_RETRIES: u32 = 4;
const QUIT_RETRY: std::time::Duration = std::time::Duration::from_millis(50);

fn flush_retrying(hub: &Mutex<Option<Hub>>, wait: std::time::Duration) -> bool {
	let t0 = Instant::now();
	let left = || wait.saturating_sub(t0.elapsed());
	for retry in 0..=QUIT_RETRIES {
		let Some(path) = flush_hub(hub, left()) else { return false };
		if !writer::wait_idle(left()) {
			return false;
		}
		if !path.is_some_and(|p| writer::failed(&p)) {
			return true;
		}
		if retry < QUIT_RETRIES {
			std::thread::sleep(QUIT_RETRY.min(left()));
		}
	}
	false
}

#[cfg(test)]
fn flush_in(hub: &Mutex<Option<Hub>>, wait: std::time::Duration) -> bool {
	flush_hub(hub, wait).is_some()
}

/// Hands a dirty hub's text to the writer, or the whole state again when its last write failed (the worker's `observe` notices that
/// only at the next field, which a quit never reaches). Waits at most `wait` for the worker to let go of the hub: None = it stayed
/// busy; Some(the hub's file, None without a hub) = done.
fn flush_hub(hub: &Mutex<Option<Hub>>, wait: std::time::Duration) -> Option<Option<std::path::PathBuf>> {
	let t0 = Instant::now();
	loop {
		let mut g = match hub.try_lock() {
			Ok(g) => g,
			Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
			Err(std::sync::TryLockError::WouldBlock) if t0.elapsed() < wait => {
				std::thread::sleep(std::time::Duration::from_millis(2));
				continue;
			}
			Err(std::sync::TryLockError::WouldBlock) => return None,
		};
		if let Some(h) = g.as_mut().filter(|h| h.dirty || writer::take_failed(&h.path)) {
			h.flush(true);
		}
		return Some(g.as_ref().map(|h| h.path.clone()));
	}
}

impl Hub {
	/// Applies one reading; unlocks queue their pop-ups and are written at once, counters at most every 10 s.
	pub(crate) fn observe(&mut self, r: &[Reading], now: u64) {
		let (new, changed) = self.store.apply(r, now);
		// A write that failed (disk full, file locked) is repeated with the whole state at the next flush, at most every 10 s.
		self.dirty |= changed | writer::take_failed(&self.path);
		let urgent = !new.is_empty();
		self.queue.extend(new);
		if self.dirty {
			self.flush(urgent);
		}
	}

	/// Hands the file's text to the writer thread (built here, under the hub lock; the disk work happens on the thread, latest text wins).
	pub(crate) fn flush(&mut self, force: bool) {
		if self.unreadable {
			self.dirty = false;
			return;
		}
		if !force && self.last_save.is_some_and(|t| t.elapsed().as_secs() < 10) {
			return;
		}
		writer::submit(self.path.clone(), self.store.to_json(now_unix()));
		self.dirty = false;
		self.last_save = Some(Instant::now());
	}
}
