//! Game events for the mods (ABI 1.2 `omni_event`): `level_loaded`, `level_unloading`, `mode` and `pause_page`, and the mode / page names.
//!
//! Read RAM only, never write. Once per field (`omni::tick`, before the mods' `on_frame`) the words below are read, compared with what the
//! mods were last told, and one event is sent per change. Nothing runs without a loaded mod or when the DLL has no `omni_event`.
//!
//! Words: the game's level word, level-loop exit word and game-mode word (`Game::level_word`, `exit_word`, `mode_word`; R&C1's are listed
//! in rac1/omni_bind.rs). The exit word set is "unloading"; the level word changing while a level is loaded counts as unloading too.
//! The title screen: the front-end flag of `worker/field_front.rs`, handed over by `note_front` (one field old).
//!
//! Mode names for `omni.mode()`: `title` (front end), `loading` (exit word set, or no level word), else the game's name of the mode word's
//! value (`Game::modes`; `unknown` for one it does not name). Pause pages: `Game::pause_page` reads the open page's id, `Game::pages` names it.
//! The memory-card events come from the game (`Game::card`; R&C1: rac1/omni_bind/card.rs).
//!
//! Not verified on a run yet: that the exit word is seen set for at least one field around every level change (the level word check
//! covers a change to another level; a reload of the same level through the title is covered by the title flag).

use super::{game, lock, native_host, Omni};
use ee_oracle::cpu::Cpu;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// The title / front end is up (set by `note_front` from field_front.rs every field).
pub static FRONT: AtomicBool = AtomicBool::new(false);

/// `retail/worker/field_front.rs` once per field: whether the front end (title screen and its menus) is running.
pub fn note_front(front: bool) {
	FRONT.store(front, Ordering::Relaxed);
}

const MASK: u32 = 0x01ff_ffff;

/// The words the events are made from.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Snapshot {
	pub front: bool,
	pub level_word: i32,
	pub exit: bool,
	pub mode: u32,
	/// The open pause page's id while `mode` is 3.
	pub page_id: Option<u32>,
}

/// One thing to tell the mods.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
	LevelLoaded(i32),
	LevelUnloading(i32),
	/// The new mode name.
	Mode(&'static str),
	/// The new page name, empty when no page is open.
	PausePage(String),
	/// A new game starts (true: New Game+), from `newgame.rs`.
	NewGame(bool),
	/// The game starts writing memory-card slot N (`card.rs`).
	Saving(i32),
	/// A save was loaded from memory-card slot N (`card.rs`).
	SaveLoaded(i32),
}

impl Event {
	/// The `omni_event` arguments: name, the number `a`, the text.
	pub fn parts(&self) -> (&'static str, i64, &str) {
		match self {
			Event::LevelLoaded(l) => ("level_loaded", *l as i64, ""),
			Event::LevelUnloading(l) => ("level_unloading", *l as i64, ""),
			Event::Mode(m) => ("mode", 0, m),
			Event::PausePage(p) => ("pause_page", 0, p),
			Event::NewGame(plus) => ("new_game", i64::from(*plus), ""),
			Event::Saving(s) => ("saving", *s as i64, ""),
			Event::SaveLoaded(s) => ("save_loaded", *s as i64, ""),
		}
	}
}

/// The level a snapshot is in: 0..=18, or -1 on the title screen, while the level loop is ending, or with no level word.
pub fn level_of(s: &Snapshot) -> i32 {
	if s.front || s.exit {
		-1
	} else {
		native_host::loaded_level_of(s.level_word)
	}
}

/// The mode name of `omni.mode()`.
pub fn mode_name(s: &Snapshot) -> &'static str {
	if s.front {
		return "title";
	}
	if s.exit || level_of(s) < 0 {
		return "loading";
	}
	game::get().modes.iter().find(|m| m.0 == s.mode).map_or("unknown", |m| m.1)
}

/// The name of pause page `id`: the API's names where the id maps, else `page_<hex>`.
pub fn page_name(id: u32) -> String {
	game::get().pages.iter().find(|p| p.0 == id).map_or_else(|| format!("page_{id:x}"), |p| p.1.to_string())
}

/// What the mods were told so far.
#[derive(Debug, Default)]
pub struct Watcher {
	level: Option<i32>,
	mode: Option<&'static str>,
	page: String,
	/// New Game from the title (newgame.rs).
	front: super::newgame::FrontWatcher,
	/// The code was scanned for the mods' game functions (prescan.rs) since the front end last came up.
	scanned_front: bool,
}

