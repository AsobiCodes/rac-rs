//! EFFECTS LAYER, VU1 owners: a queue callback whose packets go through a VU1 microprogram (VIF MSCAL /
//! MSCALF / MSCNT) is drawn from the GS packets that program XGKICKs, with no per-effect code.
//!
//! Source of the VU1 state at the owner's start [C]: the field's own VIF1 chain.
//! - Timing [C, measured r0230 2026-10-13]: the chain the game builds in field N is kicked in field N + 1 (every queue
//!   callback's words show up in the next field's jobs, none in its own). So at field N's end the oracle has kicked
//!   chain N - 1 and chain N sits in RAM (tag list page + packet buffers, double-buffered).
//! - The chain start: the start of the chain kicked this field (chain N - 1) is found by matching its first job
//!   (`choose`). A start counts only if its first tag opens like job 0 (the same two VIF words), its walk hits the first tag
//!   (`c0`) of EVERY owner of the field (`OwnerHits`; a walk that hits none is never taken) and passes through and leaves the last
//!   owner's range. Tried first: the level's known starts, the one not used last field first; when none hits every owner, a scan of
//!   the 4 KB pages that open like job 0 (only a scan winner is remembered; a scan that could not beat a partial known start is not
//!   repeated while that start gives the same hits, `Starts.dry`). The known starts are forgotten on a level change. The tag pages
//!   alternate (Rilgar r0230 0x330000 / 0x490000; Eudora two pages with the same first block, so a first-block match alone is ambiguous).
//! - Chain N is walked from RAM the way the oracle's DMA walk does it (`mem.rs vif1_kick`: tag ids 0-7, call / ret
//!   stack, TTE tag words with block marks, a piece ends at a tag with the IRQ bit = one kick) only up to the end of
//!   the field's last VU1 owner (point P): the chain's final tag is written after the field's end (r0230: RAM at the
//!   field's end goes on past the kicked end into stale words), so the rest of the chain comes from the jobs.
//! - Threads (stage 7): `prep` walks RAM on the worker, `run` feeds and runs VU1 on another thread while the worker
//!   builds the field's snapshot (retail.rs), `end` only keeps the jobs (moved, not copied, in the mirror mode).
//! - At field N + 1's end (`end`) the jobs (= chain N as kicked) are checked word for word against the walk up to P
//!   and fed into the model from P on; a field without a walk feeds all its jobs. So the model sees each chain once,
//!   exactly as kicked; a failed check is counted (`fails`, logged) and the chain start is searched again.
//! - The model: a persistent VIF1 + VU1 (`gsfull::vif::Vif1` in shadow mode + `gsfull::vu1::Vu1`, started from the
//!   savestate's vu1Memory.bin / vu1MicroMem.bin) applies the whole chain: MPG (micro memory), UNPACK (data memory
//!   with CYCLE / MASK / ROW / COL / MODE), BASE / OFFSET / ITOP / double buffer. Inside an owner's block range (its
//!   tags lie in [c0, c1), efx_own) VU1 runs (native program ports, interpreter fallback) and its XGKICK packets plus
//!   DIRECT data are collected in program order; elsewhere the programs are not run (the world passes: cost).
//!   RAC_EFX_VU_FULL=1 runs every program (check of that shortcut: the owners' output hashes must not change).
use super::efx_own::{self, Own};
use crate::gsfull::vif::{Out, Vif1};
use crate::gsfull::vu1::Vu1;
use ee_oracle::gs::Job;
use ee_oracle::Machine;
use std::sync::mpsc::{channel, Receiver, Sender};

/// One kick's worth of the chain: VIF words, block marks (index, phase 2 = tag words / 0 = data), tag addresses.
#[derive(Default, Clone)]
struct Piece {
	w: Vec<u32>,
	marks: Vec<(usize, u32)>,
	tags: Vec<(usize, u32)>,
}

/// Buffers of the chain pieces that are free again (speed lane A): the walk copies ~2.4 MB per field into fresh vectors, and
/// fresh large allocations cost page faults (about half of the copy time). The VU1 thread sends the buffers of a finished
/// field back (`VuModel::back`), the picture half takes them for the next walk (`Picker::absorb`, `take_buf`).
#[derive(Default)]
struct Spare {
	w: Vec<Vec<u32>>,
	m: Vec<Vec<(usize, u32)>>,
	t: Vec<Vec<(usize, u32)>>,
}

/// A cleared buffer of at least `need` elements from `pool` (the smallest that fits), else a new one with some slack.
fn take_buf<T>(pool: &mut Vec<Vec<T>>, need: usize) -> Vec<T> {
	let best = pool.iter().enumerate().filter(|(_, b)| b.capacity() >= need).min_by_key(|(_, b)| b.capacity()).map(|(i, _)| i);
	match best {
		Some(i) => {
			let mut v = pool.swap_remove(i);
			v.clear();
			v
		}
		None => {
			FRESH.fetch_add(need as u64 * std::mem::size_of::<T>() as u64, std::sync::atomic::Ordering::Relaxed);
			Vec::with_capacity(need + need / 8)
		}
	}
}

/// The VIF1 + VU1 model and the check state of `run`: lives on the VU1 thread (`Shadow::submit`), no machine access.
struct VuModel {
	vif: Vif1,
	vu: Vu1,
	full: bool,
	/// The chain walked at the last field's end up to P (pieces' words, the last one cut at P).
	prev: Vec<Vec<u32>>,
	pub fails: u32,
	scratch: Out,
	/// The finished field's chain buffers go back to the picture half.
	back: Option<Sender<Spare>>,
	/// RAC_VIF_FEED_CHECK=1: a second VIF1 + VU1 fed the same words with the eager (non-lazy) model; (feeds, mismatches).
	chk: Option<Box<(Vif1, Vu1, Out)>>,
	chk_n: (u64, u64),
	chk_fields: u64,
	fields_seen: u64,
}

/// One field for the VU1 thread: the walk (`Shadow::prep`), the machine words the build needs, the reply (None: the
/// snapshot has no effects layer this field; the model still runs).
struct VuJob {
	w: Work,
	bi: super::efx_draw::BuildIn,
	field: u32,
	reply: Option<Sender<FxOut>>,
}

/// The effects layer's world quads and strips of one field (`efx_draw::build_in`).
pub type FxOut = (Vec<crate::particles::WorldQuad>, Vec<crate::particles::WorldStrip>);

/// The VIF1 jobs of the fields since the last `prep`: (words, block marks).
pub type Pending = Vec<(Vec<u32>, Vec<(usize, u32)>)>;

/// The worker's half: the tee that keeps the machine's VIF1 jobs (`begin` / `end`).
pub struct Shadow {
	tx: Sender<Job>,
	rx: Receiver<Job>,
	/// The machine's own sender while the tee is in.
	fwd: Option<Option<Sender<Job>>>,
	/// The VIF1 jobs of the fields since the last `run` (words, block marks): the chain kicked then, fed by `run`.
	pending: Pending,
}

/// The picture half (speed lane A: it may live on its own thread, `retail/snap_pipe.rs`): the chain walk of the field's end
/// (`prep`) and the hand-over to the VU1 thread (`submit`). No machine access except through the `&Machine` it is given.
pub struct Picker {
	/// The model until the first `submit` moves it to its thread.
	model: Option<VuModel>,
	th: Option<Sender<VuJob>>,
	/// Chain start addresses of this level (see `Starts`).
	starts: Starts,
	spare: Spare,
	spare_rx: Receiver<Spare>,
}

/// One field's work for `Shadow::run`, prepared from RAM by `Shadow::prep` on the worker (the walk reads RAM; the
/// feed and the VU1 runs do not, so they run on another thread while the worker builds the field's snapshot).
pub struct Work {
	chain: Option<Vec<Piece>>,
	/// (owner index, c0, c1) of the unclaimed VU1 owners, chain order.
	tg: Vec<(usize, u32, u32)>,
	walk_ms: f64,
	fns: Vec<u32>,
	/// The VIF1 jobs kicked this field (`Shadow::pending` at the prep): chain N - 1 as the machine kicked it.
	vj: Vec<(Vec<u32>, Vec<(usize, u32)>)>,
}

/// Speed lane A: picture-half section timers (ns, summed since the last `snap_pipe::report`): 0 capture, 1 census, 2 plan, 3 build
/// (copy), 4 discovery scan (incl. its plan / build), 5 words copied, 6 discovery runs, 7 tags planned.
/// Bytes of chain buffers allocated fresh because the pool had none big enough (RAC_PERF_LOG spike print).
pub static FRESH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static SP: [std::sync::atomic::AtomicU64; 8] = [const { std::sync::atomic::AtomicU64::new(0) }; 8];

