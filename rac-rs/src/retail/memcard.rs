//! Persistent player memory card for retail mode.
//!
//! The oracle's memory card server (oracle/src/mc.rs) reads a base image once (ORACLE_MCD, else
//! <dev>/oracle/memcards/Mcd001.ps2, which stays read-only) and keeps every write in memory only. This module
//! makes a save survive the process: after a field in which the stored card contents changed (`Mc::gen` moved and `content_hash` differs from the
//! last contents loaded or written) and no card file is open, it writes the oracle's own card dump (`Mc::save`) to `<player dir>/card.bin`; at start `restore` loads that
//! dump back into the machine before the first card call. The player dir is `RAC_PLAYER_DIR`, else
//! `<dev>/rac-rs/player`. Never PCSX2's folder; the base image is never written. A cold boot (`--retail-cold`) has no base image: it starts
//! from a freshly formatted blank card (`Mc::base_cards`) unless the player dir holds a card.bin.
//!
//! Wiring (retail.rs, owner applies): `memcard::restore(&mut m)` after `Machine::load` in `Retail::load`; in the
//! worker loop `let mut card = memcard::Saver::new(&m);` before the loop and `card.after_field(&m);` after
//! `m.frame()`; `card.flush(&m)` when the loop ends.
use ee_oracle::snapfile::{Rd, Wr};
use ee_oracle::Machine;
use std::path::PathBuf;

pub fn player_dir() -> PathBuf {
	std::env::var_os("RAC_PLAYER_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/player")))
}

fn card_file() -> PathBuf {
	player_dir().join("card.bin")
}

/// Loads the player's card dump into the machine (true) when there is one. RAC_PLAYER_CARD=0 ignores it.
pub fn restore(m: &mut Machine) -> bool {
	if std::env::var("RAC_PLAYER_CARD").is_ok_and(|v| v == "0") {
		return false;
	}
	match std::fs::read(card_file()) {
		Ok(b) => restore_bytes(m, &b),
		Err(_) => false,
	}
}

pub fn restore_bytes(m: &mut Machine, b: &[u8]) -> bool {
	if b.len() < 8 {
		return false;
	}
	// A dump the loader cannot read must not take the game down: parse inside catch_unwind on a scratch copy.
	let mut mc = m.cpu.mc.clone();
	let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
		mc.load_dyn(&mut Rd::new(b));
		mc
	}));
	let Ok(mut mc) = ok else {
		eprintln!("memcard: {} is unreadable, starting from the base image", card_file().display());
		return false;
	};
	// No call was in flight when it was written (`after_field` writes only with every file closed).
	mc.fds = (0..3).map(|_| None).collect();
	mc.list.clear();
	mc.list_pos = 0;
	mc.bump();
	m.cpu.mc = mc;
	eprintln!("memcard: player card restored from {}", card_file().display());
	true
}

/// The card as the oracle serialises it.
pub fn dump(m: &Machine) -> Vec<u8> {
	let mut w = Wr::new();
	m.cpu.mc.save(&mut w);
	w.0
}

/// Writes the dump atomically (temp file + rename), keeping the previous one as card.prev.bin and a timestamped copy of this
/// one in `<player dir>/history/` (the newest `HISTORY` stay), so no save is ever lost for good.
pub fn write_dump(bytes: &[u8]) -> std::io::Result<()> {
	let dir = player_dir();
	std::fs::create_dir_all(&dir)?;
	let (f, tmp, prev) = (card_file(), dir.join("card.tmp"), dir.join("card.prev.bin"));
	std::fs::write(&tmp, bytes)?;
	if f.exists() {
		let _ = std::fs::copy(&f, &prev);
	}
	std::fs::rename(&tmp, &f)?;
	if let Err(e) = keep_history(&dir.join("history"), bytes, std::time::SystemTime::now()) {
		eprintln!("memcard: cannot keep a history copy: {e}");
	}
	Ok(())
}

/// How many timestamped card copies `history/` keeps.
pub const HISTORY: usize = 50;

