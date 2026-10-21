//! Debug keys for the user's own playtests (2026-10-13), keyboard only. Inert unless the environment variable RAC_DEBUG_KEYS=1 is
//! set (the dev launcher sets it) or the player switched Debug mode on (Pause -> Options -> Controls -> Debug, `devtools::set_menu`,
//! default Off; the release Play.cmd sets neither). Off, nothing is registered,
//! nothing is written and the window title is unchanged, so retail stays bit-exact (the savestate smoke hashes do not move).
//!
//! Keys (in game, retail mode; F7 is the modern camera):
//! - F5  immortality toggle: while on, Ratchet's health is topped up to his max (nanotech capacity) before every field.
//! - F6  infinite ammo toggle: while on, every owned weapon with an ammo capacity is topped up to it before every field.
//! - F8  +10000 bolts (once per press, capped at the HUD counter's own 9999999).
//!
//! The window thread only records the presses (atomics); the retail worker applies them between two fields, just before
//! `Machine::frame` (`pre_field`), never mid-field. The title shows the state ("GOD ON", "AMMO ON") and, for 3 s, the last
//! event ("GOD OFF", "+10000 BOLTS"). Addresses, limits and the retail code places that could do this cleanly:
//! decomp/notes/debug_cheats.md.
//!
//! Tests without a keyboard: RAC_DEBUGKEYS_FEED=`FIELD:LETTERS;...` simulates presses at the worker's field number (g = F5,
//! a = F6, b = F8) plus two test pokes applied AFTER that field's top-up, so the next field's top-up shows: `hN` health = N,
//! `lN` ammo of every owned ammo weapon = N. RAC_DEBUGKEYS_TRACE=1 (implied by the feed) logs each top-up on stderr.
//! A run with the keys on is not reproducible from its pad recording (the pokes are not recorded).

use ee_oracle::Machine;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Hero health (iRam001415f8) and max health = nanotech capacity (iRam0015eda0): boot data, the same on every level.
const HEALTH: u32 = 0x1415f8;
const MAX_HEALTH: u32 = 0x15eda0;
/// Bolt counter (iRam0015ed98). The HUD bolt element (FUN_00228840 slot 2, id 0x754e, 9999999) reads this very word and
/// pops up for 90 ticks when it differs from the value it showed last: no separate shown counter exists.
const BOLTS: u32 = 0x15ed98;
const BOLT_ADD: i32 = 10_000;
const BOLT_CAP: i32 = 9_999_999;
/// Owned bytes (37) and ammo ints (id * 4) of the 37 item ids.
const HAVE: u32 = 0x13d4c0;
const AMMO: u32 = 0x13d428;
const ITEMS: usize = 37;
/// The price/ammo table (stride 0x18, index = item id, u16 max ammo at +0xe, 0 = no ammo concept) lives in the level
/// overlay at a per-level address (0x1c4030 .. 0x1c49b0 over the 19 levels), so it is found by its content.
const ROW: usize = 0x18;
const MAX_AT: usize = 0xe;
const SCAN_LO: usize = 0x15ef00;
const SCAN_HI: usize = 0x300000;
/// How long the last event stays in the title.
const NOTE_FOR: Duration = Duration::from_secs(3);

const P_GOD: u32 = 1;
const P_AMMO: u32 = 2;
/// Presses the window thread has seen since the last field.
static PENDING: AtomicU32 = AtomicU32::new(0);
static BOLT_PRESSES: AtomicU32 = AtomicU32::new(0);

struct St {
	god: bool,
	ammo: bool,
	/// The price/ammo table's RAM address (0 = not located); checked against its content every field.
	table: usize,
	/// The field from which a failed locate may be tried again (the scan is ~1 ms).
	retry: u32,
	note: Option<(String, Instant)>,
}

static ST: Mutex<St> = Mutex::new(St { god: false, ammo: false, table: 0, retry: 0, note: None });

