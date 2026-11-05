//! [PC] The mod log queue. A mod's `omni.log` line reaches the game through the loader's `log` callback (host.rs `log_cb`) on the
//! game's own thread, in the middle of a tick. Written straight with `eprintln!` it cost about 1 ms per line when stderr is a file
//! (a test run's err.txt, the player's run log): Rust's unbuffered stderr makes one WriteFile call for each piece of the line
//! (about 7), and each call took ~150 us. A mod that logged a few lines a tick lost a tenth of the tick to it.
//!
//! Now `log_cb` only appends the line to a pending text and wakes a writer thread (`omnilog`), which writes everything pending in ONE
//! call. The queued lines keep their order among themselves; the loader's own `omni:` lines and every other subsystem's stderr lines are
//! still written at once, so a queued line can reach the log after a direct line printed later. A normal way out of the process loses
//! nothing: `flush` writes what is pending on the calling thread (the panic hook, a normal exit, the exits of the test aids). The Windows
//! crash filter (runlog.rs) cannot wait for any lock: `drain_raw` hands over what is pending there, and a batch the writer has already
//! taken but not yet written can still be lost, as can a line in a hard kill of the process.
use std::io::Write;
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};

type Sink = Box<dyn Fn(&str) + Send + Sync>;

/// Pending text plus the writer's wake-up. Lock order: `pending`, then `writing`, everywhere (a batch is taken while both are
/// held, so two writers cannot swap the order of two batches).
pub struct LogQueue {
	/// Whole lines, each ending in a newline, not written yet.
	pending: Mutex<String>,
	wake: Condvar,
	/// Held while a batch goes to the sink.
	writing: Mutex<()>,
	sink: Sink,
	/// The writer thread runs; without it `line` writes at once.
	threaded: OnceLock<bool>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
	m.lock().unwrap_or_else(|e| e.into_inner())
}

impl LogQueue {
	pub fn new(sink: Sink) -> LogQueue {
		LogQueue { pending: Mutex::new(String::new()), wake: Condvar::new(), writing: Mutex::new(()), sink, threaded: OnceLock::new() }
	}

	/// Starts the writer thread. False (and `line` stays synchronous) if the thread cannot be made.
	pub fn start(&'static self) -> bool {
		let ok = std::thread::Builder::new().name("omnilog".into()).spawn(move || self.run()).is_ok();
		let _ = self.threaded.set(ok);
		ok
	}

	/// Queues one line (the newline is added here).
	pub fn line(&self, text: &str) {
		{
			let mut p = lock(&self.pending);
			p.push_str(text);
			p.push('\n');
		}
		if self.threaded.get().copied().unwrap_or(false) {
			self.wake.notify_one();
		} else {
			self.flush();
		}
	}

	/// Writes everything pending on the calling thread, after any batch the writer is writing.
	pub fn flush(&self) {
		let mut p = lock(&self.pending);
		let w = lock(&self.writing);
		let batch = std::mem::take(&mut *p);
		drop(p);
		if !batch.is_empty() {
			(self.sink)(&batch);
		}
		drop(w);
	}

	/// For the crash filter: what is pending goes to `write` with no waiting. A queue locked by the crashing thread itself, or by the writer for the
	/// instant it takes a batch, is skipped.
	pub fn drain_raw(&self, write: fn(&str)) {
		let Ok(mut p) = self.pending.try_lock() else { return };
		let batch = std::mem::take(&mut *p);
		drop(p);
		if !batch.is_empty() {
			write(&batch);
		}
	}

	fn run(&self) {
		loop {
			let mut p = lock(&self.pending);
			while p.is_empty() {
				p = self.wake.wait(p).unwrap_or_else(|e| e.into_inner());
			}
			let w = lock(&self.writing);
			let batch = std::mem::take(&mut *p);
			drop(p);
			(self.sink)(&batch);
			drop(w);
		}
	}
}

/// The process-wide queue, writing to stderr (the run log, a test's err.txt, a console).
static QUEUE: OnceLock<&'static LogQueue> = OnceLock::new();

fn stderr_sink(text: &str) {
	// One call for the whole batch; a missing stderr (no console, no run log) is not an error for Rust's handle.
	let _ = std::io::stderr().lock().write_all(text.as_bytes());
}

fn queue() -> &'static LogQueue {
	QUEUE.get_or_init(|| {
		let q: &'static LogQueue = Box::leak(Box::new(LogQueue::new(Box::new(stderr_sink))));
		q.start();
		q
	})
}

/// One line of the loader or a mod for stderr (host.rs `log_cb`).
pub fn line(text: &str) {
	queue().line(text);
}

/// Writes the pending lines now. For the panic hook and every normal way out of the process.
pub fn flush() {
	if let Some(q) = QUEUE.get() {
		q.flush();
	}
}

