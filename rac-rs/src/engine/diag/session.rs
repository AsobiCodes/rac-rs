//! [PC] RAC_DIAG (diag.rs): the session folder `<player dir>/diag/<yyyy-mm-dd_hhmmss>/` (the newest `KEEP` sessions stay, older
//! ones are deleted at the start of a new one) and its writer thread: the render thread only sends texts and pictures down a
//! channel; the thread appends them to the files and flushes after each message, so a killed run keeps everything up to its
//! last second.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};
use std::sync::Mutex;

/// Sessions kept (the new one included).
pub const KEEP: usize = 10;
/// Bytes one file of a session may reach; past it one "cap reached" line is written and the rest dropped.
pub const FILE_CAP: u64 = 50 << 20;
/// Mark pictures one session saves at most.
pub const PNG_CAP: u32 = 50;
/// Bytes all kept sessions together may take at the start of a new one (the oldest go first).
pub const TOTAL_CAP: u64 = 500 << 20;

pub enum Msg {
	/// Append text to a file of the session folder.
	Append(&'static str, String),
	/// Encode and save a picture as `name` in the session folder.
	Png(String, crate::render::Image),
}

pub struct Session {
	pub dir: PathBuf,
	tx: Mutex<Sender<Msg>>,
}

impl Session {
	pub fn send(&self, m: Msg) {
		if let Ok(tx) = self.tx.lock() {
			let _ = tx.send(m);
		}
	}

	pub fn append(&self, file: &'static str, text: String) {
		self.send(Msg::Append(file, text));
	}
}

/// The folder name of a session that starts at local time `stamp` ("YYYY-MM-DD HH:MM:SS"): "YYYY-MM-DD_HHMMSS".
pub fn folder_name(stamp: &str) -> String {
	let (d, t) = stamp.split_once(' ').unwrap_or((stamp, ""));
	format!("{d}_{}", t.replace(':', ""))
}

/// Whether a folder name is a session's (`folder_name`'s shape, so nothing else in `diag` is ever deleted).
pub fn is_session(name: &str) -> bool {
	let b = name.as_bytes();
	let stamp = b.len() >= 17 && b[4] == b'-' && b[7] == b'-' && b[10] == b'_' && b[..17].iter().enumerate().all(|(i, c)| matches!(i, 4 | 7 | 10) || c.is_ascii_digit());
	// optional clash suffix `_N`
	stamp && (b.len() == 17 || (b.len() > 18 && b[17] == b'_' && b[18..].iter().all(u8::is_ascii_digit)))
}

/// The session folders to delete so that `KEEP - 1` stay besides a new one (oldest first; names sort by time).
pub fn to_delete(mut names: Vec<String>) -> Vec<String> {
	names.retain(|n| is_session(n));
	names.sort();
	let n = names.len().saturating_sub(KEEP - 1);
	names.truncate(n);
	names
}

/// The sessions (oldest first) to delete so that the rest stay under `cap` bytes: `sizes` = (name, bytes) of the kept ones.
pub fn over_cap(mut sizes: Vec<(String, u64)>, cap: u64) -> Vec<String> {
	sizes.sort();
	let mut total: u64 = sizes.iter().map(|s| s.1).sum();
	let mut out = Vec::new();
	for (n, b) in sizes {
		if total <= cap {
			break;
		}
		total -= b;
		out.push(n);
	}
	out
}

/// Bytes of the files in `dir` (one level: a session holds no subfolders).
fn dir_bytes(dir: &Path) -> u64 {
	std::fs::read_dir(dir).map(|r| r.flatten().filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).map(|m| m.len()).sum()).unwrap_or(0)
}

