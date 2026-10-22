//! Speed lane A: the effects layer's picture work (`efx_draw::capture` + `efx_vu::Picker::prep`, which copy the field's
//! packets and the VIF1 chain out of RAM) runs on its own thread, while the game thread already runs the next field.
//!
//! Hand-over, per field, on the worker (the `retail` thread) after `Machine::frame` returned (the game thread is parked):
//!  1. wait until the picture thread has given the ghost machine back (it is done reading the previous field's RAM),
//!  2. `Machine::mirror_to`: copy into the ghost the RAM pages written since the last sync (the OS write watch,
//!     `ee_oracle::mem::watch`), the scratchpad and the GS registers: these are all `capture` and `prep` read,
//!  3. send the job (ghost + the field's owners, level, VIF1 jobs).
//! The picture thread runs capture + prep on the ghost, returns the ghost, then waits for the worker's reply flag
//! (`snap()` built an effects layer this field) and submits the work to the VU1 thread. The receiver of the VU1 thread's
//! answer travels back to the worker as `Handle::finish`'s result and is resolved by `Retail::tick` (`FxLate::Later`).
//! The picture input is the same bytes as the old inline path: the ghost holds the RAM the worker would have read.
//!
//! RAC_SNAP_PIPE=0 keeps the old inline path. RAC_SNAP_CHECK=1 also runs the old path on the live machine every field and
//! compares: the ghost's RAM / scratchpad / GS registers against the live ones, and the hash of the capture and of the
//! walk (`BuildIn` Debug text, `Work::sig`); mismatches are logged ("snappipe CHECK"). RAC_PERF_LOG=1 prints the timing
//! every 100 fields ("snappipe:").
use super::efx_draw;
use super::efx_own::Own;
use super::efx_vu::{FxOut, Pending, Picker};
use ee_oracle::Machine;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Instant;

/// RAC_SNAP_PIPE=0 off, any other value on (even where the write watch is slow or missing: the sync then copies more or all of
/// RAM, still exact). Unset: on only where the OS write watch is cheap (native Windows), off on Wine / Proton and non-Windows
/// builds (the old inline path). One stderr line says which path runs.
pub fn enabled() -> bool {
	static E: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*E.get_or_init(|| {
		let wine = ee_oracle::mem::watch::is_wine();
		let (on, why) = match std::env::var("RAC_SNAP_PIPE") {
			Ok(v) if v == "0" => (false, "RAC_SNAP_PIPE=0"),
			Ok(_) => (true, "RAC_SNAP_PIPE forced on"),
			Err(_) if ee_oracle::mem::watch::available() => (false, "native Windows, write watch available but opt-in: RAC_SNAP_PIPE=1; the parallel prep (RAC_SNAP_PAR) is the default"),
			Err(_) if wine => (false, "Wine/Proton: the write watch is emulated with page faults"),
			Err(_) => (false, "no OS write watch on this platform"),
		};
		eprintln!("snappipe: {} ({why})", if on { "picture thread on" } else { "inline picture path" });
		on
	})
}

fn check_on() -> bool {
	static E: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*E.get_or_init(|| std::env::var("RAC_SNAP_CHECK").is_ok_and(|v| v == "1"))
}

struct Ghost(Machine);
// The ghost never runs: no game thread, its pointers are only read by the thread that holds it.
unsafe impl Send for Ghost {}

struct Job {
	ghost: Ghost,
	own: Vec<Own>,
	lv: u32,
	field: u32,
	pending: Pending,
	reply: Receiver<bool>,
	out: Sender<Option<Receiver<FxOut>>>,
	/// RAC_SNAP_CHECK: hashes of the old path's capture and walk.
	old: Option<(u64, u64)>,
}

/// The picture thread's busy time (ns, summed since the last print).
static BUSY: AtomicU64 = AtomicU64::new(0);
static BAD: AtomicU64 = AtomicU64::new(0);

pub type Late = Receiver<Option<Receiver<FxOut>>>;

pub struct Handle {
	reply: Sender<bool>,
	out: Late,
}

impl Handle {
	/// `snap()` is done: whether it built an effects layer. The result resolves on the main thread.
	pub fn finish(self, fx: bool) -> Late {
		let _ = self.reply.send(fx);
		self.out
	}
}

pub struct Pipe {
	tx: Sender<Job>,
	free: Receiver<Ghost>,
	fresh: bool,
	old: Option<Picker>,
	n: u32,
	wait_ns: u64,
	sync_ns: u64,
	bytes: u64,
	bad_view: u32,
}

