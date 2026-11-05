//! Diagnostics of the PAL -> USA address layer (`disc::va`, `va_level`, `g`): the USA run log notes an address the map can
//! only guess (once each, at most 64), an address outside every map segment panics when a unit test runs that `va` call on
//! the USA disc (no other test drives the feature readers on USA, so a hole there is caught by `disc_guard_tests.rs` and the
//! USA smoke traces, not by a unit test of the feature), and RAC_VA_TRACE=<file> writes every distinct mapping the run makes
//! to a file for tools/addr/gen_disc_map.py --check-trace (the independent proof of the map).
//! PAL never gets here (`va` returns first). With RAC_VA_TRACE unset a USA mapping costs one relaxed atomic load: of `MODE`
//! for a run / gap / ext / core address, of the address' slot in `NOTED` for a tail / ambig / outside-segment address (a
//! fixed array of atomics: no lock, no scan, no allocation). `seen` and `on` contain no lock (unit test
//! `hot_functions_have_no_lock_or_scan`); only the cold paths, and only with the trace on, lock the trace file.
use std::collections::HashSet;
use std::io::Write;
use std::panic::Location;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock}; // Mutex only inside `Lock`

/// Segment kinds as disc_map.rs gives them (0 run, 1 gap, 2 ext, 3 tail, 4 ambig) plus the three outcomes of `map_kind`.
pub const K_CORE: u8 = 5; // below the overlay part (0x15ef00): the same on both discs
pub const K_NONE: u8 = 6; // inside the overlay part but outside every segment of the scope: kept as it is
pub const K_G: u8 = 7; // `disc::g` (the fixed +0x20 of the boot globals)
const KIND_NAME: [&str; 8] = ["run", "gap", "ext", "tail", "ambig", "core", "none", "g"];

/// Distinct mappings a trace file keeps (one line each); more are counted but not written.
const TRACE_CAP: usize = 1 << 16;
/// Run-log notes of guessed addresses.
const NOTE_CAP: usize = 64;