pub fn sp_add(i: usize, v: u64) {
	SP[i].fetch_add(v, std::sync::atomic::Ordering::Relaxed);
}

/// RAC_EFX_LOG set.
fn log() -> bool {
	static L: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*L.get_or_init(|| std::env::var_os("RAC_EFX_LOG").is_some())
}

/// One owner's GS output (index into the field's `Own` list, GIF qwords in program order).
pub type VuOut = Vec<(usize, Vec<u128>)>;

fn qw(m: &Machine, a: u32) -> u128 {
	let (b, i) = if a & 0x8000_0000 != 0 { (&m.cpu.mem.spr, (a & 0x3ff0) as usize) } else { (&m.cpu.mem.ram, (a & 0x01ff_fff0) as usize) };
	b.get(i..i + 16).map(|s| u128::from_le_bytes(s.try_into().unwrap())).unwrap_or(0)
}

/// One tag of a planned chain walk: its address, the tag qword, the data source and the qword count.
type PlanTag = (u32, u128, u32, u32);

/// The VIF1 chain from `start` as the oracle's DMA walk feeds it (TTE on, a piece per kick: it ends at a tag with the
/// IRQ bit, the next kick goes on behind it; the chain ends at an end / refe tag or a ret with an empty stack).
/// `stop(tadr)` true ends the walk before that tag (the chain so far is returned).
/// Two phases (speed lane A): `plan_chain` only follows the tags (cheap, no data copy: a candidate start that does not pass
/// through the owners costs just this), `build_chain` copies the data of the winner once. The pieces are exactly what the
/// one-pass walk built.
#[allow(dead_code)]
fn walk_chain(m: &Machine, start: u32, stop: impl FnMut(u32) -> bool) -> Option<Vec<Piece>> {
	let plan = plan_chain(m, start, stop)?;
	Some(build_chain(m, &plan, &mut Spare::default()))
}

fn plan_chain(m: &Machine, start: u32, mut stop: impl FnMut(u32) -> bool) -> Option<Vec<Vec<PlanTag>>> {
	let mut out: Vec<Vec<PlanTag>> = Vec::new();
	let mut tadr = start;
	let mut asr = [0u32; 2];
	let mut asp = 0usize;
	let mut n = 0u32;
	for _ in 0..16 {
		let mut p: Vec<PlanTag> = Vec::new();
		// Words the piece has so far (`Piece::w.len()` of the one-pass walk: 2 tag words + the data per tag).
		let mut wl = 0usize;
		loop {
			n += 1;
			if n > 400_000 || wl > 4_000_000 {
				return None;
			}
			if stop(tadr) {
				out.push(p);
				return Some(out);
			}
			let t = qw(m, tadr & 0x01ff_fff0);
			let tag = t as u64;
			let q = (tag & 0xffff) as u32;
			let id = ((tag >> 28) & 7) as u32;
			let addr = ((tag >> 32) as u32) & 0x7fff_fff0;
			let (data, next, end) = match id {
				0 => (addr, tadr + 16, true),
				1 => (tadr + 16, tadr + 16 + q * 16, false),
				2 => (tadr + 16, addr, false),
				3 | 4 => (addr, tadr + 16, false),
				5 => {
					if asp < 2 {
						asr[asp] = tadr + 16 + q * 16;
						asp += 1;
					}
					(tadr + 16, addr, false)
				}
				6 => {
					if asp > 0 {
						asp -= 1;
						(tadr + 16, asr[asp], false)
					} else {
						(tadr + 16, tadr + 16 + q * 16, true)
					}
				}
				_ => (tadr + 16, tadr + 16 + q * 16, true),
			};
			let src = if id == 3 || id == 4 { (tag >> 32) as u32 & 0xffff_fff0 } else { data };
			p.push((tadr, t, src, q));
			wl += 2 + q as usize * 4;
			tadr = next;
			if end {
				out.push(p);
				return Some(out);
			}
			if (tag >> 31) & 1 != 0 {
				break;
			}
		}
		out.push(p);
	}
	None
}

fn build_chain(m: &Machine, plan: &[Vec<PlanTag>], spare: &mut Spare) -> Vec<Piece> {
	plan.iter()
		.map(|tags| {
			// The sizes are known from the plan: one allocation per vector (the one-pass walk grew them tag by tag), reused
			// buffers when the VU1 thread gave some back.
			let mut p = Piece { w: take_buf(&mut spare.w, tags.iter().map(|t| 2 + t.3 as usize * 4).sum()), marks: take_buf(&mut spare.m, tags.len() * 2), tags: take_buf(&mut spare.t, tags.len()) };
			for &(tadr, t, src, q) in tags {
				p.marks.push((p.w.len(), 2));
				p.tags.push((p.w.len(), tadr));
				p.w.push((t >> 64) as u32);
				p.w.push((t >> 96) as u32);
				p.marks.push((p.w.len(), 0));
				// RAM data in one piece (the usual case: the chain's geometry blocks are most of the walk) copies as a slice; the
				// scratchpad, the wrap of the address mask and anything out of range go qword by qword as `qw` reads them.
				let lo = (src & 0x01ff_fff0) as usize;
				let n = q as usize * 16;
				if src & 0x8000_0000 == 0 && src.checked_add(q * 16).is_some_and(|e| e <= 0x8000_0000) && lo + n <= m.cpu.mem.ram.len() {
					append_words(&mut p.w, &m.cpu.mem.ram[lo..lo + n]);
				} else {
					for k in 0..q {
						let d = qw(m, src.wrapping_add(k * 16));
						p.w.extend([d as u32, (d >> 32) as u32, (d >> 64) as u32, (d >> 96) as u32]);
					}
				}
			}
			p
		})
		.collect()
}

/// Appends little-endian u32 words read from `b` (len a multiple of 4): on little-endian hosts one memcpy, elsewhere word by word.
fn append_words(w: &mut Vec<u32>, b: &[u8]) {
	debug_assert!(b.len() % 4 == 0);
	#[cfg(target_endian = "little")]
	{
		w.reserve(b.len() / 4);
		// SAFETY: `reserve` made room for b.len() / 4 more words; the destination is the Vec's spare capacity, disjoint from `b`.
		unsafe {
			std::ptr::copy_nonoverlapping(b.as_ptr(), w.as_mut_ptr().add(w.len()) as *mut u8, b.len());
			w.set_len(w.len() + b.len() / 4);
		}
	}
	#[cfg(not(target_endian = "little"))]
	w.extend(b.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])));
}

impl Work {
	/// FNV-1a of everything the walk produced (not the timing): RAC_SNAP_CHECK compares the old and the new path with it.
	pub fn sig(&self) -> u64 {
		let mut h = 0xcbf2_9ce4_8422_2325u64;
		let mut put = |v: u64| h = (h ^ v).wrapping_mul(0x0100_0000_01b3);
		match &self.chain {
			None => put(0xdead),
			Some(c) => {
				put(c.len() as u64);
				for p in c {
					put(p.w.len() as u64);
					p.w.iter().for_each(|&x| put(x as u64));
					p.marks.iter().for_each(|&(a, b)| put((a as u64) << 8 ^ b as u64));
					p.tags.iter().for_each(|&(a, b)| put((a as u64) << 32 ^ b as u64));
				}
			}
		}
		self.tg.iter().for_each(|&(a, b, c)| put((a as u64) << 48 ^ (b as u64) << 24 ^ c as u64));
		self.fns.iter().for_each(|&x| put(x as u64));
		for (w, mk) in &self.vj {
			w.iter().for_each(|&x| put(x as u64));
			mk.iter().for_each(|&(a, b)| put((a as u64) << 8 ^ b as u64));
		}
		h
	}
}

impl Shadow {
	pub fn new() -> Shadow {
		let (tx, rx) = channel();
		Shadow { tx, rx, fwd: None, pending: Vec::new() }
	}

	/// The VIF1 jobs kicked since the last call, for `Picker::prep` (moved out: the next field's jobs collect again).
	pub fn take_pending(&mut self) -> Pending {
		std::mem::take(&mut self.pending)
	}
}