fn fnv(b: &[u8]) -> u64 {
	b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &x| (h ^ x as u64).wrapping_mul(0x0100_0000_01b3))
}

impl Pipe {
	/// `picker` moves to the picture thread; with RAC_SNAP_CHECK `old` is the worker's own picker (the old inline path).
	pub fn new(picker: Picker, old: Option<Picker>) -> Option<Pipe> {
		let (tx, rx) = channel::<Job>();
		let (ftx, free) = channel::<Ghost>();
		ftx.send(Ghost(Machine::ghost())).ok()?;
		std::thread::Builder::new().name("snap".into()).stack_size(16 << 20).spawn(move || run(rx, ftx, picker)).ok()?;
		Some(Pipe { tx, free, fresh: true, old, n: 0, wait_ns: 0, sync_ns: 0, bytes: 0, bad_view: 0 })
	}

	/// Syncs the ghost and hands the field to the picture thread. None: the thread is gone (no effects layer this field).
	pub fn hand(&mut self, m: &Machine, own: &[Own], lv: u32, field: u32, pending: Pending) -> Option<Handle> {
		let t = Instant::now();
		let w = crate::runlog::waiting("a free picture-thread buffer (snap pipe)");
		let mut g = self.free.recv().ok()?;
		drop(w);
		self.wait_ns += t.elapsed().as_nanos() as u64;
		let t = Instant::now();
		self.bytes += m.mirror_to(&mut g.0, &mut self.fresh) as u64;
		self.sync_ns += t.elapsed().as_nanos() as u64;
		let mut old = None;
		if let Some(pk) = self.old.as_mut() {
			if !m.same_view(&g.0) {
				self.bad_view += 1;
				BAD.fetch_add(1, Ordering::Relaxed);
				eprintln!("snappipe CHECK field {field}: ghost RAM/scratchpad/GS differs from the live machine");
			}
			let b = efx_draw::capture(m, lv, own);
			let w = pk.prep(m, own, &b, pending.clone());
			old = Some((fnv(format!("{b:?}").as_bytes()), w.sig()));
		}
		let (rtx, reply) = channel();
		let (out_tx, out) = channel();
		self.tx.send(Job { ghost: g, own: own.to_vec(), lv, field, pending, reply, out: out_tx, old }).ok()?;
		self.n += 1;
		if self.n % 100 == 0 && std::env::var("RAC_PERF_LOG").is_ok_and(|v| v == "1") {
			let f = |v: u64| v as f64 / 1e6 / 100.0;
			eprintln!("snappipe: field {field}: per field wait-for-ghost {:.2} ms, sync {:.2} ms ({} KB), picture thread busy {:.2} ms (check mismatches {} view {})", f(self.wait_ns), f(self.sync_ns), self.bytes / 100 / 1024, f(BUSY.swap(0, Ordering::Relaxed)), BAD.load(Ordering::Relaxed), self.bad_view);
			(self.wait_ns, self.sync_ns, self.bytes) = (0, 0, 0);
		}
		Some(Handle { reply: rtx, out })
	}
}

fn run(rx: Receiver<Job>, free: Sender<Ghost>, mut picker: Picker) {
	while let Ok(j) = rx.recv() {
		let t = Instant::now();
		let bi = efx_draw::capture(&j.ghost.0, j.lv, &j.own);
		let w = picker.prep(&j.ghost.0, &j.own, &bi, j.pending);
		if let Some((sb, sw)) = j.old {
			let (nb, nw) = (fnv(format!("{bi:?}").as_bytes()), w.sig());
			if (nb, nw) != (sb, sw) {
				BAD.fetch_add(1, Ordering::Relaxed);
				eprintln!("snappipe CHECK field {}: picture input differs (capture old {sb:016x} new {nb:016x}, walk old {sw:016x} new {nw:016x})", j.field);
			}
		}
		BUSY.fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
		let _ = free.send(j.ghost);
		let fx = j.reply.recv().unwrap_or(false);
		let r = picker.submit(w, bi, j.field, fx);
		let _ = j.out.send(r);
	}
}

/// For `Pipe::new` callers: RAC_SNAP_CHECK on.
pub fn check() -> bool {
	check_on()
}

