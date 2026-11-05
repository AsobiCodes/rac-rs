//! `RAC_OMNI_TRACE=FROM-TO` (or one number): a debug log of the pad hand-off for the logic ticks FROM..=TO (1-based, the same count
//! as a mod's own `on_pad` calls): the 18-byte report the host gives the mods, what comes back, and, after the field, the pad
//! object the game filled from it (the game's pad words, `Game::pad_words`: PAL addresses; USA says so once). Read-only: it never changes the machine.

use ee_oracle::cpu::Cpu;
use std::sync::OnceLock;

/// `"290-312"` -> (290, 312); `"300"` -> (300, 300); anything else None.
pub fn parse_range(v: &str) -> Option<(u64, u64)> {
	let (a, b) = v.split_once('-').unwrap_or((v, v));
	Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

fn range() -> Option<(u64, u64)> {
	static R: OnceLock<Option<(u64, u64)>> = OnceLock::new();
	*R.get_or_init(|| parse_range(&std::env::var("RAC_OMNI_TRACE").ok()?))
}

fn on(n: u64) -> bool {
	range().is_some_and(|(a, b)| (a..=b).contains(&n))
}

fn hex(r: &[u8]) -> String {
	r.iter().map(|b| format!("{b:02x}")).collect()
}

/// Tick `n`: an event sent to the mods (events.rs).
pub fn event(n: u64, name: &str, a: i64, text: &str) {
	if on(n) {
		eprintln!("omni-trace: tick {n} event {name} {a} {text}");
	}
}

/// Tick `n`: a line of the game pack (R&C1: its memory-card words, rac1/omni_bind/card.rs), made only when the tick is traced.
pub fn line(n: u64, text: impl FnOnce() -> String) {
	if on(n) {
		eprintln!("omni-trace: tick {n} {}", text());
	}
}

/// Tick `n`: the report before and after the mods' `on_pad` callbacks.
pub fn report(n: u64, before: &[u8; 18], after: &[u8; 18]) {
	if on(n) {
		let what = if before == after { "unchanged" } else { "changed by the mods" };
		eprintln!("omni-trace: tick {n} pad report in {} out {} ({what})", hex(before), hex(after));
	}
}

/// Tick `n`, after the EE ran the field: what the game's own pad update made of the report (Cross 0x40, Square 0x80).
pub fn game_pad(n: u64, c: &Cpu, ntsc: bool) {
	if !on(n) {
		return;
	}
	if ntsc {
		if range().is_some_and(|(a, _)| a == n) {
			eprintln!("omni-trace: the game pad object address is known for PAL only; no game-side lines on this disc");
		}
		return;
	}
	let rd = |a: u32| c.mem.ram.get(a as usize..a as usize + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
	let [held, new] = super::game::get().pad_words;
	eprintln!("omni-trace: tick {n} game pad held {:05x} new {:05x}", rd(held), rd(new));
}