/// Which of a field's VU1 owners a chain walk meets (v0.4.1, Hoven / Kalebo water that vanished): `run` finds an owner's packets by the
/// tag that STARTS its range (`c0`, a tag address of the walk), so a candidate chain start is only the field's chain when its walk hits
/// the `c0` of every owner. Passing through and leaving the last owner's range is not enough: a stale tag page (the other page of the
/// pair, or one from an earlier field) whose old tags lie in the same packet buffer passes that test with other tag addresses and
/// leaves every owner without output (the effects layer lost its water, in the user's log "walked 203374 owners 1 drawn []").
struct OwnerHits {
	/// The owners' first tags, sorted, one entry per distinct address.
	c0: Vec<u32>,
	lo: u32,
	hi: u32,
	seen: Vec<bool>,
}

impl OwnerHits {
	/// `tg`: (owner index, c0, c1) as `prep` collects them.
	fn new(tg: &[(usize, u32, u32)]) -> OwnerHits {
		let mut c0: Vec<u32> = tg.iter().map(|t| t.1).collect();
		c0.sort_unstable();
		c0.dedup();
		let lo = c0.first().copied().unwrap_or(0);
		let hi = tg.iter().map(|t| t.2).max().unwrap_or(0);
		let seen = vec![false; c0.len()];
		OwnerHits { c0, lo, hi, seen }
	}

	/// The walk is at the tag `tadr` (called for every tag of the walk, before it is read).
	fn note(&mut self, tadr: u32) {
		if tadr >= self.lo && tadr < self.hi {
			if let Ok(i) = self.c0.binary_search(&tadr) {
				self.seen[i] = true;
			}
		}
	}

	/// Owners (distinct first tags) the walk has hit.
	fn count(&self) -> usize {
		self.seen.iter().filter(|&&s| s).count()
	}

	/// Owners (distinct first tags) in all.
	fn need(&self) -> usize {
		self.c0.len()
	}
}

/// The best candidate chain so far: the first one that hits every owner wins at once; else the one that hits the most owners (the
/// first of equals), which is what the walk did before the owners were checked (the first that passed through the last one).
struct Pick<T> {
	best: Option<(T, usize)>,
	need: usize,
}

impl<T> Pick<T> {
	fn new(need: usize) -> Pick<T> {
		Pick { best: None, need }
	}

	/// Offers a candidate that hit `hits` owners; true: it hits all of them (stop looking).
	fn offer(&mut self, item: T, hits: usize) -> bool {
		if self.best.as_ref().is_none_or(|b| hits > b.1) {
			self.best = Some((item, hits));
		}
		hits >= self.need
	}

	/// No candidate hits every owner yet.
	fn wants_more(&self) -> bool {
		self.best.as_ref().is_none_or(|b| b.1 < self.need)
	}
}

/// The chain starts `Picker::prep` found on this level (v0.4.1, Hoven river that vanished). Rilgar and Eudora alternate two tag pages, so
/// the starts that walked through an owner are kept and tried first. They belong to ONE level: the packet buffers lie elsewhere on the
/// next one, where a kept page can hold vertex floats that read as DMA tags (Hoven after Pokitaru: 0x32c000 walked 3.17M words and still
/// met the river's first tag), so `enter` forgets them when the level changes.
#[derive(Default)]
struct Starts {
	level: Option<u32>,
	/// Starts that walked through an owner, the oldest first (at most 4).
	known: Vec<u32>,
	/// The start used last field.
	last: Option<u32>,
	/// (start, hits): the last page scan found nothing better than this known start's partial hit count. While the best known candidate is
	/// that start with the same hits, `choose` does not scan again (a level where some owner's first tag is never a walk tag would scan
	/// the 32 MB of RAM every field).
	dry: Option<(u32, usize)>,
}

impl Starts {
	/// A field of `level` begins. True: the level changed and starts were forgotten.
	fn enter(&mut self, level: u32) -> bool {
		if self.level == Some(level) {
			return false;
		}
		self.level = Some(level);
		let had = !self.known.is_empty() || self.last.is_some();
		self.known.clear();
		self.last = None;
		self.dry = None;
		had
	}

	/// The field took the start `chosen.0` (`.1`: the page scan found it) or had no chain: the start is the one used last, and a start the
	/// scan found is kept. A known start is never added again, so a stale page is not promoted by being tried.
	fn used(&mut self, chosen: Option<(u32, bool)>) {
		if let Some((a, true)) = chosen {
			self.remember(a);
		}
		self.last = chosen.map(|c| c.0);
	}

	/// The starts to try, in order: the known ones except the one used last field (alternating tag pages), then that one. Only pages whose
	/// first qword (`first`) carries the kicked chain's first job words (`opens_like`, the test of the page scan) are tried; the number
	/// left out comes second.
	fn tryable(&self, job0: Option<(u32, u32)>, first: impl Fn(u32) -> u128) -> (Vec<u32>, usize) {
		let mut all: Vec<u32> = self.known.iter().copied().filter(|&a| Some(a) != self.last).collect();
		all.extend(self.last);
		let n = all.len();
		all.retain(|&a| opens_like(first(a), job0));
		let left = n - all.len();
		(all, left)
	}

	/// A start the page scan found.
	fn remember(&mut self, a: u32) {
		if !self.known.contains(&a) {
			self.known.push(a);
			if self.known.len() > 4 {
				self.known.remove(0);
			}
		}
	}
}

/// Can the page whose first qword is `t` start the chain the machine kicked? Its first tag carries the same two VIF words as the kicked
/// chain's first job `job0` (words 0 and 1). No job (or one with fewer words): nothing to compare, every page may.
fn opens_like(t: u128, job0: Option<(u32, u32)>) -> bool {
	job0.is_none_or(|(a0, a1)| (t >> 64) as u32 == a0 && (t >> 96) as u32 == a1)
}

/// The chain start of a field (pure, `Picker::prep` gives it the machine walk as `plan_at`). `cands`: the known starts to try, in order;
/// `need`: the number of distinct owner first tags; `plan_at(start)`: the walk of that start (None: it does not pass through and leave
/// the last owner) with the number of first tags it hit. A walk that hits none is no candidate (`run` would find no packets for any owner
/// and the chain would be fed from the wrong place). The first candidate that hits every owner wins; else the page scan `scan` (the
/// addresses of the pages that open like job 0 and are not known yet, lazily) is tried, unless `dry` says it already found nothing
/// better than the best known candidate; else the candidate with the most hits stays. `dry` is set when a scan could not beat a known
/// partial start and cleared when it found a better one or a full candidate turned up. Returns (start, plan, found by the scan).
fn choose<P, I: Iterator<Item = u32>>(
	need: usize,
	cands: &[u32],
	dry: &mut Option<(u32, usize)>,
	scan: Option<impl FnOnce() -> I>,
	mut plan_at: impl FnMut(u32) -> Option<(P, usize)>,
) -> Option<(u32, P, bool)> {
	let mut pick: Pick<(u32, P, bool)> = Pick::new(need);
	for &a in cands {
		if let Some((plan, h)) = plan_at(a).filter(|p| p.1 > 0) {
			if pick.offer((a, plan, false), h) {
				break;
			}
		}
	}
	if !pick.wants_more() {
		*dry = None;
	} else {
		let known = pick.best.as_ref().map(|b| (b.0.0, b.1));
		if let Some(scan) = scan.filter(|_| !(known.is_some() && *dry == known)) {
			for a in scan() {
				if let Some((plan, h)) = plan_at(a).filter(|p| p.1 > 0) {
					if pick.offer((a, plan, true), h) {
						break;
					}
				}
			}
			*dry = match &pick.best {
				Some(((a, _, false), h)) => Some((*a, *h)),
				_ => None,
			};
		}
	}
	pick.best.map(|((a, p, s), _)| (a, p, s))
}

impl Picker {
	/// `dir`: the savestate directory (VU1 memories as the savestate left them; zero when missing, e.g. a cold boot).
	pub fn new(dir: &str) -> Picker {
		let d = std::fs::read(format!("{dir}/vu1Memory.bin")).unwrap_or_default();
		let mi = std::fs::read(format!("{dir}/vu1MicroMem.bin")).unwrap_or_default();
		let mut vif = Vif1::new();
		vif.shadow = true;
		let full = std::env::var("RAC_EFX_VU_FULL").is_ok_and(|v| v == "1");
		let chk = std::env::var("RAC_VIF_FEED_CHECK").is_ok_and(|v| v == "1").then(|| {
			let mut v = Vif1::new();
			v.shadow = true;
			v.lazy = false;
			Box::new((v, Vu1::new(&d, &mi), Out::default()))
		});
		let (btx, spare_rx) = channel();
		let model = VuModel { vif, vu: Vu1::new(&d, &mi), full, prev: Vec::new(), fails: 0, scratch: Out::default(), back: Some(btx), chk, chk_n: (0, 0), chk_fields: 0, fields_seen: 0 };
		Picker { model: Some(model), th: None, starts: Starts::default(), spare: Spare::default(), spare_rx }
	}
}

