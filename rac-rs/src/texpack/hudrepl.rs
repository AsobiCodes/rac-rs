//! [PC] Replacement images for the GPU capture's HUD / menu / text textures (lane tex3, B2a): the capture names a texture at run time
//! (`gsname::name_for_draw`) and asks `lookup` for the pack's image; the file is decoded off the render thread to RGBA8 and kept in a
//! `Slot`. A name resolves like the level textures do: the index first, then the PAL <-> USA twin names (`xmap`). BC7 files are
//! decoded (`bc7dec`); anything past `MAX_SIDE` or the memory budget of the decoded images is refused; the capture then keeps the
//! original texture. With no textures mod enabled (or `RAC_TEXPACK_HUD=0`) `enabled()` is false, no registry exists and no thread is
//! started: the capture never calls in.
//!
//! The image is on the engine's alpha scale (`dds::engine_alpha`, min(2a, 255)); the GPU side converts to the GS scale (a * 128 / 255).

use super::chain::{Mip, ReplTex};
use super::dds::Fmt;
use super::index::{PackFile, PackIndex};
use super::name::TexName;
use super::xmap::XMap;
use super::{packs, repl, xmap};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering::*};
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};

/// Largest side of a replacement image in texels (larger files are refused).
pub const MAX_SIDE: u32 = 4096;
/// Default budget of all decoded images together, in MB.
pub const BUDGET_MB: usize = 512;
/// Most names the registry remembers (a run that names more is not a HUD: the rest count as missing at once).
const MAX_NAMES: usize = 1 << 16;
/// Loader threads, started with the first file to decode.
const WORKERS: usize = 2;

/// Where a name's replacement stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
	/// A loader has the file queued or is decoding it.
	Pending,
	/// `Slot::image` is there.
	Ready,
	/// The pack has no file for the name (nor for a twin name).
	Missing,
	/// Refused: a side of 0 or past `MAX_SIDE`.
	TooBig,
	/// Refused: the file could not be read or decoded.
	Broken,
	/// Refused: the decoded images already fill the budget.
	OverBudget,
	/// The image went into the GPU atlas or the CPU tables (`Slot::release`): the CPU copy is gone and its budget is free again. A capture
	/// that lost its copy (an eviction) calls `Slot::rearm` and the file is decoded again.
	Released,
	/// Decoded but never taken by a capture, and dropped at a level load (`drop_unused`) so that unused images do not pile up. `lookup`
	/// and `Slot::rearm` decode it again on demand.
	Dropped,
}

impl State {
	fn code(self) -> u8 {
		self as u8
	}
	fn from_code(c: u8) -> State {
		[State::Pending, State::Ready, State::Missing, State::TooBig, State::Broken, State::OverBudget, State::Released, State::Dropped][c as usize]
	}
}

/// A decoded replacement: RGBA8, straight alpha on the engine scale.
pub struct HudImg {
	pub w: u32,
	pub h: u32,
	pub rgba: Vec<u8>,
}

/// One name's replacement. Cheap to poll from the render thread (one atomic load).
pub struct Slot {
	state: AtomicU8,
	img: Mutex<Option<Arc<HudImg>>>,
	/// The registry's counters, set with the image (so `release` can give the budget back).
	owner: OnceLock<Arc<Shared>>,
	/// The pack file (a slot with a file can be decoded again after `release` / `discard`).
	file: Option<PackFile>,
	/// Bytes the budget must have free before an `OverBudget` slot is decoded again (the refused image's size; 1 when the decode was
	/// refused before its size was known).
	over: AtomicUsize,
}

impl Slot {
	fn new(s: State, file: Option<PackFile>) -> Slot {
		Slot { state: AtomicU8::new(s.code()), img: Mutex::new(None), owner: OnceLock::new(), file, over: AtomicUsize::new(0) }
	}

	pub fn state(&self) -> State {
		State::from_code(self.state.load(Acquire))
	}

	/// The image once `state()` is `Ready` (a shared handle: the slot's own copy stays until `release`).
	pub fn image(&self) -> Option<Arc<HudImg>> {
		if self.state() == State::Ready { self.img.lock().unwrap().clone() } else { None }
	}

	/// The image is in the GPU atlas: drop the slot's CPU copy and give its bytes back to the registry budget. Idempotent.
	pub fn release(&self) {
		self.take_image(State::Released);
	}

	/// An image nobody took is dropped (`Registry::drop_unused`); a slot that is not `Ready` is left alone.
	fn discard(&self) {
		if self.state() == State::Ready {
			self.take_image(State::Dropped);
		}
	}

	fn take_image(&self, to: State) {
		if let Some(img) = self.img.lock().unwrap().take() {
			if let Some(sh) = self.owner.get() {
				sh.used.fetch_sub(img.rgba.len(), Relaxed);
			}
			self.finish(to);
		}
	}

