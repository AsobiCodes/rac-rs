//! EFFECTS LAYER, VU1 owners (stage 5): a queue callback whose packets go through a VU1 microprogram (VIF MSCAL /
//! MSCALF / MSCNT) is drawn from the GS packets that program XGKICKs, with no per-effect code.
//!
//! Source of the VU1 state at the owner's start [C]: the field's own VIF1 chain.
//! - Timing [C, measured r0230 2026-10-13]: the chain the game builds in field N is kicked in field N + 1 (every queue
//!   callback's words show up in the next field's jobs, none in its own). So at field N's end the oracle has kicked
//!   chain N - 1 and chain N sits in RAM (tag list page + packet buffers, double-buffered).
//! - The chain start: the start of the chain kicked this field (chain N - 1) is found by matching its first job
//!   (stage 7: a known start whose walk passes through the field's owners, the one not used last field first; else a
//!   4 KB page whose first tag carries the kicked chain's first tag words): the tag pages alternate (Rilgar r0230
//!   0x330000 / 0x490000; Eudora two pages with the same first block, so a first-block match alone is ambiguous).
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
	/// Chain start addresses that walked through an owner (Rilgar and Eudora alternate two tag pages) and the one
	/// used last field.
	known: Vec<u32>,
	last: Option<u32>,
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
		Picker { model: Some(model), th: None, known: Vec::new(), last: None, spare: Spare::default(), spare_rx }
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
	/// the end of the last one. The start: a known start whose walk passes through the owners (the one not used last
	/// field first: alternating tag pages), else a 4 KB page whose first tag carries the kicked chain's first tag words
	/// and whose walk passes through them. `run` checks the walk against the next field's jobs word for word (`fails`).
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
		let mut chain = None;
		if let Some(&(_, lc0, lc1)) = tg.last() {
			let try_walk = |s: u32, sp: &mut Spare| -> Option<Vec<Piece>> {
				let mut inside = false;
				let mut left = false;
				// Plan first (tags only): a start that does not pass through the owners costs no data copy, the winner is copied once.
				let t_pl = std::time::Instant::now();
				let plan = plan_chain(m, s, |t| {
					let i = t >= lc0 && t < lc1;
					left = inside && !i;
					inside |= i;
					left
				});
				sp_add(2, t_pl.elapsed().as_nanos() as u64);
				let plan = plan?;
				sp_add(7, plan.iter().map(|p| p.len() as u64).sum());
				left.then(|| {
					let t_b = std::time::Instant::now();
					let c = build_chain(m, &plan, sp);
					sp_add(3, t_b.elapsed().as_nanos() as u64);
					sp_add(5, c.iter().map(|p| p.w.len() as u64).sum());
					c
				})
			};
			let mut cand: Vec<u32> = self.known.iter().copied().filter(|&a| Some(a) != self.last).collect();
			cand.extend(self.last);
			for a in cand {
				if let Some(c) = try_walk(a, &mut self.spare) {
					chain = Some((a, c));
					break;
				}
			}
			if chain.is_none() {
				// Discovery: a page whose first tag carries the kicked chain's first tag words (job 0's first two words).
				if let Some((Some(&a0), Some(&a1))) = pending.first().map(|j| (j.0.first(), j.0.get(1))) {
					let ram = (m.cpu.mem.ram.len() as u32) & !0xfff;
					let t_disc = std::time::Instant::now();
					sp_add(6, 1);
					for a in (0..ram).step_by(0x1000) {
						let t = qw(m, a);
						if (t >> 64) as u32 != a0 || (t >> 96) as u32 != a1 || self.known.contains(&a) {
							continue;
						}
						if let Some(c) = try_walk(a, &mut self.spare) {
							self.known.push(a);
							if self.known.len() > 4 {
								self.known.remove(0);
							}
							chain = Some((a, c));
							break;
						}
					}
					sp_add(4, t_disc.elapsed().as_nanos() as u64);
				}
			}
			if chain.is_none() && log() {
				eprintln!("efxvu no chain start walks through the last owner {lc0:x} (known {:x?})", self.known);
			}
		}
		self.last = chain.as_ref().map(|c| c.0);
		let fns = tg.iter().map(|t| own[t.0].fnp).collect();
		Work { chain: chain.map(|c| c.1), tg, walk_ms: t0.elapsed().as_secs_f64() * 1e3, fns, vj: pending }
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
