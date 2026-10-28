//! The replacement textures of one level and the worker threads that load them (lane tex, S2). `level_loaded` makes a `LevelRepl`
//! from the pack hits: per world texture (texture key) up to 4 files (one per original mip level), decoded off the render thread into
//! a `ReplTex` chain. The scene build uploads what is ready and keeps the original for the rest; the rest is swapped in later
//! (`viewer/gpu/repl_upload.rs`). The CPU copies live as long as the `LevelRepl` (level unload drops them, workers stop on their own).

use super::chain::{self, ReplTex};
use super::dds::{self, Image};
use super::guard;
use super::index::PackFile;
use super::name::Ext;
use crate::level::Texture;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

/// Identity of the original texture a replacement was named for: size and XXH3 of its RGBA. Keys are shared with other textures
/// (the warp actors' keys 0..2, ship and gadget textures), so an upload only takes a replacement whose original it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fp {
	pub w: u32,
	pub h: u32,
	pub hash: u64,
}

impl Fp {
	pub fn of(t: &Texture) -> Fp {
		Fp { w: t.w as u32, h: t.h as u32, hash: xxhash_rust::xxh3::xxh3_64(bytemuck::cast_slice(&t.rgba)) }
	}

	/// Cheap test first (size), the hash only when the sizes agree.
	pub fn is(&self, t: &Texture) -> bool {
		self.w == t.w as u32 && self.h == t.h as u32 && *self == Fp::of(t)
	}
}

/// Load state of one replacement.
pub enum Cpu {
	Pending,
	Ready(Arc<ReplTex>),
	Failed,
}

/// One replaced texture: the files, the decoded chain, and the GPU textures made from it ([0] with the authored chain, [1] as a
/// texture without authored levels, from the plain chain).
///
/// Two chains per entry (the budget, `RAC_TEXPACK_MB`, charges both, the plain one only while it exists):
/// - `cpu` = the MAIN chain, kept until the level is dropped. Exactly what v0.4.0 kept: `chain::assemble(files, n)`, the authored chain
///   of `n` levels (n = 1 for most textures: level 0 alone), in the file's format (BC stays BC). Admission, the BC3 re-encode rule and
///   the plan look at the main chains only.
/// - `plain` = the PLAIN (upload) chain, a transient hand-off for the slots without an authored chain (every slot of an `n < 2`
///   texture): a full chain down to 1x1 made from the ORIGINAL level-0 file (`chain::plain_of`: the file's own levels when it has all of
///   them, else RGBA8 with box-filtered levels), never from the main chain, so it is the same whatever the thread timing and whatever the
///   main chain was stored as (BC3 included). The loader builds it AFTER the main chain was accepted, `SceneCtx::repl_bind` uploads it once
///   and drops it (the GPU texture is cached in `gpu[1]`). It is dropped without upload when a later main chain needs the room
///   (`LevelRepl::shed_plains`) and when the scene build has bound every slot (`LevelRepl::release_plains`); never made for a main chain
///   that failed or when it cannot fit. If it is missing when a slot needs it (`plain_for_bind`) nothing is decoded on the calling
///   thread: the slot keeps its current picture, a loader thread rebuilds the same chain from the file (`LevelRepl::queue_plain`) and
///   the slot is swapped in when it is ready (`Gpu::swap_repl`).
pub struct Entry {
	pub key: u32,
	pub fp: Fp,
	/// Levels of the original's chain (1..=4).
	pub n: usize,
	files: [Option<PackFile>; 4],
	pub cpu: Mutex<Cpu>,
	pub gpu: Mutex<[Option<wgpu::Texture>; 2]>,
	plain: Mutex<Option<Arc<ReplTex>>>,
	/// `P_NONE`, `P_QUEUED` (a loader thread is to rebuild the plain chain) or `P_FAILED` (it cannot be built: the slot keeps the original).
	pstate: AtomicU8,
	/// The held plain chain was rebuilt on request: it is not shed for room (it is waiting for the upload that asked for it).
	pinned: AtomicBool,
	/// No slot of the finished scene build uses this key (its slots were in a prop pass that `build_scene` threw away): no plain chain is
	/// rebuilt or held for it, so an upload that never comes cannot keep rebuilt chains in `REBUILT` (`LevelRepl::unwant`). Cleared when a
	/// slot asks for the chain again (`LevelRepl::queue_plain`).
	unwanted: AtomicBool,
}

const P_NONE: u8 = 0;
const P_QUEUED: u8 = 1;
const P_FAILED: u8 = 2;

impl Entry {
	/// The plain chain a slot without an authored chain is uploaded from: `Ok(Some)` while the loader holds it; `Ok(None)` when it is not
	/// held, after a loader thread was asked to rebuild it (the caller keeps the slot's current picture and asks again later); `Err` when
	/// it cannot be built. Never decodes or filters anything itself (render thread).
	pub fn plain_for_bind(&self, repl: &Arc<LevelRepl>) -> Result<Option<Arc<ReplTex>>, String> {
		if let Some(p) = self.plain.lock().unwrap().clone() {
			return Ok(Some(p));
		}
		if self.pstate.load(Relaxed) == P_FAILED {
			return Err(format!("key {:#x}: the upload chain cannot be built from the pack files", self.key));
		}
		repl.queue_plain(self.key);
		Ok(None)
	}

	/// Drops the plain chain and gives its bytes back to the budget. The entry lock is held over the subtractions, so a `shed_plains` that
	/// returns has seen every release finished.
	pub fn release_plain(&self) {
		let mut held = self.plain.lock().unwrap();
		if let Some(p) = held.take() {
			PLAIN.fetch_sub(p.bytes(), Relaxed);
			USED.fetch_sub(p.bytes(), Relaxed);
			if self.pinned.swap(false, Relaxed) {
				REBUILT.fetch_sub(p.bytes(), Relaxed);
			}
			let _ = self.pstate.compare_exchange(P_QUEUED, P_NONE, Relaxed, Relaxed);
		}
	}

	/// Holds `p` as the plain chain if the budget `limit` has room for it (charged to `USED` and `PLAIN` while it is held). The check, the
	/// two additions and the store are one step under `BUDGET`, so `LevelRepl::admit` never sees the counters half way.
	fn keep_plain(&self, p: ReplTex, limit: usize) -> bool {
		let b = p.bytes();
		let _budget = budget_lock();
		if USED.load(Relaxed) + b > limit {
			return false;
		}
		USED.fetch_add(b, Relaxed);
		PLAIN.fetch_add(b, Relaxed);
		*self.plain.lock().unwrap() = Some(Arc::new(p));
		true
	}

	/// Holds a plain chain somebody asked for, whatever the budget says (the upload is waiting for it; `REBUILT` keeps the amount of them
	/// in memory small, see `LevelRepl::rebuild_plain`). Charged like any plain chain; `shed_plains` leaves it alone.
	fn hold_rebuilt(&self, p: ReplTex) {
		let b = p.bytes();
		let _budget = budget_lock();
		let mut held = self.plain.lock().unwrap();
		if held.is_some() {
			return;
		}
		// `unwant` stores the flag before it takes this lock: a chain finished after it is dropped here, one held before it is released there.
		if self.unwanted.load(Relaxed) {
			let _ = self.pstate.compare_exchange(P_QUEUED, P_NONE, Relaxed, Relaxed);
			return;
		}
		USED.fetch_add(b, Relaxed);
		PLAIN.fetch_add(b, Relaxed);
		REBUILT.fetch_add(b, Relaxed);
		self.pinned.store(true, Relaxed);
		*held = Some(Arc::new(p));
	}
}

impl Drop for Entry {
	fn drop(&mut self) {
		self.release_plain();
	}
}

/// What `level_loaded` hands over per texture.
pub struct Job {
	pub key: u32,
	pub fp: Fp,
	pub n: usize,
	pub files: [Option<PackFile>; 4],
}

#[derive(Default)]
struct Stats {
	ready: usize,
	failed: usize,
	over_budget: usize,
	/// RGBA8 chains stored as BC3 to fit the budget (`bcenc`).
	recompressed: usize,
	bytes_bc: usize,
	bytes_rgba: usize,
	/// Plain chains held for upload (count, bytes), and entries whose plain chain was not made or did not fit.
	plain_kept: usize,
	plain_bytes: usize,
	plain_skipped: usize,
	errors: Vec<String>,
}

pub struct LevelRepl {
	pub level: u32,
	entries: HashMap<u32, Entry>,
	order: Vec<u32>,
	next: AtomicUsize,
	remaining: AtomicUsize,
	/// Counts finished entries; the swap code rescans its pending list when it changed.
	pub done: AtomicUsize,
	/// Bytes of the MAIN chains of this level (also in `USED` and `MAINS`; a plain chain is charged by its entry while it is held): the
	/// level's own count that `admit` checks against `limit`. Given back by `Drop for LevelRepl`.
	bytes: AtomicUsize,
	stats: Mutex<Stats>,
	t0: Instant,
	/// The device takes BC chains as they are (`BC_NATIVE` when the level was made); without it BC is decoded and not re-encoded.
	bc: bool,
	/// The budget in bytes (`budget()`; a test passes its own).
	limit: usize,
	/// Which keys are stored as BC3 or dropped (`plan_bc3`), decided once from the files' headers by the first worker, before any worker
	/// decodes.
	plan: OnceLock<Plan>,
	/// The scene build is through with this level: no more plain chains are made.
	released: AtomicBool,
	/// The scene that owns this level was dropped (travel, Load Game): the loader threads take no further job, so a level that is still
	/// loading stops giving out CPU copies at once (`scene_dropped`).
	gone: AtomicBool,
	/// Plain chains to rebuild on a loader thread (`queue_plain`) and how many threads are doing it.
	rebuild: Mutex<Rebuild>,
}

#[derive(Default)]
struct Rebuild {
	queue: std::collections::VecDeque<u32>,
	running: usize,
}

/// Loader threads that rebuild missing plain chains at a time, and how many bytes of rebuilt chains may wait for their upload before the
/// next one is built (the render thread uploads at most 16 MB a frame).
const REBUILD_THREADS: usize = 2;
const REBUILD_AHEAD: usize = 48 << 20;

/// Bytes of rebuilt plain chains in memory that wait for their upload (part of `PLAIN`).
static REBUILT: AtomicUsize = AtomicUsize::new(0);