static MODE: AtomicU8 = AtomicU8::new(0); // 0 not read yet, 1 off, 2 on
/// The only lock of this module: every mutex here is a `Lock`, so the unit test can count the acquisitions (`LOCKS`) and the
/// structure test can see that no other `Mutex` exists.
struct Lock<T>(Mutex<T>); // the one Mutex of the module
impl<T> Lock<T> {
	fn new(v: T) -> Self {
		Lock(Mutex::new(v)) // the one Mutex of the module
	}
	fn lock(&self) -> MutexGuard<'_, T> { // the one Mutex of the module
		#[cfg(test)]
		LOCKS.fetch_add(1, Ordering::Relaxed);
		self.0.lock().unwrap_or_else(|e| e.into_inner())
	}
}
/// Lock acquisitions made (unit tests only).
#[cfg(test)]
static LOCKS: AtomicU64 = AtomicU64::new(0);
type Key = (u8, u32, u32, u8, &'static str, u32);
struct Sink {
	seen: HashSet<Key>,
	out: std::io::BufWriter<std::fs::File>,
}
/// The trace file, created once (`OnceLock`: two threads that race on the first `on()` cannot both create and truncate it).
static SINK: OnceLock<Option<Lock<Sink>>> = OnceLock::new();
/// `(scope << 32 | pal) + 1` of the tail / ambig / none addresses already noted (trace off only), in the slot
/// `hash >> 58`; 0 = empty. A colliding address replaces the older one and may be noted again; `NOTE_CAP` lines in all.
static NOTED: [AtomicU64; 64] = [const { AtomicU64::new(0) }; 64];
/// Run-log notes made so far.
static NOTES: AtomicU64 = AtomicU64::new(0);
/// Set once `NOTE_CAP` notes are made with the trace off: from then on a slot miss returns after this one relaxed load (no
/// store, no `fetch_add`, nothing shared is written). Never set with the trace on (the trace file has its own cap).
static QUIET: AtomicBool = AtomicBool::new(false);

/// RAC_VA_TRACE is set and the file opens (read once).
#[inline]
pub fn on() -> bool {
	match MODE.load(Ordering::Relaxed) {
		1 => false,
		2 => true,
		_ => init(),
	}
}

#[cold]
fn init() -> bool {
	let sink = SINK.get_or_init(|| {
		let f = std::env::var_os("RAC_VA_TRACE").filter(|p| !p.is_empty()).and_then(|p| std::fs::File::create(p).ok())?;
		let mut out = std::io::BufWriter::new(f);
		let _ = writeln!(out, "# scope\tpal\tusa\tkind\tat   (RAC_VA_TRACE of rac-rs: one line per distinct mapping; check with tools/addr/gen_disc_map.py --check-trace)");
		Some(Lock::new(Sink { seen: HashSet::new(), out }))
	});
	let on = sink.is_some();
	MODE.store(if on { 2 } else { 1 }, Ordering::Relaxed);
	on
}

/// The scope in the file's words: L0..L18, front (19), boot (20).
pub fn scope_name(sc: u8) -> String {
	match sc {
		super::disc::SCOPE_FRONT => "front".into(),
		super::disc::SCOPE_BOOT => "boot".into(),
		n => format!("L{n}"),
	}
}

/// One mapping `va` made: scope `sc`, PAL address `pal` -> USA address `usa` in a segment of `kind`, called from `at`.
/// `disc::traced` calls it for every tail / ambig / none / core address, and for a run / gap / ext address only when the
/// trace is on; `disc::g` calls it (kind `K_G`) only when the trace is on.
/// Tail / ambig / none: one relaxed load of the address' slot in `NOTED`; the slot only ever holds the address when the trace
/// is off (with the trace on every call takes the cold path, which records it). Everything else: one relaxed load of `MODE`.
#[inline]
pub fn seen(sc: u8, pal: u32, usa: u32, kind: u8, at: &'static Location<'static>) {
	if matches!(kind, 3 | 4 | K_NONE) {
		let key = ((sc as u64) << 32 | pal as u64) + 1;
		let slot = &NOTED[(key.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 58) as usize];
		if slot.load(Ordering::Relaxed) != key {
			first_seen(slot, key, sc, pal, usa, kind, at);
		}
	} else if on() {
		record(sc, pal, usa, kind, at);
	}
}

/// An address the map cannot place exactly (tail / ambig segment) or not at all (outside every segment), met for the first
/// time (or after a slot collision): noted once in the run log (the game keeps running with the unchanged or guessed
/// address), remembered in its slot when the trace is off, recorded when it is on. A unit test that runs a `va` call on the
/// USA disc into a hole panics here (before the slot is set, so every such call panics).
#[cold]
fn first_seen(slot: &AtomicU64, key: u64, sc: u8, pal: u32, usa: u32, kind: u8, at: &'static Location<'static>) {
	if cfg!(test) && kind == K_NONE {
		panic!("disc: PAL address {pal:#x} is outside every USA map segment of scope {} (at {}:{})", scope_name(sc), at.file(), at.line());
	}
	if QUIET.load(Ordering::Relaxed) {
		return; // capped: one relaxed load, nothing is written
	}
	let tracing = on();
	let new = if tracing {
		record(sc, pal, usa, kind, at)
	} else {
		slot.store(key, Ordering::Relaxed);
		true
	};
	if new && NOTES.fetch_add(1, Ordering::Relaxed) < NOTE_CAP as u64 {
		if !tracing && NOTES.load(Ordering::Relaxed) >= NOTE_CAP as u64 {
			QUIET.store(true, Ordering::Relaxed);
		}
		eprintln!(
			"retail: disc address {pal:#x} in scope {} maps {} -> {usa:#x} ({}) at {}:{}",
			scope_name(sc),
			if kind == K_NONE { "outside every segment, kept" } else { "by a guess" },
			KIND_NAME[kind as usize],
			at.file(),
			at.line()
		);
	}
}

/// Writes the mapping to the trace file once per (scope, address, site); false when it was written before, the file is full
/// (`TRACE_CAP` lines) or there is no file.
#[cold]
fn record(sc: u8, pal: u32, usa: u32, kind: u8, at: &'static Location<'static>) -> bool {
	let Some(Some(sink)) = SINK.get() else { return false };
	let mut g = sink.lock();
	if g.seen.len() >= TRACE_CAP || !g.seen.insert((sc, pal, usa, kind, at.file(), at.line())) {
		return false;
	}
	let file = at.file().replace('\\', "/");
	let file = file.strip_prefix("src/").unwrap_or(&file);
	let _ = writeln!(g.out, "{}\t{pal:06x}\t{usa:06x}\t{}\t{}:{}", scope_name(sc), KIND_NAME[kind as usize], file, at.line());
	let _ = g.out.flush();
	true
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A1: with RAC_VA_TRACE unset, mapping takes no lock. Every mutex of this module is a `Lock`, which counts its
	/// acquisitions in `LOCKS`; 3500 mappings of every kind (run / gap / ext, tail, ambig, core, `g`) must leave the count
	/// unchanged. A `seen` that locks anything (the round-1 mutex design) raises it and fails here. The structure test below
	/// proves that no `Mutex` exists outside `Lock`, so no lock can hide from the counter.
	#[test]
	fn off_path_takes_no_lock() {
		if on() {
			return; // RAC_VA_TRACE set for this test run: the trace path locks by design
		}
		let at = Location::caller();
		let before = LOCKS.load(Ordering::Relaxed);
		for _ in 0..500 {
			for kind in [0u8, 1, 2, 3, 4, K_CORE, K_G] {
				seen(3, 0x2a_0000, 0x2a_0080, kind, at);
			}
		}
		assert_eq!(LOCKS.load(Ordering::Relaxed), before, "a mapping with the trace off took a lock");
	}

	/// F9: after `NOTE_CAP` notes a slot miss does nothing but one relaxed load of `QUIET`: no store into the slot, no
	/// `fetch_add` (the miss would otherwise write a cache line shared by every mapping thread).
	#[test]
	fn capped_slot_miss_writes_nothing() {
		if on() {
			return;
		}
		let at = Location::caller();
		for i in 0..(4 * NOTE_CAP as u32) {
			seen(2, 0x30_0000 + 0x100 * i, 0x30_0000 + 0x100 * i, 3, at);
		}
		assert!(QUIET.load(Ordering::Relaxed), "{NOTE_CAP} notes with the trace off must set QUIET");
		let (sc, pal) = (4u8, 0x3f_1230u32);
		let key = ((sc as u64) << 32 | pal as u64) + 1;
		let slot = &NOTED[(key.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 58) as usize];
		let notes = NOTES.load(Ordering::Relaxed);
		seen(sc, pal, pal, 4, at);
		assert_ne!(slot.load(Ordering::Relaxed), key, "a capped miss stored into its slot");
		assert!(NOTES.load(Ordering::Relaxed) <= notes + 8, "a capped miss counted a note"); // slack: other tests may race in a note before QUIET
	}

	/// A1 / F9 (structure): `seen` and `on` hold no lock and no collection scan; `first_seen` opens with the `QUIET` load
	/// (after the test-only hole panic) and writes nothing before it; only the cold `record` locks; no `Mutex` exists outside `Lock`.
	#[test]
	fn hot_functions_have_no_lock_or_scan() {
		let src = include_str!("disc_trace.rs");
		let body = |name: &str| {
			let a = src.find(&format!("fn {name}(")).unwrap_or_else(|| panic!("fn {name}"));
			let rest = &src[a..];
			let end = rest.find("\n}\n").map_or(rest.len(), |p| p + 3);
			rest[..end].to_string()
		};
		for f in ["seen", "on"] {
			let b = body(f);
			for bad in ["lock(", "Mutex", "contains(", ".iter()", "Vec", "HashSet", "SINK"] {
				assert!(!b.contains(bad), "fn {f} must stay lock-free and scan-free, found `{bad}`");
			}
		}
		assert!(body("record").contains("lock("), "only the cold record() takes the sink lock");
		let fs = body("first_seen");
		assert!(!fs.contains("lock("), "first_seen reaches the lock only through record()");
		let quiet = fs.find("QUIET.load").expect("first_seen starts with the QUIET load");
		for later in ["store(", "fetch_add", "record(", "on()", "eprintln"] {
			assert!(fs.find(later).map_or(true, |p| p > quiet), "first_seen: `{later}` before the QUIET load (the capped path must be one load)");
		}
		let code = &src[..src.find("#[cfg(test)]\nmod tests {").unwrap()];
		for l in code.lines().filter(|l| l.contains("Mutex") && !l.trim_start().starts_with("//")) {
			assert!(l.contains("the one Mutex") || l.contains("Mutex only inside `Lock`"), "a Mutex outside `Lock` would hide from the lock counter: {l}");
		}
	}
}
