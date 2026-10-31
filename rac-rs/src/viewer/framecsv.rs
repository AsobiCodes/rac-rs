//! [PC] RAC_FRAME_CSV=path (debug aid for frame-pacing checks, no retail counterpart): one text line per display frame and per
//! present, on a common clock (milliseconds since the first line), so the real cadence of the frames can be analysed (percentiles,
//! spikes, the spacing of the present calls that a frame-time overlay like RTSS sees). Off by default: every call is one relaxed load.
//!
//! Lines (space separated):
//! - `F start redraw ticks acquire encode present` - a redraw of the render thread: start time, its length, the game ticks it ran, and
//!   the time in the `acquire` / `encode` / `present` phases (profile.rs), all in ms.
//! - `P start length` - one `queue.present` call (on the present thread when there is one): when it began and how long it blocked.
//! - `C start length` - the frame cap's sleep after a redraw (pace.rs).
//! - `G start length` - the game thread waiting for the GS capture (`Capture::pace`, GS_LAG fields back); `T` - the snapshot thread waiting
//!   for a published overlay (`CapTake::take`); `S` - the capture thread's `Queue::submit` incl. the wait for the submit lock
//!   (gpu_shared::submit); `R` - the capture thread waiting for the picture's readback (`Gpu::rb_finish` blocking / `submit` non-lazy).
//!   Only waits longer than 50 us are written.
//! - `W start redraw ticks` + 13 phases, then `X start frame=wall:cpu name=wall:cpu ...` - a redraw longer than `RAC_SLOW_MS` and its
//!   biggest spans (`slow`), each as wall-clock us and the render thread's on-CPU us (profile.rs: QueryThreadCycleTime on Windows, 0 elsewhere);
//!   `frame` is the whole redraw. wall minus cpu = time the thread did not run (waits, locks, preemption); a pseudo-span such as `ctf_n_verts`
//!   holds a count, not time. tools/cpuwall.py (lane lows) lists the stalls per span.
//! - `U` / `Q` / `V` - the present thread before a present: `U` waited for the frame's command buffers (the `enc-finish` jobs), `Q` applied
//!   the frame's deferred queue writes (viewer/qdefer.rs, RAC_WRITE_DEFER), `V` ran `Queue::submit` (`present_step`); only steps longer than
//!   50 us are written.
//! Written about once a second (so a killed run keeps its data) and the rest when the event loop ends (`flush`, viewer/startup.rs). The
//! file write happens outside the lock that the game, capture and present threads push their rows under.
//! tools/frames.py (lane) analyses the file.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static ON: AtomicBool = AtomicBool::new(false);
static SINK: OnceLock<Mutex<Sink>> = OnceLock::new();
/// The output file, apart from `SINK` so that its writes never hold the lock of the threads that push rows.
static FILE: OnceLock<Mutex<std::fs::File>> = OnceLock::new();

struct Sink {
	t0: Instant,
	buf: String,
	last_flush: Instant,
}

/// Reads RAC_FRAME_CSV once at start-up; true when a file was opened.
pub(super) fn init_from_env() -> bool {
	std::env::var("RAC_FRAME_CSV").ok().filter(|p| !p.is_empty()).is_some_and(|p| open(&p))
}

/// Creates the file and switches the rows on; false when the file cannot be created.
fn open(path: &str) -> bool {
	let Ok(file) = std::fs::File::create(path) else {
		eprintln!("frame csv: cannot create {path}");
		return false;
	};
	let now = Instant::now();
	let _ = FILE.set(Mutex::new(file));
	let _ = SINK.set(Mutex::new(Sink { t0: now, buf: String::with_capacity(1 << 16), last_flush: now }));
	ON.store(true, Ordering::Relaxed);
	true
}

#[inline]
pub(crate) fn on() -> bool {
	ON.load(Ordering::Relaxed)
}

/// One line of the file: `kind`, the start time `t` (ms on the file's clock) and the fields, space separated, 3 decimals.
fn line(kind: char, t: f64, fields: &[f32]) -> String {
	let mut l = format!("{kind} {t:.3}");
	for f in fields {
		l.push_str(&format!(" {f:.3}"));
	}
	l.push('\n');
	l
}

fn push(kind: char, start: Instant, fields: &[f32]) {
	let Some(s) = SINK.get() else { return };
	let full = {
		let mut s = s.lock().unwrap_or_else(|e| e.into_inner());
		let t = start.saturating_duration_since(s.t0).as_secs_f64() * 1000.0;
		let l = line(kind, t, fields);
		s.buf.push_str(&l);
		(s.last_flush.elapsed().as_millis() >= 1000).then(|| {
			s.last_flush = Instant::now();
			std::mem::take(&mut s.buf)
		})
	};
	if let Some(b) = full {
		write_out(&b);
	}
}

fn write_out(b: &str) {
	if let Some(f) = FILE.get() {
		let _ = f.lock().unwrap_or_else(|e| e.into_inner()).write_all(b.as_bytes());
	}
}

/// Writes the rows not written yet (the event loop ended: viewer/startup.rs `run`). Nothing when the CSV is off.
pub(super) fn flush() {
	let Some(s) = SINK.get() else { return };
	let b = std::mem::take(&mut s.lock().unwrap_or_else(|e| e.into_inner()).buf);
	write_out(&b);
}