impl Shadow {
	/// Before a field: the machine's jobs come to the tee first.
	pub fn begin(&mut self, gsq: &mut Option<Sender<Job>>) {
		if self.fwd.is_none() {
			self.fwd = Some(std::mem::replace(gsq, Some(self.tx.clone())));
		}
	}

	/// After the field: the VIF1 jobs (the chain kicked this field) are kept for `run`, and the jobs are passed on to the
	/// machine's own sender in order (the mirror mode has none: the jobs are moved, not copied).
	pub fn end(&mut self, gsq: &mut Option<Sender<Job>>) {
		let Some(f) = self.fwd.take() else {
			return;
		};
		*gsq = f;
		match gsq.as_ref() {
			Some(s) => {
				for j in self.rx.try_iter() {
					if let Job::Vif1(w, mk, _) = &j {
						self.pending.push((w.clone(), mk.clone()));
					}
					let _ = s.send(j);
				}
			}
			None => {
				for j in self.rx.try_iter() {
					if let Job::Vif1(w, mk, _) = j {
						self.pending.push((w, mk));
					}
				}
			}
		}
	}

}

impl Picker {
	/// The field's end, on the worker (reads RAM): the unclaimed VU1 owners of `own` and chain N walked from RAM up to
	/// the end of the last one. The start (`choose`): the first tag must open like job 0 (the kicked chain's first two VIF words), the
	/// walk must hit the first tag (`c0`) of every owner (`OwnerHits`) and leave the last owner's range. Known starts of this level come
	/// first (the one not used last field first: alternating tag pages); only when none hits every owner, a scan of the 4 KB pages that
	/// open like job 0 (a scan winner is remembered, a dry scan is not repeated). The starts are forgotten when the level changes.
	/// `run` checks the walk against the next field's jobs word for word (`fails`).
	pub fn prep(&mut self, m: &Machine, own: &[Own], bi: &super::efx_draw::BuildIn, pending: Pending) -> Work {
		let t0 = std::time::Instant::now();
		for s in self.spare_rx.try_iter() {
			self.spare.w.extend(s.w);
			self.spare.m.extend(s.m);
			self.spare.t.extend(s.t);
		}
		// Enough for three fields in flight (a chain has up to 16 pieces); older ones are dropped.
		fn cap<T>(v: &mut Vec<T>) {
			if v.len() > 48 {
				v.drain(..v.len() - 48);
			}
		}
		(cap(&mut self.spare.w), cap(&mut self.spare.m), cap(&mut self.spare.t));
		let mut tg: Vec<(usize, u32, u32)> = Vec::new();
		let t_cen = std::time::Instant::now();
		for (k, o) in own.iter().enumerate() {
			let Some(p) = bi.pkts[k].as_ref().filter(|_| o.ok) else {
				continue;
			};
			if !p.ok {
				continue;
			}
			let (vc, _) = efx_own::vif_census(&p.words);
			if vc[0] + vc[1] + vc[2] != 0 {
				tg.push((k, o.c0, o.c1));
			}
		}
		sp_add(1, t_cen.elapsed().as_nanos() as u64);
		self.starts.enter(bi.level);
		let job0 = pending.first().and_then(|j| Some((*j.0.first()?, *j.0.get(1)?)));
		let mut chain = None;
		let mut used = None;
		if let Some(&(_, lc0, lc1)) = tg.last() {
			// Plan first (tags only): a start that does not pass through the owners costs no data copy, the winner is copied once.
			// Returns the plan and how many owners' first tags it hit (none hit: not a candidate).
			let plan_at = |s: u32| -> Option<(Vec<Vec<PlanTag>>, usize)> {
				let mut inside = false;
				let mut left = false;
				let mut hits = OwnerHits::new(&tg);
				let t_pl = std::time::Instant::now();
				let plan = plan_chain(m, s, |t| {
					let i = t >= lc0 && t < lc1;
					left = inside && !i;
					inside |= i;
					hits.note(t);
					left
				});
				sp_add(2, t_pl.elapsed().as_nanos() as u64);
				let plan = plan?;
				sp_add(7, plan.iter().map(|p| p.len() as u64).sum());
				(left && hits.count() > 0).then(|| (plan, hits.count()))
			};
			let (cand, left_out) = self.starts.tryable(job0, |a| qw(m, a));
			// Discovery: a page whose first tag carries the kicked chain's first tag words (job 0's first two words).
			let t_disc = std::cell::Cell::new(None);
			let known = &self.starts.known;
			let scan = job0.map(|j| {
				let t_disc = &t_disc;
				move || {
					t_disc.set(Some(std::time::Instant::now()));
					sp_add(6, 1);
					let ram = (m.cpu.mem.ram.len() as u32) & !0xfff;
					(0..ram).step_by(0x1000).filter(move |&a| opens_like(qw(m, a), Some(j)) && !known.contains(&a))
				}
			});
			let chosen = choose(OwnerHits::new(&tg).need(), &cand, &mut self.starts.dry, scan, plan_at);
			if let Some(t) = t_disc.get() {
				sp_add(4, t.elapsed().as_nanos() as u64);
			}
			if let Some((a, plan, from_scan)) = chosen {
				let t_b = std::time::Instant::now();
				let c = build_chain(m, &plan, &mut self.spare);
				sp_add(3, t_b.elapsed().as_nanos() as u64);
				sp_add(5, c.iter().map(|p| p.w.len() as u64).sum());
				chain = Some(c);
				used = Some((a, from_scan));
			} else if log() {
				eprintln!("efxvu no chain: no start hits an owner's first tag and leaves the last owner {lc0:x} (owners {}, known {:x?}, {left_out} left out by their first words, dry {:x?})", tg.len(), self.starts.known, self.starts.dry);
			}
		}
		self.starts.used(used);
		let fns = tg.iter().map(|t| own[t.0].fnp).collect();
		Work { chain, tg, walk_ms: t0.elapsed().as_secs_f64() * 1e3, fns, vj: pending }
	}

	/// The field's VU1 work and effects build go to the VU1 thread (started on the first call) and run while the worker
	/// goes on: the reply arrives on the returned receiver (`reply` false: no reply, the model still runs).
	pub fn submit(&mut self, w: Work, bi: super::efx_draw::BuildIn, field: u32, reply: bool) -> Option<std::sync::mpsc::Receiver<FxOut>> {
		if self.th.is_none() {
			let mut model = self.model.take()?;
			let (tx, rx) = channel::<VuJob>();
			let spawned = std::thread::Builder::new().name("vu1".into()).stack_size(16 << 20).spawn(move || {
				while let Ok(j) = rx.recv() {
					let t_busy = std::time::Instant::now();
					let out = model.run(j.w);
					let t_bi = std::time::Instant::now();
					let mut nq = (0usize, 0usize);
					if let Some(r) = j.reply {
						let (mut qa, mut pools) = (Vec::new(), Vec::new());
						let st = super::efx_draw::build_in(&j.bi, &out, &mut qa, &mut pools);
						super::free_water::apply(&j.bi.free_water, &mut pools);
						nq = (qa.len(), pools.len());
						if log() {
							eprintln!("efxdraw field {}: {st:x?}", j.field);
						}
						let _ = r.send((qa, pools));
					}
					prof_job(t_busy.elapsed().as_nanos() as u64, t_bi.elapsed().as_nanos() as u64, nq, out.iter().map(|o| o.1.len() as u64).sum());
				}
			});
			spawned.ok()?;
			self.th = Some(tx);
		}
		let (rtx, rrx) = channel();
		self.th.as_ref()?.send(VuJob { w, bi, field, reply: reply.then_some(rtx) }).ok()?;
		reply.then_some(rrx)
	}
}

