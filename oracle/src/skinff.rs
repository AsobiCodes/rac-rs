//! [PC] (speed) Native fast path for the one-joint vertex loop of skinned_vertex_stream (Veldin finale f_1f7440 and
//! its binary twins on the other levels: +0x694 .. +0x928, 4 vertices per pass, 166 EE instructions, four CTC2 kicks
//! that each resume prog0 on VU0 for 28 pairs). About a third of the EE thread on character-heavy scenes.
//!
//! The recompiled loop head (recgen hook, matched by the loop's code words) calls `Cpu::skin_ff`. It runs whole
//! passes on local copies of the registers: the EE half is one statement per retail instruction with ee-core's
//! semantics (`skinff_ee.rs`, generated from the code words by scratch_slow/t37/skinff_ee.py), the VU half is the native
//! prog0 translation's own node chain for each resume (`skinff_vu.rs`, generated from vu0_prog0.rs by
//! skinff_gen.py). Same results, same instruction count, same VU0 counters as the compiled code + CTC2 + native prog0.
//!
//! It runs only where the outcome of every branch is known in advance:
//! - no poll can fire in a pass: no interrupt pending and icount + 166 <= due (the compiled blocks' poll test);
//! - native prog0 is the VU0 path (VU0AOT on, prog0 verified, no coverage / trace / profile) and VU0 is parked at a
//!   vi02 wait; vi01 != 0 (the first resume's vi01 waits fall through; later ones see vi01 = 1 from resume b);
//! - [PC] prog0 holds one copy of the vertex code per vertex group (counted by vi03, vi04, vi05, then the
//!   drain copy counted by vi11; waits vi02 / vi01: 0x418 / 0x2f0, 0x6a8 / 0x560, 0x1c8 / 0x0f0, 0x858 / 0x780). Each
//!   resume tests its group counter, then decrements it; a counter at 0 runs into the next group's copy (vi03 -> vi04
//!   -> vi05 -> drain), which skinff_vu.rs follows as a real branch, and the program end (E bit) marks the pass bad.
//!   So a pass starts only with 4 or more group counts left (`left`): the end then cannot come inside it, and the
//!   compiled code runs the last passes of a call (a bad pass would cost an undo and a redo of the whole call).
//!   Every other branch outcome the straight-line code assumes is tested at run time: a different one marks it bad.
//! - the sticky status bits (vi16 bits 6, 7) are set, so only each run's last flag result changes vi16 / vi17.
//! A wait-branch exit (QMFC2 sign, the EE waiting on VU0) and the loop end leave at a block boundary.
//!
//! Memory: loads and stores take Mem::fbuf's plain bytes (RAM, scratchpad, their mirrors, VU0 data memory), as the
//! compiled code's fast path does. Any other address (a hardware register) marks the run bad: every store of the call
//! is undone (log) and VU0 data memory restored, the registers were never written back, and the compiled code runs
//! that pass (the fast path redoes the good passes before it first).
//!
//! SKINFF=0 turns it off; SKINFF=diff predicts each pass (then undoes it), lets the compiled code run it and compares the
//! whole state at the next loop head (`SkinFf` counters, first differences on stderr).

#[path = "skinff_ee.rs"]
mod ee;
#[path = "skinff_vu.rs"]
mod vu;

use crate::cpu::Cpu;
use crate::vu0aot::{zs_masks, V, MODE_ON, REV4};
use ee_core::cpu::{sx32, HI64, SDL_MASK, SDR_MASK};
use std::arch::x86_64::*;

pub const MODE_OFF: u8 = 0;
pub const MODE_RUN: u8 = 1;
pub const MODE_DIFF: u8 = 2;

// [PC] prog0 holds one copy of the vertex code per vertex group (vi03, vi04, vi05 count the groups' vertices,
// vi11 the drain copy's); the loop head finds VU0 parked at one of the copies' vi02 waits (skinff_vu::VI02_WAITS).

type LogEnt = (usize, u8, [u8; 16]);
const LOG_CAP: usize = 4096;

pub struct SkinFf {
    pub mode: u8,
    /// No fast path before this icount (set after a bad pass, so the compiled code runs it).
    pub resume_at: u64,
    /// Fast-path calls that ran / passes / bad runs (undone).
    pub calls: u64,
    pub passes: u64,
    pub bad: u64,
    /// Store log of the current call: (host pointer, bytes, old bytes); `nlog` entries are in use (fixed size: a run stops
    /// at a pass boundary when fewer than one pass of stores would still fit).
    log: Vec<LogEnt>,
    nlog: usize,
    /// VU0 data memory at the start of the current call.
    dsave: Vec<u8>,
    /// Differential mode: the prediction for the pass now running in the compiled code, passes compared, differing.
    pred: Option<Box<Pred>>,
    pub diff_runs: u64,
    pub diff_bad: u64,
}