/// The switch: RAC_DEBUG_KEYS=1 or Debug mode On in the menu (`devtools::menu`).
pub fn enabled() -> bool {
	static ON: OnceLock<bool> = OnceLock::new();
	super::devtools::gate(*ON.get_or_init(|| std::env::var("RAC_DEBUG_KEYS").is_ok_and(|v| v == "1")), super::devtools::menu())
}

/// Debug mode was switched Off (`devtools::set_menu`): drops the immortality / infinite ammo switches and every unapplied press, so a
/// later On starts from nothing (the env switch alone keeps working as before: it never goes through here).
pub fn menu_off() {
	PENDING.store(0, Ordering::SeqCst);
	BOLT_PRESSES.store(0, Ordering::SeqCst);
	if let Ok(mut st) = ST.lock() {
		st.god = false;
		st.ammo = false;
		st.note = None;
	}
}

fn trace() -> bool {
	static ON: OnceLock<bool> = OnceLock::new();
	*ON.get_or_init(|| std::env::var_os("RAC_DEBUGKEYS_TRACE").is_some() || std::env::var_os("RAC_DEBUGKEYS_FEED").is_some())
}

/// Window thread: a key went down (not a repeat). Only records it; the worker applies it before the next field.
pub fn ui_key(k: winit::keyboard::KeyCode) {
	use winit::keyboard::KeyCode as K;
	match k {
		K::F5 => {
			PENDING.fetch_or(P_GOD, Ordering::SeqCst);
		}
		K::F6 => {
			PENDING.fetch_or(P_AMMO, Ordering::SeqCst);
		}
		K::F8 => {
			BOLT_PRESSES.fetch_add(1, Ordering::SeqCst);
		}
		_ => {}
	}
}

/// What the window title gets appended: the switches that are on and, for a few seconds, the last event.
pub fn title_suffix() -> String {
	if !enabled() {
		return String::new();
	}
	ST.lock().map(|st| suffix_of(&st)).unwrap_or_default()
}

fn suffix_of(st: &St) -> String {
	let mut s = String::new();
	if st.god {
		s += " - GOD ON";
	}
	if st.ammo {
		s += " - AMMO ON";
	}
	if let Some((t, at)) = &st.note {
		if at.elapsed() < NOTE_FOR {
			s += " - ";
			s += t;
		}
	}
	s
}

/// RAC_DEBUGKEYS_FEED: `FIELD:LETTERS;...`, a letter optionally followed by a number (`80:h1;90:l3;120:gab`).
fn parse_feed(v: &str) -> Vec<(u32, char, u32)> {
	let mut out = Vec::new();
	for part in v.split(';') {
		let Some((f, keys)) = part.split_once(':') else { continue };
		let Ok(f) = f.trim().parse::<u32>() else { continue };
		let mut it = keys.trim().chars().peekable();
		while let Some(c) = it.next() {
			let mut n = String::new();
			while let Some(d) = it.next_if(|d| d.is_ascii_digit()) {
				n.push(d);
			}
			out.push((f, c, n.parse().unwrap_or(0)));
		}
	}
	out
}

fn feed() -> &'static [(u32, char, u32)] {
	static F: OnceLock<Vec<(u32, char, u32)>> = OnceLock::new();
	F.get_or_init(|| parse_feed(&std::env::var("RAC_DEBUGKEYS_FEED").unwrap_or_default()))
}

fn say(st: &mut St, field: u32, title: Option<&str>, log: String) {
	eprintln!("retail: debugkeys: field {field}: {log}");
	if let Some(t) = title {
		st.note = Some((t.to_string(), Instant::now()));
	}
}