// ---- Parallel prep (portable, no OS support needed) -------------------------------------------------------------------
//
// While the worker builds the snapshot (`snap()`), which only reads the machine, a helper thread runs capture + the chain walk
// (`Picker::prep`), which only read the machine too: the EE thread is parked at the field's end the whole time, so nothing
// writes. The worker joins the helper before it touches the machine, the picker or `own` again (the channel is the
// happens-before). Same bytes as the inline path, so exact; it needs no write watch, so it also runs under Wine / Proton,
// Linux and Android. `Tuner` measures both ways on the real machine (the helper may sit on a slow core of a big.LITTLE phone,
// or compete with the other threads of a 4-core PC) and keeps the faster one.
// RAC_SNAP_PAR=0 never, =1 always, unset: automatic.

struct ParJob {
	m: *const Machine,
	picker: *mut Picker,
	own: Vec<Own>,
	lv: u32,
	pending: Pending,
	sent: Instant,
}
/// Helper timings of the last job (ns): wake-up latency, capture, walk (RAC_PERF_LOG spike print).
pub static HS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
// SAFETY: the pointers are only used while the worker waits in `Par::join` and neither the machine (shared reads only) nor the
// picker (exclusive to the helper then) is used by the worker.
unsafe impl Send for ParJob {}

pub struct Par {
	tx: Sender<ParJob>,
	rx: Receiver<(efx_draw::BuildIn, super::efx_vu::Work)>,
	started: bool,
	tuner: Tuner,
	/// RAC_SNAP_CHECK: the inline path's own picker and the field's pending jobs; `join` runs the inline walk too and compares.
	old: Option<(Picker, Pending)>,
	own: Vec<Own>,
	lv: u32,
	bad: u32,
}

/// Which way runs each field: after a start-up guess it times both ways on alternate fields for a short trial, picks the
/// faster median and repeats the trial now and then (the load of a PC changes: a new level, other programs).
struct Tuner {
	n: u64,
	t: [Vec<u64>; 2],
	pick: usize,
	force: Option<usize>,
	cur: usize,
	log: bool,
}

const TRIAL: u64 = 48;
const PERIOD: u64 = 1200;

impl Tuner {
	fn new() -> Tuner {
		let hw = std::thread::available_parallelism().map_or(1, |n| n.get());
		let force = match std::env::var("RAC_SNAP_PAR") {
			Ok(v) if v == "0" => Some(0),
			Ok(_) => Some(1),
			Err(_) => None,
		};
		Tuner { n: 0, t: [Vec::new(), Vec::new()], pick: if hw >= 4 { 1 } else { 0 }, force, cur: 0, log: std::env::var("RAC_PERF_LOG").is_ok_and(|v| v == "1") }
	}

	/// 1: this field runs capture + walk on the helper, 0: inline.
	fn mode(&mut self) -> usize {
		let k = self.n % PERIOD;
		self.cur = match self.force {
			Some(f) => f,
			None if k < TRIAL => (k % 2) as usize,
			None => self.pick,
		};
		self.cur
	}

	/// The wall time of the field's prep + snap() section.
	fn done(&mut self, ns: u64) {
		self.n += 1;
		if self.force.is_some() {
			return;
		}
		let k = (self.n - 1) % PERIOD;
		if k < TRIAL {
			// The first fields of a trial warm the threads up.
			if k >= 4 {
				self.t[self.cur].push(ns);
			}
			if k == TRIAL - 1 {
				let med = |v: &mut Vec<u64>| {
					v.sort_unstable();
					v.get(v.len() / 2).copied()
				};
				let (m0, m1) = (med(&mut self.t[0]), med(&mut self.t[1]));
				if let (Some(a), Some(b)) = (m0, m1) {
					self.pick = (b < a) as usize;
					if self.log {
						eprintln!("snappar: trial at field {}: inline {:.2} ms, helper thread {:.2} ms -> {}", self.n, a as f64 / 1e6, b as f64 / 1e6, if self.pick == 1 { "helper thread" } else { "inline" });
					}
				}
				self.t = [Vec::new(), Vec::new()];
			}
		}
	}
}