impl Default for SkinFf {
    fn default() -> SkinFf {
        let mode = match std::env::var("SKINFF").as_deref() {
            Ok("0") | Ok("off") => MODE_OFF,
            Ok("diff") => MODE_DIFF,
            _ => MODE_RUN,
        };
        SkinFf {
            mode,
            resume_at: 0,
            calls: 0,
            passes: 0,
            bad: 0,
            log: vec![(0, 0, [0; 16]); LOG_CAP],
            nlog: 0,
            dsave: Vec::with_capacity(0x1000),
            pred: None,
            diff_runs: 0,
            diff_bad: 0,
        }
    }
}

struct Pred {
    pc: u32,
    icount: u64,
    r: [u128; 32],
    lo: u128,
    hi: u128,
    vf: [[u32; 4]; 32],
    vacc: [u32; 4],
    vi: [u32; 32],
    stats: [u64; 6],
    n_native: u64,
    d: Vec<u8>,
    /// Final bytes at every stored address.
    w: Vec<(usize, u8, [u8; 16])>,
}

/// The loop's state while it runs natively.
#[repr(C, align(16))]
pub struct St {
    pub vf: [V; 32],
    pub e: ee::Regs,
    pub lo: V,
    pub hi: V,
    pub vi: [u32; 32],
    pub acc: V,
    ram: *mut u8,
    ram_len: usize,
    spr: *mut u8,
    spr_len: usize,
    dm: *mut u8,
    mem: *mut ee_core::mem::Mem,
    logp: *mut LogEnt,
    nlog: usize,
    pub bad: bool,
    /// Resumes a (vi02 kick) and b (vi01 kick); the pc the last one parked at.
    na: u64,
    nb: u64,
    park: usize,
    /// prog0 instruction pairs the resumes ran.
    pairs: u64,
}

/// Any other address (scratchpad aliases, hardware registers, VU memory): Mem::fbuf's answer. A free function on the
/// pointer so `St` itself never escapes and can live in registers.
#[inline(never)]
fn at_slow(mem: *mut ee_core::mem::Mem, a: u32, n: usize) -> Option<*mut u8> {
    unsafe { (*mem).fbuf(a, n).map(|b| b.as_mut_ptr()) }
}