/// Worker, before each field (`active`: a level is running, not the front end or the cold boot logos; presses made
/// elsewhere are dropped): applies the presses of the window thread and of the test feed, then the switches that are on.
pub fn pre_field(m: &mut Machine, field: u32, active: bool) {
	if !enabled() {
		return;
	}
	let mut keys = PENDING.swap(0, Ordering::SeqCst);
	let mut bolts = BOLT_PRESSES.swap(0, Ordering::SeqCst);
	let mut pokes: Vec<(char, u32)> = Vec::new();
	for &(_, c, n) in feed().iter().filter(|e| e.0 == field) {
		match c {
			'g' => keys |= P_GOD,
			'a' => keys |= P_AMMO,
			'b' => bolts += 1,
			'h' | 'l' => pokes.push((c, n)),
			_ => {}
		}
	}
	if !active {
		return;
	}
	let Ok(mut st) = ST.lock() else { return };
	let tr = trace();
	let ram = &mut m.cpu.mem.ram[..];
	if keys & P_GOD != 0 {
		st.god = !st.god;
		let on = st.god;
		say(&mut st, field, (!on).then_some("GOD OFF"), format!("immortality {}", if on { "ON" } else { "OFF" }));
	}
	if keys & P_AMMO != 0 {
		st.ammo = !st.ammo;
		let on = st.ammo;
		say(&mut st, field, (!on).then_some("AMMO OFF"), format!("infinite ammo {}", if on { "ON" } else { "OFF" }));
	}
	if bolts > 0 {
		let (from, to) = bolts_add(ram, bolts);
		let note = format!("+{} BOLTS", to - from);
		say(&mut st, field, Some(note.as_str()), format!("bolts {from} -> {to}"));
	}
	if st.god {
		if let Some((from, to)) = god_step(ram) {
			if tr {
				eprintln!("retail: debugkeys: field {field}: health {from} -> {to}");
			}
		}
	}
	if st.ammo {
		let t = ensure_table(ram, &mut st, field, tr);
		if t != 0 {
			let n = ammo_fill(ram, t, None);
			if tr && n > 0 {
				eprintln!("retail: debugkeys: field {field}: ammo topped up for {n} weapon(s)");
			}
		}
	}
	for (c, n) in pokes {
		match c {
			'h' => wr32(ram, HEALTH, n),
			'l' => {
				let t = ensure_table(ram, &mut st, field, tr);
				if t != 0 {
					ammo_fill(ram, t, Some(n));
				}
			}
			_ => {}
		}
		eprintln!("retail: debugkeys: field {field}: test poke {c}{n}");
	}
}

fn rd32(ram: &[u8], a: u32) -> u32 {
	let i = a as usize;
	u32::from_le_bytes(ram[i..i + 4].try_into().unwrap())
}

fn wr32(ram: &mut [u8], a: u32, v: u32) {
	let i = a as usize;
	ram[i..i + 4].copy_from_slice(&v.to_le_bytes());
}

fn h16(ram: &[u8], at: usize) -> u16 {
	u16::from_le_bytes([ram[at], ram[at + 1]])
}

/// Immortality: health back to the max when a hit took some (0 stays 0: a dying hero is not revived; the damage routine
/// takes at most 1 per call, so a full meter cannot reach 0 within one field). Some((from, to)) when it wrote.
fn god_step(ram: &mut [u8]) -> Option<(i32, i32)> {
	let (h, mx) = (rd32(ram, HEALTH) as i32, rd32(ram, MAX_HEALTH) as i32);
	if (1..=16).contains(&mx) && h > 0 && h < mx {
		wr32(ram, HEALTH, mx as u32);
		return Some((h, mx));
	}
	None
}

/// +10000 bolts per press, never past the counter's own cap (a value already past it is left alone).
fn bolts_add(ram: &mut [u8], presses: u32) -> (i32, i32) {
	let from = rd32(ram, BOLTS) as i32;
	let to = if from >= BOLT_CAP { from } else { from.saturating_add(BOLT_ADD.saturating_mul(presses.min(1000) as i32)).min(BOLT_CAP) };
	wr32(ram, BOLTS, to as u32);
	(from, to)
}