/// Bytes of all live replacement chains, main and plain (every level alive: the prefetch holds two).
static USED: AtomicUsize = AtomicUsize::new(0);

/// Makes every decision on `USED` / `PLAIN` against the budget (admitting a main chain, holding a plain chain) one step: taken for the
/// check and the additions only (never while decoding or compressing). Lock order: `BUDGET`, then an entry's `plain`.
static BUDGET: Mutex<()> = Mutex::new(());

fn budget_lock() -> std::sync::MutexGuard<'static, ()> {
	BUDGET.lock().unwrap_or_else(|e| e.into_inner())
}

/// The part of `USED` that is main chains (kept for good by the loaded levels). Memory accounting and tests only: neither the plan nor
/// `admit` reads it (each level counts its own main chains, `LevelRepl::bytes`), so no level's result depends on another level.
static MAINS: AtomicUsize = AtomicUsize::new(0);

/// The part of `USED` that is plain chains (transient, see `Entry`); `USED - PLAIN` = `MAINS` is what the levels keep for good.
static PLAIN: AtomicUsize = AtomicUsize::new(0);

/// Budget for the CPU copies of the world's replacement textures in MB (`RAC_TEXPACK_MB`, default 1536: 8 GB machines and the Steam Deck;
/// read once): the budget of EACH level for its main chains (never over it; the plan and `admit` look at the level alone, so the same
/// textures are replaced however the previous level is doing; during a trip the leaving level's copies that are not swapped in yet can
/// coexist with the new level's, up to 2x the budget until the old scene is dropped, normally none) plus the plain chains while they wait for their upload (over it by at most the rebuilt
/// ones: `REBUILD_AHEAD` + one chain per rebuild thread, `REBUILD_THREADS` = 2 per level and 2 levels alive during a trip = up to 4 chains, and the held chains of a level still being swapped in). Not the HUD atlas, the fx / sky / ship / warp
/// textures or the grass colours.
fn budget() -> usize {
	static MB: OnceLock<usize> = OnceLock::new();
	*MB.get_or_init(|| std::env::var("RAC_TEXPACK_MB").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(1536)) << 20
}

/// Whether the GPU takes BC textures as they are (`TEXTURE_COMPRESSION_BC`). Written by `texpack::gpu_features`, which `Gpu::new` calls to
/// choose the device's features (so before any level loads); read when a `LevelRepl` is made. While it is not set, the loader decodes BC
/// chains to RGBA8 itself.
static BC_NATIVE: AtomicBool = AtomicBool::new(false);

pub fn set_bc_native(on: bool) {
	BC_NATIVE.store(on, Relaxed);
}

pub fn bc_native() -> bool {
	BC_NATIVE.load(Relaxed)
}

/// A chain ready for upload: BC is decoded here, on the loader thread, when the device cannot take it (`bc` = it can).
fn for_device(t: ReplTex, bc: bool) -> Result<ReplTex, String> {
	if t.fmt.compressed() && !bc { t.to_rgba8() } else { Ok(t) }
}

/// Reads and decodes one pack file. A decoder that panics on a bad file is caught here (one log line naming the file, the file counts as
/// unusable), so one bad file can never stop a loader thread or crash a level load.
pub(super) fn load_file(f: &PackFile) -> Result<Image, String> {
	#[cfg(test)]
	tests::LOADS_HERE.with(|c| c.set(c.get() + 1));
	let name = f.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
	let bytes = std::fs::read(&f.path).map_err(|e| format!("{}: {e}", f.path.display()))?;
	let decoded = guard::guarded(|| match f.ext {
		Ext::Dds => dds::load(&bytes),
		Ext::Png => super::png::load(&bytes),
	});
	decoded.unwrap_or_else(|_| Err("the decoder panicked on this file".into())).map_err(|e| format!("{name}: {e}"))
}

/// What the level loads and keeps, decided before any file is loaded (`plan_bc3`).
#[derive(Debug, Default, PartialEq, Eq)]
struct Plan {
	/// The keys whose RGBA8 chain is stored as BC3.
	bc3: std::collections::HashSet<u32>,
	/// The keys not loaded at all because the level does not fit even with every candidate in BC3.
	drop: std::collections::HashSet<u32>,
}

/// What the MAIN chain of a texture will be, from the headers alone, or None when it cannot be built (the entry fails
/// when loaded and costs nothing). Mirrors `LevelRepl::build`. `levels` = the original's level count (`Job::n`).
/// `chain::assemble` for `levels` levels (1 for most textures): each level from the NAMED file of the right size if there is one (any
/// format), else from the file's own mip `k < mips`, else generated (RGBA8); the chain stays BC only when every level is the same BC
/// format and level 0 is whole blocks, else the whole chain is RGBA8; a device without BC decodes a BC chain to RGBA8 (`for_device`).
/// The plain (upload) chain is not in the plan: it is transient (`Entry`).
fn planned_chain(levels: usize, files: &[Option<PackFile>], bc: bool) -> Option<PlannedChain> {
	let (w, h, fmt, mips) = files.first().and_then(Option::as_ref).and_then(peek_head)?;
	// A header past what `load` accepts (or an empty one) is refused later anyway; skipping it keeps the sums below far from overflow.
	if w == 0 || h == 0 || w > 8192 || h > 8192 {
		return None;
	}
	let full = 1 + w.max(h).ilog2() as usize;
	let n = levels.clamp(1, full);
	let lf: Vec<dds::Fmt> = (0..n)
		.map(|k| match files.get(k).and_then(Option::as_ref).and_then(peek_head) {
			Some((lw, lh, lf, _)) if (lw, lh) == ((w >> k).max(1), (h >> k).max(1)) => lf,
			_ if k < mips as usize => fmt,
			_ => dds::Fmt::Rgba8,
		})
		.collect();
	let kept = lf.iter().all(|&f| f == lf[0]) && (!lf[0].compressed() || (w % 4 == 0 && h % 4 == 0));
	let out = if kept { lf[0] } else { dds::Fmt::Rgba8 };
	let out = if out.compressed() && !bc { dds::Fmt::Rgba8 } else { out };
	let bytes = (0..n as u32).map(|k| out.level_bytes((w >> k).max(1), (h >> k).max(1))).sum();
	Some(PlannedChain { fmt: out, w, h, n, bytes })
}

/// The header-level size of one main chain (`planned_chain`).
struct PlannedChain {
	fmt: dds::Fmt,
	w: u32,
	h: u32,
	/// Levels in the chain.
	n: usize,
	bytes: usize,
}

/// [PC] v0.4.1 (#32): which chains a level stores as BC3 and which it drops, so that it fits `budget` (the level is planned alone,
/// whatever other levels are alive; plain chains are transient and not in the plan). `items` = (key, levels, files) per texture.
/// Decided from the files' headers before any file is loaded (`LevelRepl::plan`: the first worker makes it, no worker decodes before it
/// is done), so it does not depend on the order the loader threads finish in: nothing changes when the whole level fits as it is;
/// otherwise, if the sum does not fit even with every candidate in BC3, the largest chains (as BC3 where they can be) are dropped (key
/// order breaks ties) until it does, and then the largest RGBA8 chains go to BC3 (same order) until the sum fits. `bc` = the device takes BC chains as they are:
/// without it nothing can be stored as BC3 and BC files count as their RGBA8 decode.
fn plan_bc3<'a>(items: impl Iterator<Item = (u32, usize, &'a [Option<PackFile>])>, budget: usize, bc: bool) -> Plan {
	let mut total = 0usize;
	// (key, bytes now, bytes as BC3 when it can be stored as BC3)
	let mut chains: Vec<(u32, usize, Option<usize>)> = Vec::new();
	for (key, levels, files) in items {
		let Some(c) = planned_chain(levels, files, bc) else { continue };
		total = total.saturating_add(c.bytes);
		// What `bcenc::compress` makes of an RGBA8 chain whose level 0 is whole blocks.
		let packed = (bc && c.fmt == dds::Fmt::Rgba8 && c.w % 4 == 0 && c.h % 4 == 0).then(|| (0..c.n as u32).map(|k| dds::Fmt::Bc3.level_bytes((c.w >> k).max(1), (c.h >> k).max(1))).sum());
		chains.push((key, c.bytes, packed));
	}
	let mut plan = Plan::default();
	if total <= budget {
		return plan;
	}
	// First what cannot be kept even with every candidate in BC3: the largest such chains are dropped (key order on ties) until the rest fits.
	let floor = |&(_, full, packed): &(u32, usize, Option<usize>)| packed.unwrap_or(full);
	let mut at_floor = total - chains.iter().map(|c| c.1 - floor(c)).sum::<usize>();
	if at_floor > budget {
		let mut order: Vec<_> = chains.iter().collect();
		order.sort_by_key(|c| (std::cmp::Reverse(floor(c)), c.0));
		for c in order {
			if at_floor <= budget {
				break;
			}
			at_floor -= floor(c);
			total -= c.1;
			plan.drop.insert(c.0);
		}
	}
	// Then as few BC3 chains as fit the rest: the largest candidates first (key order on ties).
	let mut cand: Vec<_> = chains.iter().filter(|c| !plan.drop.contains(&c.0)).filter_map(|&(key, full, packed)| packed.map(|p| (key, full, p))).collect();
	cand.sort_by_key(|&(key, full, _)| (std::cmp::Reverse(full), key));
	for (key, full, packed) in cand {
		if total <= budget {
			break;
		}
		total -= full - packed;
		plan.bc3.insert(key);
	}
	plan
}

/// Size and format of a pack file from its header alone (a PNG is decoded to RGBA8), or None when the header is not readable.
pub(super) fn peek_file(f: &PackFile) -> Option<(u32, u32, dds::Fmt)> {
	peek_head(f).map(|(w, h, fmt, _)| (w, h, fmt))
}