impl St {
    #[inline(always)]
    fn at(&mut self, a: u32, n: usize) -> Option<*mut u8> {
        // RAM and scratchpad alternate in the loop, so choose the base with selects (cmov) instead of two branches that
        // mispredict; only the rare other address (hardware registers) branches away.
        let i = a as usize;
        let s = a.wrapping_sub(0x7000_0000) as usize;
        let in_ram = i + n <= self.ram_len;
        let in_spr = s + n <= self.spr_len;
        if in_ram | in_spr {
            let (base, off) = if in_ram { (self.ram, i) } else { (self.spr, s) };
            return Some(unsafe { base.add(off) });
        }
        at_slow(self.mem, a, n)
    }
    #[inline(always)]
    pub fn rd128(&mut self, a: u32) -> V {
        match self.at(a, 16) {
            Some(p) => unsafe { _mm_loadu_si128(p as *const __m128i) },
            None => {
                self.bad = true;
                unsafe { _mm_setzero_si128() }
            }
        }
    }
    #[inline(always)]
    pub fn rd64(&mut self, a: u32) -> u64 {
        match self.at(a, 8) {
            Some(p) => unsafe { (p as *const u64).read_unaligned() },
            None => {
                self.bad = true;
                0
            }
        }
    }
    #[inline(always)]
    pub fn wr128(&mut self, a: u32, v: V) {
        match self.at(a, 16) {
            Some(p) => unsafe {
                let e = self.logp.add(self.nlog);
                (*e).0 = p as usize;
                (*e).1 = 16;
                _mm_storeu_si128((*e).2.as_mut_ptr() as *mut __m128i, _mm_loadu_si128(p as *const __m128i));
                self.nlog += 1;
                _mm_storeu_si128(p as *mut __m128i, v);
            },
            None => self.bad = true,
        }
    }
    #[inline(always)]
    pub fn wr64(&mut self, a: u32, v: u64) {
        match self.at(a, 8) {
            Some(p) => unsafe {
                let e = self.logp.add(self.nlog);
                (*e).0 = p as usize;
                (*e).1 = 8;
                (*e).2 = [0; 16];
                ((*e).2.as_mut_ptr() as *mut u64).write_unaligned((p as *const u64).read_unaligned());
                self.nlog += 1;
                (p as *mut u64).write_unaligned(v);
            },
            None => self.bad = true,
        }
    }
    #[inline(always)]
    pub fn wr32(&mut self, a: u32, v: u32) {
        match self.at(a, 4) {
            Some(p) => unsafe {
                let e = self.logp.add(self.nlog);
                (*e).0 = p as usize;
                (*e).1 = 4;
                (*e).2 = [0; 16];
                ((*e).2.as_mut_ptr() as *mut u32).write_unaligned((p as *const u32).read_unaligned());
                self.nlog += 1;
                (p as *mut u32).write_unaligned(v);
            },
            None => self.bad = true,
        }
    }
    #[inline(always)]
    pub fn vget(&self, n: usize) -> V {
        self.vf[n]
    }
    /// QMTC2 / LQC2 (vf00 is constant: the emitter never targets it).
    #[inline(always)]
    pub fn vset(&mut self, n: usize, v: V) {
        self.vf[n] = v;
    }
    /// PMULTH (ee-core mmi2 sa 28): the eight signed halfword products; lo = [p0, p1, p4, p5], hi = [p2, p3, p6, p7], the
    /// result is [lo0, hi0, lo2, hi2].
    #[inline(always)]
    pub fn pmulth(&mut self, x: V, y: V) -> V {
        unsafe {
            let (pl, ph) = (_mm_mullo_epi16(x, y), _mm_mulhi_epi16(x, y));
            let (p0, p1) = (_mm_unpacklo_epi16(pl, ph), _mm_unpackhi_epi16(pl, ph));
            let (l, h) = (_mm_unpacklo_epi64(p0, p1), _mm_unpackhi_epi64(p0, p1));
            self.lo = l;
            self.hi = h;
            _mm_unpacklo_epi64(_mm_unpacklo_epi32(l, h), _mm_unpackhi_epi32(l, h))
        }
    }
    /// CTC2 zero, viN (N = 1, 2) with VU0 parked at the wait on viN: the resume (vu0_resume -> native prog0 run).
    #[inline(always)]
    pub fn ctc2(&mut self, n: usize) {
        if self.bad {
            return;
        }
        self.vi[n] = 0;
        // A wrong wait, a branch outcome other than the straight-line code's, or the program's end marks the pass bad
        // (undone; the compiled code runs it).
        let r = unsafe { vu::resume(self.park, n, &mut self.vf, self.dm, &mut self.vi, &mut self.acc) };
        if n == 2 {
            self.na += 1;
        } else {
            self.nb += 1;
        }
        if !r.ok {
            self.bad = true;
            return;
        }
        self.park = r.park;
        self.pairs += r.pairs;
        let (fu, fm) = (r.fu, r.fm);
        // vu0_prog0_run's epilogue (each resume has flag results); the sticky bits are already set.
        let (zl, sl) = zs_masks(fu, fm);
        self.vi[17] = REV4[zl as usize] | REV4[sl as usize] << 4;
        self.vi[16] = (self.vi[16] & !0xf) | (zl != 0) as u32 | ((sl != 0) as u32) << 1;
        self.vi[29] = 1 | (self.park as u32) << 16;
    }
}

