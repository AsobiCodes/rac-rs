//! The writer of `achievements.json` (fix round 3, m5 / B14). The hub builds the file's text under its lock and hands it over here; a small
//! thread writes it, so the HUB lock (taken by the worker every field and by the overlay every frame) never waits for the disk.
//!
//! - One job per path: a text that is not written yet and is already out of date is replaced (latest text wins).
//! - A write is `File::create` + `write_all` + `sync_all` on a tmp name that is unique per process (`achievements.json.<pid>.tmp`), then
//!   a rename over the file: a power loss leaves the old file or the new one, never an empty one, and two game processes never share a
//!   tmp file. A failed write is noted per path (`take_failed`): the hub then writes the whole state again at its next flush.
//! - The folder is made once, by `achievements::init`; the thread makes it again only after a failed write (the folder was deleted).
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

struct Slot {
	/// Texts waiting to be written, at most one per path, oldest first.
	jobs: Vec<(PathBuf, String)>,
	/// The thread is writing a job right now (taken out of `jobs`).
	busy: bool,
	/// The thread was started (it runs for the rest of the process).
	spawned: bool,
	/// Paths whose last write failed.
	failed: Vec<PathBuf>,
}

static SLOT: Mutex<Slot> = Mutex::new(Slot { jobs: Vec::new(), busy: false, spawned: false, failed: Vec::new() });
/// The thread waits here for a job.
static WAKE: Condvar = Condvar::new();
/// `wait_idle` waits here for the jobs to drain.
static IDLE: Condvar = Condvar::new();

fn lock() -> MutexGuard<'static, Slot> {
	SLOT.lock().unwrap_or_else(|e| e.into_inner())
}

/// The tmp file of a write to `path` in this process: `achievements.json` -> `achievements.json.<pid>.tmp`.
pub(super) fn tmp_path(path: &Path) -> PathBuf {
	path.with_extension(format!("json.{}.tmp", std::process::id()))
}

/// Removes the tmp files a crashed game left next to `path`: exactly `<file name>.<digits>.tmp` with a pid other than `own_pid`
/// (`achievements.json.4242.tmp`). Nothing else in the folder is touched. Returns how many were removed.
pub(super) fn remove_stale_tmp(path: &Path, own_pid: u32) -> usize {
	let (Some(dir), Some(name)) = (path.parent(), path.file_name().map(|n| n.to_string_lossy().into_owned())) else { return 0 };
	let Ok(rd) = fs::read_dir(dir) else { return 0 };
	let mut n = 0;
	for e in rd.flatten() {
		let f = e.file_name().to_string_lossy().into_owned();
		let Some(pid) = f.strip_prefix(name.as_str()).and_then(|r| r.strip_prefix('.')).and_then(|r| r.strip_suffix(".tmp")) else { continue };
		if pid.is_empty() || !pid.bytes().all(|b| b.is_ascii_digit()) || pid.parse::<u32>().is_ok_and(|p| p == own_pid) {
			continue;
		}
		if e.file_type().is_ok_and(|t| t.is_file()) && fs::remove_file(e.path()).is_ok() {
			n += 1;
		}
	}
	n
}

/// Writes `text` to a tmp file, forces it to disk, and renames it over `path`. A failed step removes the tmp file again.
pub(super) fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
	let tmp = tmp_path(path);
	let written = File::create(&tmp).and_then(|mut f| {
		f.write_all(text.as_bytes())?;
		f.sync_all()
	});
	let done = written.and_then(|()| fs::rename(&tmp, path));
	if done.is_err() {
		let _ = fs::remove_file(&tmp);
	}
	done
}

/// One job: a second try after making the folder again covers a folder that was deleted while the game ran.
fn write_job(path: &Path, text: &str) -> bool {
	if write_atomic(path, text).is_ok() {
		return true;
	}
	if let Some(dir) = path.parent() {
		let _ = fs::create_dir_all(dir);
	}
	write_atomic(path, text).is_ok()
}

/// Hands `text` over to be written to `path` (replaces a text for `path` that was not written yet). Starts the thread on first use;
/// if no thread can be started the write happens here.
pub(super) fn submit(path: PathBuf, text: String) {
	let mut s = lock();
	match s.jobs.iter_mut().find(|j| j.0 == path) {
		Some(j) => j.1 = text,
		None => s.jobs.push((path, text)),
	}
	if !s.spawned {
		s.spawned = std::thread::Builder::new().name("ach-writer".into()).spawn(run).is_ok();
		if !s.spawned {
			let jobs = std::mem::take(&mut s.jobs);
			drop(s);
			for (path, text) in jobs {
				let ok = write_job(&path, &text);
				note(&path, ok);
			}
			return;
		}
	}
	drop(s);
	WAKE.notify_one();
}

fn note(path: &Path, ok: bool) {
	let mut s = lock();
	s.failed.retain(|p| p != path);
	if !ok {
		s.failed.push(path.to_path_buf());
	}
}

/// Whether the last write to `path` failed; the answer is given once (the hub marks itself dirty and writes everything again).
pub(super) fn take_failed(path: &Path) -> bool {
	let mut s = lock();
	let n = s.failed.len();
	s.failed.retain(|p| p != path);
	s.failed.len() != n
}

/// Whether the last write to `path` failed, without taking the answer (the quit's retry, `achievements::flush_now`).
pub(super) fn failed(path: &Path) -> bool {
	lock().failed.iter().any(|p| p == path)
}

/// Waits until every handed-over text is on disk (or failed: `failed` tells), at most `limit`; true = drained. For the quit
/// (`flush_now`) and the tests.
pub(super) fn wait_idle(limit: Duration) -> bool {
	let end = Instant::now() + limit;
	let mut s = lock();
	while !s.jobs.is_empty() || s.busy {
		let Some(left) = end.checked_duration_since(Instant::now()) else { return false };
		s = IDLE.wait_timeout(s, left).unwrap_or_else(|e| e.into_inner()).0;
	}
	true
}

/// The thread: takes the oldest job, writes it without holding the slot, records the result, and sleeps when nothing is left.
fn run() {
	let mut s = lock();
	loop {
		if s.jobs.is_empty() {
			s = WAKE.wait(s).unwrap_or_else(|e| e.into_inner());
			continue;
		}
		let (path, text) = s.jobs.remove(0);
		s.busy = true;
		drop(s);
		let ok = write_job(&path, &text);
		note(&path, ok);
		s = lock();
		s.busy = false;
		IDLE.notify_all();
	}
}