impl VuModel {
	/// Feeds words [a, b) of a piece (block marks re-based) with VU1 off (`run` false) or on.
	fn feed(&mut self, w: &[u32], marks: &[(usize, u32)], a: usize, b: usize, run: bool, out: &mut Out) {
		if a >= b {
			return;
		}
		let mut sub: Vec<(usize, u32)> = marks.iter().filter(|m| m.0 >= a && m.0 < b).map(|m| (m.0 - a, m.1)).collect();
		if sub.first().is_none_or(|m| m.0 != 0) {
			sub.insert(0, (0, 0));
		}
		if run {
			// the run reads VU1 data memory: apply the queued UNPACKs and check the state it starts from
			self.vif.flush_now(&mut self.vu);
			self.chk_cmp("pre-run", true);
		}
		self.vif.shadow = !run && !self.full;
		self.vif.feed(&w[a..b], &sub, &mut self.vu, out);
		self.vif.shadow = true;
		if let Some(c) = self.chk.as_mut() {
			let (cv, cu, co) = &mut **c;
			cv.shadow = !run && !self.full;
			*co = Out::default();
			cv.feed(&w[a..b], &sub, cu, co);
			cv.shadow = true;
			self.chk_n.0 += 1;
			let same_out = !run || (co.qs == out.qs && co.segs == out.segs);
			if !same_out {
				self.chk_n.1 += 1;
				eprintln!("vif feed check MISMATCH at feed {}: GIF output of a run feed differs", self.chk_n.0);
			}
			// registers every feed; the data memory where the lazy model is exact (after a run feed: it flushed first)
			self.chk_cmp(if run { "run" } else { "feed" }, run);
		}
		if !run {
			out.qs.clear();
			out.segs.clear();
		}
	}

	/// RAC_VIF_FEED_CHECK: compares the lazy model with the eager one (VIF registers; with `data` also VU1 data / micro
	/// memory and the registers: the lazy queue must be flushed before).
	fn chk_cmp(&mut self, what: &str, data: bool) {
		let Some(c) = self.chk.as_ref() else {
			return;
		};
		let (cv, cu, _) = &**c;
		let regs = cv.state_eq(&self.vif);
		let mem = !data || (cu.data == self.vu.data && cu.micro_image() == self.vu.micro_image() && cu.vf == self.vu.vf && cu.vi == self.vu.vi && cu.acc == self.vu.acc && cu.tpc == self.vu.tpc);
		if !(regs && mem) {
			self.chk_n.1 += 1;
			if self.chk_n.1 <= 5 {
				eprintln!("vif feed check MISMATCH at feed {} ({what}): vif regs equal {regs} memory equal {mem}", self.chk_n.0);
			}
		}
	}

	fn feed_off(&mut self, w: &[u32], marks: &[(usize, u32)], a: usize, b: usize) {
		let mut sc = std::mem::take(&mut self.scratch);
		self.feed(w, marks, a, b, false, &mut sc);
		self.scratch = sc;
	}

	/// The feed (any thread, no RAM): the kept jobs are checked against the last walk and fed from its stop point on,
	/// then chain N's walk is fed with VU1 run inside the owners' block ranges. Returns the owners' GS packets.
	fn run(&mut self, mut w: Work) -> VuOut {
		let t1 = std::time::Instant::now();
		self.vif.hold = true;
		let mut pf = [0u64; 6];
		let prev = std::mem::take(&mut self.prev);
		let vj = std::mem::take(&mut w.vj);
		let np = prev.len();
		if np > 0 {
			let ok = vj.len() >= np && prev.iter().enumerate().all(|(k, p)| if k + 1 < np { &vj[k].0 == p } else { vj[k].0.get(..p.len()) == Some(&p[..]) });
			if !ok {
				self.fails += 1;
				if log() {
					let d: Vec<String> = vj.iter().zip(&prev).enumerate().map(|(k, (j, p))| format!("#{k} job {} walk {} first diff {:?}", j.0.len(), p.len(), j.0.iter().zip(p).position(|(a, b)| a != b))).collect();
					eprintln!("efxvu check bad: jobs {} walk {np} {}", vj.len(), d.join("; "));
				}
			}
		}
		pf[0] = t1.elapsed().as_nanos() as u64;
		for (k, (wd, mk)) in vj.iter().enumerate() {
			pf[4] += (wd.len()) as u64;
			let from = if k + 1 > np { 0 } else if k + 1 < np { wd.len() } else { prev[k].len().min(wd.len()) };
			self.feed_off(wd, mk, from, wd.len());
		}
		pf[1] = t1.elapsed().as_nanos() as u64;
		let mut res: VuOut = Vec::new();
		let mut tail_ns = 0u64;
		let mut words = 0usize;
		if let Some(chain) = &w.chain {
			for p in chain {
				words += p.w.len();
				let mut pos = 0usize;
				for &(k, c0, c1) in &w.tg {
					let Some(a) = p.tags.iter().position(|t| t.1 == c0 && t.0 >= pos) else {
						continue;
					};
					let b = p.tags[a..].iter().position(|t| t.1 < c0 || t.1 >= c1).map(|i| p.tags[a + i].0).unwrap_or(p.w.len());
					let a = p.tags[a].0;
					let tt = std::time::Instant::now();
					self.feed_off(&p.w, &p.marks, pos, a);
					pf[2] += tt.elapsed().as_nanos() as u64;
					let tt = std::time::Instant::now();
					let mut o = Out::default();
					self.feed(&p.w, &p.marks, a, b, true, &mut o);
					let ons = tt.elapsed().as_nanos() as u64;
					pf[3] += ons;
					if prof_on() {
						let f = w.tg.iter().position(|t| t.0 == k).map_or(0, |i| w.fns[i]);
						prof_owner(f, ons, &o.stats, (b - a) as u64, crate::gsfull::vu1aot::native_prog(&mut self.vu).map_or(15, |x| x as usize));
					}
					pf[5] += (b - a) as u64;
					pos = b;
					res.push((k, o.qs));
				}
				let tt = std::time::Instant::now();
				self.feed_off(&p.w, &p.marks, pos, p.w.len());
				tail_ns += tt.elapsed().as_nanos() as u64;
			}
		}
		if prof_on() {
			self.fields_seen += 1;
			if self.fields_seen % 200 == 0 {
				let n = self.fields_seen as f64;
				let st = &self.scratch.stats;
				eprintln!("vu1prof shadow-feed per field: vif_words {:.0} codes {:.0} unpack {:.0} unpack_words {:.0} mpg {:.1} mpg_words {:.0} fill {:.0} | lazy ops {:.0} skipped {:.0} words {:.0} skipped {:.0} marks {:.0} feeds {:.0} | survivors v3 {:.0} masked {:.0} mode1 {:.0} cl!=wl {:.0}", st.vif_words as f64 / n, st.codes as f64 / n, st.unpack as f64 / n, st.unpack_words as f64 / n, st.mpg as f64 / n, st.mpg_words as f64 / n, st.unpack_fill as f64 / n, self.vif.cnt[0] as f64 / n, self.vif.cnt[1] as f64 / n, self.vif.cnt[2] as f64 / n, self.vif.cnt[3] as f64 / n, self.vif.cnt[4] as f64 / n, self.vif.cnt[5] as f64 / n, self.vif.cnt[6] as f64 / n, self.vif.cnt[7] as f64 / n, self.vif.cnt[8] as f64 / n, self.vif.cnt[9] as f64 / n);
			}
		}
		self.vif.flush_now(&mut self.vu);
		self.vif.hold = false;
		self.chk_cmp("field end", true);
		prof_run(pf[0], pf[2], tail_ns, w.walk_ms);
		if self.chk.is_some() {
			self.chk_fields += 1;
			if self.chk_fields % 100 == 0 {
				eprintln!("vif feed check: fields {} feeds {} mismatches {}", self.chk_fields, self.chk_n.0, self.chk_n.1);
			}
		}
		pf_add(&pf, t1.elapsed().as_nanos() as u64);
		if std::env::var_os("RAC_PERF_VU").is_some() && self.vu.st.runs % 400 < 3 {
			eprintln!("perf: vu1 runs {} pairs {} native {} bails {} interp(not covered) {} compared {} differed {} bail_at {:?} {:?}", self.vu.st.runs, self.vu.st.pairs, self.vu.aot.native, self.vu.aot.bails, self.vu.aot.interp, self.vu.aot.compared, self.vu.aot.differed, self.vu.aot.bail_at, self.vu.diff_log);
		}
		// The pieces' words stay for the next field's check; the rest of the pieces and last field's words are free again.
		let mut sp = Spare { w: prev, ..Default::default() };
		for p in w.chain.take().unwrap_or_default() {
			self.prev.push(p.w);
			sp.m.push(p.marks);
			sp.t.push(p.tags);
		}
		if let Some(b) = &self.back {
			let _ = b.send(sp);
		}
		if log() {
			let h = |q: &[u128]| crate::gsfull::pipe::hash(&q.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>());
			let name = |k: usize| w.tg.iter().position(|t| t.0 == k).map_or(0, |i| w.fns[i]);
			let hs: Vec<String> = res.iter().map(|(k, q)| format!("{:x}:{}q:{:016x}/gs {:016x}", name(*k), q.len(), h(q), h(&gs_bits(q)))).collect();
			eprintln!("efxvu fails {} jobs {} walked {} owners {} drawn [{}] errs {} prep {:.2} ms run {:.2} ms", self.fails, vj.len(), words, w.tg.len(), hs.join(" "), self.vu.st.errors, w.walk_ms, t1.elapsed().as_secs_f64() * 1e3);
		}
		res
	}
}