#[inline(always)]
fn m(x: u128) -> __m128i {
    unsafe { std::mem::transmute(x) }
}
#[inline(always)]
fn u(x: __m128i) -> u128 {
    unsafe { std::mem::transmute(x) }
}
/// The low 64 bits of a vector register (a scalar read of a register that is also used as a vector).
#[inline(always)]
pub fn lo64v(v: V) -> u64 {
    unsafe { _mm_cvtsi128_si64(v) as u64 }
}
/// The low 64 bits replaced, the high 64 kept.
#[inline(always)]
pub fn setlo(v: V, x: u64) -> V {
    unsafe { _mm_castpd_si128(_mm_move_sd(_mm_castsi128_pd(v), _mm_castsi128_pd(_mm_cvtsi64_si128(x as i64)))) }
}
#[inline(always)]
pub fn zero() -> V {
    unsafe { _mm_setzero_si128() }
}
/// PCPYLD rs, rt: [rt.lo, rs.lo].
#[inline(always)]
pub fn pcpyld(a: V, b: V) -> V {
    unsafe { _mm_unpacklo_epi64(b, a) }
}
/// PCPYUD rs, rt: [rs.hi, rt.hi].
#[inline(always)]
pub fn pcpyud(a: V, b: V) -> V {
    unsafe { _mm_unpackhi_epi64(a, b) }
}
/// PEXTLB rs, rt: bytes rt[0], rs[0], rt[1], rs[1], ...
#[inline(always)]
pub fn pextlb(a: V, b: V) -> V {
    unsafe { _mm_unpacklo_epi8(b, a) }
}
/// PEXTUB rs, rt: bytes rt[8], rs[8], rt[9], rs[9], ...
#[inline(always)]
pub fn pextub(a: V, b: V) -> V {
    unsafe { _mm_unpackhi_epi8(b, a) }
}
/// PPACB rs, rt: the even bytes of rt, then the even bytes of rs.
#[inline(always)]
pub fn ppacb(a: V, b: V) -> V {
    unsafe {
        let k = _mm_set1_epi16(0xff);
        _mm_packus_epi16(_mm_and_si128(b, k), _mm_and_si128(a, k))
    }
}
/// PMFHL.SH: the eight words of lo / hi saturated to halfwords (ee-core `ssh`), in the order l0 l1 h0 h1 l2 l3 h2 h3.
#[inline(always)]
pub fn pmfhl_sh(l: V, h: V) -> V {
    unsafe { _mm_packs_epi32(_mm_unpacklo_epi64(l, h), _mm_unpackhi_epi64(l, h)) }
}
/// PEXTLH rs, rt: halfwords rt[0], rs[0], rt[1], rs[1], ...
#[inline(always)]
pub fn pextlh(a: V, b: V) -> V {
    unsafe { _mm_unpacklo_epi16(b, a) }
}
/// PEXTUH rs, rt: halfwords rt[4], rs[4], rt[5], rs[5], ...
#[inline(always)]
pub fn pextuh(a: V, b: V) -> V {
    unsafe { _mm_unpackhi_epi16(b, a) }
}
/// The even halfwords of x in the low 64 bits.
#[inline(always)]
fn even_h(x: __m128i) -> __m128i {
    unsafe { _mm_shuffle_epi32::<0x08>(_mm_shufflehi_epi16::<0x08>(_mm_shufflelo_epi16::<0x08>(x))) }
}
/// PPACH rs, rt: the even halfwords of rt, then the even halfwords of rs.
#[inline(always)]
pub fn ppach(a: V, b: V) -> V {
    unsafe { _mm_unpacklo_epi64(even_h(b), even_h(a)) }
}
/// PSRAH / PSLLW / PSRAW rd, rt, sa (ee-core mmi 55 / 60 / 63).
#[inline(always)]
pub fn psrah<const S: i32>(b: V) -> V {
    unsafe { _mm_srai_epi16::<S>(b) }
}
#[inline(always)]
pub fn psllw<const S: i32>(b: V) -> V {
    unsafe { _mm_slli_epi32::<S>(b) }
}
#[inline(always)]
pub fn psraw<const S: i32>(b: V) -> V {
    unsafe { _mm_srai_epi32::<S>(b) }
}

/// How a run ended: offset of the next pc in the function, icount, passes completed.
struct RunEnd {
    off: u32,
    icount: u64,
    passes: u64,
    blocks: u64,
}

/// Group counts still to run, from the group VU0 is parked in to the drain copy's (vi03 -> vi04 -> vi05 -> vi11):
/// each resume takes exactly one, and the program ends (E bit) at a resume that finds the drain counter at 0, so a
/// pass (four resumes) that starts with 4 or more left cannot reach the end.
#[inline(always)]
fn left(park: usize, vi: &[u32; 32]) -> u32 {
    let c = |n: usize| vi[n] & 0xffff;
    match park {
        0x418 => c(3) + c(4) + c(5) + c(11),
        0x6a8 => c(4) + c(5) + c(11),
        0x1c8 => c(5) + c(11),
        0x858 => c(11),
        _ => 0,
    }
}

impl Cpu {
    /// The recompiled loop head (`pc` = function + 0x694) offers the loop: true when passes ran natively (pc / npc
    /// then point to where the compiled code continues).
    pub fn skin_ff(&mut self, pc: u32) -> bool {
        let mode = self.skinff.mode;
        if mode == MODE_OFF || self.icount < self.skinff.resume_at || !self.skin_ok() {
            if mode == MODE_DIFF {
                self.skin_diff_check(pc);
            }
            return false;
        }
        if mode == MODE_DIFF {
            self.skin_diff(pc);
            return false;
        }
        let _chop = crate::fpu::ChopGuard::new();
        let mut st = self.skin_st();
        self.skin_begin();
        let (e, bad) = self.skin_run(&mut st, u64::MAX);
        if let Some(k) = bad {
            // Undo the whole call, then redo its good passes (deterministic: same state, same result).
            self.skin_undo();
            self.skinff.bad += 1;
            self.skinff.resume_at = self.icount + (k + 1) * ee::PASS_INSNS;
            if k == 0 {
                return false;
            }
            return self.skin_redo(pc, k);
        }
        if e.blocks == 0 {
            return false;
        }
        self.skin_commit(&st, &e, pc);
        true
    }