	/// A `Released` or `Dropped` slot is queued for a loader again (the file is decoded anew, state `Pending`, then `Ready`); so is an
	/// `OverBudget` one once the budget has room for it (an image evicted from the atlas and decoded again while the registry was full is
	/// not lost for the session). True when this call queued it; false for any other state, a slot without a file or one never decoded.
	pub fn rearm(self: &Arc<Slot>) -> bool {
		let (Some(file), Some(sh)) = (self.file.clone(), self.owner.get()) else { return false };
		let from_gone = |s: State| self.state.compare_exchange(s.code(), State::Pending.code(), AcqRel, Acquire).is_ok();
		let room = || self.state() == State::OverBudget && sh.used.load(Relaxed).saturating_add(self.over.load(Relaxed)) <= sh.budget;
		if !(from_gone(State::Released) || from_gone(State::Dropped)) {
			if !(room() && from_gone(State::OverBudget)) {
				return false;
			}
			sh.refused.fetch_sub(1, Relaxed);
		}
		sh.queue.lock().unwrap().push_back(Job { slot: self.clone(), file });
		sh.cv.notify_one();
		true
	}

	fn finish(&self, s: State) {
		self.state.store(s.code(), Release);
	}
}

/// Counters of one registry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
	pub names: usize,
	pub ready: usize,
	pub missing: usize,
	pub refused: usize,
	pub bytes: usize,
}

struct Job {
	slot: Arc<Slot>,
	file: PackFile,
}

struct Shared {
	queue: Mutex<VecDeque<Job>>,
	cv: Condvar,
	closed: AtomicBool,
	used: AtomicUsize,
	budget: usize,
	ready: AtomicUsize,
	refused: AtomicUsize,
	errors: Mutex<Vec<String>>,
}

/// The names asked for so far and the loader threads behind them.
pub struct Registry {
	map: Mutex<HashMap<TexName, Arc<Slot>>>,
	sh: Arc<Shared>,
	started: AtomicBool,
	missing: AtomicU64,
}

impl Registry {
	/// An empty registry: no thread, no allocation beyond the struct. `budget`: bytes of decoded images it may hold.
	pub fn new(budget: usize) -> Registry {
		let sh = Shared { queue: Mutex::new(VecDeque::new()), cv: Condvar::new(), closed: AtomicBool::new(false), used: AtomicUsize::new(0), budget, ready: AtomicUsize::new(0), refused: AtomicUsize::new(0), errors: Mutex::new(Vec::new()) };
		Registry { map: Mutex::new(HashMap::new()), sh: Arc::new(sh), started: AtomicBool::new(false), missing: AtomicU64::new(0) }
	}

	/// The slot of `name`: from the map, else resolved against `index` and the `xmap` twins. A name with no file is `Missing` at once
	/// (no thread); one with a file is queued for a loader.
	pub fn lookup(&self, name: &TexName, index: &PackIndex, twins: &XMap) -> Arc<Slot> {
		let mut map = self.map.lock().unwrap();
		if let Some(s) = map.get(name) {
			if matches!(s.state(), State::Dropped | State::OverBudget) {
				s.rearm();
			}
			return s.clone();
		}
		if map.len() >= MAX_NAMES {
			self.missing.fetch_add(1, Relaxed);
			return Arc::new(Slot::new(State::Missing, None));
		}
		let file = resolve(index, twins, name);
		let slot = Arc::new(Slot::new(if file.is_some() { State::Pending } else { State::Missing }, file.clone()));
		if file.is_none() {
			self.missing.fetch_add(1, Relaxed);
		}
		map.insert(*name, slot.clone());
		drop(map);
		if let Some(file) = file {
			self.sh.queue.lock().unwrap().push_back(Job { slot: slot.clone(), file });
			self.sh.cv.notify_one();
			self.start_workers();
		}
		slot
	}

	/// Level load: every decoded image no capture took is dropped (`State::Dropped`), so images prefetched for an earlier level (or never
	/// drawn) do not stay resident or fill the budget; asking for the name again decodes it again. Returns how many were dropped.
	pub fn drop_unused(&self) -> usize {
		let map = self.map.lock().unwrap();
		map.values().filter(|s| s.state() == State::Ready).map(|s| s.discard()).count()
	}

	fn start_workers(&self) {
		if self.started.swap(true, AcqRel) {
			return;
		}
		for i in 0..WORKERS {
			let sh: Weak<Shared> = Arc::downgrade(&self.sh);
			let _ = std::thread::Builder::new().name(format!("texpack-hud-{i}")).spawn(move || worker(sh));
		}
	}

	/// Whether any loader thread was started.
	pub fn workers_started(&self) -> bool {
		self.started.load(Acquire)
	}