/// `peek_file` plus the number of mip levels the file itself STORES (a header that claims more than the file holds is cut to what fits).
/// A BC7 file the loader may decode (`bc7alpha::bc7_may_decode`: DX10 alpha mode other than 3) is reported as `Rgba8`, the size the
/// planner has to count: texconv / Compressonator BC7 almost never has alpha exactly 255 everywhere, so the loader turns it into RGBA8
/// (and a file that does stay BC7 only makes the plan too careful).
fn peek_head(f: &PackFile) -> Option<(u32, u32, dds::Fmt, u32)> {
	use std::io::Read;
	let mut head = Vec::with_capacity(dds::PEEK_BYTES);
	let file = std::fs::File::open(&f.path).ok()?;
	let len = file.metadata().ok()?.len();
	file.take(dds::PEEK_BYTES as u64).read_to_end(&mut head).ok()?;
	match f.ext {
		Ext::Dds => dds::peek(&head).map(|(w, h, fmt)| {
			let fmt = if fmt == dds::Fmt::Bc7 && super::bc7alpha::bc7_may_decode(&head) { dds::Fmt::Rgba8 } else { fmt };
			(w, h, fmt, dds::peek_stored_mips(&head, len))
		}),
		Ext::Png => super::png::peek(&head).map(|(w, h)| (w, h, dds::Fmt::Rgba8, 1)),
	}
}

impl LevelRepl {
	pub fn new(level: u32, jobs: Vec<Job>) -> Arc<LevelRepl> {
		Self::with_bc(level, jobs, bc_native())
	}

	/// `new` for a device that does (`bc`) or does not take BC chains as they are.
	pub fn with_bc(level: u32, jobs: Vec<Job>, bc: bool) -> Arc<LevelRepl> {
		Self::with_limit(level, jobs, bc, budget())
	}

	/// `with_bc` with the budget in bytes given (tests).
	fn with_limit(level: u32, jobs: Vec<Job>, bc: bool, limit: usize) -> Arc<LevelRepl> {
		let order = jobs.iter().map(|j| j.key).collect::<Vec<_>>();
		let entries = jobs.into_iter().map(|j| (j.key, Entry { key: j.key, fp: j.fp, n: j.n, files: j.files, cpu: Mutex::new(Cpu::Pending), gpu: Mutex::new([None, None]), plain: Mutex::new(None), pstate: AtomicU8::new(P_NONE), pinned: AtomicBool::new(false), unwanted: AtomicBool::new(false) })).collect::<HashMap<_, _>>();
		Arc::new(LevelRepl { level, remaining: AtomicUsize::new(order.len()), entries, order, next: AtomicUsize::new(0), done: AtomicUsize::new(0), bytes: AtomicUsize::new(0), stats: Mutex::default(), t0: Instant::now(), bc, limit, plan: OnceLock::new(), released: AtomicBool::new(false), gone: AtomicBool::new(false), rebuild: Mutex::default() })
	}

	/// Starts `threads` workers; they end when the queue is empty or the level is dropped.
	pub fn start(self: &Arc<Self>, threads: usize) {
		drop(self.spawn_workers(threads));
	}

	/// `start`, handing back the workers' handles (a test joins them, so nothing of the level is alive when it goes on).
	fn spawn_workers(self: &Arc<Self>, threads: usize) -> Vec<std::thread::JoinHandle<()>> {
		(0..threads.max(1))
			.filter_map(|i| {
				let weak: Weak<LevelRepl> = Arc::downgrade(self);
				std::thread::Builder::new()
					.name(format!("texpack-load-{i}"))
					.spawn(move || {
						while let Some(r) = weak.upgrade() {
							if r.gone.load(Relaxed) {
								break; // the scene is gone: the rest of the keys is never published, the copies made so far go with the level
							}
							let at = r.next.fetch_add(1, Relaxed);
							let Some(&key) = r.order.get(at) else { break };
							r.run(key);
						}
					})
					.ok()
			})
			.collect()
	}

	pub fn entry(&self, key: u32) -> Option<&Entry> {
		self.entries.get(&key)
	}

	pub fn len(&self) -> usize {
		self.order.len()
	}

	pub fn is_empty(&self) -> bool {
		self.order.is_empty()
	}

	/// Keys still being loaded.
	pub fn pending(&self) -> usize {
		self.remaining.load(Relaxed)
	}

	/// Waits until everything is loaded or `max` has passed (the scene build gives the workers a head start; 0 = do not wait).
	pub fn wait(&self, max: Duration) {
		let t = Instant::now();
		while self.pending() > 0 && t.elapsed() < max {
			std::thread::sleep(Duration::from_millis(2));
		}
	}

	/// Frees the plain chains of this level that are still held (a main chain needs the room), except the ones rebuilt on request.
	fn shed_plains(&self) {
		self.entries.values().filter(|e| !e.pinned.load(Relaxed)).for_each(Entry::release_plain);
	}

	/// True once `release_plains` has run (tests).
	#[cfg(test)]
	pub(crate) fn is_released(&self) -> bool {
		self.released.load(Relaxed)
	}

	/// True once `scene_dropped` has run (tests).
	#[cfg(test)]
	pub(crate) fn is_gone(&self) -> bool {
		self.gone.load(Relaxed)
	}

	/// The scene build has bound every slot: the held plain chains are of no use any more and none is made from now on.
	pub fn release_plains(&self) {
		self.released.store(true, Relaxed);
		self.entries.values().for_each(Entry::release_plain);
	}

	/// The scene that owns this level is dropped (travel, Load Game): ends the plain chains and rebuilds (`release_plains`) and, if the
	/// level is still loading, the loader threads (each finishes the key it is on, takes no other). Without it the old level would go on
	/// admitting main chains until its queue was empty, and its CPU copies would outlive its scene.
	pub fn scene_dropped(&self) {
		self.gone.store(true, Relaxed);
		self.release_plains();
	}

	/// No slot of the finished scene build uses `key` (`SceneCtx::repl_end`): gives its plain chain back and no rebuild is made or held for
	/// it. Without it the rebuilt chain of a slot that a prop pass threw away would stay in `REBUILT` (nobody uploads it) and, with enough
	/// of them, stop every other rebuild of the level for the visit. A later `queue_plain` of the key (a slot asks again) clears it.
	pub fn unwant(&self, key: u32) {
		if let Some(e) = self.entries.get(&key) {
			e.unwanted.store(true, Relaxed); // before the release: `hold_rebuilt` checks it under the entry's lock
			e.release_plain();
		}
	}

	/// Asks for the plain chain of `key` to be rebuilt from its pack file on a loader thread (once; `Entry::plain_for_bind`). When it is held
	/// the entry's chain is there for the next `plain_for_bind` and `done` has counted (so `Gpu::swap_repl` looks again).
	pub fn queue_plain(self: &Arc<Self>, key: u32) {
		let Some(e) = self.entries.get(&key) else { return };
		if self.released.load(Relaxed) {
			return;
		}
		e.unwanted.store(false, Relaxed);
		if e.pstate.compare_exchange(P_NONE, P_QUEUED, Relaxed, Relaxed).is_err() {
			return;
		}
		let mut r = self.rebuild.lock().unwrap();
		r.queue.push_back(key);
		if r.running < REBUILD_THREADS {
			r.running += 1;
			let weak = Arc::downgrade(self);
			if std::thread::Builder::new().name("texpack-plain".into()).spawn(move || Self::rebuild_loop(weak)).is_err() {
				// No thread to pop it: the slot keeps the original instead of waiting for the whole visit.
				r.running -= 1;
				r.queue.pop_back();
				e.pstate.store(P_FAILED, Relaxed);
			}
		}
	}

	fn rebuild_loop(weak: Weak<LevelRepl>) {
		let mut made = 0;
		loop {
			let Some(r) = weak.upgrade() else { return };
			let next = {
				let mut g = r.rebuild.lock().unwrap();
				let next = g.queue.pop_front();
				if next.is_none() {
					g.running -= 1;
				}
				next
			};
			let Some(key) = next else {
				if made > 0 && !r.released.load(Relaxed) {
					eprintln!("texpack: level {}: {made} upload chains rebuilt from the pack files (missing when a slot was bound)", r.level);
				}
				return;
			};
			drop(r); // the level is held only while a chain is built
			made += Self::rebuild_plain(&weak, key) as usize;
		}
	}

	/// One queued rebuild: the chain `make_plain` would have held, made from the level-0 file again. Waits (without keeping the level
	/// alive) while the level is still being loaded (`pending`: the main chains are admitted first, so their room does not depend on
	/// the rebuilds) and while too many rebuilt chains wait for their upload; gives up when the level is gone or the scene build has
	/// released it. True when a chain was made.
	fn rebuild_plain(weak: &Weak<LevelRepl>, key: u32) -> bool {
		let Some(this) = Self::rebuild_turn(weak, key) else { return false };
		let e = &this.entries[&key];
		let res = guard::guarded(|| this.plain_from_file(e)).unwrap_or_else(|_| Err("the loader panicked".into()));
		let made = match res {
			Ok(p) => {
				e.hold_rebuilt(p);
				if this.released.load(Relaxed) {
					e.release_plain(); // the scene went away while it was made
				}
				true
			}
			Err(m) => {
				e.pstate.store(P_FAILED, Relaxed);
				eprintln!("texpack: level {}: key {key:#x}: the upload chain cannot be rebuilt: {m}", this.level);
				false
			}
		};
		this.done.fetch_add(1, Relaxed);
		made
	}

	/// The wait of `rebuild_plain`: the level once it is its turn, None when the level is gone, released or the key unwanted (the entry may ask again).
	/// Only the `Weak` is held while waiting, so a level whose scene went away ends here and returns its budget.
	fn rebuild_turn(weak: &Weak<LevelRepl>, key: u32) -> Option<Arc<Self>> {
		loop {
			let this = weak.upgrade()?;
			if this.released.load(Relaxed) {
				let _ = this.entries[&key].pstate.compare_exchange(P_QUEUED, P_NONE, Relaxed, Relaxed);
				return None;
			}
			let e = &this.entries[&key];
			if e.unwanted.load(Relaxed) {
				// No slot uses the key: nothing is built for it (it would sit in `REBUILT` for good). A slot that asks again between the check
				// and the reset (`queue_plain` cleared the flag but found the key still queued) is queued once more.
				let _ = e.pstate.compare_exchange(P_QUEUED, P_NONE, Relaxed, Relaxed);
				if !e.unwanted.load(Relaxed) {
					this.queue_plain(key);
				}
				return None;
			}
			if this.pending() == 0 && REBUILT.load(Relaxed) <= REBUILD_AHEAD {
				return Some(this);
			}
			drop(this);
			std::thread::sleep(Duration::from_millis(4));
		}
	}

