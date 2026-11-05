//! The unlock state and `achievements.json` (format version 1; the launcher reads the same file: scratch_launcher achievements.rs).
//!
//! ```text
//! {"version":1,"game":"rac1","updated":"2026-11-02T12:00:00Z","achievements":[
//!  {"id":"skillful","name":"Skillful","description":"...","grade":"gold","hidden":false,"unlocked":false,
//!   "unlocked_at":null,"progress":12,"progress_max":30}, ...]}
//! ```
//! Written by `writer.rs` to `achievements.json.<pid>.tmp`, fsynced, and renamed over the file (atomic). A missing file is an empty
//! start; a broken one is kept as `achievements.json.broken` (later ones `.broken.2`, ...; never overwritten) and the list starts empty,
//! then the file is replaced on the next write. A file that exists but cannot be read (locked, permission) is retried a few times and
//! then never written over this session (`Loaded::Unreadable`). Unknown ids in the file are dropped, new ids start locked.
use super::{AchPack, Def, Reading};
use crate::omni::json::{self, Json};
use std::path::Path;

/// A condition must hold this many fields in a row before it unlocks (a load or level change shows half-built RAM for a moment).
pub const DEBOUNCE: u16 = 20;

/// Reading an existing file that fails (another program holds it, permission): tries, and the pause between them.
const READ_TRIES: u32 = 4;
const READ_PAUSE: std::time::Duration = std::time::Duration::from_millis(60);

/// What `Store::load` found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loaded {
	/// No file (NotFound): the first write starts a new one.
	New,
	/// The file was read (a broken one was kept aside and the list is empty).
	Read,
	/// The file exists but could not be read after the tries: never write achievements.json over it.
	Unreadable,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
	pub unlocked: bool,
	/// Unix seconds (UTC) of the unlock; 0 = unknown.
	pub at: u64,
	pub progress: u32,
	/// Counters with a `Reading::epoch`: the total of the finished cycles, and the newest cycle seen (both 0 for plain counters).
	pub carry: u32,
	pub epoch: u32,
}

pub struct Store {
	pub game: &'static str,
	pub defs: &'static [Def],
	pub entries: Vec<Entry>,
	streak: Vec<u16>,
	/// Counters: the lowest value of the run of fields that all read above the stored progress, and the run's length (`climb`).
	pend: Vec<u32>,
	pstreak: Vec<u16>,
}

/// A counter's progress rises only after a run of `DEBOUNCE` fields that all read above the stored value; it rises to the LOWEST value of
/// the run (m4: one field of transient RAM, the very case `DEBOUNCE` is for, must not be stored for good, nor carried into the next
/// cycle). Returns true when `progress` rose. A reading that is not above `progress` ends the run.
fn climb(progress: &mut u32, pend: &mut u32, streak: &mut u16, p: u32) -> bool {
	if p <= *progress {
		*streak = 0;
		return false;
	}
	*pend = if *streak == 0 { p } else { (*pend).min(p) };
	*streak = streak.saturating_add(1);
	if *streak < DEBOUNCE {
		return false;
	}
	(*progress, *streak) = (*pend, 0);
	true
}

impl Store {
	pub fn new(p: &AchPack) -> Store {
		let n = p.defs.len();
		Store { game: p.game, defs: p.defs, entries: vec![Entry::default(); n], streak: vec![0; n], pend: vec![0; n], pstreak: vec![0; n] }
	}

	/// (unlocked, all) entries.
	pub fn counts(&self) -> (usize, usize) {
		(self.entries.iter().filter(|e| e.unlocked).count(), self.entries.len())
	}

	/// One field's reading. Returns the newly unlocked indices (platinum last) and whether anything changed (to save).
	pub fn apply(&mut self, r: &[Reading], now: u64) -> (Vec<usize>, bool) {
		let (mut new, mut changed) = (Vec::new(), false);
		for (i, d) in self.defs.iter().enumerate() {
			if d.platinum {
				continue;
			}
			let rd = r.get(i).copied().unwrap_or_default();
			let e = &mut self.entries[i];
			let mut met = rd.met;
			if d.max > 0 {
				let mut total = rd.progress;
				if let Some(ep) = rd.epoch {
					if ep > e.epoch {
						(e.carry, e.epoch, changed) = (e.progress, ep, true);
						self.pstreak[i] = 0;
					}
					// An older cycle (an earlier save) adds nothing; the stored total stays.
					total = if ep == e.epoch { e.carry.saturating_add(rd.progress) } else { 0 };
					met |= total >= d.max;
				}
				changed |= climb(&mut e.progress, &mut self.pend[i], &mut self.pstreak[i], total.min(d.max));
			}
			if e.unlocked {
				continue;
			}
			self.streak[i] = if met { self.streak[i].saturating_add(1) } else { 0 };
			if self.streak[i] >= DEBOUNCE {
				e.unlocked = true;
				e.at = now;
				if d.max > 0 {
					e.progress = d.max;
				}
				new.push(i);
				changed = true;
			}
		}
		let others = self.defs.iter().zip(&self.entries).filter(|(d, _)| !d.platinum).all(|(_, e)| e.unlocked);
		for (i, d) in self.defs.iter().enumerate() {
			if d.platinum && others && !self.entries[i].unlocked {
				self.entries[i] = Entry { unlocked: true, at: now, progress: d.max, ..Entry::default() };
				new.push(i);
				changed = true;
			}
		}
		(new, changed)
	}