	pub fn stats(&self) -> Stats {
		Stats { names: self.map.lock().unwrap().len(), ready: self.sh.ready.load(Relaxed), missing: self.missing.load(Relaxed) as usize, refused: self.sh.refused.load(Relaxed), bytes: self.sh.used.load(Relaxed) }
	}

	/// The first few decode errors (for the exit report).
	pub fn errors(&self) -> Vec<String> {
		self.sh.errors.lock().unwrap().clone()
	}
}

impl Drop for Registry {
	fn drop(&mut self) {
		self.sh.closed.store(true, Release);
		// Taking the queue lock first: a loader between its `closed` check and its wait cannot miss the wake-up.
		drop(self.sh.queue.lock());
		self.sh.cv.notify_all();
	}
}

/// The pack file of `name`: the index first, then each twin name (PAL <-> USA) in the index.
pub fn resolve(index: &PackIndex, twins: &XMap, name: &TexName) -> Option<PackFile> {
	index.get(name).or_else(|| twins.twins(name).iter().find_map(|t| index.get(t))).cloned()
}

fn worker(sh: Weak<Shared>) {
	loop {
		let Some(s) = sh.upgrade() else { return };
		let job = {
			let mut q = s.queue.lock().unwrap();
			loop {
				if let Some(j) = q.pop_front() {
					break Some(j);
				}
				if s.closed.load(Acquire) {
					break None;
				}
				q = s.cv.wait(q).unwrap();
			}
		};
		match job {
			Some(j) => run(&s, j),
			None => return,
		}
	}
}

fn run(sh: &Arc<Shared>, job: Job) {
	let res = decode(sh, &job.file);
	match res {
		Ok(img) => {
			let bytes = img.rgba.len();
			if sh.used.fetch_add(bytes, Relaxed) + bytes > sh.budget {
				sh.used.fetch_sub(bytes, Relaxed);
				sh.refused.fetch_add(1, Relaxed);
				job.slot.over.store(bytes, Relaxed);
				let _ = job.slot.owner.set(sh.clone());
				job.slot.finish(State::OverBudget);
				return;
			}
			*job.slot.img.lock().unwrap() = Some(Arc::new(img));
			let _ = job.slot.owner.set(sh.clone());
			sh.ready.fetch_add(1, Relaxed);
			job.slot.finish(State::Ready);
		}
		Err((s, msg)) => {
			sh.refused.fetch_add(1, Relaxed);
			let mut e = sh.errors.lock().unwrap();
			if e.len() < 3 {
				e.push(msg);
			}
			if s == State::OverBudget {
				job.slot.over.store(1, Relaxed);
				let _ = job.slot.owner.set(sh.clone());
			}
			job.slot.finish(s);
		}
	}
}

/// Level 0 of the file as RGBA8, or the refusal.
fn decode(sh: &Shared, f: &PackFile) -> Result<HudImg, (State, String)> {
	if sh.used.load(Relaxed) >= sh.budget {
		return Err((State::OverBudget, format!("texpack hud: the {} MB budget is full, {} left as it is", sh.budget >> 20, f.path.display())));
	}
	let name = || f.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
	// The header says the size: an oversized file is refused before its pixels are decoded.
	if let Some((w, h, _)) = repl::peek_file(f) {
		if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
			return Err((State::TooBig, format!("{}: {w}x{h} is outside 1..{MAX_SIDE}", name())));
		}
	}
	let img = repl::load_file(f).map_err(|e| (State::Broken, e))?;
	if img.w == 0 || img.h == 0 || img.w > MAX_SIDE || img.h > MAX_SIDE {
		return Err((State::TooBig, format!("{}: {}x{} is outside 1..{MAX_SIDE}", name(), img.w, img.h)));
	}
	let (w, h) = (img.w, img.h);
	let data = img.levels.into_iter().next().ok_or_else(|| (State::Broken, format!("{}: no pixel data", name())))?;
	let tex = ReplTex { fmt: img.fmt, mips: vec![Mip { w, h, data }] };
	let mut rgba = tex.to_rgba8().map_err(|e| (State::Broken, format!("{}: {e}", name())))?;
	let m = rgba.mips.swap_remove(0);
	if m.data.len() != w as usize * h as usize * 4 {
		return Err((State::Broken, format!("{}: {} bytes for {w}x{h}", name(), m.data.len())));
	}
	Ok(HudImg { w, h, rgba: m.data })
}

/// The switch value `RAC_TEXPACK_HUD` (`"0"` = off) with whether any pack is enabled.
pub fn enabled_for(has_packs: bool, env: Option<&str>) -> bool {
	has_packs && env != Some("0")
}