    /// Redo after an undone bad run: the first `k` (good) passes. Out of line, so the hot `skin_ff` holds only one
    /// inlined copy of `skin_run`.
    #[inline(never)]
    fn skin_redo(&mut self, pc: u32, k: u64) -> bool {
        let mut st = self.skin_st();
        self.skin_begin();
        let (e, bad) = self.skin_run_cold(&mut st, k);
        debug_assert!(bad.is_none());
        self.skin_commit(&st, &e, pc);
        true
    }

    /// The conditions that hold for the whole call (see the module comment); the per-pass ones are in `skin_run`.
    fn skin_ok(&self) -> bool {
        let a = &self.vu0_aot;
        !self.mem.irq_dirty
            && a.mode == MODE_ON
            && a.checked
            && a.ok
            && !a.prog1
            && a.ok_gen == self.mem.vu0m_gen
            && !a.cov_on
            && !a.prof
            && self.vu0_trace.is_none()
            && self.vi[29] & 0xffff == 1
            && vu::VI02_WAITS.contains(&((self.vi[29] >> 16) as usize))
            && self.vi[16] & 0xc0 == 0xc0
            && self.vi[1] & 0xffff != 0
            && left((self.vi[29] >> 16) as usize, &self.vi) >= 4
            && self.icount + ee::PASS_INSNS <= self.mem.due
    }

    #[inline(always)]
    fn skin_st(&mut self) -> St {
        let mem: *mut ee_core::mem::Mem = &mut self.core_mut().mem;
        let logp: *mut LogEnt = self.skinff.log.as_mut_ptr();
        St {
            vf: unsafe { std::mem::transmute(self.vf) },
            e: ee::Regs::load(&self.r),
            lo: m(self.lo),
            hi: m(self.hi),
            vi: self.vi,
            acc: unsafe { crate::vu0aot::ldv(self.vacc.as_ptr()) },
            ram: self.mem.ram.as_mut_ptr(),
            ram_len: self.mem.ram.len(),
            spr: self.mem.spr.as_mut_ptr(),
            spr_len: self.mem.spr.len(),
            dm: self.mem.vu0d.as_mut_ptr(),
            mem,
            logp,
            nlog: 0,
            bad: false,
            na: 0,
            nb: 0,
            park: (self.vi[29] >> 16) as usize,
            pairs: 0,
        }
    }

    fn skin_begin(&mut self) {
        self.skinff.nlog = 0;
        self.skinff.dsave.clear();
        self.skinff.dsave.extend_from_slice(&self.mem.vu0d);
    }

    fn skin_undo(&mut self) {
        for &(p, n, old) in self.skinff.log[..self.skinff.nlog].iter().rev() {
            unsafe { std::ptr::copy_nonoverlapping(old.as_ptr(), p as *mut u8, n as usize) };
        }
        self.skinff.nlog = 0;
        self.mem.vu0d.copy_from_slice(&self.skinff.dsave);
    }

    /// Up to `max` passes. Returns the end and, when an access was not plain memory, the pass it happened in.
    #[inline(always)]
    fn skin_run(&mut self, st: &mut St, max: u64) -> (RunEnd, Option<u64>) {
        let due = self.mem.due;
        let mut e = RunEnd { off: ee::BLOCKS[0], icount: self.icount, passes: 0, blocks: 0 };
        macro_rules! blk {
            ($f:path) => {{
                let (tk, n, tg) = $f(st);
                e.icount += n;
                e.blocks += 1;
                if st.bad {
                    self.skinff.nlog = st.nlog;
                    let k = e.passes;
                    return (e, Some(k));
                }
                (tk, tg)
            }};
        }
        while e.passes < max && e.icount + ee::PASS_INSNS <= due
            && st.nlog + ee::PASS_STORES <= LOG_CAP
            && left(st.park, &st.vi) >= 4
        {
            let (tk, tg) = blk!(ee::block_0);
            if tk {
                e.off = tg;
                self.skinff.nlog = st.nlog;
                return (e, None);
            }
            let (tk, tg) = blk!(ee::block_1);
            if tk {
                e.off = tg;
                self.skinff.nlog = st.nlog;
                return (e, None);
            }
            let (tk, tg) = blk!(ee::block_2);
            if tk {
                e.off = tg;
                self.skinff.nlog = st.nlog;
                return (e, None);
            }
            let (tk, tg) = blk!(ee::block_3);
            if tk {
                e.off = tg;
                self.skinff.nlog = st.nlog;
                return (e, None);
            }
            let (tk, _) = blk!(ee::block_4);
            e.passes += 1;
            if !tk {
                // The loop's bne fell through: the code after it.
                e.off = ee::LOOP_END;
                self.skinff.nlog = st.nlog;
                return (e, None);
            }
        }
        e.off = ee::BLOCKS[0];
        self.skinff.nlog = st.nlog;
        (e, None)
    }