/// Deletes the old sessions under `root`: all but the newest `KEEP - 1`, then the oldest while they take over `TOTAL_CAP`.
fn prune(root: &Path) {
	let old: Vec<String> = std::fs::read_dir(root).map(|r| r.flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
	let del = to_delete(old.clone());
	for n in &del {
		let _ = std::fs::remove_dir_all(root.join(n));
	}
	let kept: Vec<(String, u64)> = old.into_iter().filter(|n| is_session(n) && !del.contains(n)).map(|n| { let b = dir_bytes(&root.join(&n)); (n, b) }).collect();
	for n in over_cap(kept, TOTAL_CAP) {
		let _ = std::fs::remove_dir_all(root.join(n));
	}
}

/// One open file of the writer: name, handle, bytes written, whether the cap line went out.
struct Out {
	name: &'static str,
	f: std::fs::File,
	bytes: u64,
	capped: bool,
}

/// The writer thread's loop: appends with the per-file cap, saves at most `PNG_CAP` pictures.
fn write_loop(d: PathBuf, rx: std::sync::mpsc::Receiver<Msg>) {
	let mut files: Vec<Out> = Vec::new();
	let mut pngs = 0u32;
	for m in rx {
		match m {
			Msg::Append(name, text) => {
				if !files.iter().any(|f| f.name == name) {
					match std::fs::OpenOptions::new().create(true).append(true).open(d.join(name)) {
						Ok(f) => files.push(Out { name, f, bytes: 0, capped: false }),
						Err(_) => continue,
					}
				}
				let Some(o) = files.iter_mut().find(|f| f.name == name) else { continue };
				if o.capped {
					continue;
				}
				let t = if o.bytes + text.len() as u64 > FILE_CAP {
					o.capped = true;
					format!("\n[diag] size cap of {} MB reached: nothing more is written to this file\n", FILE_CAP >> 20)
				} else {
					text
				};
				o.bytes += t.len() as u64;
				let _ = o.f.write_all(t.as_bytes());
				let _ = o.f.flush();
			}
			Msg::Png(name, img) => {
				pngs += 1;
				if pngs <= PNG_CAP {
					let _ = std::fs::write(d.join(name), crate::render::png(&img));
				}
			}
		}
	}
}

/// Makes the session folder under `root` (`<player dir>/diag`), deletes the old ones and starts the writer thread.
pub fn open(root: &Path, stamp: &str) -> Option<Session> {
	prune(root);
	std::fs::create_dir_all(root).ok()?;
	// Two starts within one second: a suffix, never a shared folder (create_dir fails on an existing one).
	let mut dir = None;
	for k in 1..10 {
		let d = if k == 1 { root.join(folder_name(stamp)) } else { root.join(format!("{}_{k}", folder_name(stamp))) };
		match std::fs::create_dir(&d) {
			Ok(()) => {
				dir = Some(d);
				break;
			}
			Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
			Err(_) => return None,
		}
	}
	let dir = dir?;
	let (tx, rx) = channel::<Msg>();
	let d = dir.clone();
	std::thread::Builder::new().name("diag".into()).spawn(move || write_loop(d, rx)).ok()?;
	Some(Session { dir, tx: Mutex::new(tx) })
}

/// UTC "YYYY-MM-DD HH:MM:SS" of `secs` since 1970 (systems without the local-time call).
pub fn utc_stamp(secs: u64) -> String {
	let (days, s) = (secs / 86400, secs % 86400);
	// Civil date from days since 1970-01-01 (H. Hinnant's algorithm).
	let z = days as i64 + 719468;
	let era = z.div_euclid(146097);
	let doe = z - era * 146097;
	let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
	let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
	let mp = (5 * doy + 2) / 153;
	let d = doy - (153 * mp + 2) / 5 + 1;
	let m = if mp < 10 { mp + 3 } else { mp - 9 };
	let y = yoe + era * 400 + (m <= 2) as i64;
	format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn folder_names_follow_the_local_time() {
		assert_eq!(folder_name("2026-11-07 09:05:03"), "2026-11-07_090503");
		assert!(is_session("2026-11-07_090503"));
		assert!(is_session("2026-11-07_090503_2"));
		assert!(!is_session("mods"));
		assert!(!is_session("2026-11-07x090503"));
	}

	#[test]
	fn retention_keeps_the_newest_nine_besides_the_new_one() {
		let mut names: Vec<String> = (0..12).map(|k| format!("2026-11-{:02}_120000", k + 1)).collect();
		names.push("notes".into());
		let del = to_delete(names);
		assert_eq!(del, vec!["2026-11-01_120000", "2026-11-02_120000", "2026-11-03_120000"]);
		assert!(to_delete(vec!["2026-11-01_120000".into()]).is_empty());
	}

	#[test]
	fn the_total_cap_drops_the_oldest_first() {
		let v = vec![("b".to_string(), 300), ("a".to_string(), 300), ("c".to_string(), 100)];
		assert_eq!(over_cap(v.clone(), 500), vec!["a".to_string()]);
		assert_eq!(over_cap(v.clone(), 700), Vec::<String>::new());
		assert_eq!(over_cap(v, 50), vec!["a".to_string(), "b".to_string(), "c".to_string()]);
	}

	#[test]
	fn a_clash_gets_its_own_folder() {
		let root = std::env::temp_dir().join(format!("diag_clash_test_{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		let a = open(&root, "2026-11-07 10:00:00").unwrap();
		let b = open(&root, "2026-11-07 10:00:00").unwrap();
		assert_ne!(a.dir, b.dir);
		assert!(b.dir.ends_with("2026-11-07_100000_2"));
		drop((a, b));
		std::thread::sleep(std::time::Duration::from_millis(100));
		let _ = std::fs::remove_dir_all(&root);
	}

	#[test]
	fn utc_stamps_are_civil_dates() {
		assert_eq!(utc_stamp(0), "1970-01-01 00:00:00");
		assert_eq!(utc_stamp(1_762_516_800 + 3723), "2025-11-07 13:02:03");
	}

	#[test]
	fn open_writes_and_prunes() {
		let root = std::env::temp_dir().join(format!("diag_session_test_{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		for k in 0..11 {
			std::fs::create_dir_all(root.join(format!("2020-01-{:02}_000000", k + 1))).unwrap();
		}
		let s = open(&root, "2026-11-07 10:00:00").unwrap();
		s.append("a.txt", "x\n".into());
		drop(s); // the channel closes: the thread writes what it has and ends
		std::thread::sleep(std::time::Duration::from_millis(200));
		let left = std::fs::read_dir(&root).unwrap().count();
		assert_eq!(left, KEEP);
		assert_eq!(std::fs::read_to_string(root.join("2026-11-07_100000/a.txt")).unwrap(), "x\n");
		let _ = std::fs::remove_dir_all(&root);
	}
}