/// The price/ammo table is at `base`: item 10 (Bomb Glove) 40 max / 10 start, 15 (Blaster) 200, 16 (Pyrocitor) 240, 17 50;
/// the same rows on every level (a savestate of each of the 19 levels checked).
fn table_ok(ram: &[u8], base: usize) -> bool {
	base != 0
		&& base + ITEMS * ROW <= ram.len()
		&& h16(ram, base + 10 * ROW + MAX_AT) == 40
		&& h16(ram, base + 10 * ROW + 0x12) == 10
		&& h16(ram, base + 15 * ROW + MAX_AT) == 200
		&& h16(ram, base + 16 * ROW + MAX_AT) == 240
		&& h16(ram, base + 17 * ROW + MAX_AT) == 50
		&& h16(ram, base + 8 * ROW + MAX_AT) == 0
}

fn find_table(ram: &[u8]) -> Option<usize> {
	(SCAN_LO..SCAN_HI.min(ram.len())).step_by(4).find(|&b| table_ok(ram, b))
}

/// The table's address (0 when it cannot be found now, e.g. while a level loads; the scan is retried every 50 fields).
fn ensure_table(ram: &[u8], st: &mut St, field: u32, tr: bool) -> usize {
	if !table_ok(ram, st.table) {
		st.table = 0;
		if field >= st.retry {
			st.retry = field.saturating_add(50);
			st.table = find_table(ram).unwrap_or(0);
			if tr && st.table != 0 {
				eprintln!("retail: debugkeys: field {field}: ammo table at {:#x}", st.table);
			}
		}
	}
	st.table
}