    /// The one out-of-line copy of `skin_run` (redo and diff mode).
    #[inline(never)]
    fn skin_run_cold(&mut self, st: &mut St, max: u64) -> (RunEnd, Option<u64>) {
        self.skin_run(st, max)
    }

    #[inline(always)]
    fn skin_commit(&mut self, st: &St, e: &RunEnd, pc: u32) {
        st.e.store(&mut self.r);
        self.lo = u(st.lo);
        self.hi = u(st.hi);
        self.vf = unsafe { std::mem::transmute(st.vf) };
        self.vi = st.vi;
        unsafe { crate::vu0aot::stv(self.vacc.as_mut_ptr(), st.acc) };
        self.icount = e.icount;
        let base = pc.wrapping_sub(ee::BLOCKS[0]);
        self.pc = base.wrapping_add(e.off);
        self.npc = self.pc.wrapping_add(4);
        let n = st.na + st.nb;
        if n > 0 {
            // Per resume: vu0_resume, vu0_exec_native, the park.
            self.vu0_stats[2] += n;
            self.vu0_stats[1] += n;
            self.vu0_stats[3] += st.pairs;
            self.vu0_stats[4] = st.park as u64;
            self.vu0_stats[5] = 1;
            self.vu0_aot.n_native += n;
        }
        self.skinff.calls += 1;
        self.skinff.passes += e.passes;
    }

    /// Differential mode: compares the pending prediction (if any) with the state the compiled code reached.
    #[inline(never)]
    fn skin_diff_check(&mut self, pc: u32) {
        let Some(p) = self.skinff.pred.take() else { return };
        self.skinff.diff_runs += 1;
        let mut bad: Vec<String> = Vec::new();
        if p.pc != pc {
            bad.push(format!("pc {:08x} vs {:08x}", p.pc, pc));
        }
        if p.icount != self.icount {
            bad.push(format!("icount {} vs {}", p.icount, self.icount));
        }
        for i in 0..32 {
            if p.r[i] != self.r[i] {
                bad.push(format!("r{i} {:032x} vs {:032x}", p.r[i], self.r[i]));
            }
            if p.vf[i] != self.vf[i] {
                bad.push(format!("vf{i} {:08x?} vs {:08x?}", p.vf[i], self.vf[i]));
            }
            if p.vi[i] != self.vi[i] {
                bad.push(format!("vi{i} {:x} vs {:x}", p.vi[i], self.vi[i]));
            }
        }
        if p.lo != self.lo || p.hi != self.hi {
            bad.push("lo/hi".into());
        }
        if p.vacc != self.vacc {
            bad.push(format!("acc {:08x?} vs {:08x?}", p.vacc, self.vacc));
        }
        if p.stats != self.vu0_stats || p.n_native != self.vu0_aot.n_native {
            bad.push(format!("vu0 stats {:?}/{} vs {:?}/{}", p.stats, p.n_native, self.vu0_stats, self.vu0_aot.n_native));
        }
        if p.d[..] != self.mem.vu0d[..] {
            bad.push("vu0 data memory".into());
        }
        for &(q, n, v) in &p.w {
            let mut b = [0u8; 16];
            unsafe { std::ptr::copy_nonoverlapping(q as *const u8, b.as_mut_ptr(), n as usize) };
            if b != v {
                bad.push(format!("store at host {:#x}: {:02x?} vs {:02x?}", q, &v[..n as usize], &b[..n as usize]));
                break;
            }
        }
        if !bad.is_empty() {
            self.skinff.diff_bad += 1;
            if self.skinff.diff_bad <= 10 {
                eprintln!("skinff diff at {pc:08x}: {}", bad.join("; "));
            }
        }
        let k = self.skinff.diff_runs;
        if k.is_power_of_two() && k >= 1024 {
            eprintln!("skinff diff: {} passes compared, {} differ", k, self.skinff.diff_bad);
        }
    }

