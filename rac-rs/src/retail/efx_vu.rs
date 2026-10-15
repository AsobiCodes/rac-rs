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

/// The VIF1 + VU1 model and the check state of `run`: lives on the VU1 thread (`Shadow::submit`), no machine access.
struct VuModel {
	vif: Vif1,
	vu: Vu1,
	full: bool,
	/// The chain walked at the last field's end up to P (pieces' words, the last one cut at P).
	prev: Vec<Vec<u32>>,
	pub fails: u32,
	scratch: Out,
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

pub struct Shadow {
	/// The model until the first `submit` moves it to its thread.
	model: Option<VuModel>,
	th: Option<Sender<VuJob>>,
	tx: Sender<Job>,
	rx: Receiver<Job>,
	/// The machine's own sender while the tee is in.
	fwd: Option<Option<Sender<Job>>>,
	/// The VIF1 jobs of the fields since the last `run` (words, block marks): the chain kicked then, fed by `run`.
	pending: Vec<(Vec<u32>, Vec<(usize, u32)>)>,
	/// Chain start addresses that walked through an owner (Rilgar and Eudora alternate two tag pages) and the one
	/// used last field.
	known: Vec<u32>,
	last: Option<u32>,
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

/// The VIF1 chain from `start` as the oracle's DMA walk feeds it (TTE on, a piece per kick: it ends at a tag with the
/// IRQ bit, the next kick goes on behind it; the chain ends at an end / refe tag or a ret with an empty stack).
/// `stop(tadr)` true ends the walk before that tag (the chain so far is returned).
fn walk_chain(m: &Machine, start: u32, mut stop: impl FnMut(u32) -> bool) -> Option<Vec<Piece>> {
	let mut out = Vec::new();
	let mut tadr = start;
	let mut asr = [0u32; 2];
	let mut asp = 0usize;
	let mut n = 0u32;
	for _ in 0..16 {
		let mut p = Piece::default();
		loop {
			n += 1;
			if n > 400_000 || p.w.len() > 4_000_000 {
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
			p.marks.push((p.w.len(), 2));
			p.tags.push((p.w.len(), tadr));
			p.w.push((t >> 64) as u32);
			p.w.push((t >> 96) as u32);
			let src = if id == 3 || id == 4 { (tag >> 32) as u32 & 0xffff_fff0 } else { data };
			p.marks.push((p.w.len(), 0));
			// RAM data in one piece (the usual case: the chain's geometry blocks are most of the walk) copies as a slice; the
			// scratchpad, the wrap of the address mask and anything out of range go qword by qword as `qw` reads them.
			let lo = (src & 0x01ff_fff0) as usize;
			let n = q as usize * 16;
			if src & 0x8000_0000 == 0 && src.checked_add(q * 16).is_some_and(|e| e <= 0x8000_0000) && lo + n <= m.cpu.mem.ram.len() {
				p.w.reserve(n / 4);
				p.w.extend(m.cpu.mem.ram[lo..lo + n].chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])));
			} else {
				for k in 0..q {
					let d = qw(m, src.wrapping_add(k * 16));
					p.w.extend([d as u32, (d >> 32) as u32, (d >> 64) as u32, (d >> 96) as u32]);
				}
			}
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

impl Shadow {
	/// `dir`: the savestate directory (VU1 memories as the savestate left them; zero when missing, e.g. a cold boot).
	pub fn new(dir: &str) -> Shadow {
		let d = std::fs::read(format!("{dir}/vu1Memory.bin")).unwrap_or_default();
		let mi = std::fs::read(format!("{dir}/vu1MicroMem.bin")).unwrap_or_default();
		let (tx, rx) = channel();
		let mut vif = Vif1::new();
		vif.shadow = true;
		let full = std::env::var("RAC_EFX_VU_FULL").is_ok_and(|v| v == "1");
		let model = VuModel { vif, vu: Vu1::new(&d, &mi), full, prev: Vec::new(), fails: 0, scratch: Out::default() };
		Shadow { model: Some(model), th: None, tx, rx, fwd: None, pending: Vec::new(), known: Vec::new(), last: None }
	}

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

	/// The field's end, on the worker (reads RAM): the unclaimed VU1 owners of `own` and chain N walked from RAM up to
	/// the end of the last one. The start: a known start whose walk passes through the owners (the one not used last
	/// field first: alternating tag pages), else a 4 KB page whose first tag carries the kicked chain's first tag words
	/// and whose walk passes through them. `run` checks the walk against the next field's jobs word for word (`fails`).
	pub fn prep(&mut self, m: &Machine, own: &[Own], bi: &super::efx_draw::BuildIn) -> Work {
		let t0 = std::time::Instant::now();
		let mut tg: Vec<(usize, u32, u32)> = Vec::new();
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
		let mut chain = None;
		if let Some(&(_, lc0, lc1)) = tg.last() {
			let try_walk = |s: u32| -> Option<Vec<Piece>> {
				let mut inside = false;
				let mut left = false;
				let c = walk_chain(m, s, |t| {
					let i = t >= lc0 && t < lc1;
					left = inside && !i;
					inside |= i;
					left
				})?;
				left.then_some(c)
			};
			let mut cand: Vec<u32> = self.known.iter().copied().filter(|&a| Some(a) != self.last).collect();
			cand.extend(self.last);
			for a in cand {
				if let Some(c) = try_walk(a) {
					chain = Some((a, c));
					break;
				}
			}
			if chain.is_none() {
				// Discovery: a page whose first tag carries the kicked chain's first tag words (job 0's first two words).
				if let Some((Some(&a0), Some(&a1))) = self.pending.first().map(|j| (j.0.first(), j.0.get(1))) {
					let ram = (m.cpu.mem.ram.len() as u32) & !0xfff;
					for a in (0..ram).step_by(0x1000) {
						let t = qw(m, a);
						if (t >> 64) as u32 != a0 || (t >> 96) as u32 != a1 || self.known.contains(&a) {
							continue;
						}
						if let Some(c) = try_walk(a) {
							self.known.push(a);
							if self.known.len() > 4 {
								self.known.remove(0);
							}
							chain = Some((a, c));
							break;
						}
					}
				}
			}
			if chain.is_none() && log() {
				eprintln!("efxvu no chain start walks through the last owner {lc0:x} (known {:x?})", self.known);
			}
		}
		self.last = chain.as_ref().map(|c| c.0);
		let fns = tg.iter().map(|t| own[t.0].fnp).collect();
		Work { chain: chain.map(|c| c.1), tg, walk_ms: t0.elapsed().as_secs_f64() * 1e3, fns, vj: std::mem::take(&mut self.pending) }
	}

	/// The field's VU1 work and effects build go to the VU1 thread (started on the first call) and run while the worker
	/// goes on: the reply arrives on the returned receiver (`reply` false: no reply, the model still runs).
	pub fn submit(&mut self, w: Work, bi: super::efx_draw::BuildIn, field: u32, reply: bool) -> Option<std::sync::mpsc::Receiver<FxOut>> {
		if self.th.is_none() {
			let mut model = self.model.take()?;
			let (tx, rx) = channel::<VuJob>();
			let spawned = std::thread::Builder::new().name("vu1".into()).stack_size(16 << 20).spawn(move || {
				while let Ok(j) = rx.recv() {
					let out = model.run(j.w);
					if let Some(r) = j.reply {
						let (mut qa, mut pools) = (Vec::new(), Vec::new());
						let st = super::efx_draw::build_in(&j.bi, &out, &mut qa, &mut pools);
						if log() {
							eprintln!("efxdraw field {}: {st:x?}", j.field);
						}
						let _ = r.send((qa, pools));
					}
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
		self.vif.shadow = !run && !self.full;
		self.vif.feed(&w[a..b], &sub, &mut self.vu, out);
		self.vif.shadow = true;
		if !run {
			out.qs.clear();
			out.segs.clear();
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
					pf[3] += tt.elapsed().as_nanos() as u64;
					pf[5] += (b - a) as u64;
					pos = b;
					res.push((k, o.qs));
				}
				self.feed_off(&p.w, &p.marks, pos, p.w.len());
			}
		}
		pf_add(&pf, t1.elapsed().as_nanos() as u64);
		if std::env::var_os("RAC_PERF_VU").is_some() && self.vu.st.runs % 400 < 3 {
			eprintln!("perf: vu1 runs {} pairs {} native {} bails {} interp(not covered) {} compared {} differed {} bail_at {:?} {:?}", self.vu.st.runs, self.vu.st.pairs, self.vu.aot.native, self.vu.aot.bails, self.vu.aot.interp, self.vu.aot.compared, self.vu.aot.differed, self.vu.aot.bail_at, self.vu.diff_log);
		}
		self.prev = w.chain.map(|c| c.into_iter().map(|p| p.w).collect()).unwrap_or_default();
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