/// The bits of a GIF stream the GS reads (log hash for the RAC_EFX_VU_FULL check): PACKED ST keeps S / T / Q (bits
/// 96-127 are not read), RGBAQ the four colour bytes, UV / XYZF2 / XYZ2 their fields and ADC, A+D the data and the
/// register byte; tags keep their low 64 bits and the register list (NREG entries) [C: GS manual, packed formats].
fn gs_bits(q: &[u128]) -> Vec<u128> {
	let mut out = q.to_vec();
	let mut i = 0usize;
	while i < q.len() {
		let t = q[i];
		let nreg = match ((t >> 60) & 15) as usize { 0 => 16, n => n };
		out[i] = (t & 0xffff_ffff_ffff_ffff) | (t >> 64 & ((1u128 << (4 * nreg)) - 1)) << 64;
		i += 1;
		let nloop = (t & 0x7fff) as usize;
		match (t >> 58) & 3 {
			0 => {
				for _ in 0..nloop {
					for k in 0..nreg {
						let Some(&d) = q.get(i) else { return out };
						let m: u128 = match (t >> (64 + 4 * k)) & 15 {
							1 => 0x0000_00ff_0000_00ff_0000_00ff_0000_00ff,
							2 => (1 << 96) - 1,
							3 => 0x3fff_0000_3fff,
							4 => 0x0000_8ff0_0fff_fff0_0000_ffff_0000_ffff,
							5 => 0x0000_8000_ffff_ffff_0000_ffff_0000_ffff,
							0xe => (1 << 72) - 1,
							_ => !0,
						};
						out[i] = d & m;
						i += 1;
					}
				}
			}
			1 => i += (nloop * nreg).div_ceil(2),
			_ => i += nloop,
		}
	}
	out
}

/// RAC_PERF_LOG=1: sums of `run`'s parts over 100 fields (ns): check, job feed (VU off), chain feed before an owner, owner feed (VU on), words of the jobs, words run.
fn pf_add(pf: &[u64; 6], total: u64) {
	use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
	static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	if !*ON.get_or_init(|| std::env::var("RAC_PERF_LOG").is_ok_and(|v| v == "1")) {
		return;
	}
	static ACC: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];
	for i in 0..6 {
		ACC[i].fetch_add(pf[i], Relaxed);
	}
	ACC[6].fetch_add(total, Relaxed);
	if ACC[7].fetch_add(1, Relaxed) == 99 {
		let g = |i: usize| ACC[i].swap(0, Relaxed);
		let (c0, c1, c2, c3, w4, w5, t) = (g(0), g(1), g(2), g(3), g(4), g(5), g(6));
		ACC[7].store(0, Relaxed);
		eprintln!("perf: vu.run avg ms: total {:.2} check {:.2} jobsfeed {:.2} chainoff {:.2} ownerrun {:.2} | words/field jobs {} run {}", t as f64 / 1e8, c0 as f64 / 1e8, (c1 - c0.min(c1)) as f64 / 1e8, c2 as f64 / 1e8, c3 as f64 / 1e8, w4 / 100, w5 / 100);
	}
}

/// RAC_VU1_PROF=1: the vu1 thread's cost split (B8 measurement): per field averages over 200 fields and per owner function /
/// VU1 program (ns of the owner's VIF feed with VU1 on = UNPACK + program runs).
fn prof_on() -> bool {
	static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*ON.get_or_init(|| std::env::var("RAC_VU1_PROF").is_ok_and(|v| v == "1"))
}

#[derive(Default)]
struct Prof {
	fields: u64,
	busy: u64,
	build: u64,
	check: u64,
	pre: u64,
	tail: u64,
	walk: f64,
	owner: u64,
	qw: u64,
	quads: u64,
	strips: u64,
	by_fn: std::collections::HashMap<u32, [u64; 5]>,
	by_prog: [[u64; 5]; 16],
}

static PROF: std::sync::Mutex<Option<Prof>> = std::sync::Mutex::new(None);

fn prof_run(check: u64, pre: u64, tail: u64, walk_ms: f64) {
	if !prof_on() {
		return;
	}
	let mut g = PROF.lock().unwrap();
	let p = g.get_or_insert_with(Prof::default);
	p.check += check;
	p.pre += pre;
	p.tail += tail;
	p.walk += walk_ms;
}

fn prof_owner(f: u32, ns: u64, st: &crate::gsfull::vif::Stats, words: u64, prog: usize) {
	let mut g = PROF.lock().unwrap();
	let p = g.get_or_insert_with(Prof::default);
	p.owner += ns;
	let runs: u64 = st.prog_runs.iter().sum();
	let pairs: u64 = st.prog_pairs.iter().sum();
	let qw: u64 = st.prog_qw.iter().sum();
	let e = p.by_fn.entry(f).or_default();
	e[0] += ns;
	e[1] += runs;
	e[2] += pairs;
	e[3] += qw;
	e[4] += words;
	let e = &mut p.by_prog[prog & 15];
	e[0] += ns;
	e[1] += runs;
	e[2] += pairs;
	e[3] += qw;
	e[4] += words;
}

fn prof_job(busy: u64, build: u64, nq: (usize, usize), qw: u64) {
	if !prof_on() {
		return;
	}
	let mut g = PROF.lock().unwrap();
	let p = g.get_or_insert_with(Prof::default);
	p.busy += busy;
	p.build += build;
	p.qw += qw;
	p.quads += nq.0 as u64;
	p.strips += nq.1 as u64;
	p.fields += 1;
	if p.fields % 200 == 0 {
		let n = p.fields as f64;
		let ms = |x: u64| x as f64 / n / 1e6;
		eprintln!(
			"vu1prof: fields {} busy {:.2} ms/field = check {:.2} + walk(worker, not here) {:.2} | preowner feed {:.2} + owner feed+run {:.2} + tail feed {:.2} + build_in {:.2} | gs qwords/field {:.0} quads {:.1} strips {:.1}",
			p.fields, ms(p.busy), ms(p.check), p.walk / n, ms(p.pre), ms(p.owner), ms(p.tail), ms(p.build), p.qw as f64 / n, p.quads as f64 / n, p.strips as f64 / n
		);
		let mut v: Vec<_> = p.by_fn.iter().collect();
		v.sort_by_key(|x| std::cmp::Reverse(x.1[0]));
		for (f, e) in v.iter().take(8) {
			eprintln!("vu1prof  fn {:x}: {:.3} ms/field  runs/field {:.2} pairs/field {:.0} qw/field {:.0} words/field {:.0}", f, ms(e[0]), e[1] as f64 / n, e[2] as f64 / n, e[3] as f64 / n, e[4] as f64 / n);
		}
		for (k, e) in p.by_prog.iter().enumerate().filter(|x| x.1[0] > 0) {
			eprintln!("vu1prof  prog {k}: {:.3} ms/field  runs/field {:.2} pairs/field {:.0} qw/field {:.0} words/field {:.0}", ms(e[0]), e[1] as f64 / n, e[2] as f64 / n, e[3] as f64 / n, e[4] as f64 / n);
		}
	}
}