	pub fn to_json(&self, now: u64) -> String {
		let mut s = format!("{{\n \"version\": 1,\n \"game\": {},\n \"updated\": \"{}\",\n \"achievements\": [\n", quote(self.game), unix_to_iso(now));
		for (i, (d, e)) in self.defs.iter().zip(&self.entries).enumerate() {
			let at = if e.unlocked && e.at > 0 { format!("\"{}\"", unix_to_iso(e.at)) } else { "null".to_string() };
			let (p, pm) = if d.max > 0 { (e.progress.to_string(), d.max.to_string()) } else { ("null".into(), "null".into()) };
			let cycles = if e.epoch > 0 || e.carry > 0 { format!(", \"carry\": {}, \"epoch\": {}", e.carry, e.epoch) } else { String::new() };
			s += &format!(
				"  {{\"id\": {}, \"name\": {}, \"description\": {}, \"grade\": \"{}\", \"hidden\": {}, \"unlocked\": {}, \"unlocked_at\": {}, \"progress\": {}, \"progress_max\": {}{}}}{}\n",
				quote(d.id),
				quote(d.name),
				quote(d.desc),
				d.grade.key(),
				d.hidden,
				e.unlocked,
				at,
				p,
				pm,
				cycles,
				if i + 1 < self.defs.len() { "," } else { "" }
			);
		}
		s + " ]\n}\n"
	}

	/// Merges a file's text into the entries; false when the text is not a version-1 file of this game (entries untouched).
	pub fn load_text(&mut self, text: &str) -> bool {
		let Ok(j) = json::parse(text) else { return false };
		if j.get("version").and_then(Json::as_num) != Some(1.0) || j.get("game").and_then(Json::str).is_some_and(|g| g != self.game) {
			return false;
		}
		let Some(Json::Arr(list)) = j.get("achievements") else { return false };
		for a in list {
			let Some(i) = a.get("id").and_then(Json::str).and_then(|id| self.defs.iter().position(|d| d.id == id)) else { continue };
			let unlocked = matches!(a.get("unlocked"), Some(Json::Bool(true)));
			let at = a.get("unlocked_at").and_then(Json::str).and_then(iso_to_unix).unwrap_or(0);
			let p = a.get("progress").and_then(Json::as_num).map_or(0, |v| v.clamp(0.0, 4.0e9) as u32);
			let max = self.defs[i].max;
			let n = |k: &str| a.get(k).and_then(Json::as_num).map_or(0, |v| v.clamp(0.0, 4.0e9) as u32);
			let (carry, epoch) = if max > 0 { (n("carry").min(max), n("epoch")) } else { (0, 0) };
			self.entries[i] = Entry { unlocked, at: if unlocked { at } else { 0 }, progress: if max > 0 { p.min(max) } else { 0 }, carry, epoch };
		}
		true
	}

	/// Reads `path`: missing = `Loaded::New` (a new file is started), broken = copied aside (`keep_broken`: the game then starts from an
	/// empty list and the next write replaces the file, the damaged copy stays). A file that is there but cannot be read (sharing violation,
	/// permission, ...) is tried `READ_TRIES` times and then reported as `Loaded::Unreadable`: the caller must NOT write over it.
	pub fn load(&mut self, path: &Path) -> Loaded {
		self.load_via(path, &mut |p| std::fs::read(p), READ_TRIES, READ_PAUSE)
	}

	/// `load` for a session that writes nothing (a cheated one, review R22 m1): a broken file is not copied aside (the list starts
	/// empty and the file stays as it is).
	pub fn peek(&mut self, path: &Path) -> Loaded {
		self.read_in(path, &mut |p| std::fs::read(p), READ_TRIES, READ_PAUSE, false)
	}