/// Every owned item with an ammo capacity gets `to` ammo (None: its capacity). Returns how many words changed.
fn ammo_fill(ram: &mut [u8], table: usize, to: Option<u32>) -> usize {
	let mut changed = 0;
	for id in 0..ITEMS {
		if ram[HAVE as usize + id] == 0 {
			continue;
		}
		let max = h16(ram, table + id * ROW + MAX_AT) as u32;
		if max == 0 {
			continue;
		}
		let want = to.unwrap_or(max);
		let at = AMMO + 4 * id as u32;
		if rd32(ram, at) != want {
			wr32(ram, at, want);
			changed += 1;
		}
	}
	changed
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A RAM image with the price/ammo table at `base` (only the rows the signature and the fill read).
	fn ram_with_table(base: usize) -> Vec<u8> {
		let mut r = vec![0u8; SCAN_HI];
		for (id, max, start) in [(10usize, 40u16, 10u16), (11, 20, 10), (15, 200, 100), (16, 240, 120), (17, 50, 25), (20, 10, 3)] {
			r[base + id * ROW + MAX_AT..][..2].copy_from_slice(&max.to_le_bytes());
			r[base + id * ROW + 0x12..][..2].copy_from_slice(&start.to_le_bytes());
		}
		r
	}

	#[test]
	fn feed_parses_letters_and_numbers() {
		assert_eq!(parse_feed("40:g;60:ab;80:h1;90:l3;x;7"), vec![(40, 'g', 0), (60, 'a', 0), (60, 'b', 0), (80, 'h', 1), (90, 'l', 3)]);
		assert!(parse_feed("").is_empty());
	}

	#[test]
	fn table_is_found_by_content_at_any_level_address() {
		for base in [0x1c4030usize, 0x1c4130, 0x1c49b0] {
			let r = ram_with_table(base);
			assert_eq!(find_table(&r), Some(base));
			assert!(table_ok(&r, base) && !table_ok(&r, base + 4) && !table_ok(&r, 0));
		}
		assert_eq!(find_table(&vec![0u8; SCAN_HI]), None);
	}

	#[test]
	fn god_tops_up_a_hurt_hero_but_never_revives() {
		let mut r = vec![0u8; SCAN_HI];
		wr32(&mut r, MAX_HEALTH, 5);
		for (h, want) in [(3u32, Some((3, 5))), (1, Some((1, 5))), (5, None), (0, None)] {
			wr32(&mut r, HEALTH, h);
			assert_eq!(god_step(&mut r), want, "health {h}");
			assert_eq!(rd32(&r, HEALTH), if want.is_some() { 5 } else { h });
		}
		// A max outside 1..=16 (nothing loaded yet) is left alone.
		wr32(&mut r, MAX_HEALTH, 0);
		wr32(&mut r, HEALTH, 2);
		assert_eq!(god_step(&mut r), None);
		assert_eq!(rd32(&r, HEALTH), 2);
	}

	#[test]
	fn ammo_fills_owned_weapons_with_a_capacity_only() {
		let base = 0x1c4130;
		let mut r = ram_with_table(base);
		// Owned: Bomb Glove 10 (ammo 3), Blaster 15 (ammo 0), Glove of Doom 20 (full already), wrench 8 (no capacity), Devastator 11 NOT owned.
		for id in [10usize, 15, 20, 8] {
			r[HAVE as usize + id] = 1;
		}
		for (id, a) in [(10u32, 3u32), (15, 0), (20, 10), (8, 7), (11, 5)] {
			wr32(&mut r, AMMO + 4 * id, a);
		}
		assert_eq!(ammo_fill(&mut r, base, None), 2);
		let a = |r: &Vec<u8>, id: u32| rd32(r, AMMO + 4 * id);
		assert_eq!((a(&r, 10), a(&r, 15), a(&r, 20), a(&r, 8), a(&r, 11)), (40, 200, 10, 7, 5));
		assert_eq!(ammo_fill(&mut r, base, None), 0, "already full");
		// The test poke: every owned ammo weapon to N (the wrench has no capacity and stays).
		assert_eq!(ammo_fill(&mut r, base, Some(1)), 3);
		assert_eq!((a(&r, 10), a(&r, 15), a(&r, 20), a(&r, 8), a(&r, 11)), (1, 1, 1, 7, 5));
	}

	#[test]
	fn bolts_add_ten_thousand_per_press_up_to_the_cap() {
		let mut r = vec![0u8; SCAN_HI];
		wr32(&mut r, BOLTS, 1117);
		assert_eq!(bolts_add(&mut r, 1), (1117, 11117));
		assert_eq!(bolts_add(&mut r, 2), (11117, 31117));
		wr32(&mut r, BOLTS, 9_995_000);
		assert_eq!(bolts_add(&mut r, 1), (9_995_000, 9_999_999));
		assert_eq!(bolts_add(&mut r, 1), (9_999_999, 9_999_999));
		wr32(&mut r, BOLTS, 12_000_000);
		assert_eq!(bolts_add(&mut r, 1).1, 12_000_000, "past the cap stays");
	}

	/// Debug mode Off (`devtools::set_menu`) drops the switches and the unapplied presses; the gate is env OR menu.
	#[test]
	fn menu_off_clears_the_switches_and_the_gate_is_env_or_menu() {
		assert!(!super::super::devtools::gate(false, false));
		assert!(super::super::devtools::gate(true, false) && super::super::devtools::gate(false, true) && super::super::devtools::gate(true, true));
		{
			let mut st = ST.lock().unwrap();
			st.god = true;
			st.ammo = true;
			st.note = Some(("GOD OFF".into(), Instant::now()));
		}
		PENDING.store(P_GOD | P_AMMO, Ordering::SeqCst);
		BOLT_PRESSES.store(3, Ordering::SeqCst);
		menu_off();
		let st = ST.lock().unwrap();
		assert!(!st.god && !st.ammo && st.note.is_none());
		assert_eq!((PENDING.load(Ordering::SeqCst), BOLT_PRESSES.load(Ordering::SeqCst)), (0, 0));
	}

	#[test]
	fn title_shows_the_switches_and_the_fresh_event() {
		let mut st = St { god: false, ammo: false, table: 0, retry: 0, note: None };
		assert_eq!(suffix_of(&st), "");
		st.god = true;
		st.ammo = true;
		st.note = Some(("+10000 BOLTS".into(), Instant::now()));
		assert_eq!(suffix_of(&st), " - GOD ON - AMMO ON - +10000 BOLTS");
		st.note = Some(("GOD OFF".into(), Instant::now() - Duration::from_secs(4)));
		assert_eq!(suffix_of(&st), " - GOD ON - AMMO ON", "an old event is gone");
	}
}
