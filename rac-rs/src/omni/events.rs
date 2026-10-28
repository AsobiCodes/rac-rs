//! Game events for the mods (ABI 1.2 `omni_event`): `level_loaded`, `level_unloading`, `mode` and `pause_page`, and the mode / page names.
//!
//! Read RAM only, never write. Once per field (`omni::tick`, before the mods' `on_frame`) the words below are read, compared with what the
//! mods were last told, and one event is sent per change. Nothing runs without a loaded mod or when the DLL has no `omni_event`.
//!
//! Words (PAL addresses; the 0x15f5xx ones go through `disc::va` so the USA disc works too):
//! - level word 0x15ed84: 0..=18 while a level is loaded; anything else is no level.
//! - level-loop exit 0x15f550: nonzero from the moment the level loop is told to end (travel, quit to title, load game) until the next level
//!   starts. Seen as "unloading"; the level word changing while a level is loaded counts as unloading too.
//! - game mode 0x15f5a4: 0 gameplay, 1 movie, 2 in-engine cutscene, 3 page menu (pause), 4 dialog box, 5 vendor, 6 ship, 7 sprite screen.
//! - title screen: the front-end flag of `worker/field_front.rs`, handed over by `note_front` (one field old).
//!
//! Mode names for `omni.mode()`: `title` (front end), `loading` (exit word set, or no level word), `gameplay`, `movie`, `cutscene`,
//! `pause` (mode 3), `dialog` (4), `vendor`, `ship`, `screen` (7, full-screen sprite screens). [I]
//! Pause pages are the page ids of the page-menu engine (`mouse_menu`, id at page+0x3c): see `page_name`.
//!
//! Not verified on a run yet: that the exit word is seen set for at least one field around every level change (the level word check
//! covers a change to another level; a reload of the same level through the title is covered by the title flag).

use super::{lock, native_host, Omni};
use crate::retail::disc::va;
use crate::retail::mouse_menu;
use ee_oracle::cpu::Cpu;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// The title / front end is up (set by `note_front` from field_front.rs every field).
static FRONT: AtomicBool = AtomicBool::new(false);

/// `retail/worker/field_front.rs` once per field: whether the front end (title screen and its menus) is running.
pub fn note_front(front: bool) {
	FRONT.store(front, Ordering::Relaxed);
}

const LEVEL_WORD: u32 = super::native_host::LEVEL_WORD as u32;
const EXIT_WORD: u32 = 0x15_F550;
const MODE_WORD: u32 = 0x15_F5A4;
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
	match s.mode {
		0 => "gameplay",
		1 => "movie",
		2 => "cutscene",
		3 => "pause",
		4 => "dialog",
		5 => "vendor",
		7 => "screen",
		6 => "ship",
		_ => "unknown",
	}
}

/// The name of pause page `id`: the API's names where the id maps, else `page_<hex>`.
pub fn page_name(id: u32) -> String {
	match id {
		2 => "root".into(),
		3 => "weapons".into(),
		4 => "gadgets".into(),
		5 => "quick_select".into(),
		6 => "items".into(),
		7 => "help".into(),
		8 => "options".into(),
		9 => "goodies".into(),
		0xb => "map".into(),
		0xd => "quit".into(),
		0x15 => "goals".into(),
		0x17 => "moves".into(),
		0x1e => "save".into(),
		0x1f => "load".into(),
		0x24 => "skill_points".into(),
		_ => format!("page_{id:x}"),
	}
}

/// What the mods were told so far.
#[derive(Debug, Default)]
pub struct Watcher {
	level: Option<i32>,
	mode: Option<&'static str>,
	page: String,
	/// The memory-card state machine (card.rs).
	card: super::card::CardWatcher,
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
	let level_word = rd(ram, LEVEL_WORD) as i32;
	let mode = rd(ram, va(MODE_WORD));
	let page_id = if mode == mouse_menu::MODE && (0..=18).contains(&level_word) {
		mouse_menu::read(&|a| rd(ram, a), level_word as u32, mode).map(|m| m.id)
	} else {
		None
	};
	Snapshot { front, level_word, exit: rd(ram, va(EXIT_WORD)) != 0, mode, page_id }
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
		let words = super::card::read(c);
		if w.card.changed(&words) {
			super::trace::card(tick, &words);
		}
		let card: Vec<Event> = w
			.card
			.step(&words)
			.into_iter()
			.map(|e| match e {
				super::card::CardEvent::Saving(s) => Event::Saving(s),
				super::card::CardEvent::SaveLoaded(s) => Event::SaveLoaded(s),
			})
			.collect();
		let loaded = card.iter().any(|e| matches!(e, Event::SaveLoaded(_)));
		let level_word = rd(&c.mem.ram, LEVEL_WORD) as i32;
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

#[cfg(test)]
mod tests {
	use super::*;

	fn snap(front: bool, level: i32, exit: bool, mode: u32, page: Option<u32>) -> Snapshot {
		Snapshot { front, level_word: level, exit, mode, page_id: page }
	}

	#[test]
	fn mode_names_follow_the_game_mode_word_and_the_front_end_flag() {
		let n = |m| mode_name(&snap(false, 1, false, m, None));
		assert_eq!((n(0), n(1), n(2), n(3), n(5), n(6)), ("gameplay", "movie", "cutscene", "pause", "vendor", "ship"));
		assert_eq!((n(4), n(7), n(9)), ("dialog", "screen", "unknown"));
		assert_eq!(mode_name(&snap(true, 1, false, 0, None)), "title");
		assert_eq!(mode_name(&snap(false, 1, true, 0, None)), "loading");
		assert_eq!(mode_name(&snap(false, 99, false, 0, None)), "loading");
		assert_eq!(mode_name(&snap(false, -1, false, 3, None)), "loading");
	}