	/// `load` with the read as a parameter (the test seam for an unreadable file) and the number of tries and the pause between them.
	pub fn load_via(&mut self, path: &Path, read: &mut dyn FnMut(&Path) -> std::io::Result<Vec<u8>>, tries: u32, pause: std::time::Duration) -> Loaded {
		self.read_in(path, read, tries, pause, true)
	}

	/// The read of `load_via` and `peek`; `keep` = a broken file is copied aside (`keep_broken`).
	fn read_in(&mut self, path: &Path, read: &mut dyn FnMut(&Path) -> std::io::Result<Vec<u8>>, tries: u32, pause: std::time::Duration, keep: bool) -> Loaded {
		let mut last = None;
		for n in 0..tries.max(1) {
			match read(path) {
				Ok(bytes) => {
					let text = String::from_utf8_lossy(&bytes);
					if !self.load_text(text.trim_start_matches('\u{feff}')) && keep {
						keep_broken(path, &bytes);
					}
					return Loaded::Read;
				}
				Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Loaded::New,
				Err(e) => {
					last = Some(e);
					if n + 1 < tries {
						std::thread::sleep(pause);
					}
				}
			}
		}
		if let Some(e) = last {
			eprintln!("achievements: {} exists but cannot be read ({e}): it is not overwritten, nothing is saved this session", path.display());
		}
		Loaded::Unreadable
	}

	/// Atomic write of the whole state now (unique tmp, fsync, rename; `writer.rs`). The running game goes through the writer thread
	/// instead (`Hub::flush`); this is the same write on the calling thread.
	pub fn save(&self, path: &Path, now: u64) -> std::io::Result<()> {
		super::writer::write_atomic(path, &self.to_json(now))
	}
}

/// How many damaged copies are kept next to the file; more are not copied (the first, oldest ones are the ones worth having).
const BROKEN_KEEP: usize = 20;

/// Copies a damaged file aside without ever overwriting an earlier copy: the first goes to `achievements.json.broken`, later ones to
/// `.broken.2`, `.broken.3`, ...; a copy with the same bytes as one already kept is not made again (a restart with the same damaged file).
fn keep_broken(path: &Path, bytes: &[u8]) {
	for n in 1..=BROKEN_KEEP {
		let target = if n == 1 { path.with_extension("json.broken") } else { path.with_extension(format!("json.broken.{n}")) };
		match std::fs::read(&target) {
			Ok(kept) if kept == bytes => return,
			Ok(_) => continue,
			Err(_) => {
				// `create_new`: never writes over a file that is there (two game processes starting together).
				if let Ok(mut f) = std::fs::OpenOptions::new().write(true).create_new(true).open(&target) {
					let _ = std::io::Write::write_all(&mut f, bytes);
				}
				return;
			}
		}
	}
}

fn quote(s: &str) -> String {
	let mut o = String::with_capacity(s.len() + 2);
	o.push('"');
	for c in s.chars() {
		match c {
			'"' => o.push_str("\\\""),
			'\\' => o.push_str("\\\\"),
			c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
			c => o.push(c),
		}
	}
	o.push('"');
	o
}

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
	let y = if m <= 2 { y - 1 } else { y };
	let era = y.div_euclid(400);
	let yoe = y - era * 400;
	let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
	let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
	era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
	let z = z + 719468;
	let era = z.div_euclid(146097);
	let doe = z - era * 146097;
	let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
	let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
	let mp = (5 * doy + 2) / 153;
	let d = doy - (153 * mp + 2) / 5 + 1;
	let m = if mp < 10 { mp + 3 } else { mp - 9 };
	(yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// `2026-11-02T12:00:00Z` for unix seconds.
pub fn unix_to_iso(t: u64) -> String {
	let (days, rem) = ((t / 86400) as i64, t % 86400);
	let (y, m, d) = civil_from_days(days);
	format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Unix seconds of `YYYY-MM-DDTHH:MM:SSZ`; None for anything else.
pub fn iso_to_unix(s: &str) -> Option<u64> {
	let b = s.as_bytes();
	if b.len() != 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' || b[19] != b'Z' {
		return None;
	}
	let n = |a: usize, z: usize| s.get(a..z)?.parse::<i64>().ok();
	let (y, mo, d, h, mi, se) = (n(0, 4)?, n(5, 7)?, n(8, 10)?, n(11, 13)?, n(14, 16)?, n(17, 19)?);
	if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
		return None;
	}
	u64::try_from(days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + se).ok()
}