    /// Differential mode: checks the last prediction, predicts one pass (then undoes it) for the compiled code to run.
    #[inline(never)]
    fn skin_diff(&mut self, pc: u32) {
        self.skin_diff_check(pc);
        let _chop = crate::fpu::ChopGuard::new();
        let mut st = self.skin_st();
        self.skin_begin();
        let (e, bad) = self.skin_run_cold(&mut st, 1);
        if bad.is_some() || e.passes != 1 || e.off != ee::BLOCKS[0] {
            self.skin_undo();
            return;
        }
        let save = (self.r, self.lo, self.hi, self.vf, self.vi, self.vacc, self.icount, self.pc, self.npc, self.vu0_stats, self.vu0_aot.n_native);
        self.skin_commit(&st, &e, pc);
        let w = self
            .skinff
            .log[..self.skinff.nlog]
            .iter()
            .map(|&(q, n, _)| {
                let mut b = [0u8; 16];
                unsafe { std::ptr::copy_nonoverlapping(q as *const u8, b.as_mut_ptr(), n as usize) };
                (q, n, b)
            })
            .collect();
        let p = Pred {
            pc,
            icount: self.icount,
            r: self.r,
            lo: self.lo,
            hi: self.hi,
            vf: self.vf,
            vacc: self.vacc,
            vi: self.vi,
            stats: self.vu0_stats,
            n_native: self.vu0_aot.n_native,
            d: self.mem.vu0d.clone(),
            w,
        };
        self.skinff.pred = Some(Box::new(p));
        self.skin_undo();
        (self.r, self.lo, self.hi, self.vf, self.vi, self.vacc, self.icount, self.pc, self.npc, self.vu0_stats, self.vu0_aot.n_native) = save;
        self.skinff.calls -= 1;
        self.skinff.passes -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ee_core::cpu::{h8, mapw, maph, ssh, uh8, uw4, w4, LO64};

    fn rnd(s: &mut u64) -> u128 {
        let mut x = 0u128;
        for _ in 0..2 {
            *s ^= *s << 13;
            *s ^= *s >> 7;
            *s ^= *s << 17;
            x = x << 64 | *s as u128;
        }
        x
    }

    /// The vector helpers against ee-core's scalar MMI definitions.
    #[test]
    fn mmi_helpers_match_ee_core() {
        let mut s = 0x9e37_79b9_7f4a_7c15u64;
        for k in 0..20000 {
            let (mut a, mut b) = (rnd(&mut s), rnd(&mut s));
            if k % 7 == 0 {
                a = mapw(a, 0, |x, _| x | 0x7fff_8000);
                b = mapw(b, 0, |x, _| x & 0x8000_7fff);
            }
            let (x, y) = (h8(a), h8(b));
            assert_eq!(u(pextlh(m(a), m(b))), uh8([y[0], x[0], y[1], x[1], y[2], x[2], y[3], x[3]]));
            assert_eq!(u(pextuh(m(a), m(b))), uh8([y[4], x[4], y[5], x[5], y[6], x[6], y[7], x[7]]));
            assert_eq!(u(ppach(m(a), m(b))), uh8([y[0], y[2], y[4], y[6], x[0], x[2], x[4], x[6]]));
            assert_eq!(u(pcpyld(m(a), m(b))), (b & LO64) | (a << 64));
            assert_eq!(u(pcpyud(m(a), m(b))), (a >> 64) | (b & HI64));
            assert_eq!(u(setlo(m(a), b as u64)), (a & HI64) | (b as u64 as u128));
            assert_eq!(lo64v(m(a)), a as u64);
            let (xb, yb) = (a.to_le_bytes(), b.to_le_bytes());
            let mut r = [0u8; 16];
            for i in 0..8 {
                r[2 * i] = yb[i];
                r[2 * i + 1] = xb[i];
            }
            assert_eq!(u(pextlb(m(a), m(b))), u128::from_le_bytes(r));
            for i in 0..8 {
                r[2 * i] = yb[8 + i];
                r[2 * i + 1] = xb[8 + i];
            }
            assert_eq!(u(pextub(m(a), m(b))), u128::from_le_bytes(r));
            for i in 0..8 {
                r[i] = yb[2 * i];
                r[8 + i] = xb[2 * i];
            }
            assert_eq!(u(ppacb(m(a), m(b))), u128::from_le_bytes(r));
            assert_eq!(u(psrah::<5>(m(b))), maph(b, 0, |x, _| ((x as i16) >> 5) as u16));
            assert_eq!(u(psrah::<15>(m(b))), maph(b, 0, |x, _| ((x as i16) >> 15) as u16));
            assert_eq!(u(psllw::<7>(m(b))), mapw(b, 0, |x, _| x << 7));
            assert_eq!(u(psraw::<16>(m(b))), mapw(b, 0, |x, _| ((x as i32) >> 16) as u32));
            assert_eq!(u(psraw::<31>(m(b))), mapw(b, 0, |x, _| ((x as i32) >> 31) as u32));
            // PMULTH and PMFHL.SH (ee-core mmi2 sa 28, mmi 48 sa 4).
            let p = |i: usize| (x[i] as i16 as i32).wrapping_mul(y[i] as i16 as i32) as u32;
            let (l, h) = ([p(0), p(1), p(4), p(5)], [p(2), p(3), p(6), p(7)]);
            let mut st: St = unsafe { std::mem::zeroed() };
            assert_eq!(u(st.pmulth(m(a), m(b))), uw4([l[0], h[0], l[2], h[2]]));
            assert_eq!((u(st.lo), u(st.hi)), (uw4(l), uw4(h)));
            let (lo, hi) = (rnd(&mut s), rnd(&mut s));
            let (l, h) = (w4(lo), w4(hi));
            let c = |v: u32| ssh(v as i32);
            assert_eq!(u(pmfhl_sh(m(lo), m(hi))), uh8([c(l[0]), c(l[1]), c(h[0]), c(h[1]), c(l[2]), c(l[3]), c(h[2]), c(h[3])]));
            assert_eq!(u(pmfhl_sh(st.lo, st.hi)), {
                let (l, h) = (w4(u(st.lo)), w4(u(st.hi)));
                uh8([c(l[0]), c(l[1]), c(h[0]), c(h[1]), c(l[2]), c(l[3]), c(h[2]), c(h[3])])
            });
        }
    }

    /// itof4 / ftoi4 (vector) against the scalar definitions they replaced, under the same MXCSR as the fast path.
    #[test]
    fn itof_ftoi_match_scalar() {
        use crate::fpu::{cvt_w, cvtsi2ss_nc, fin, fout, mulss_nc};
        let _g = crate::fpu::ChopGuard::new();
        let mut s = 0x1234_5678_9abc_def1u64;
        let edge = [0u32, 1, 0x7f_ffff, 0x80_0000, 0x8000_0000, 0x8000_0001, 0x3f80_0000, 0x4e7f_ffff, 0x4e80_0000, 0x4e80_0001, 0x4efe_ffff,
            0x4eff_ffff, 0x4f00_0000, 0x4f00_0001, 0xcf00_0000, 0xcf00_0001, 0x7f7f_ffff, 0x7f80_0000, 0x7f80_0001, 0x7fc0_0000, 0xff80_0000,
            0xffff_ffff, 0xce80_0000, 0x7fff_ffff, 0x4d00_0000, 0x4cff_ffff, 0x4b80_0000, 0x4900_0000];
        let mut vals: Vec<u32> = edge.to_vec();
        for _ in 0..200000 {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            vals.push(s as u32);
            // exponents around the saturation boundary and small values
            vals.push(((s >> 40) as u32 & 0x80ff_ffff) | ((0x90 + (s >> 33) as u32 % 24) << 23 & 0x7f80_0000));
            vals.push((s >> 20) as u32 % 100000);
        }
        for c in vals.chunks(4) {
            if c.len() < 4 {
                break;
            }
            let v = unsafe { _mm_loadu_si128(c.as_ptr() as *const V) };
            let mut a = [0u32; 4];
            macro_rules! chk {
                ($k:literal, $sc_f:expr, $sc_i:expr) => {
                    unsafe { _mm_storeu_si128(a.as_mut_ptr() as *mut V, crate::vu0aot::ftoi4::<$k>(v)) };
                    for l in 0..4 {
                        assert_eq!(a[l], cvt_w(fout(mulss_nc(fin(c[l]), $sc_f))), "ftoi{} {:08x}", $k, c[l]);
                    }
                    unsafe { _mm_storeu_si128(a.as_mut_ptr() as *mut V, crate::vu0aot::itof4::<$k>(v)) };
                    for l in 0..4 {
                        assert_eq!(a[l], fout(mulss_nc(cvtsi2ss_nc(c[l] as i32), $sc_i)), "itof{} {:08x}", $k, c[l]);
                    }
                };
            }
            chk!(0, 1.0f32, 1.0f32);
            chk!(1, 16.0f32, 0.0625f32);
            chk!(2, 4096.0f32, 1.0f32 / 4096.0);
            chk!(3, 32768.0f32, 1.0f32 / 32768.0);
        }
    }
}