	/// The plain chain of an entry from its level-0 file, ready for the device: what `make_plain` holds, byte for byte.
	fn plain_from_file(&self, e: &Entry) -> Result<ReplTex, String> {
		let f = e.files[0].as_ref().ok_or("no file for level 0")?;
		self.plain_chain_of(load_file(f)?)
	}

	/// The one rule of the plain chain, for the loader (`make_plain`) and for the rebuild.
	fn plain_chain_of(&self, base: Image) -> Result<ReplTex, String> {
		chain::plain_of(base).and_then(|p| for_device(p, self.bc))
	}

	fn run(&self, key: u32) {
		let e = &self.entries[&key];
		// Every worker waits here for the plan (the first one makes it from all the headers), so no chain is decoded before it is known.
		self.plan();
		let res = guard::guarded(|| self.build(e)).unwrap_or_else(|_| Err(format!("key {key:#x}: the loader panicked")));
		let state = match res {
			Ok((t, base)) => match self.admit(key, t) {
				Some(t) => {
					// The plain chain for a main chain that was accepted, and before the entry is published as ready.
					self.make_plain(e, base);
					Cpu::Ready(Arc::new(t))
				}
				None => Cpu::Failed,
			},
			Err(m) => {
				let mut s = self.stats.lock().unwrap();
				s.failed += 1;
				if s.errors.len() < 3 {
					s.errors.push(m);
				}
				Cpu::Failed
			}
		};
		*e.cpu.lock().unwrap() = state;
		self.done.fetch_add(1, Relaxed);
		if self.remaining.fetch_sub(1, Relaxed) == 1 {
			let s = self.stats.lock().unwrap();
			eprintln!(
				"texpack: level {}: {} replacements ready ({:.1} MB compressed + {:.1} MB RGBA8), {} failed, {} over the {} MB budget, {} stored as BC3 to fit, loaded in {:.0} ms; {} upload chains made ({:.1} MB, held until uploaded), {} not made",
				self.level, s.ready, s.bytes_bc as f64 / 1048576.0, s.bytes_rgba as f64 / 1048576.0, s.failed, s.over_budget, self.limit >> 20, s.recompressed, self.t0.elapsed().as_secs_f64() * 1000.0,
				s.plain_kept, s.plain_bytes as f64 / 1048576.0, s.plain_skipped
			);
			for m in &s.errors {
				eprintln!("texpack: level {}: {m}", self.level);
			}
			drop(s);
			self.log_keys();
		}
	}

	/// `RAC_TEXPACK_KEYS=1` (test aid): one log line with the sorted keys that ended ready, the ones stored as BC3 and the ones the plan
	/// dropped, so two runs can be compared as SETS and not only by their counts.
	fn log_keys(&self) {
		static ON: OnceLock<bool> = OnceLock::new();
		if !*ON.get_or_init(|| std::env::var("RAC_TEXPACK_KEYS").is_ok_and(|v| v == "1")) {
			return;
		}
		let mut ready: Vec<u32> = self.order.iter().copied().filter(|k| matches!(&*self.entries[k].cpu.lock().unwrap(), Cpu::Ready(_))).collect();
		ready.sort_unstable();
		let sorted = |set: &std::collections::HashSet<u32>| {
			let mut v: Vec<u32> = set.iter().copied().collect();
			v.sort_unstable();
			v
		};
		let (bc3, drop) = (sorted(&self.plan().bc3), sorted(&self.plan().drop));
		eprintln!("texpack: level {}: keys ready {ready:x?} bc3 {bc3:x?} dropped {drop:x?}", self.level);
	}

	/// What this level stores as BC3 or drops (`plan_bc3`), made once by whichever worker comes first.
	fn plan(&self) -> &Plan {
		// As if the level were alone: another level's main chains never change this level's ready / BC3 / dropped sets.
		self.plan.get_or_init(|| plan_bc3(self.order.iter().map(|k| (*k, self.entries[k].n, &self.entries[k].files[..])), self.limit, self.bc))
	}

	/// Budget check of a main chain: the limit looks at the main chains this level keeps plus this chain (`self.bytes`, which no plain chain
	/// and no other level ever changes, so the answer depends on neither timing nor the previous level); when `USED` goes over the limit with it, this level gives up its plain chains
	/// (best effort: memory, not admission). None = does not fit. A key in the BC3 plan is stored as BC3 (a quarter of the size) first; a
	/// key the plan drops is refused without being charged. The compression runs before the lock; under `BUDGET` the check, the shed and
	/// the addition are one step.
	fn admit(&self, key: u32, t: ReplTex) -> Option<ReplTex> {
		let plan = self.plan();
		if plan.drop.contains(&key) {
			self.stats.lock().unwrap().over_budget += 1;
			return None;
		}
		// [PC] v0.4.1 (#32): the chains `plan_bc3` picked (a level that does not fit as RGBA8, PNG packs) are kept as BC3.
		let (t, packed) = if plan.bc3.contains(&key) { super::bcenc::compress(&t).map(|c| (c, true)).unwrap_or((t, false)) } else { (t, false) };
		let b = t.bytes();
		let fits = {
			let _budget = budget_lock();
			// The main chains decide, not the transient plain chains (timing): a chain the plan said fits is never refused.
			let fits = self.bytes.load(Relaxed) + b <= self.limit;
			if fits {
				self.bytes.fetch_add(b, Relaxed);
				if USED.load(Relaxed) + b > self.limit && PLAIN.load(Relaxed) > 0 {
					self.shed_plains(); // best effort, for memory; the rebuilt ones stay (pinned until their upload)
				}
				USED.fetch_add(b, Relaxed);
				MAINS.fetch_add(b, Relaxed);
			}
			fits
		};
		let mut s = self.stats.lock().unwrap();
		if !fits {
			s.over_budget += 1;
			return None;
		}
		s.recompressed += packed as usize;
		s.ready += 1;
		if t.fmt.compressed() { s.bytes_bc += b } else { s.bytes_rgba += b }
		Some(t)
	}

	/// Builds the plain chain of an accepted entry from its level-0 file (`base`, decoded already) and holds it if the budget has room (see
	/// `Entry`). Not held: it is rebuilt by `queue_plain` when a slot needs it.
	fn make_plain(&self, e: &Entry, base: Image) {
		let mut kept = None;
		if !self.released.load(Relaxed) && USED.load(Relaxed) + chain::plain_bytes_on(base.fmt, base.w, base.h, base.levels.len(), self.bc) <= self.limit {
			match guard::guarded(|| self.plain_chain_of(base)) {
				Ok(Ok(p)) => {
					let b = p.bytes();
					kept = e.keep_plain(p, self.limit).then_some(b);
				}
				// A chain that cannot be built from the file cannot be rebuilt either: the slot keeps the original.
				_ => e.pstate.store(P_FAILED, Relaxed),
			}
		}
		if self.released.load(Relaxed) {
			e.release_plain(); // the scene build finished while this was made
		}
		let mut s = self.stats.lock().unwrap();
		match kept {
			Some(b) => {
				s.plain_kept += 1;
				s.plain_bytes += b;
			}
			None => s.plain_skipped += 1,
		}
	}

	/// Loads the files of one entry: its main chain (`chain::assemble`, as v0.4.0) and the decoded level-0 file the plain chain is made from.
	fn build(&self, e: &Entry) -> Result<(ReplTex, Image), String> {
		let mut imgs: Vec<Option<Image>> = Vec::new();
		for (k, f) in e.files.iter().enumerate() {
			imgs.push(match f {
				Some(f) if k == 0 => Some(load_file(f)?),
				// A level-k file that cannot be read is generated like a missing one (the chain ignores files past its length).
				Some(f) if k < e.n => load_file(f).ok(),
				_ => None,
			});
		}
		let main = chain::assemble(&imgs, e.n).and_then(|t| for_device(t, self.bc))?;
		let base = imgs.swap_remove(0).ok_or("no file for level 0")?;
		Ok((main, base))
	}
}

impl Drop for LevelRepl {
	fn drop(&mut self) {
		let b = self.bytes.load(Relaxed);
		USED.fetch_sub(b, Relaxed);
		MAINS.fetch_sub(b, Relaxed);
		if b > 0 {
			eprintln!("texpack: level {}: dropped, {:.1} MB of main chains released (all levels now keep {:.1} MB)", self.level, b as f64 / 1048576.0, MAINS.load(Relaxed) as f64 / 1048576.0);
		}
	}
}

thread_local! {
	static THREAD_REPL: std::cell::RefCell<Option<Arc<LevelRepl>>> = const { std::cell::RefCell::new(None) };
}

/// `level_loaded` leaves the level's replacements for the scene build that runs next on the same thread (the level job and
/// `build_scene` share a thread in both load paths).
pub fn set_thread_repl(r: Option<Arc<LevelRepl>>) {
	THREAD_REPL.with(|t| *t.borrow_mut() = r);
}

pub fn take_thread_repl() -> Option<Arc<LevelRepl>> {
	THREAD_REPL.with(|t| t.borrow_mut().take())
}

#[cfg(test)]
mod rebuild_tests;

#[cfg(test)]
mod level_tests;

#[cfg(test)]
mod tests {
	use super::*;

	/// The budget counters are process-wide: the tests that load a level one at a time.
	pub(super) static ONE_LEVEL_AT_A_TIME: Mutex<()> = Mutex::new(());

	thread_local! {
		/// Files this thread has read and decoded (`load_file`): a test shows that a thread did not.
		pub(super) static LOADS_HERE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
	}

	/// Waits (up to 20 s) until a loader thread has rebuilt and holds the plain chain of `e`.
	pub(super) fn wait_plain(e: &Entry) -> Arc<ReplTex> {
		for _ in 0..5000 {
			if let Some(p) = e.plain.lock().unwrap().clone() {
				return p;
			}
			std::thread::sleep(Duration::from_millis(4));
		}
		panic!("key {:#x}: the plain chain was not rebuilt", e.key);
	}