/// UTC date and time of `t` as `YYYYMMDD-HHMMSS` and the milliseconds (Howard Hinnant's civil-from-days).
fn stamp(t: std::time::SystemTime) -> String {
	let d = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
	let (days, secs) = ((d.as_secs() / 86400) as i64, d.as_secs() % 86400);
	let z = days + 719468;
	let era = z.div_euclid(146097);
	let doe = z.rem_euclid(146097);
	let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
	let (y, doy) = (yoe + era * 400, doe - (365 * yoe + yoe / 4 - yoe / 100));
	let mp = (5 * doy + 2) / 153;
	let (dd, mm) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
	let y = if mm <= 2 { y + 1 } else { y };
	format!("{y:04}{mm:02}{dd:02}-{:02}{:02}{:02}-{:03}", secs / 3600, secs % 3600 / 60, secs % 60, d.subsec_millis())
}

/// Writes `history/card_<stamp>.bin` and deletes the oldest copies beyond `HISTORY` (the stamp sorts by time).
fn keep_history(dir: &std::path::Path, bytes: &[u8], now: std::time::SystemTime) -> std::io::Result<()> {
	std::fs::create_dir_all(dir)?;
	std::fs::write(dir.join(format!("card_{}.bin", stamp(now))), bytes)?;
	let mut names: Vec<String> = std::fs::read_dir(dir)?.filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|n| n.starts_with("card_") && n.ends_with(".bin")).collect();
	names.sort();
	let extra = names.len().saturating_sub(HISTORY);
	for n in &names[..extra] {
		let _ = std::fs::remove_file(dir.join(n));
	}
	Ok(())
}

/// Hash of what is stored on the cards (the card trees only: open fds, a GetDir listing and `gen` are left out).
/// `Mc::gen` moves at every mcserv call but GetInfo/Flush, also at the base image's first load and at every Open,
/// Read, GetDir and Close of a card scan, so it alone cannot tell a real save.
pub fn content_hash(mc: &ee_oracle::mc::Mc) -> u64 {
	use std::hash::Hasher;
	let c = ee_oracle::mc::Mc { loaded: true, cards: mc.cards.clone(), ..Default::default() };
	let mut w = Wr::new();
	c.save(&mut w);
	let mut h = std::collections::hash_map::DefaultHasher::new();
	h.write(&w.0);
	h.finish()
}

/// The contents of the cards as the oracle would load them as the base cards (`Mc::base_cards`: ORACLE_MCD / Mcd00N.ps2, or the
/// blank card of a cold boot).
fn base_hash(mc: &ee_oracle::mc::Mc) -> Option<u64> {
	let mut base = ee_oracle::mc::Mc::default();
	base.cards = mc.base_cards().ok()?;
	Some(content_hash(&base))
}

/// Watches the card between fields and writes card.bin only when the stored contents differ from the last ones
/// written or loaded (the base image, or the restored player card): a game save, a delete or a format.
pub struct Saver {
	seen: u64,
	/// Content hash of the card the game started from, then of the last card.bin written. None: the base image
	/// could not be read here; the first contents seen become the reference.
	base: Option<u64>,
	/// The card as it was when the title boot began (title_boot.rs): while this is Some nothing is written to card.bin.
	held: Option<ee_oracle::mc::Mc>,
}

impl Saver {
	pub fn new(m: &Machine) -> Saver {
		let mc = &m.cpu.mc;
		let base = if mc.loaded { Some(content_hash(mc)) } else { base_hash(mc) };
		Saver { seen: mc.r#gen, base, held: None }
	}

	/// Keeps the card as it is now and writes nothing to card.bin until `release`.
	pub fn hold(&mut self, m: &Machine) {
		self.held = Some(m.cpu.mc.clone());
	}

	pub fn is_held(&self) -> bool {
		self.held.is_some()
	}

	/// Ends the hold. `rewind`: the card in the machine goes back to what it held at `hold` (what the game wrote meanwhile, an
	/// autosave of the fresh game into the player's active slot, is undone, as it was never persisted); else it stays as it is
	/// and the next change is written as usual.
	pub fn release(&mut self, m: &mut Machine, rewind: bool) {
		let Some(mut mc) = self.held.take() else { return };
		if rewind {
			let wrote = content_hash(&mc) != content_hash(&m.cpu.mc);
			mc.fds = (0..3).map(|_| None).collect();
			mc.list.clear();
			mc.list_pos = 0;
			mc.bump();
			m.cpu.mc = mc;
			eprintln!("memcard: card put back to its contents at start (the game wrote to it meanwhile: {wrote})");
		}
		self.seen = m.cpu.mc.r#gen;
	}

	/// Call after every `Machine::frame`: cheap (one integer compare) unless mcserv was called.
	pub fn after_field(&mut self, m: &Machine) {
		let mc = &m.cpu.mc;
		if self.held.is_some() {
			self.seen = mc.r#gen;
			return;
		}
		if mc.r#gen == self.seen || !mc.loaded || mc.fds.iter().any(|f| f.is_some()) {
			return;
		}
		self.seen = mc.r#gen;
		let h = content_hash(mc);
		match self.base {
			Some(b) if b == h => return,
			None => {
				self.base = Some(h);
				return;
			}
			_ => {}
		}
		match write_dump(&dump(m)) {
			Ok(()) => {
				self.base = Some(h);
				eprintln!("memcard: saved {}", card_file().display())
			}
			Err(e) => eprintln!("memcard: cannot write {}: {e}", card_file().display()),
		}
	}