/// The pending lines to `write`, from the crash filter (runlog.rs): never waits.
pub fn drain_raw(write: fn(&str)) {
	if let Some(q) = QUEUE.get() {
		q.drain_raw(write);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::Arc;
	use std::time::{Duration, Instant};

	/// A queue whose sink records each write call; `delay` makes the sink slow like a file write under a virus scanner.
	fn rig(delay: Duration, threaded: bool) -> (&'static LogQueue, Arc<Mutex<Vec<String>>>) {
		let writes = Arc::new(Mutex::new(Vec::new()));
		let w = writes.clone();
		let q: &'static LogQueue = Box::leak(Box::new(LogQueue::new(Box::new(move |s: &str| {
			std::thread::sleep(delay);
			lock(&w).push(s.to_string());
		}))));
		if threaded {
			assert!(q.start());
		}
		(q, writes)
	}

	fn all(writes: &Arc<Mutex<Vec<String>>>) -> String {
		lock(writes).concat()
	}

	#[test]
	fn lines_come_out_whole_and_in_order() {
		let (q, writes) = rig(Duration::from_millis(1), true);
		for i in 0..200 {
			q.line(&format!("omni: [m] line {i}"));
		}
		q.flush();
		let want: String = (0..200).map(|i| format!("omni: [m] line {i}\n")).collect();
		assert_eq!(all(&writes), want);
	}

	#[test]
	fn a_slow_sink_does_not_slow_the_caller() {
		let (q, writes) = rig(Duration::from_millis(30), true);
		let t = Instant::now();
		for i in 0..100 {
			q.line(&format!("omni: [m] tick {i} something happened"));
		}
		let took = t.elapsed();
		// Written one by one this is 100 x 30 ms = 3 s.
		assert!(took < Duration::from_millis(100), "100 lines took {took:?}");
		eprintln!("logq: 100 lines queued in {took:?} ({:?} per line) with a 30 ms sink", took / 100);
		q.flush();
		assert_eq!(all(&writes).lines().count(), 100);
		// Lines that pile up while a write is on go out together (how many batches form depends on the machine's load: only a loose bound).
		assert!(lock(&writes).len() < 50, "{} write calls for 100 lines", lock(&writes).len());
	}

	#[test]
	fn the_writer_thread_writes_without_a_flush() {
		let (q, writes) = rig(Duration::ZERO, true);
		q.line("omni: warning [m] alone");
		let t = Instant::now();
		while all(&writes).is_empty() && t.elapsed() < Duration::from_secs(5) {
			std::thread::sleep(Duration::from_millis(2));
		}
		assert_eq!(all(&writes), "omni: warning [m] alone\n");
	}

	#[test]
	fn without_a_writer_thread_a_line_is_written_at_once() {
		let (q, writes) = rig(Duration::ZERO, false);
		q.line("a");
		assert_eq!(all(&writes), "a\n");
	}

	#[test]
	fn flush_from_two_threads_keeps_the_order() {
		let (q, writes) = rig(Duration::from_millis(2), true);
		let other = std::thread::spawn(move || {
			for _ in 0..50 {
				q.flush();
				std::thread::sleep(Duration::from_millis(1));
			}
		});
		for i in 0..300 {
			q.line(&format!("{i}"));
			if i % 50 == 0 {
				std::thread::sleep(Duration::from_millis(3));
			}
		}
		other.join().unwrap();
		q.flush();
		let want: String = (0..300).map(|i| format!("{i}\n")).collect();
		assert_eq!(all(&writes), want);
	}

	static RAW: Mutex<String> = Mutex::new(String::new());
	fn raw(s: &str) {
		lock(&RAW).push_str(s);
	}

	#[test]
	fn drain_raw_hands_over_the_pending_lines_and_never_waits() {
		// No writer thread and a sink that would stall: the pending text can only leave through `drain_raw`.
		let q: &'static LogQueue = Box::leak(Box::new(LogQueue::new(Box::new(|_: &str| {}))));
		let _ = q.threaded.set(true);
		q.line("one");
		q.line("two");
		q.drain_raw(raw);
		assert_eq!(*lock(&RAW), "one\ntwo\n");
		q.drain_raw(raw);
		assert_eq!(*lock(&RAW), "one\ntwo\n");
		// The crashing thread itself holds the pending lock: skipped, not a deadlock.
		q.line("three");
		let held = lock(&q.pending);
		q.drain_raw(raw);
		drop(held);
		assert_eq!(*lock(&RAW), "one\ntwo\n");
		q.drain_raw(raw);
		assert_eq!(*lock(&RAW), "one\ntwo\nthree\n");
	}
}