	#[test]
	fn fingerprint_tells_textures_apart() {
		let a = Texture { w: 2, h: 1, rgba: vec![[1, 2, 3, 4], [5, 6, 7, 8]] };
		let b = Texture { w: 2, h: 1, rgba: vec![[1, 2, 3, 4], [5, 6, 7, 9]] };
		let fp = Fp::of(&a);
		assert!(fp.is(&a) && !fp.is(&b));
	}

	#[test]
	fn workers_load_a_job_and_the_level_drop_frees_the_budget() {
		let _one = ONE_LEVEL_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
		let dir = std::env::temp_dir().join(format!("texpack_repl_{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let mut blk = [0u8; 16];
		blk[0] = 128;
		blk[1] = 128;
		let one = super::super::dds::tests::dds(4, 4, b"DXT5", [0; 4], 0, &[&blk]);
		// Every level down to 1x1 (4x4, 2x2, 1x1: one block each).
		let full = super::super::dds::tests::dds(4, 4, b"DXT5", [0; 4], 0, &[&blk, &blk, &blk]);
		let file = |name: &str, bytes: Vec<u8>| {
			let path = dir.join(name);
			std::fs::write(&path, bytes).unwrap();
			Some(PackFile { path, ext: Ext::Dds, mod_idx: 0 })
		};
		let t = Texture { w: 1, h: 1, rgba: vec![[0; 4]] };
		let jobs = |tag: &str| {
			vec![
				Job { key: 3, fp: Fp::of(&t), n: 1, files: [file(&format!("{tag}full.dds"), full.clone()), None, None, None] },
				Job { key: 4, fp: Fp::of(&t), n: 2, files: [file(&format!("{tag}one.dds"), one.clone()), None, None, None] },
				// A file the decoder panicked on (24-bit with an alpha mask in the top byte) is one failed entry, the others still load.
				Job { key: 5, fp: Fp::of(&t), n: 1, files: [file(&format!("{tag}bad.dds"), super::super::dds::tests::dds(1, 1, &[0; 4], [0xff0000, 0xff00, 0xff, 0xff000000], 24, &[&[1, 2, 3]])), None, None, None] },
			]
		};
		let load = |r: &Arc<LevelRepl>| {
			r.start(2);
			r.wait(Duration::from_secs(5));
			assert_eq!(r.pending(), 0);
		};
		let ready = |r: &Arc<LevelRepl>, k: u32| match &*r.entry(k).unwrap().cpu.lock().unwrap() {
			Cpu::Ready(t) => Some(t.clone()),
			_ => None,
		};
		let (before, mains_before) = (USED.load(Relaxed), MAINS.load(Relaxed));
		let r = LevelRepl::with_bc(7, jobs("a"), true);
		load(&r);
		let bc3 = super::super::dds::Fmt::Bc3;
		// n = 1: the main chain is level 0 alone as v0.4.0 kept it (one BC3 block, 16 bytes). n = 2: the authored chain, BC level 0 + a
		// generated level = RGBA8 (64 + 16).
		let (a, e3) = (ready(&r, 3).unwrap(), r.entry(3).unwrap());
		assert_eq!((a.fmt, a.bytes()), (bc3, 16));
		let (b, e4) = (ready(&r, 4).unwrap(), r.entry(4).unwrap());
		assert_eq!((b.fmt, b.bytes()), (super::super::dds::Fmt::Rgba8, 4 * 16 + 4 * 4));
		assert!(ready(&r, 5).is_none(), "the bad file failed");
		// The loader holds the upload chain of both, made from the level-0 FILE: entry 3's file has every level (BC3, 48 bytes as it is),
		// entry 4's has level 0 only (4x4, 2x2, 1x1 RGBA8 = 84 bytes). Charged while held.
		let (p3, p4) = (e3.plain.lock().unwrap().clone().unwrap(), e4.plain.lock().unwrap().clone().unwrap());
		assert_eq!((p3.fmt, p3.bytes()), (bc3, 48));
		assert_eq!((p4.fmt, p4.bytes()), (super::super::dds::Fmt::Rgba8, 84));
		assert_eq!(USED.load(Relaxed), before + 16 + 80 + 48 + 84);
		assert_eq!(MAINS.load(Relaxed), mains_before + 16 + 80);
		// An upload takes it: the bytes go back at once. A later request (the render thread) decodes nothing: a loader thread rebuilds
		// the same chain and the entry holds it again.
		e4.release_plain();
		assert_eq!(USED.load(Relaxed), before + 16 + 80 + 48);
		let here = LOADS_HERE.with(|c| c.get());
		assert!(e4.plain_for_bind(&r).unwrap().is_none(), "not held: the caller keeps its picture");
		assert_eq!(LOADS_HERE.with(|c| c.get()), here, "no decode on the calling thread");
		let again = wait_plain(e4);
		assert_eq!((again.fmt, again.bytes()), (p4.fmt, p4.bytes()));
		e4.release_plain();
		// The scene is done: the rest goes, and no plain chain is made after that.
		r.release_plains();
		assert_eq!(USED.load(Relaxed), before + 16 + 80);
		// The same level on a device without BC: BC chains are decoded by the loader (no BC3 re-encode either).
		let r2 = LevelRepl::with_bc(8, jobs("b"), false);
		load(&r2);
		let a2 = ready(&r2, 3).unwrap();
		assert_eq!((a2.fmt, a2.bytes()), (super::super::dds::Fmt::Rgba8, 64));
		assert_eq!(r2.entry(3).unwrap().plain.lock().unwrap().clone().unwrap().fmt, super::super::dds::Fmt::Rgba8);
		// A main chain that needs the room makes the level give up its plain chains.
		let held = USED.load(Relaxed);
		r2.shed_plains();
		assert!(USED.load(Relaxed) < held);
		drop((r, r2));
		// A worker may still hold its last reference for a moment.
		for _ in 0..500 {
			if USED.load(Relaxed) == before && MAINS.load(Relaxed) == mains_before {
				break;
			}
			std::thread::sleep(Duration::from_millis(4));
		}
		assert_eq!((USED.load(Relaxed), MAINS.load(Relaxed)), (before, mains_before));
		let _ = std::fs::remove_dir_all(&dir);
	}

	/// A job with `n` levels whose level-0 file is an uncompressed `side` x `side` DDS of pixels that differ from texel to texel.
	pub(super) fn rgba_job_n(dir: &std::path::Path, key: u32, side: u32, n: usize) -> Job {
		let px: Vec<u8> = (0..side * side * 4).map(|i| (i.wrapping_mul(37).wrapping_add(key * 11) % 251) as u8).collect();
		let f = super::super::dds::tests::dds(side, side, &[0; 4], [0xff, 0xff00, 0xff0000, 0xff00_0000], 32, &[&px]);
		let path = dir.join(format!("k{key}.dds"));
		std::fs::write(&path, f).unwrap();
		Job { key, fp: Fp { w: 1, h: 1, hash: 0 }, n, files: [Some(PackFile { path, ext: Ext::Dds, mod_idx: 0 }), None, None, None] }
	}

	/// `rgba_job_n` for an original that has the whole chain down to 1x1 (the chain is generated: sizes of a full RGBA8 chain).
	fn rgba_job(dir: &std::path::Path, key: u32, side: u32) -> Job {
		rgba_job_n(dir, key, side, 1 + side.ilog2() as usize)
	}

	pub(super) fn tmp(tag: &str) -> std::path::PathBuf {
		let dir = std::env::temp_dir().join(format!("texpack_{tag}_{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		dir
	}

	/// Runs `jobs` as a level of `limit` bytes with `workers` threads, waits until the workers are gone and the level is dropped, and
	/// returns (ready keys, stats (ready, failed, over, BC3), the BC3 keys). Nothing of the level is alive afterwards.
	pub(super) fn run_level(level: u32, jobs: Vec<Job>, limit: usize, workers: usize) -> (Vec<u32>, [usize; 4], Vec<u32>) {
		let r = LevelRepl::with_limit(level, jobs, true, limit);
		for h in r.spawn_workers(workers) {
			h.join().unwrap();
		}
		assert_eq!(r.pending(), 0);
		let mut ready = Vec::new();
		let mut bc3 = Vec::new();
		for &k in &r.order {
			if let Cpu::Ready(t) = &*r.entry(k).unwrap().cpu.lock().unwrap() {
				ready.push(k);
				if t.fmt == dds::Fmt::Bc3 {
					bc3.push(k);
				}
			}
		}
		ready.sort_unstable();
		bc3.sort_unstable();
		let s = r.stats.lock().unwrap();
		let out = (ready, [s.ready, s.failed, s.over_budget, s.recompressed], bc3);
		drop(s);
		drop(r);
		out
	}

	#[test]
	fn bc3_plan_is_none_when_the_level_fits_and_independent_of_the_job_order_when_not() {
		let dir = tmp("plan");
		// The original has the whole chain, so the chain is the whole one down to 1x1: 16x16 = 1364 bytes as RGBA8 (368 as BC3), 8x8 = 340 (112): 2044.
		let mut jobs = vec![rgba_job(&dir, 5, 16), rgba_job(&dir, 6, 8), rgba_job(&dir, 7, 8)];
		let plan = |jobs: &[Job], budget, bc| plan_bc3(jobs.iter().map(|j| (j.key, j.n, &j.files[..])), budget, bc);
		let set = |v: &[u32]| v.iter().copied().collect::<std::collections::HashSet<u32>>();
		let p = |bc3: &[u32], drop: &[u32]| Plan { bc3: set(bc3), drop: set(drop) };
		assert_eq!(plan(&jobs, 2044, true), p(&[], &[]), "a level that fits stays RGBA8");
		assert_eq!(plan(&jobs, 2043, true), p(&[5], &[]), "the largest chain goes first");
		assert_eq!(plan(&jobs, 900, true), p(&[5, 6], &[]), "1048 left is still over 900: the next largest (lowest key on a tie)");
		// A device without BC cannot store BC3: what does not fit is dropped, the largest first.
		assert_eq!(plan(&jobs, 700, false), p(&[], &[5]), "680 left fits");
		// With every candidate in BC3 (592) the level still does not fit 500: the largest is dropped, then as few BC3 as needed.
		assert_eq!(plan(&jobs, 500, true), p(&[6], &[5]));
		assert_eq!(plan(&jobs, 150, true), p(&[7], &[5, 6]), "ties go by key");
		assert_eq!(plan(&jobs, 100, true), p(&[], &[5, 6, 7]));
		jobs.reverse();
		assert_eq!(plan(&jobs, 900, true), p(&[5, 6], &[]), "the same textures whatever the order of the jobs");
		assert_eq!(plan(&jobs, 150, true), p(&[7], &[5, 6]));
		let _ = std::fs::remove_dir_all(&dir);
	}

	/// A job whose level-0 file is a `side` x `side` DXT5 DDS with `mips` levels in the file, for a chain of `n` levels.
	fn dxt5_job(dir: &std::path::Path, key: u32, side: u32, mips: u32, n: usize) -> Job {
		let lv: Vec<Vec<u8>> = (0..mips).map(|k| vec![3u8; 16 * ((side >> k).max(4) as usize / 4).pow(2)]).collect();
		let f = super::super::dds::tests::dds(side, side, b"DXT5", [0; 4], 0, &lv.iter().map(Vec::as_slice).collect::<Vec<_>>());
		let path = dir.join(format!("d{key}.dds"));
		std::fs::write(&path, f).unwrap();
		Job { key, fp: Fp { w: 1, h: 1, hash: 0 }, n, files: [Some(PackFile { path, ext: Ext::Dds, mod_idx: 0 }), None, None, None] }
	}

	#[test]
	fn a_bc_file_with_generated_levels_is_planned_as_the_rgba8_chain_it_becomes() {
		let dir = tmp("plan_dxt");
		// 16x16 DXT5, 3 chain levels: with all 3 mips in the file it stays BC3 (256 + 64 + 16 = 336 bytes); with only level 0 in the file the
		// two lower levels are generated, the chain is RGBA8 (1024 + 256 + 64 = 1344 bytes) and may go to BC3 (336) when the level is over.
		let plan = |j: &Job, budget| plan_bc3(std::iter::once((j.key, j.n, &j.files[..])), budget, true);
		let full = dxt5_job(&dir, 1, 16, 3, 3);
		assert_eq!(plan(&full, 336), Plan::default(), "a complete BC chain fits as it is");
		let short = dxt5_job(&dir, 2, 16, 1, 3);
		assert_eq!(plan(&short, 1344), Plan::default(), "the RGBA8 chain fits");
		assert_eq!(plan(&short, 1343).bc3, [2].into(), "one byte over: stored as BC3");
		let _ = std::fs::remove_dir_all(&dir);
	}

	/// The plan sizes the chain `build` makes from the same files, in every case the two rules (named level file first, the file's own mip
	/// otherwise; whole chain or generated) tell apart.
	#[test]
	fn the_plan_sizes_agree_with_the_chains_that_get_built() {
		let _one = ONE_LEVEL_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
		let dir = tmp("plan_eq");
		let rgba = |w: u32, k: u32| {
			let px = vec![k as u8; (w * w * 4) as usize];
			super::super::dds::tests::dds(w, w, &[0; 4], [0xff, 0xff00, 0xff0000, 0xff00_0000], 32, &[&px])
		};
		let mut jobs = vec![
			// level 0 alone (n = 1, most textures): a BC3 file with every level, a short one, RGBA8; and a whole chain of RGBA8
			dxt5_job(&dir, 1, 16, 5, 1),
			dxt5_job(&dir, 2, 16, 1, 1),
			rgba_job_n(&dir, 3, 16, 1),
			rgba_job(&dir, 8, 16),
			// authored chain: complete file, short file, and the file's own mips next to a NAMED level file of another format (RGBA8 chain)
			dxt5_job(&dir, 4, 16, 3, 3),
			dxt5_job(&dir, 5, 16, 1, 3),
			rgba_job_n(&dir, 6, 8, 2),
			dxt5_job(&dir, 7, 16, 3, 3),
		];
		let named = dir.join("named.dds");
		std::fs::write(&named, rgba(8, 9)).unwrap();
		jobs[7].files[1] = Some(PackFile { path: named, ext: Ext::Dds, mod_idx: 0 });
		for bc in [true, false] {
			let plan: Vec<(u32, Option<(dds::Fmt, usize)>)> = jobs.iter().map(|j| (j.key, planned_chain(j.n, &j.files, bc).map(|c| (c.fmt, c.bytes)))).collect();
			let r = LevelRepl::with_limit(70, jobs.iter().map(|j| Job { key: j.key, fp: j.fp, n: j.n, files: j.files.clone() }).collect(), bc, usize::MAX >> 4);
			for h in r.spawn_workers(2) {
				h.join().unwrap();
			}
			for (key, want) in plan {
				let built = match &*r.entry(key).unwrap().cpu.lock().unwrap() {
					Cpu::Ready(t) => Some((t.fmt, t.bytes())),
					_ => None,
				};
				assert_eq!(built, want, "key {key} (device with BC: {bc})");
			}
			drop(r);
		}
		// The named RGBA8 level makes the whole chain RGBA8 although the DDS carries its own mips.
		assert_eq!(planned_chain(3, &jobs[7].files, true).unwrap().fmt, dds::Fmt::Rgba8);
		assert_eq!(planned_chain(3, &jobs[4].files, true).unwrap().fmt, dds::Fmt::Bc3);
		let _ = std::fs::remove_dir_all(&dir);
	}

	/// A BC7 DDS file (DXGI 98, DX10 alpha mode `alpha`) `side` x `side` with `mips` stored levels (the header claims `claim` of them) of
	/// mode 6 blocks with every field 0 (colour black, alpha 0: translucent, so the loader decodes it unless the alpha mode is 3).
	fn bc7_file(dir: &std::path::Path, name: &str, side: u32, mips: usize, claim: usize, alpha: u32) -> PackFile {
		let level = |k: usize| {
			let s = (side >> k).max(1) as usize;
			let mut v = vec![0u8; 16 * s.div_ceil(4).pow(2)];
			(0..v.len()).step_by(16).for_each(|i| v[i] = 0x40);
			v
		};
		let mut first = vec![0u8; 20];
		first.extend(level(0));
		let rest: Vec<Vec<u8>> = (1..mips).map(level).collect();
		let mut levels: Vec<&[u8]> = vec![&first];
		levels.extend(rest.iter().map(|v| &v[..]));
		let mut f = super::super::dds::tests::dds(side, side, b"DX10", [0; 4], 0, &levels);
		f[28..32].copy_from_slice(&(claim as u32).to_le_bytes());
		f[8..12].copy_from_slice(&(0x1007u32 | if claim > 1 { 0x20000 } else { 0 }).to_le_bytes());
		f[128..132].copy_from_slice(&98u32.to_le_bytes());
		f[140..144].copy_from_slice(&1u32.to_le_bytes());
		f[144..148].copy_from_slice(&alpha.to_le_bytes());
		let path = dir.join(name);
		std::fs::write(&path, f).unwrap();
		PackFile { path, ext: Ext::Dds, mod_idx: 0 }
	}

	/// The planner and the builder agree on BC7 chains: what `planned_chain` says is what the loader makes (format and bytes), on a device
	/// with and without BC. Covers the corner of the lane-bc7j review: a mode-3 BC7 base with its own mips plus a NAMED mode-0 BC7 level.
	#[test]
	fn a_bc7_chain_is_planned_as_the_chain_assemble_builds() {
		let _one = ONE_LEVEL_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
		let dir = tmp("plan_bc7");
		let job = |key: u32, n: usize, files: Vec<PackFile>| {
			let mut f: [Option<PackFile>; 4] = [None, None, None, None];
			files.into_iter().enumerate().for_each(|(k, p)| f[k] = Some(p));
			Job { key, fp: Fp { w: 1, h: 1, hash: 0 }, n, files: f }
		};
		let m3 = |name: &str, mips: usize, claim: usize| bc7_file(&dir, name, 8, mips, claim, 3);
		let jobs = vec![
			// 1: mode 3, own mips, no named level: stays BC7 (64 + 16 bytes), RGBA8 (256 + 64) on a device without BC.
			job(1, 2, vec![m3("a.dds", 4, 4)]),
			// 2: the same plus a named mode-0 BC7 level 1 (decoded to RGBA8): the whole chain is RGBA8 although the base has its own mips.
			job(2, 2, vec![m3("b.dds", 4, 4), bc7_file(&dir, "b1.dds", 4, 1, 1, 0)]),
			// 3: a file the loader decodes (alpha mode 0): RGBA8 with every mip (256 + 64 + 16 + 4).
			job(3, 4, vec![bc7_file(&dir, "c.dds", 8, 4, 4, 0)]),
			// 4: mode 3 whose header claims 4 mips but the file stores 1: level 1 is generated, RGBA8.
			job(4, 2, vec![m3("d.dds", 1, 4)]),
			// 5: mode 3 with an authored chain of 2 and a named mode-3 level 1: stays BC7.
			job(5, 2, vec![m3("e.dds", 1, 1), bc7_file(&dir, "e1.dds", 4, 1, 1, 3)]),
		];
		let want = |bc: bool| -> Vec<(u32, Option<(dds::Fmt, usize)>)> {
			let b7 = |f, bytes| Some(if bc { (f, bytes) } else { (dds::Fmt::Rgba8, 320) });
			vec![(1, b7(dds::Fmt::Bc7, 80)), (2, Some((dds::Fmt::Rgba8, 320))), (3, Some((dds::Fmt::Rgba8, 340))), (4, Some((dds::Fmt::Rgba8, 320))), (5, b7(dds::Fmt::Bc7, 80))]
		};
		for bc in [true, false] {
			let plan: Vec<_> = jobs.iter().map(|j| (j.key, planned_chain(j.n, &j.files, bc).map(|c| (c.fmt, c.bytes)))).collect();
			assert_eq!(plan, want(bc), "device with BC: {bc}");
			let r = LevelRepl::with_limit(80, jobs.iter().map(|j| Job { key: j.key, fp: j.fp, n: j.n, files: j.files.clone() }).collect(), bc, usize::MAX >> 4);
			for h in r.spawn_workers(2) {
				h.join().unwrap();
			}
			for (key, want) in want(bc) {
				let built = match &*r.entry(key).unwrap().cpu.lock().unwrap() {
					Cpu::Ready(t) => Some((t.fmt, t.bytes())),
					_ => None,
				};
				assert_eq!(built, want, "key {key} (device with BC: {bc})");
			}
			drop(r);
		}
		// A budget the chains do not fit as RGBA8 makes the plan store the RGBA8 ones as BC3 (key 3: 340 bytes, 106 as BC3) and nothing else.
		let plan = plan_bc3(jobs.iter().map(|j| (j.key, j.n, &j.files[..])), 1100, true);
		assert_eq!(plan, Plan { bc3: [3].into(), drop: Default::default() }, "the 340-byte chain is the largest RGBA8 candidate that frees enough");
		let _ = std::fs::remove_dir_all(&dir);
	}

	/// F2: the upload chain is made from the original files only, so the chain a loader thread rebuilds when it is missing is byte for byte
	/// the chain it held, for n < 2 and n >= 2, BC and RGBA8 files, and on a device with or without BC.
	#[test]
	fn a_held_plain_chain_and_a_rebuilt_one_are_the_same_bytes() {
		let _one = ONE_LEVEL_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
		let dir = tmp("plain_same");
		let before = rebuild_tests::counters();
		for bc in [true, false] {
			let jobs = vec![rgba_job_n(&dir, 1, 8, 2), dxt5_job(&dir, 2, 8, 1, 2), dxt5_job(&dir, 3, 8, 4, 2), rgba_job_n(&dir, 4, 8, 1), dxt5_job(&dir, 5, 8, 4, 1), dxt5_job(&dir, 6, 16, 1, 1)];
			let r = LevelRepl::with_limit(71, jobs, bc, usize::MAX >> 4);
			for h in r.spawn_workers(3) {
				h.join().unwrap();
			}
			for key in 1..=6 {
				let e = r.entry(key).unwrap();
				assert!(matches!(&*e.cpu.lock().unwrap(), Cpu::Ready(_)), "key {key} not ready");
				let held = e.plain.lock().unwrap().clone().unwrap_or_else(|| panic!("key {key}: no plain chain held"));
				e.release_plain();
				let here = LOADS_HERE.with(|c| c.get());
				assert!(e.plain_for_bind(&r).unwrap().is_none());
				assert_eq!(LOADS_HERE.with(|c| c.get()), here, "key {key}: the calling thread decoded a file");
				let rebuilt = wait_plain(e);
				assert_eq!(held.fmt, rebuilt.fmt, "key {key} (device with BC: {bc})");
				assert_eq!(held.mips.len(), rebuilt.mips.len(), "key {key}");
				for (a, b) in held.mips.iter().zip(&rebuilt.mips) {
					assert_eq!((a.w, a.h, &a.data), (b.w, b.h, &b.data), "key {key}: held plain chain and rebuilt one differ");
				}
				assert_eq!(held.mips.last().map(|m| (m.w, m.h)), Some((1, 1)), "key {key}: a full chain");
				e.release_plain();
			}
			drop(r);
			rebuild_tests::settle(before);
		}
		let _ = std::fs::remove_dir_all(&dir);
	}

	/// F2: the plain chain does not depend on how the MAIN chain was stored. A texture the plan stores as BC3 still uploads the chain of
	/// the original file (v0.4.0), also when the budget had no room to hold it and a loader thread rebuilds it. A missing plain chain at
	/// bind time never decodes on the calling thread, never takes the main chain, and leaves the entry waiting until it is rebuilt.
	#[test]
	fn a_missing_plain_chain_is_rebuilt_from_the_file_and_not_from_the_main_chain() {
		let _one = ONE_LEVEL_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
		let dir = tmp("plain_bc3");
		let (plain_before, rebuilt_before) = (PLAIN.load(Relaxed), REBUILT.load(Relaxed));
		let before = rebuild_tests::counters();
		// 16x16 RGBA8, n = 1: main 1024 bytes (256 as BC3), plain chain 1364. The limit holds the main chain as BC3 only, no plain at all.
		let job = rgba_job_n(&dir, 9, 16, 1);
		let want = chain::plain_of(load_file(job.files[0].as_ref().unwrap()).unwrap()).unwrap();
		let r = LevelRepl::with_limit(73, vec![job], true, 300);
		for h in r.spawn_workers(1) {
			h.join().unwrap();
		}
		let e = r.entry(9).unwrap();
		let main_fmt = match &*e.cpu.lock().unwrap() {
			Cpu::Ready(main) => main.fmt,
			_ => panic!("not ready"),
		};
		assert_eq!(main_fmt, dds::Fmt::Bc3, "the main chain was stored as BC3 to fit");
		assert!(e.plain.lock().unwrap().is_none(), "no room for the plain chain");
		let here = LOADS_HERE.with(|c| c.get());
		assert!(e.plain_for_bind(&r).unwrap().is_none(), "the slot waits");
		assert!(e.plain_for_bind(&r).unwrap().is_none(), "...and asks once");
		assert_eq!(LOADS_HERE.with(|c| c.get()), here, "the calling thread decoded nothing");
		let rebuilt = wait_plain(e);
		assert_eq!(rebuilt.fmt, dds::Fmt::Rgba8, "the original's picture, not a BC3 re-encode");
		assert_eq!(rebuilt.mips.iter().map(|m| &m.data).collect::<Vec<_>>(), want.mips.iter().map(|m| &m.data).collect::<Vec<_>>());
		assert!(Arc::ptr_eq(&e.plain_for_bind(&r).unwrap().unwrap(), &rebuilt), "held: handed out as it is");
		// A rebuilt chain is not shed for room; the upload gives it back.
		r.shed_plains();
		assert!(e.plain.lock().unwrap().is_some());
		e.release_plain();
		assert_eq!((PLAIN.load(Relaxed), REBUILT.load(Relaxed)), (plain_before, rebuilt_before));
		drop(r);
		rebuild_tests::settle(before);
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn a_level_that_fits_only_with_bc3_ends_with_every_entry_ready_whatever_the_thread_order() {
		let _one = ONE_LEVEL_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
		let dir = tmp("bc3");
		let (before, mains_before) = (USED.load(Relaxed), MAINS.load(Relaxed));
		// 16x16 (1364 bytes as RGBA8, 368 as BC3) + two 8x8 (340 each, 112) = 2044 as RGBA8, 1048 with the large one in BC3.
		let limit = 1100;
		for round in 0..24u32 {
			let workers = 1 + (round % 4) as usize;
			let (ready, counts, bc3) = run_level(40 + round, vec![rgba_job(&dir, 5, 16), rgba_job(&dir, 6, 8), rgba_job(&dir, 7, 8)], limit, workers);
			assert_eq!((ready, counts, bc3), (vec![5, 6, 7], [3, 0, 0, 1], vec![5]), "round {round} ({workers} workers): the plan, not the thread order, picks the BC3 texture");
		}
		assert_eq!((USED.load(Relaxed), MAINS.load(Relaxed)), (before, mains_before));
		let _ = std::fs::remove_dir_all(&dir);
	}

	/// F1: the plain chains compete with the main chains for the slack the budget leaves. Every main chain fits (the plan sees no
	/// reason to compress or drop), so none may ever be refused, whatever the workers do at the same time. The slack is exactly one
	/// plain chain minus a byte, with as many workers as jobs: all of them reach `make_plain` / `admit` together, the regime where
	/// agent 2's unlocked `keep_plain` (USED added first, rolled back when it did not fit) made `admit` see a budget that was not there
	/// (it refused a fitting chain in 21 of 6000 rounds there (about 10 of the 3000 rounds here); this test fails on that code).
	#[test]
	fn many_workers_never_refuse_a_main_chain_that_fits() {
		let _one = ONE_LEVEL_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
		let dir = tmp("race");
		let (before, mains_before) = (USED.load(Relaxed), MAINS.load(Relaxed));
		let mut bad = Vec::new();
		for &(jobs_n, workers_max) in &[(16u32, 16usize), (8, 8), (32, 32)] {
			// 16x16, two levels: 1024 + 256 = 1280 bytes main; the plain chain is 1364.
			let limit = jobs_n as usize * 1280 + 1364 - 1;
			// The files are written once per config; every round builds its jobs from the paths.
			let proto = (0..jobs_n).map(|k| rgba_job_n(&dir, k, 16, 2)).collect::<Vec<_>>();
			let jobs = || proto.iter().map(|j| Job { key: j.key, fp: j.fp, n: j.n, files: j.files.clone() }).collect::<Vec<_>>();
			let all: Vec<u32> = (0..jobs_n).collect();
			for round in 0..1000u32 {
				let workers = 2 + round as usize % (workers_max - 1);
				let (ready, counts, bc3) = run_level(100 + round, jobs(), limit, workers);
				if (&ready, counts, &bc3) != (&all, [jobs_n as usize, 0, 0, 0], &vec![]) {
					bad.push((jobs_n, round, workers));
				}
			}
		}
		assert!(bad.is_empty(), "a chain that fits was refused ({} rounds; jobs, round, workers): {:?}", bad.len(), &bad[..bad.len().min(8)]);
		assert_eq!((USED.load(Relaxed), MAINS.load(Relaxed), PLAIN.load(Relaxed)), (before, mains_before, 0));
		let _ = std::fs::remove_dir_all(&dir);
	}

	/// F3: a level that does not fit even with every candidate in BC3 ends with the same textures ready on every run.
	#[test]
	fn a_level_that_cannot_fit_drops_the_same_textures_whatever_the_thread_order() {
		let _one = ONE_LEVEL_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
		let dir = tmp("drop");
		let (before, mains_before) = (USED.load(Relaxed), MAINS.load(Relaxed));
		let sides = [8u32, 16, 24, 32, 12, 20];
		let jobs = || (0..18u32).map(|k| rgba_job_n(&dir, k, sides[k as usize % 6], 1 + (k % 3) as usize)).collect::<Vec<_>>();
		let limit = 6000;
		let plan = plan_bc3(jobs().iter().map(|j| (j.key, j.n, &j.files[..])), limit, true);
		assert!(!plan.drop.is_empty() && !plan.bc3.is_empty(), "the test needs a level that drops and compresses: {plan:?}");
		let want: Vec<u32> = (0..18).filter(|k| !plan.drop.contains(k)).collect();
		let mut want_bc3: Vec<u32> = plan.bc3.iter().copied().collect();
		want_bc3.sort_unstable();
		for round in 0..24u32 {
			let workers = 1 + (round % 6) as usize;
			let (ready, counts, bc3) = run_level(200 + round, jobs(), limit, workers);
			assert_eq!((&ready, &bc3), (&want, &want_bc3), "round {round} ({workers} workers)");
			assert_eq!(counts, [want.len(), 0, plan.drop.len(), want_bc3.len()], "round {round}");
		}
		assert_eq!((USED.load(Relaxed), MAINS.load(Relaxed)), (before, mains_before));
		let _ = std::fs::remove_dir_all(&dir);
	}

	/// F4 (a): a BC base that stores its mips next to a NAMED level file of another format. `assemble` takes the named file first, so
	/// the chain is mixed and becomes RGBA8, whatever the base's own mips are. (b): a header that claims more mips than the file holds
	/// has no such levels: they are generated, so again the RGBA8 chain.
	#[test]
	fn the_plan_follows_assemble_for_named_level_files_and_for_a_header_that_claims_too_many_mips() {
		let dir = tmp("plan_f4");
		let plan = |j: &Job, budget| plan_bc3(std::iter::once((j.key, j.n, &j.files[..])), budget, true);
		// (a) 16x16 DXT5 with all 3 levels stored, level 1 named as an 8x8 RGBA8 file.
		let mut a = dxt5_job(&dir, 1, 16, 3, 3);
		let px = vec![5u8; 8 * 8 * 4];
		let lvl = super::super::dds::tests::dds(8, 8, &[0; 4], [0xff, 0xff00, 0xff0000, 0xff00_0000], 32, &[&px]);
		std::fs::write(dir.join("a_l1.dds"), lvl).unwrap();
		assert_eq!(planned_chain(3, &a.files, true).unwrap().fmt, dds::Fmt::Bc3, "without the named file the chain stays BC3");
		a.files[1] = Some(PackFile { path: dir.join("a_l1.dds"), ext: Ext::Dds, mod_idx: 0 });
		let c = planned_chain(3, &a.files, true).unwrap();
		assert_eq!((c.fmt, c.bytes), (dds::Fmt::Rgba8, 1024 + 256 + 64), "mixed levels: the whole chain is RGBA8");
		assert_eq!(plan(&a, 1343).bc3, [1].into());
		// (b) the header says 3 levels, the file stores 2 (256 + 64 bytes): level 2 is generated, so RGBA8 (1344).
		let b = dxt5_job(&dir, 2, 16, 2, 3);
		let mut bytes = std::fs::read(b.files[0].as_ref().unwrap().path.clone()).unwrap();
		bytes[28..32].copy_from_slice(&3u32.to_le_bytes());
		std::fs::write(&b.files[0].as_ref().unwrap().path, bytes).unwrap();
		let f = b.files[0].as_ref().unwrap();
		assert_eq!(peek_head(f).map(|h| h.3), Some(2), "stored levels, not the claimed ones");
		let c = planned_chain(3, &b.files, true).unwrap();
		assert_eq!((c.fmt, c.bytes), (dds::Fmt::Rgba8, 1344));
		assert_eq!(plan(&b, 1343).bc3, [2].into());
		// ...and `build` agrees (the generated level makes the chain RGBA8).
		let _one = ONE_LEVEL_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
		let r = LevelRepl::with_limit(72, vec![b], true, usize::MAX >> 4);
		for h in r.spawn_workers(1) {
			h.join().unwrap();
		}
		let built = match &*r.entry(2).unwrap().cpu.lock().unwrap() {
			Cpu::Ready(t) => Some((t.fmt, t.bytes())),
			_ => None,
		};
		assert_eq!(built, Some((dds::Fmt::Rgba8, 1344)));
		drop(r);
		let _ = std::fs::remove_dir_all(&dir);
	}

	/// A job whose level-0 file is a `side` x `side` BC7 DDS (DXGI 98, DX10 alpha mode `alpha`) of mode 6 blocks with every field 0:
	/// colour black, alpha 0 (translucent, so the loader turns it into RGBA8 unless the alpha mode is 3).
	fn bc7_job(dir: &std::path::Path, key: u32, side: u32, alpha: u32) -> Job {
		bc7_job_with(dir, key, side, alpha, false)
	}

	/// `bc7_job`, with `opaque` every block has both alpha endpoints 255 (A0 = A1 = 127 and both p-bits set: bits 49..64): the loader keeps it BC7.
	fn bc7_job_with(dir: &std::path::Path, key: u32, side: u32, alpha: u32, opaque: bool) -> Job {
		let mut blocks = vec![0u8; 20 + 16 * (side as usize / 4).pow(2)];
		(0..blocks.len() - 20).step_by(16).for_each(|i| blocks[20 + i] = 0x40);
		if opaque {
			(0..blocks.len() - 20).step_by(16).for_each(|i| {
				blocks[20 + i + 6] |= 0xfe;
				blocks[20 + i + 7] = 0xff;
				blocks[20 + i + 8] |= 1;
			});
		}
		let mut f = super::super::dds::tests::dds(side, side, b"DX10", [0; 4], 0, &[&blocks]);
		f[128..132].copy_from_slice(&98u32.to_le_bytes());
		f[140..144].copy_from_slice(&1u32.to_le_bytes());
		f[144..148].copy_from_slice(&alpha.to_le_bytes());
		let path = dir.join(format!("b{key}.dds"));
		std::fs::write(&path, f).unwrap();
		Job { key, fp: Fp { w: 1, h: 1, hash: 0 }, n: 1, files: [Some(PackFile { path, ext: Ext::Dds, mod_idx: 0 }), None, None, None] }
	}

	#[test]
	fn bc7_files_are_planned_as_the_rgba8_the_loader_makes_of_them_and_a_level_of_them_ends_ready() {
		let _one = ONE_LEVEL_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
		let dir = std::env::temp_dir().join(format!("texpack_bc7_{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let before = USED.load(Relaxed);
		// 16x16 (1024 bytes as RGBA8, 256 as BC7 or BC3) + two 8x8 (256 as RGBA8, 128 as BC7): 1536 as RGBA8, 512 as BC7, 768 with the
		// large one in BC3.
		let jobs = vec![bc7_job(&dir, 5, 16, 0), bc7_job(&dir, 6, 8, 0), bc7_job(&dir, 7, 8, 0)];
		let plan = |jobs: &[Job], budget, bc| plan_bc3(jobs.iter().map(|j| (j.key, j.n, &j.files[..])), budget, bc);
		assert_eq!(plan(&jobs, 1536, true), Plan::default(), "fits as the RGBA8 it becomes");
		assert_eq!(plan(&jobs, 1000, true), Plan { bc3: [5].into(), drop: Default::default() }, "512 as BC7 would fit 1000, but the loader decodes these: the large one goes to BC3");
		assert_eq!(plan(&jobs, 1000, false), Plan { bc3: Default::default(), drop: [5].into() }, "no BC3 without BC: the large chain is dropped instead");
		// DX10 alpha mode 3 stays BC7 (512 bytes): nothing to plan.
		let opaque = vec![bc7_job(&dir, 5, 16, 3), bc7_job(&dir, 6, 8, 3), bc7_job(&dir, 7, 8, 3)];
		assert_eq!(plan(&opaque, 1000, true), Plan::default());
		let limit = 1000; // a level is planned and admitted as if it were alone (its own chains against its limit)
		for round in 0..8u32 {
			let workers = 1 + (round % 4) as usize;
			let jobs = vec![bc7_job(&dir, 5, 16, 0), bc7_job(&dir, 6, 8, 0), bc7_job(&dir, 7, 8, 0)];
			let r = LevelRepl::with_limit(60 + round, jobs, true, limit);
			r.start(workers);
			r.wait(Duration::from_secs(10));
			assert_eq!(r.pending(), 0);
			for k in [5, 6, 7] {
				let t = match &*r.entry(k).unwrap().cpu.lock().unwrap() {
					Cpu::Ready(t) => t.clone(),
					_ => panic!("round {round} ({workers} workers): key {k} is not ready"),
				};
				assert_eq!(t.fmt, if k == 5 { dds::Fmt::Bc3 } else { dds::Fmt::Rgba8 }, "round {round}");
			}
			let s = r.stats.lock().unwrap();
			assert_eq!((s.ready, s.failed, s.over_budget, s.recompressed), (3, 0, 0, 1), "round {round}");
		}
		for _ in 0..500 {
			if USED.load(Relaxed) == before {
				break;
			}
			std::thread::sleep(Duration::from_millis(4));
		}
		assert_eq!(USED.load(Relaxed), before);
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn a_may_decode_bc7_that_stays_bc7_is_within_the_plan() {
		let _one = ONE_LEVEL_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
		let dir = std::env::temp_dir().join(format!("texpack_bc7o_{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		// 16x16, DX10 alpha mode 0 (may decode): planned as the RGBA8 it could become (1024), but every texel is opaque, so the loader keeps BC7 (256).
		let job = bc7_job_with(&dir, 9, 16, 0, true);
		let plan = plan_bc3([(job.key, job.n, &job.files[..])].into_iter(), 1024, true);
		assert_eq!(plan, Plan::default(), "the plan counts 1024 bytes and fits");
		let r = LevelRepl::with_limit(80, vec![job], true, usize::MAX >> 4);
		for h in r.spawn_workers(1) {
			h.join().unwrap();
		}
		let built = match &*r.entry(9).unwrap().cpu.lock().unwrap() {
			Cpu::Ready(t) => Some((t.fmt, t.bytes())),
			_ => None,
		};
		assert_eq!(built, Some((dds::Fmt::Bc7, 256)), "built BC7, within the 1024 planned");
		drop(r);
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn header_peek_reads_the_size_without_the_pixels() {
		let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
		png.extend_from_slice(b"IHDR");
		png.extend_from_slice(&5000u32.to_be_bytes());
		png.extend_from_slice(&3000u32.to_be_bytes());
		assert_eq!(super::super::png::peek(&png), Some((5000, 3000)));
		let dds = super::super::dds::tests::dds(8192, 4096, b"DXT5", [0; 4], 0, &[]);
		assert_eq!(super::super::dds::peek(&dds), Some((8192, 4096, dds::Fmt::Bc3)));
		assert_eq!(super::super::dds::peek(&dds[..100]), None);
	}
}