	/// At exit: writes a pending change even if a file was still open (the game closes it before the card idles, so
	/// this only matters when the window is closed mid-save).
	pub fn flush(&mut self, m: &Machine) {
		if self.held.is_some() {
			return;
		}
		if m.cpu.mc.r#gen != self.seen && m.cpu.mc.loaded && m.cpu.mc.fds.iter().all(|f| f.is_none()) {
			self.after_field(m);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The card dump round-trips: a file created on card 0 comes back after `restore_bytes` on a fresh machine.
	#[test]
	fn card_dump_round_trips() {
		let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/fg1/4_0159");
		let mut a = Machine::load(dir);
		let img = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/memcards/Mcd001.ps2")).expect("base card image");
		let mut card = ee_oracle::mc::Card::parse(&img).expect("parse");
		let n = card.root.kids.len();
		card.root.kids.push(ee_oracle::mc::Ent { name: "TESTSAVE".into(), mode: 0x8497, len: 3, created: [0; 8], modified: [0; 8], attr: 0, data: vec![1, 2, 3], kids: Vec::new() });
		a.cpu.mc.cards[0] = Some(card);
		a.cpu.mc.loaded = true;
		a.cpu.mc.fds = (0..3).map(|_| None).collect();
		a.cpu.mc.bump();
		let bytes = dump(&a);
		let mut b = Machine::load(dir);
		assert!(restore_bytes(&mut b, &bytes));
		let root = &b.cpu.mc.cards[0].as_ref().unwrap().root;
		assert_eq!(root.kids.len(), n + 1);
		assert_eq!(root.kids.last().unwrap().data, vec![1, 2, 3]);
		assert!(b.cpu.mc.loaded);
		// Saver: a call that changes no stored data (gen moved only: card scan, base image load) writes nothing;
		// a stored change with all files closed does.
		unsafe { std::env::set_var("RAC_PLAYER_DIR", concat!(env!("CARGO_MANIFEST_DIR"), "/target/player_test")) };
		let _ = std::fs::remove_file(card_file());
		let mut s = Saver::new(&b);
		b.cpu.mc.bump();
		s.after_field(&b);
		assert!(!card_file().exists(), "gen-only change wrote the card");
		let root = &mut b.cpu.mc.cards[0].as_mut().unwrap().root;
		root.kids.last_mut().unwrap().data.push(4);
		b.cpu.mc.bump();
		s.after_field(&b);
		assert!(card_file().exists());
		assert_eq!(std::fs::read(card_file()).unwrap(), dump(&b));
		// The same contents again: no second write.
		let t = std::fs::metadata(card_file()).unwrap().modified().unwrap();
		b.cpu.mc.bump();
		s.after_field(&b);
		assert_eq!(std::fs::metadata(card_file()).unwrap().modified().unwrap(), t);
		// A fresh machine on the base image: the first load (gen moves) is not a save.
		let mut c = Machine::load(dir);
		c.cpu.mc = Default::default(); // the card not read yet (a state may carry a loaded card)
		let mut s = Saver::new(&c);
		let _ = std::fs::remove_file(card_file());
		for p in 0..2 {
			c.cpu.mc.cards[p] = ee_oracle::mc::image_path(p).map(|f| ee_oracle::mc::Card::parse(&std::fs::read(f).unwrap()).unwrap());
		}
		c.cpu.mc.loaded = true;
		c.cpu.mc.fds = (0..3).map(|_| None).collect();
		c.cpu.mc.bump();
		s.after_field(&c);
		assert!(!card_file().exists(), "base image load wrote the card");
	}
}