/// A redraw that began at `start`: its length and the phases that matter for pacing (all ms).
pub(super) fn frame(start: Instant, redraw_ms: f32, ticks: u32, acquire_ms: f32, encode_ms: f32, present_ms: f32) {
	if on() {
		push('F', start, &[redraw_ms, ticks as f32, acquire_ms, encode_ms, present_ms]);
	}
}

/// A redraw longer than `RAC_SLOW_MS` (default 8.3): `W start redraw ticks` + the 13 phases (ms), then `X start` + its biggest spans
/// as `name=us` text (P9d: which stage spiked; tools read the W rows and skip the text rows).
pub(super) fn slow(start: Instant, redraw_ms: f32, ticks: u32, phases: &[f32], spans: &str) {
	static LIMIT: OnceLock<f32> = OnceLock::new();
	let limit = *LIMIT.get_or_init(|| std::env::var("RAC_SLOW_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(8.3));
	if on() && redraw_ms > limit {
		let mut f = vec![redraw_ms, ticks as f32];
		f.extend_from_slice(phases);
		push('W', start, &f);
		let Some(s) = SINK.get() else { return };
		let mut s = s.lock().unwrap_or_else(|e| e.into_inner());
		let t = start.saturating_duration_since(s.t0).as_secs_f64() * 1000.0;
		s.buf.push_str(&format!("X {t:.3} {spans}\n"));
	}
}

/// [PC] The present thread's steps before a present, written when longer than 50 us: `U` it waited for the frame's command
/// buffers (the `enc-finish` jobs), `Q` it applied the frame's deferred queue writes, `V` it ran `Queue::submit`. `start` is when the step began, `len_ms` how long it took.
pub(super) fn present_step(kind: char, start: Instant, len_ms: f32) {
	if on() && len_ms > 0.05 {
		push(kind, start, &[len_ms]);
	}
}

/// A `queue.present` call that began at `start` and returned after `len_ms`.
pub(super) fn present(start: Instant, len_ms: f32) {
	if on() {
		push('P', start, &[len_ms]);
	}
}

/// The frame cap's sleep that began at `start` and lasted `len_ms`.
pub(super) fn cap(start: Instant, len_ms: f32) {
	if on() {
		push('C', start, &[len_ms]);
	}
}

/// A wait of another thread (`kind` G / T / S / R, see the module doc) that began at `start`; under 50 us is not written.
pub(crate) fn wait(kind: char, start: Instant) {
	if on() {
		let ms = start.elapsed().as_secs_f32() * 1000.0;
		if ms >= 0.05 {
			push(kind, start, &[ms]);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The sink, the file and `ON` are process globals: the two tests that use them run one after the other (review N6).
	static GLOBALS: Mutex<()> = Mutex::new(());

	fn globals() -> std::sync::MutexGuard<'static, ()> {
		GLOBALS.lock().unwrap_or_else(|e| e.into_inner())
	}

	/// Bytes waiting in the sink's buffer (0 without a sink).
	fn buffered() -> usize {
		SINK.get().map_or(0, |s| s.lock().unwrap_or_else(|e| e.into_inner()).buf.len())
	}

	/// The file format tools/frames.py reads: kind, start, then the fields, three decimals, one line each.
	#[test]
	fn lines_have_the_documented_format() {
		assert_eq!(line('F', 12.5, &[5.25, 1.0, 0.1, 0.9, 0.06]), "F 12.500 5.250 1.000 0.100 0.900 0.060\n");
		assert_eq!(line('P', 0.0, &[0.1234]), "P 0.000 0.123\n");
		assert_eq!(line('C', 1000.0005, &[]), "C 1000.000\n");
	}

	/// Review R2 F8: the rows of the last second were lost at exit. `flush` (viewer/startup.rs, after the event loop) writes them.
	#[test]
	fn flush_writes_the_rows_not_yet_written() {
		let _g = globals();
		let path = std::env::temp_dir().join(format!("rac_frame_csv_test_{}.txt", std::process::id()));
		assert!(open(path.to_str().unwrap()));
		cap(Instant::now(), 2.5);
		present(Instant::now(), 0.5);
		assert_eq!(std::fs::read_to_string(&path).unwrap(), "", "under a second: still buffered");
		flush();
		ON.store(false, Ordering::Relaxed);
		let text = std::fs::read_to_string(&path).unwrap();
		let _ = std::fs::remove_file(&path);
		let kinds: Vec<&str> = text.lines().map(|l| &l[..1]).collect();
		assert_eq!(kinds, ["C", "P"]);
		assert!(text.lines().next().unwrap().ends_with(" 2.500"));
	}

	/// Off by default: without a file every hook returns at once and writes nothing.
	#[test]
	fn off_writes_nothing() {
		let _g = globals();
		ON.store(false, Ordering::Relaxed);
		let before = buffered();
		frame(Instant::now(), 1.0, 0, 0.0, 0.0, 0.0);
		present(Instant::now(), 1.0);
		cap(Instant::now(), 1.0);
		wait('G', Instant::now() - std::time::Duration::from_millis(5));
		assert_eq!(buffered(), before, "a hook with the CSV off must not add a row");
	}
}