	#[test]
	fn page_ids_map_to_the_api_names_else_page_hex() {
		assert_eq!(page_name(3), "weapons");
		assert_eq!(page_name(0x24), "skill_points");
		assert_eq!(page_name(0x1e), "save");
		assert_eq!(page_name(0x1f), "load");
		assert_eq!(page_name(0xd), "quit");
		assert_eq!(page_name(0x20), "page_20");
		assert_eq!(page_name(0x1d), "page_1d");
	}

	#[test]
	fn events_come_only_on_change_in_a_fixed_order() {
		let mut w = Watcher::default();
		// Title first: only the mode.
		assert_eq!(w.step(&snap(true, 0, false, 0, None)), vec![Event::Mode("title")]);
		assert!(w.step(&snap(true, 0, false, 0, None)).is_empty());
		// Into level 3: loaded, then mode.
		assert_eq!(w.step(&snap(false, 3, false, 0, None)), vec![Event::LevelLoaded(3), Event::Mode("gameplay")]);
		assert!(w.step(&snap(false, 3, false, 0, None)).is_empty());
		// Pause, weapons page, other page, close.
		assert_eq!(w.step(&snap(false, 3, false, 3, Some(2))), vec![Event::Mode("pause"), Event::PausePage("root".into())]);
		assert_eq!(w.step(&snap(false, 3, false, 3, Some(3))), vec![Event::PausePage("weapons".into())]);
		assert!(w.step(&snap(false, 3, false, 3, Some(3))).is_empty());
		assert_eq!(w.step(&snap(false, 3, false, 0, None)), vec![Event::Mode("gameplay"), Event::PausePage(String::new())]);
		// The level loop ends: unloading, loading mode.
		assert_eq!(w.step(&snap(false, 3, true, 0, None)), vec![Event::LevelUnloading(3), Event::Mode("loading")]);
		assert!(w.step(&snap(false, 3, true, 0, None)).is_empty());
		// Straight to another level without a field of exit: unloading then loaded.
		assert_eq!(w.step(&snap(false, 5, false, 0, None)), vec![Event::LevelLoaded(5), Event::Mode("gameplay")]);
		assert_eq!(w.step(&snap(false, 6, false, 0, None)), vec![Event::LevelUnloading(5), Event::LevelLoaded(6)]);
		// Quit to the title.
		assert_eq!(w.step(&snap(true, 6, false, 0, None)), vec![Event::LevelUnloading(6), Event::Mode("title")]);
	}

	#[test]
	fn a_page_id_outside_pause_is_not_reported() {
		let mut w = Watcher::default();
		let first = w.step(&snap(false, 1, false, 0, Some(2)));
		assert_eq!(first, vec![Event::LevelLoaded(1), Event::Mode("gameplay")]);
	}

	#[test]
	fn event_parts_are_the_omni_event_arguments() {
		assert_eq!(Event::LevelLoaded(4).parts(), ("level_loaded", 4, ""));
		assert_eq!(Event::LevelUnloading(4).parts(), ("level_unloading", 4, ""));
		assert_eq!(Event::Mode("pause").parts(), ("mode", 0, "pause"));
		assert_eq!(Event::PausePage("map".into()).parts(), ("pause_page", 0, "map"));
		assert_eq!(Event::NewGame(false).parts(), ("new_game", 0, ""));
		assert_eq!(Event::NewGame(true).parts(), ("new_game", 1, ""));
		assert_eq!(Event::Saving(2).parts(), ("saving", 2, ""));
		assert_eq!(Event::SaveLoaded(4).parts(), ("save_loaded", 4, ""));
	}

	fn put(ram: &mut [u8], a: u32, v: u32) {
		ram[a as usize..a as usize + 4].copy_from_slice(&v.to_le_bytes());
	}

	#[test]
	fn the_snapshot_reads_level_exit_mode_and_the_open_page_from_ram() {
		let mut ram = vec![0u8; 0x40_0000];
		put(&mut ram, LEVEL_WORD, 0);
		put(&mut ram, 0x15_F5A4, 3);
		// Level 0's current-page pointer (mouse_menu PAGE_PTR[0] = 0x1b9cf4) -> a page struct whose id word (+0x3c) is 6 (items).
		put(&mut ram, 0x1b_9cf4, 0x30_0000);
		put(&mut ram, 0x30_003c, 6);
		let s = snapshot(&ram, false);
		assert_eq!((s.level_word, s.exit, s.mode, s.page_id, s.front), (0, false, 3, Some(6), false));
		assert_eq!(mode_name(&s), "pause");
		assert_eq!(Watcher::default().step(&s), vec![Event::LevelLoaded(0), Event::Mode("pause"), Event::PausePage("items".into())]);
		// Exit word set: the level is leaving; outside mode 3 there is no page.
		put(&mut ram, 0x15_F550, 1);
		put(&mut ram, 0x15_F5A4, 0);
		let s = snapshot(&ram, false);
		assert_eq!((s.exit, s.page_id, level_of(&s)), (true, None, -1));
	}

	#[test]
	fn events_need_a_loaded_mod_and_a_dll_with_omni_event() {
		assert!(should_send(1, true));
		assert!(!should_send(0, true));
		assert!(!should_send(2, false));
		assert!(!should_send(0, false));
	}

	#[test]
	fn the_front_flag_is_shared_through_note_front() {
		note_front(true);
		assert!(FRONT.load(Ordering::Relaxed));
		note_front(false);
		assert!(!FRONT.load(Ordering::Relaxed));
	}
}