/// Whether the capture should look names up: a textures mod is enabled and `RAC_TEXPACK_HUD` is not `0`. Read once.
pub fn enabled() -> bool {
	static ON: OnceLock<bool> = OnceLock::new();
	*ON.get_or_init(|| enabled_for(packs().is_some(), std::env::var("RAC_TEXPACK_HUD").ok().as_deref()))
}

static REG: OnceLock<Registry> = OnceLock::new();

/// What the capture's prim hook (`gscap/texrepl.rs`) did with the textured overlay prims, for the exit line.
#[derive(Clone, Copy, Debug)]
pub enum Count {
	/// A textured overlay prim the hook looked at (formats it refuses are not counted).
	Prims,
	/// ... that was drawn with the pack's image.
	Replaced,
	/// ... whose image was still being decoded (drawn with the original).
	Pending,
	/// ... whose name the pack has no file for, or a file that was refused.
	Missing,
	/// ... whose image did not fit the atlas.
	Refused,
	/// Of the refusals (counted per try, not per prim): the image is larger than the whole atlas (or wrong), so no eviction helps.
	Never,
	/// Of the refusals: no room right now (eviction may help).
	Full,
	/// Images (GPU atlas) or tables (CPU capture) evicted at field ends.
	Evicted,
	/// Field ends that evicted (room had been refused).
	Evictions,
	/// Slots the atlas lost the image of and that decode again.
	Redecoded,
}

static COUNTS: [AtomicU64; 10] = [const { AtomicU64::new(0) }; 10];

/// Counts one prim (a relaxed add; only called with a pack enabled).
pub fn count(c: Count) {
	COUNTS[c as usize].fetch_add(1, Relaxed);
}

/// Counts `n` at once (the evictions of one field end).
pub fn count_n(c: Count, n: u64) {
	COUNTS[c as usize].fetch_add(n, Relaxed);
}

/// The counters in the order of `Count`.
pub fn counts() -> [u64; 10] {
	std::array::from_fn(|i| COUNTS[i].load(Relaxed))
}

/// The `texrepl:` exit line (None when no prim was looked at).
pub fn count_line() -> Option<String> {
	let [p, r, pe, m, f, nv, fu, ev, evn, rd] = counts();
	(p > 0).then(|| {
		format!(
			"texrepl: {p} textured overlay prims, {r} replaced ({:.1} %), {pe} pending, {m} missing, {f} refused ({nv} too big for the atlas, {fu} no room); {ev} evicted in {evn} field ends, {rd} decoded again",
			r as f64 * 100.0 / p as f64
		)
	})
}

/// The process registry, made at the first lookup (never when `enabled()` is false).
fn registry() -> &'static Registry {
	REG.get_or_init(|| Registry::new(BUDGET_MB << 20))
}

/// Bytes of decoded images the registry holds right now (0 before the first lookup): what the prefetch keeps under its cap.
pub fn used_bytes() -> usize {
	REG.get().map_or(0, |r| r.stats().bytes)
}

/// The slot of `name` in the active packs. With `enabled()` false this is a shared `Missing` slot and nothing else happens.
pub fn lookup(name: &TexName) -> Arc<Slot> {
	match packs() {
		Some(p) if enabled() => registry().lookup(name, &p.index, xmap::shipped()),
		_ => disabled_slot(),
	}
}

/// A slot with no file and no registry (tests of the capture's bookkeeping).
#[cfg(test)]
pub fn test_slot() -> Arc<Slot> {
	Arc::new(Slot::new(State::Missing, None))
}

fn disabled_slot() -> Arc<Slot> {
	static OFF: OnceLock<Arc<Slot>> = OnceLock::new();
	OFF.get_or_init(|| Arc::new(Slot::new(State::Missing, None))).clone()
}

/// Level load (before the level's prefetch): drops the decoded images no capture took (`Registry::drop_unused`). Nothing without a registry.
pub fn drop_unused() {
	if let Some(r) = REG.get() {
		let n = r.drop_unused();
		if n > 0 {
			eprintln!("texpack hud: {n} unused decoded images dropped at the level load");
		}
	}
}

/// One exit line (None when no lookup ever ran): `texpack hud: N names, R ready (X MB), M missing, F refused`.
pub fn report() -> Option<String> {
	let r = REG.get()?;
	let s = r.stats();
	let mut line = format!("texpack hud: {} names, {} ready ({:.1} MB), {} missing, {} refused", s.names, s.ready, s.bytes as f64 / 1048576.0, s.missing, s.refused);
	for e in r.errors() {
		line.push_str(&format!("\ntexpack hud: {e}"));
	}
	if let Some(c) = count_line() {
		line.push('\n');
		line.push_str(&c);
	}
	Some(line)
}

#[cfg(test)]
#[path = "hudrepl_tests.rs"]
pub(super) mod tests;