impl Watcher {
	/// The events for this snapshot, in the order unloading, loaded, mode, pause_page; empty when nothing changed.
	pub fn step(&mut self, s: &Snapshot) -> Vec<Event> {
		let mut out = Vec::new();
		let now = level_of(s);
		let before = self.level.unwrap_or(-1);
		if before >= 0 && before != now {
			out.push(Event::LevelUnloading(before));
		}
		if now >= 0 && now != before {
			out.push(Event::LevelLoaded(now));
		}
		self.level = Some(now);
		let mode = mode_name(s);
		if self.mode != Some(mode) {
			self.mode = Some(mode);
			out.push(Event::Mode(mode));
		}
		let page = if mode == "pause" { s.page_id.map(page_name).unwrap_or_default() } else { String::new() };
		if page != self.page {
			out.push(Event::PausePage(page.clone()));
			self.page = page;
		}
		out
	}
}

fn rd(ram: &[u8], a: u32) -> u32 {
	let i = (a & MASK) as usize;
	ram.get(i..i + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// The snapshot out of the machine's RAM (`ram` = all 32 MB) and the front-end flag.
pub fn snapshot(ram: &[u8], front: bool) -> Snapshot {
	let g = game::get();
	let level_word = rd(ram, g.level_word) as i32;
	let mode = rd(ram, (g.va)(g.mode_word));
	let page_id = (g.pause_page)(ram, level_word, mode);
	Snapshot { front, level_word, exit: rd(ram, (g.va)(g.exit_word)) != 0, mode, page_id }
}

/// The level (0..=18) the 3D game is running in right now, or None: title screen, level loop ending, or no level word. For host entries that
/// need the level's code and data (raycast).
pub fn live_level(c: &Cpu) -> Option<i32> {
	let s = snapshot(&c.mem.ram, FRONT.load(Ordering::Relaxed));
	Some(level_of(&s)).filter(|&l| l >= 0)
}

/// Events go out only to a loader with `omni_event` and at least one loaded mod.
pub fn should_send(mods: usize, dll_has_events: bool) -> bool {
	mods > 0 && dll_has_events
}

/// Once per field, before the mods' `on_frame`: sends the events that changed. Does nothing without events (DLL) or without a loaded mod.
pub fn send(o: &Omni, c: &mut Cpu, tick: u64) {
	if !should_send(o.mods, o.loader.has_events()) {
		return;
	}
	let events = {
		let mut w = o.events.lock().unwrap_or_else(|e| e.into_inner());
		let card = (game::get().card)(c, tick);
		let loaded = card.iter().any(|e| matches!(e, Event::SaveLoaded(_)));
		let level_word = rd(&c.mem.ram, game::get().level_word) as i32;
		let from_title = w.front.step(FRONT.load(Ordering::Relaxed), level_word, loaded);
		let mut v: Vec<Event> = match (super::newgame::take(), from_title) {
			(true, _) => vec![Event::NewGame(true)],
			(false, true) => vec![Event::NewGame(false)],
			(false, false) => Vec::new(),
		};
		v.extend(card);
		v.extend(w.step(&snapshot(&c.mem.ram, FRONT.load(Ordering::Relaxed))));
		v
	};
	prescan_if_needed(o, c, &events);
	if events.iter().any(|e| matches!(e, Event::NewGame(_) | Event::SaveLoaded(_))) {
		super::pose::clear_all();
		super::pose_follow::clear_all();
	}
	if events.is_empty() {
		return;
	}
	let _held = lock::enter();
	for e in &events {
		let (name, a, text) = e.parts();
		let sp = native_host::sp_before(c);
		let ok = super::host::scope(c, || o.loader.event(name, a, text));
		native_host::sp_check(c, sp, "an event");
		if !ok {
			native_host::say_once(&format!("event-{name}"), &format!("omni_event {name} failed"));
		}
		super::trace::event(tick, name, a, text);
		super::logq::line(&format!("omni: event {name} {a} {text}"));
	}
}

/// Resolves the mods' game functions in one pass when a level has loaded (answers cached for the level), and once while the front end runs
/// (title code: the first `call_game` of a mod there would scan 3 MB inside the mod's budget). Not for a front end that is still coming up
/// (answers for it are not cached when a function is missing).
fn prescan_if_needed(o: &Omni, c: &Cpu, events: &[Event]) {
	let level = events.iter().any(|e| matches!(e, Event::LevelLoaded(_)));
	let front = FRONT.load(Ordering::Relaxed);
	let title = {
		let mut w = o.events.lock().unwrap_or_else(|e| e.into_inner());
		let first = front && !w.scanned_front;
		w.scanned_front = front;
		first
	};
	if level || title {
		eprintln!("{}", super::prescan::run(c, level));
	}
}

/// The state the host keeps for the events (a field of `Omni`).
pub type Shared = Mutex<Watcher>;