/// RAC_VU1_PROF=1: how long `Retail::tick` waited for the VU1 thread's reply (ns; 200-tick averages and the worst).
pub fn prof_wait(ns: u64) {
	if !prof_on() {
		return;
	}
	use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
	static ACC: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
	ACC[0].fetch_add(ns, Relaxed);
	ACC[1].fetch_max(ns, Relaxed);
	if ns > 100_000 {
		ACC[3].fetch_add(1, Relaxed);
	}
	if ACC[2].fetch_add(1, Relaxed) % 200 == 199 {
		eprintln!("vu1prof: tick waited for the VU1 reply: avg {:.3} ms, worst {:.2} ms, > 0.1 ms in {} of 200 ticks", ACC[0].swap(0, Relaxed) as f64 / 200e6, ACC[1].swap(0, Relaxed) as f64 / 1e6, ACC[3].swap(0, Relaxed));
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Two owners: [0x1000, 0x1800) and [0x1800, 0x2000).
	fn tg() -> Vec<(usize, u32, u32)> {
		vec![(0, 0x1000, 0x1800), (3, 0x1800, 0x2000)]
	}

	/// The v0.4.1 water bug: a stale tag page (old tags in the same packet buffer) passes through the last owner's range and leaves it,
	/// but none of its tag addresses is an owner's first tag, so `run` finds no packets for any owner. It must count as no hit.
	#[test]
	fn a_stale_tag_page_in_the_owners_ranges_hits_no_first_tag() {
		let mut stale = OwnerHits::new(&tg());
		for t in [0x0f00u32, 0x1010, 0x1400, 0x1810, 0x1ff0, 0x2000] {
			stale.note(t);
		}
		assert_eq!((stale.count(), stale.need()), (0, 2));
		let mut real = OwnerHits::new(&tg());
		for t in [0x0f00u32, 0x1000, 0x1400, 0x1800, 0x1ff0, 0x2000] {
			real.note(t);
		}
		assert_eq!((real.count(), real.need()), (2, 2));
	}

	#[test]
	fn a_walk_that_meets_only_some_first_tags_counts_them() {
		let mut h = OwnerHits::new(&tg());
		for t in [0x1000u32, 0x1000, 0x1400] {
			h.note(t);
		}
		assert_eq!(h.count(), 1, "a first tag met twice counts once");
		// outside every owner range nothing is looked up, whatever the address
		h.note(0x0ff0);
		h.note(0x2000);
		assert_eq!(h.count(), 1);
	}

	#[test]
	fn owners_with_the_same_first_tag_are_one_hit_and_no_owners_need_nothing() {
		let h = OwnerHits::new(&[(0, 0x1000, 0x1800), (1, 0x1000, 0x1800), (2, 0x1800, 0x1c00)]);
		assert_eq!(h.need(), 2);
		let none = OwnerHits::new(&[]);
		assert_eq!((none.need(), none.count()), (0, 0));
	}

	#[test]
	fn the_first_candidate_that_hits_every_owner_wins_even_after_a_stale_one() {
		// The order `prep` tries them in: the other page of the pair first, then the page used last field.
		let mut pick = Pick::new(2);
		assert!(pick.wants_more());
		assert!(!pick.offer("stale page", 0), "passes through the last owner but meets no first tag");
		assert!(pick.wants_more());
		assert!(pick.offer("real page", 2));
		assert!(!pick.wants_more());
		assert_eq!(pick.best.map(|b| b.0), Some("real page"));
		// a later candidate is not needed: the caller stops at the first full one, but offering more cannot take the win away
		let mut pick = Pick::new(2);
		assert!(pick.offer("real", 2));
		assert!(pick.offer("other full", 2));
		assert_eq!(pick.best.map(|b| b.0), Some("real"));
	}

	#[test]
	fn without_a_full_candidate_the_one_with_most_hits_is_used_like_before() {
		let mut pick = Pick::new(3);
		assert!(!pick.offer("a", 1));
		assert!(!pick.offer("b", 2));
		assert!(!pick.offer("c", 2), "the first of equals stays");
		assert!(pick.wants_more());
		assert_eq!(pick.best.map(|b| b.0), Some("b"));
		let empty: Pick<&str> = Pick::new(1);
		assert!(empty.best.is_none() && empty.wants_more());
	}

	/// The first qword of a tag page: low half the DMA tag, high half the two VIF words the page opens with.
	fn page(w2: u32, w3: u32) -> u128 {
		(w3 as u128) << 96 | (w2 as u128) << 64 | 0x1234
	}

	/// Starts as Pokitaru (level 11) left them: two pages alternate, 0x49c000 was used last.
	fn pokitaru() -> Starts {
		let mut s = Starts::default();
		assert!(!s.enter(11), "the first level has nothing to forget");
		s.remember(0x32c000);
		s.remember(0x49c000);
		s.last = Some(0x49c000);
		s
	}

	/// The v0.4.1 Hoven bug: the kept pages of the old level are tried first on the next one (known / last were set in `Picker::new`
	/// only). A level change forgets them; the same level keeps them.
	#[test]
	fn a_level_change_forgets_the_chain_starts_of_the_old_level() {
		let mut s = pokitaru();
		assert!(!s.enter(11), "same level: nothing forgotten");
		assert_eq!((s.known.clone(), s.last), (vec![0x32c000, 0x49c000], Some(0x49c000)));
		assert!(s.enter(12), "Hoven: the Pokitaru pages are forgotten");
		assert!(s.known.is_empty() && s.last.is_none());
		assert!(!s.enter(12));
		let (tried, left) = s.tryable(Some((1, 2)), |_| page(1, 2));
		assert!(tried.is_empty() && left == 0, "a stale known start from another level is not tried");
	}

	/// Even on the same level a kept page is only tried when its first tag opens with job 0's words (as the page scan demands).
	#[test]
	fn a_known_start_whose_first_tag_words_differ_from_job_0_is_skipped() {
		let s = pokitaru();
		let ram = |a: u32| if a == 0x32c000 { page(0x3f80_0000, 0x4000_0000) } else { page(0x1100_0000, 0x0100_0000) };
		// 0x32c000 holds floats now (vertex data of the next level); 0x49c000 still opens like the kicked chain.
		let (tried, left) = s.tryable(Some((0x1100_0000, 0x0100_0000)), ram);
		assert_eq!((tried, left), (vec![0x49c000], 1));
		// both open like it: the page not used last field first, then the one used last field
		let (tried, left) = s.tryable(Some((0x1100_0000, 0x0100_0000)), |_| page(0x1100_0000, 0x0100_0000));
		assert_eq!((tried, left), (vec![0x32c000, 0x49c000], 0));
		// the second word counts too
		let (tried, _) = s.tryable(Some((0x1100_0000, 0x0100_0000)), |_| page(0x1100_0000, 0x0200_0000));
		assert!(tried.is_empty());
	}

	#[test]
	fn without_a_job_every_known_start_is_tried_and_the_scan_remembers_at_most_four() {
		let s = pokitaru();
		assert_eq!(s.tryable(None, |_| 0), (vec![0x32c000, 0x49c000], 0));
		assert!(opens_like(0, None) && !opens_like(0, Some((1, 0))) && opens_like(page(5, 6), Some((5, 6))));
		let mut s = Starts::default();
		for a in [0x1000u32, 0x2000, 0x3000, 0x3000, 0x4000, 0x5000] {
			s.remember(a);
		}
		assert_eq!(s.known, vec![0x2000, 0x3000, 0x4000, 0x5000], "no duplicate, the oldest goes");
	}

	use std::cell::{Cell, RefCell};

	/// A fake chain walker for `choose`: `table` maps a start to the owners' first tags its walk hits (None: it does not leave the last
	/// owner, `plan_at` gives None); the plan is the start itself; `seen` records the starts walked, in order.
	fn fake<'a>(table: &'a [(u32, Option<usize>)], seen: &'a RefCell<Vec<u32>>) -> impl FnMut(u32) -> Option<(u32, usize)> + 'a {
		move |a| {
			seen.borrow_mut().push(a);
			table.iter().find(|t| t.0 == a).and_then(|t| t.1).map(|n| (a, n))
		}
	}

	/// The page scan: these pages, lazily; `called` counts the scans started.
	fn pages<'a>(list: &[u32], called: &'a Cell<u32>) -> Option<impl FnOnce() -> std::vec::IntoIter<u32> + 'a> {
		let v = list.to_vec();
		Some(move || {
			called.set(called.get() + 1);
			v.into_iter()
		})
	}

	/// No page scan possible (the field kicked no job).
	fn no_scan() -> Option<fn() -> std::vec::IntoIter<u32>> {
		None
	}

	#[test]
	fn the_scan_runs_only_when_no_known_start_hits_every_owner() {
		let (seen, scans, mut dry) = (RefCell::new(vec![]), Cell::new(0), None);
		let table = [(0xa000, Some(3)), (0xb000, Some(1)), (0xc000, Some(3))];
		// the first full candidate wins at once: neither the next candidate nor the scan is touched
		let r = choose(3, &[0xa000, 0xb000], &mut dry, pages(&[0xc000], &scans), fake(&table, &seen));
		assert_eq!(r, Some((0xa000, 0xa000, false)));
		assert_eq!((seen.borrow().clone(), scans.get()), (vec![0xa000], 0));
		// only a partial candidate: the scan runs and its full page wins
		seen.borrow_mut().clear();
		let r = choose(3, &[0xb000], &mut dry, pages(&[0xc000], &scans), fake(&table, &seen));
		assert_eq!(r, Some((0xc000, 0xc000, true)));
		assert_eq!((seen.borrow().clone(), scans.get()), (vec![0xb000, 0xc000], 1));
		// a scan page after the first full one is not walked
		seen.borrow_mut().clear();
		let r = choose(3, &[], &mut dry, pages(&[0xc000, 0xa000], &scans), fake(&table, &seen));
		assert_eq!((r, seen.borrow().clone()), (Some((0xc000, 0xc000, true)), vec![0xc000]));
		// no candidates and no scan possible: no chain
		assert_eq!(choose(3, &[], &mut dry, no_scan(), fake(&table, &seen)), None);
	}

	#[test]
	fn without_a_full_candidate_the_known_start_with_most_hits_stays_when_the_scan_finds_nothing_better() {
		let (seen, scans, mut dry) = (RefCell::new(vec![]), Cell::new(0), None);
		let table = [(0xa000, Some(1)), (0xb000, Some(2)), (0xc000, Some(2)), (0xd000, None)];
		// b hits more than a; the scan page c only ties it: the known b stays (the first of equals), nothing is remembered
		let r = choose(3, &[0xa000, 0xb000], &mut dry, pages(&[0xd000, 0xc000], &scans), fake(&table, &seen));
		assert_eq!(r, Some((0xb000, 0xb000, false)));
		assert_eq!(dry, Some((0xb000, 2)));
		// a scan page that hits more beats it (a fresh latch: the same start with the same hits would not be scanned again)
		let table = [(0xa000, Some(1)), (0xb000, Some(2)), (0xc000, Some(2)), (0xe000, Some(3))];
		let r = choose(4, &[0xa000, 0xb000], &mut None, pages(&[0xc000, 0xe000], &scans), fake(&table, &seen));
		assert_eq!(r, Some((0xe000, 0xe000, true)));
	}

	#[test]
	fn only_a_scan_winner_is_remembered_and_the_start_taken_is_the_last_one() {
		let (seen, scans) = (RefCell::new(vec![]), Cell::new(0));
		let table = [(0xa000, Some(1)), (0xb000, Some(2)), (0xc000, Some(1))];
		let mut s = Starts::default();
		s.enter(7);
		s.remember(0xa000);
		// field 1: a is partial, the scan finds b (full): b is remembered and is the last start
		let (cand, _) = s.tryable(None, |_| 0);
		let r = choose(2, &cand, &mut s.dry, pages(&[0xb000], &scans), fake(&table, &seen));
		s.used(r.map(|c| (c.0, c.2)));
		assert_eq!((s.known.clone(), s.last), (vec![0xa000, 0xb000], Some(0xb000)));
		// field 2: the known b wins (a is tried first, the page used last field second): nothing new is remembered
		let (cand, _) = s.tryable(None, |_| 0);
		assert_eq!(cand, vec![0xa000, 0xb000]);
		let r = choose(2, &cand, &mut s.dry, pages(&[0xc000], &scans), fake(&table, &seen));
		assert_eq!(r, Some((0xb000, 0xb000, false)));
		s.used(r.map(|c| (c.0, c.2)));
		assert_eq!((s.known.clone(), s.last), (vec![0xa000, 0xb000], Some(0xb000)));
		// field 3: only partial starts, the scan page c is no better than the known a: a is taken, not remembered twice, and is last
		let mut s2 = Starts::default();
		s2.remember(0xa000);
		let r = choose(2, &[0xa000], &mut s2.dry, pages(&[0xc000], &scans), fake(&table, &seen));
		assert_eq!(r, Some((0xa000, 0xa000, false)));
		s2.used(r.map(|c| (c.0, c.2)));
		assert_eq!((s2.known.clone(), s2.last), (vec![0xa000], Some(0xa000)));
		// a scan page that only ties with nothing known is a scan winner too (it was the best the scan had)
		let mut s3 = Starts::default();
		let r = choose(2, &[], &mut s3.dry, pages(&[0xc000], &scans), fake(&table, &seen));
		assert_eq!(r, Some((0xc000, 0xc000, true)));
		s3.used(r.map(|c| (c.0, c.2)));
		assert_eq!((s3.known.clone(), s3.last), (vec![0xc000], Some(0xc000)));
		// no chain this field: the last start is forgotten, the known ones stay
		s3.used(None);
		assert_eq!((s3.known.clone(), s3.last), (vec![0xc000], None));
	}

	#[test]
	fn a_walk_that_hits_no_owner_is_no_candidate_and_no_scan_winner() {
		let (seen, scans, mut dry) = (RefCell::new(vec![]), Cell::new(0), None);
		let table = [(0xa000, Some(0)), (0xb000, Some(0)), (0xc000, Some(1))];
		// the walk passes through and leaves the last owner but meets no first tag: not taken, known or found by the scan
		assert_eq!(choose(2, &[0xa000], &mut dry, pages(&[0xb000], &scans), fake(&table, &seen)), None);
		assert_eq!((dry, scans.get()), (None, 1), "no known candidate, so nothing to latch");
		// a 1-hit page behind a 0-hit one is taken
		let r = choose(2, &[0xa000], &mut dry, pages(&[0xb000, 0xc000], &scans), fake(&table, &seen));
		assert_eq!(r, Some((0xc000, 0xc000, true)));
		// a 0-hit known start does not make the next field skip the scan either
		assert_eq!(choose(2, &[0xa000], &mut dry, pages(&[0xb000], &scans), fake(&table, &seen)), None);
		assert_eq!(scans.get(), 3);
	}

	#[test]
	fn a_dry_scan_is_not_repeated_while_the_known_start_gives_the_same_hits() {
		let (seen, scans, mut dry) = (RefCell::new(vec![]), Cell::new(0), None);
		let table = [(0xa000, Some(2)), (0xb000, None), (0xc000, Some(1))];
		let one = |dry: &mut Option<(u32, usize)>, table: &[(u32, Option<usize>)], scans: &Cell<u32>| {
			choose(3, &[0xa000], dry, pages(&[0xb000, 0xc000], scans), fake(table, &seen))
		};
		// field 1: a hits 2 of 3 owners, the scan finds nothing better: latch (a, 2)
		assert_eq!(one(&mut dry, &table, &scans), Some((0xa000, 0xa000, false)));
		assert_eq!((dry, scans.get()), (Some((0xa000, 2)), 1));
		// fields 2 and 3: same start, same hits: no scan
		for _ in 0..2 {
			assert_eq!(one(&mut dry, &table, &scans), Some((0xa000, 0xa000, false)));
		}
		assert_eq!((dry, scans.get()), (Some((0xa000, 2)), 1));
		// a hits one owner less (the field is another one): scan again, the latch follows
		let table1 = [(0xa000, Some(1)), (0xb000, None), (0xc000, Some(1))];
		assert_eq!(one(&mut dry, &table1, &scans), Some((0xa000, 0xa000, false)));
		assert_eq!((dry, scans.get()), (Some((0xa000, 1)), 2));
		// a better page appears (and a hits 2 now, not the latched 1, so the scan runs): the scan finds it, the latch is cleared
		let table2 = [(0xa000, Some(2)), (0xb000, Some(3)), (0xc000, Some(1))];
		assert_eq!(one(&mut dry, &table2, &scans), Some((0xb000, 0xb000, true)));
		assert_eq!((dry, scans.get()), (None, 3));
		// a full known start clears the latch and never scans
		dry = Some((0xa000, 2));
		let table3 = [(0xa000, Some(3))];
		assert_eq!(one(&mut dry, &table3, &scans), Some((0xa000, 0xa000, false)));
		assert_eq!((dry, scans.get()), (None, 3));
		// no scan possible (no job): the latch is left as it is
		dry = Some((0xa000, 2));
		assert_eq!(choose(3, &[0xa000], &mut dry, no_scan(), fake(&table, &seen)), Some((0xa000, 0xa000, false)));
		assert_eq!(dry, Some((0xa000, 2)));
		// another candidate with the same hits does not use the latch of a
		let table4 = [(0xa000, Some(2)), (0xd000, Some(2))];
		assert_eq!(choose(3, &[0xd000], &mut dry, pages(&[], &scans), fake(&table4, &seen)), Some((0xd000, 0xd000, false)));
		assert_eq!((dry, scans.get()), (Some((0xd000, 2)), 4));
	}

	#[test]
	fn a_level_change_clears_the_dry_latch() {
		let mut s = Starts::default();
		s.enter(11);
		s.dry = Some((0x32c000, 2));
		assert!(!s.enter(11) && s.dry.is_some());
		s.enter(12);
		assert_eq!(s.dry, None);
	}
}