impl Par {
	pub fn new(old: Option<Picker>) -> Option<Par> {
		if std::env::var("RAC_SNAP_PAR").is_ok_and(|v| v == "0") {
			return None;
		}
		let (tx, jrx) = channel::<ParJob>();
		let (otx, rx) = channel();
		std::thread::Builder::new()
			.name("snap-prep".into())
			.stack_size(16 << 20)
			.spawn(move || {
				while let Ok(j) = jrx.recv() {
					// SAFETY: see `ParJob`.
					let (m, picker) = unsafe { (&*j.m, &mut *j.picker) };
					let t0 = Instant::now();
					let b = efx_draw::capture(m, j.lv, &j.own);
					let t1 = Instant::now();
					let w = picker.prep(m, &j.own, &b, j.pending);
					HS[0].store(t0.duration_since(j.sent).as_nanos() as u64, Ordering::Relaxed);
					HS[1].store(t1.duration_since(t0).as_nanos() as u64, Ordering::Relaxed);
					HS[2].store(t1.elapsed().as_nanos() as u64, Ordering::Relaxed);
					let _ = otx.send((b, w));
				}
			})
			.ok()?;
		let mut tuner = Tuner::new();
		if old.is_some() {
			// The check needs the helper's walk every field.
			tuner.force = Some(1);
		}
		Some(Par { tx, rx, started: false, tuner, old: old.map(|p| (p, Vec::new())), own: Vec::new(), lv: 0, bad: 0 })
	}

	/// True: this field's capture + walk go to the helper (`start`, then `join` after `snap()`).
	pub fn want(&mut self) -> bool {
		self.tuner.mode() == 1
	}

	/// Hands the walk to the helper. The machine, the picker and `own` must stay untouched (read-only for the machine) until `join`.
	pub fn start(&mut self, m: &Machine, own: &[Own], lv: u32, picker: &mut Picker, pending: Pending) -> bool {
		if let Some(o) = self.old.as_mut() {
			o.1 = pending.clone();
			self.own = own.to_vec();
			self.lv = lv;
		}
		let j = ParJob { m, picker, own: own.to_vec(), lv, pending, sent: Instant::now() };
		self.started = self.tx.send(j).is_ok();
		self.started
	}

	/// Waits for the helper (None: it is gone; the field has no effects layer).
	pub fn join(&mut self, m: &Machine, field: u32) -> Option<(efx_draw::BuildIn, super::efx_vu::Work)> {
		if !std::mem::take(&mut self.started) {
			return None;
		}
		let w = crate::runlog::waiting("the picture thread's helper (snap pipe)");
		let r = self.rx.recv().ok();
		drop(w);
		if let (Some((b, w)), Some((pk, pend))) = (r.as_ref(), self.old.as_mut()) {
			let b2 = efx_draw::capture(m, self.lv, &self.own);
			let w2 = pk.prep(m, &self.own, &b2, std::mem::take(pend));
			let (sb, nb) = (fnv(format!("{b2:?}").as_bytes()), fnv(format!("{b:?}").as_bytes()));
			let (sw, nw) = (w2.sig(), w.sig());
			if (sb, sw) != (nb, nw) {
				self.bad += 1;
				eprintln!("snappipe CHECK field {field}: parallel picture input differs (capture inline {sb:016x} helper {nb:016x}, walk inline {sw:016x} helper {nw:016x})");
			}
			if field % 500 == 0 {
				eprintln!("snappipe: parallel check field {field}: mismatches {}", self.bad);
			}
		}
		r
	}

	/// The field's prep + snap() section took `ns` wall time (either way).
	pub fn done(&mut self, ns: u64) {
		self.tuner.done(ns);
	}
}

/// RAC_PERF_LOG=1: every 100 calls (one per field, any path) the picture half's section timers (per field, ms).
pub fn report(field: u32) {
	static N: AtomicU64 = AtomicU64::new(0);
	if !std::env::var("RAC_PERF_LOG").is_ok_and(|v| v == "1") {
		return;
	}
	if N.fetch_add(1, Ordering::Relaxed) % 100 != 99 {
		return;
	}
	let g = |i: usize| super::efx_vu::SP[i].swap(0, Ordering::Relaxed) as f64;
	let (cap, cen, pl, bl, di) = (g(0) / 1e8, g(1) / 1e8, g(2) / 1e8, g(3) / 1e8, g(4) / 1e8);
	let (w, nd, tags) = (g(5) / 100.0, g(6), g(7) / 100.0);
	eprintln!("snapsec: field {field}: per field capture {cap:.2} census {cen:.2} plan {pl:.2} copy {bl:.2} discovery {di:.2} ms | copied {:.0} KB, {tags:.0} tags, {nd:.0} discovery runs/100", w * 4.0 / 1024.0);
}
