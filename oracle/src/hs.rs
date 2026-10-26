//! Speed hands, runtime API (lane hs, see scratch_perf_hs/HS_RUNTIME.md).
//!
//! A speed hand is a `rec::Hand` registered with `est: hs::EXACT`. `Cpu::run_hand` then does nothing around it: the hand
//! sets the instruction count, the PC and everything else itself, exactly as the recompiled function would have.
//!
//! Shape of a hand (the entry has `c.pc == entry`, `ra` in r31):
//! ```ignore
//! fn hand(c: &mut Cpu) -> bool {
//!     let fr = Frame::enter(c);
//!     // block 0 (one per original basic block leader; the table comes from cutter.py `blocks`)
//!     if due_within(c, MAX_PATH) { return deopt(c, &fr, ENTRY, c.icount); }   // event due: retail runs it
//!     let t = blk(c, &T, 0);                      // c.icount = t + count of block 0; returns t
//!     ... Rust locals ...
//!     // an access that is not plain RAM/scratchpad, or any path the hand does not cover, at instruction k of the block:
//!     //   write every live register back to c, then
//!     //   return deopt(c, &fr, pc_of_that_instruction, t + k);
//!     // the end of the function:
//!     //   write the live-outs back, then ret(c, &fr); true
//! }
//! ```
//! `deopt` runs the rest of the call as retail (interpreter; compiled callees, hands, interrupts, vblank as in
//! `Cpu::tail` / cutrt3 `transfer`) from `pc` until the call returns (`Frame::done`), with `c.icount = ic` at that
//! instruction. A deopt point is an instruction boundary that is not in a branch delay slot (use the branch's pc).
use crate::cpu::Cpu;

/// Number of `deopt` calls (statistics; the gate prints it).
pub static DEOPTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// `Hand.est` of a speed hand: the hand does its own exact icount and sets pc/npc (`ret`).
pub const EXACT: u64 = u64::MAX;

/// What a call was entered with: it is over when the PC is `ra` again with the entry `sp` (not in a delay slot).
/// S7: `sp_free` (a function that uses $sp as a scratch register, see `sp_scratch`): over at `ra` whatever $sp holds.
#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
    pub ra: u32,
    pub sp: u32,
    pub sp_free: bool,
    /// Relocatable hands (S2i): the entry of the copy that runs; `fr.pc(OFF)` = base + OFF. 0 for an absolute hand.
    pub base: u32,
    /// Relocatable hands: the template's `zv_[k]` of that copy (hs_reloc.rs `Reloc::zvals`). Empty for an absolute hand.
    zv: &'a [u64],
}

/// S7: the prologue of skinned_vertex_stream (Kerwan 0x1e80c0, identical in all level forms: `lui at,0x7000; sq ra,0x1ff0(at);
/// lw at,0x14(a0); lui a2,0x7000`). This hand-written function keeps $ra in the scratchpad and uses $sp as a plain data register
/// (`ld sp,0(s0)`, `addi sp,sp,2`, `sb sp,3(a1)` ...); its caller (0x244018..: saves every register in a global block and never reads
/// $sp again) does not care, so the call returns with a junk $sp and a frame that also compares $sp never sees it end.
const SP_SCRATCH_SIG: [u32; 4] = [0x3c01_7000, 0x7c3f_1ff0, 0x8c81_0014, 0x3c06_7000];
/// True when the code at `pc` starts with one of the `SP_SCRATCH_SIG` prologues.
#[inline(always)]
pub fn sp_scratch(c: &Cpu, pc: u32) -> bool {
    let p = (pc & 0x01ff_fffc) as usize;
    p + 16 <= c.mem.ram.len() && (0..4).all(|k| u32::from_le_bytes(c.mem.ram[p + 4 * k..p + 4 * k + 4].try_into().unwrap()) == SP_SCRATCH_SIG[k])
}

impl<'a> Frame<'a> {
    #[inline(always)]
    pub fn enter(c: &Cpu) -> Frame<'static> {
        Frame { ra: c.g(31) as u32, sp: c.g(29) as u32, sp_free: sp_scratch(c, c.pc), base: 0, zv: &[] }
    }
    /// A relocatable hand's frame: `base` = entry of this copy (`c.pc`), `zv` = `Reloc::zvals(&c.mem, base)`.
    #[inline(always)]
    pub fn enter_rel(c: &Cpu, base: u32, zv: &'a [u64]) -> Frame<'a> {
        Frame { ra: c.g(31) as u32, sp: c.g(29) as u32, sp_free: sp_scratch(c, base), base, zv }
    }
    /// A pc as an offset from the function entry (relocatable form of a literal pc).
    #[inline(always)]
    pub fn pc(&self, off: u32) -> u32 {
        self.base.wrapping_add(off)
    }
    /// The template's `zv_[k]` for this copy (relocatable form of a literal immediate that differs between levels).
    #[inline(always)]
    pub fn zv(&self, k: usize) -> u64 {
        self.zv[k]
    }
    #[inline(always)]
    pub fn done(&self, c: &Cpu) -> bool {
        c.pc == self.ra && c.npc == self.ra.wrapping_add(4) && (self.sp_free || c.g(29) as u32 == self.sp)
    }
}

/// Block marker: the block at table index `idx` runs now; its exact instruction count (delay slot included, from the
/// generated per-function table) is added to the instruction count. Returns the count at the block's start `t`
/// (instruction k of the block runs as number `t + k + 1`; a deopt before instruction k passes `t + k`).
#[inline(always)]
pub fn blk(c: &mut Cpu, tbl: &[u16], idx: usize) -> u64 {
    let t = c.icount;
    c.icount = t + tbl[idx] as u64;
    t
}

/// An interrupt or DMA stop may fall within the next `n` instructions (the recompiled code's `!c.poll(n)`, without
/// servicing anything: the caller deopts and the interpreter takes the event at the exact instruction). `n` is the block's
/// count, or the longest path to the next check. Call it before `blk`.
#[inline(always)]
pub fn due_within(c: &mut Cpu, n: u64) -> bool {
    settle(c);
    c.icount + n > c.mem.limit
}

/// S9 (2026-10-22): a stale interrupt mark. `mark_dirty` (a DMA kick run by the interpreter, eret, the kernel's interrupt return, the
/// vblank field) drops the poll limit to 0 until the interpreter's next step re-tests the lines (`Cpu::try_irq`); with both lines low
/// that re-test only clears the Cause IP bits and the mark. A hand that met such a mark deopted at its next due check (a whole region
/// as retail: skin_matrix_build r2/r8 on every visit). `settle` is that re-test's low-lines case (the same as `Cpu::hw` after an
/// access); with a line high the mark stays and the due check deopts as before (retail takes the interrupt).
#[inline(always)]
pub fn settle(c: &mut Cpu) {
    if c.mem.irq_dirty {
        let (i0, i1) = c.mem.irq_lines();
        if !i0 && !i1 {
            c.cp0[13] &= !0xc00;
            c.mem.clear_dirty();
        }
    }
}

/// Normal end of a hand: the call returns to the caller (live-outs and `c.icount` already written).
#[inline(always)]
pub fn ret(c: &mut Cpu, fr: &Frame) {
    c.pc = fr.ra;
    c.npc = fr.ra.wrapping_add(4);
    c.mem.now = c.icount;
}

/// The rest of the call as retail from `pc` (instruction count `ic` at that instruction). Always returns true (the hand
/// returns it): after it the call is over (or the host unwinds to the vblank wait, PC left there, like compiled code).
pub fn deopt(c: &mut Cpu, fr: &Frame, pc: u32, ic: u64) -> bool {
    DEOPTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    NDEOPT.with(|n| n.set(n.get() + 1));
    LAST_DEOPT.with(|d| d.set((pc, ic)));
    if stats_on() {
        run_from(c, fr, pc, ic, tracing());
        let n = c.icount.wrapping_sub(ic);
        let mut h = STATS.lock().unwrap_or_else(|e| e.into_inner());
        let e = h.get_or_insert_with(Default::default).entry(pc).or_insert((0, 0));
        e.0 += 1;
        e.1 += n;
        return true;
    }
    run_from(c, fr, pc, ic, tracing());
    true
}

/// RAC_HS_STATS=1 (lane hs diagnostics): per deopt pc (deopts, retail instructions after them). 0 = not read yet, 1 off, 2 on.
static STATS_ON: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
static STATS: std::sync::Mutex<Option<std::collections::HashMap<u32, (u64, u64)>>> = std::sync::Mutex::new(None);

#[inline]
fn stats_on() -> bool {
    match STATS_ON.load(std::sync::atomic::Ordering::Relaxed) {
        0 => {
            let on = std::env::var("RAC_HS_STATS").map(|v| v == "1").unwrap_or(false);
            STATS_ON.store(if on { 2 } else { 1 }, std::sync::atomic::Ordering::Relaxed);
            on
        }
        s => s == 2,
    }
}

/// One report line (plus the top 40 deopt pcs when RAC_HS_STATS=1): deopts, resumes, retail instructions run inside hands.
pub fn stats_line() -> String {
    use std::sync::atomic::Ordering::Relaxed;
    let mut s = format!("hs: deopts {} resumes {} retail_insns {}", DEOPTS.load(Relaxed), RESUMES.load(Relaxed), RETAIL_INSNS.load(Relaxed));
    if stats_on() {
        s += &format!("\nhs: interpreted {} over_budget {} (budget {})", IB_INSNS.load(Relaxed), IB_OVER.load(Relaxed), ib_budget());
        let h = STATS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(h) = h.as_ref() {
            let mut v: Vec<_> = h.iter().map(|(pc, (n, r))| (*r, *n, *pc)).collect();
            v.sort_by(|a, b| b.cmp(a));
            for (r, n, pc) in v.iter().take(40) {
                s += &format!("\nhs: deopt pc {:08x} count {} retail_insns {}", pc, n, r);
            }
        }
    }
    s
}

/// From a hand: a retail call to `target` that returns to `ret` (r31 and the arguments already set, `c.icount` the count after
/// the call's delay slot). False: the callee did not come back (vblank unwind, tail jump handled by `Cpu::tail`): the hand
/// returns true at once.
pub fn call(c: &mut Cpu, target: u32, ret: u32) -> bool {
    c.mem.now = c.icount;
    c.call_dyn(target, ret);
    if c.pc != ret {
        c.tail();
        return false;
    }
    true
}

/// `deopt` with a choice: `pure` steps only the interpreter (callees too; the gate and the capture use it so that
/// every access is seen).
pub fn run_from(c: &mut Cpu, fr: &Frame, pc: u32, ic: u64, pure: bool) {
    c.pc = pc;
    c.npc = pc.wrapping_add(4);
    c.icount = ic;
    c.mem.now = ic;
    let mut guard = 0u64;
    let mut first = true;
    let ib0 = if pure { 0 } else { ib_budget() };
    let mut ib = ib0;
    while !fr.done(c) {
        if c.stop.is_some() || (c.pc == crate::VBLANK_WAIT && c.vblank_cb.is_none() && !pure) {
            RETAIL_INSNS.fetch_add(c.icount.wrapping_sub(ic), std::sync::atomic::Ordering::Relaxed);
            return;
        }
        // S2f resume: a stitched hand (or the region gate) registered its region entry pcs: retail runs only until the next one
        // (after at least one instruction, outside a delay slot); `deopt` then returns with the call NOT over and the hand's driver goes on in Rust.
        if !first && c.npc == c.pc.wrapping_add(4) && resume_at(fr, c.pc) {
            RESUMES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            RETAIL_INSNS.fetch_add(c.icount.wrapping_sub(ic), std::sync::atomic::Ordering::Relaxed);
            return;
        }
        first = false;
        if pure {
            pre_step(c);
            c.step();
            if c.rec_flush {
                c.rec_flushed();
            }
            guard += 1;
            if guard > 19_000_000 {
                HIST.with(|h| *h.borrow_mut().entry(c.pc).or_insert(0) += 1);
            }
            if guard > 20_000_000 {
                // S7: where it spins (a wait that needs an event the event-free run never delivers)
                HIST.with(|h| {
                    let mut v: Vec<_> = h.borrow().iter().map(|(k, n)| (*n, *k)).collect();
                    v.sort_by(|a, b| b.cmp(a));
                    eprintln!("hs: run_from guard, pc histogram of the last 1M steps (count, pc): {:x?}", &v[..v.len().min(24)]);
                    h.borrow_mut().clear();
                });
                c.stop = Some("hs: run_from guard".into());
                return;
            }
        } else if c.pc == pc && c.icount == ic && c.rec.as_ref().is_some_and(|t| t.has_hand(pc)) {
            // A deopt at a hand's own entry (its entry due check): the compiled function would call the hand again before its
            // entry poll (`Cpu::hand`), and the hand would deopt here again, forever (2026-10-22 Gaspar stack overflow). The
            // interpreter runs this instruction (and takes the due event exactly as retail).
            c.step();
            if c.rec_flush {
                c.rec_flushed();
            }
        } else if ib > 0 && in_hand(fr, c.pc) {
            // Inside the deopting hand's own function: the interpreter (exact as compiled code) one instruction at a time, so
            // the resume check above sees the next region entry. `step_rec` would run the compiled function from a block leader
            // to the END of the call (it loops internally) and the hand would be lost for the rest of it. Callees run compiled;
            // past the budget the old way (a long stub region costs less compiled than interpreted).
            ib -= 1;
            c.step();
            if c.rec_flush {
                c.rec_flushed();
            }
        } else {
            c.step_rec();
        }
    }
    RETAIL_INSNS.fetch_add(c.icount.wrapping_sub(ic), std::sync::atomic::Ordering::Relaxed);
    if stats_on() {
        IB_INSNS.fetch_add((ib0 - ib) as u64, std::sync::atomic::Ordering::Relaxed);
        IB_OVER.fetch_add((ib == 0 && ib0 > 0) as u64, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Interpreter budget per deopt inside the hand's own function (RAC_HS_IB overrides, lane A/B; 0 = the old `step_rec` way).
fn ib_budget() -> u32 {
    static B: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *B.get_or_init(|| std::env::var("RAC_HS_IB").ok().and_then(|v| v.parse().ok()).unwrap_or(2048))
}

/// True when `pc` is inside the function of the registered (resumable) call `fr`.
fn in_hand(fr: &Frame, pc: u32) -> bool {
    RESUME.with(|r| {
        let r = r.borrow();
        r.len != 0 && r.ra == fr.ra && r.sp == fr.sp && pc.wrapping_sub(r.base) < r.len
    })
}

thread_local! {
    static HIST: std::cell::RefCell<std::collections::HashMap<u32, u64>> = std::cell::RefCell::new(std::collections::HashMap::new());
    static RESUME: std::cell::RefCell<ResumeSave> = const { std::cell::RefCell::new(ResumeSave { ra: 0, sp: 0, base: 0, len: 0, abs: Vec::new(), rel: &[] }) };
}
/// The resume registration of one call (S2f; S4: a relocatable hand registers offsets from the call's base, no allocation per call).
pub struct ResumeSave {
    ra: u32,
    sp: u32,
    base: u32,
    /// The function's length (relocatable hands; 0 = unknown: no interpreted run after a deopt).
    len: u32,
    abs: Vec<u32>,
    rel: &'static [u32],
}
/// S2f: instructions run as retail inside `run_from` (deopt to resume / to the end of the call) and the number of resumes (all calls).
pub static RETAIL_INSNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static RESUMES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// RAC_HS_STATS: instructions interpreted inside hands after deopts, and deopts that ran out of the interpreter budget.
static IB_INSNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static IB_OVER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Registers the pcs at which a deopt of THIS call (frame `fr`) hands the call back to the Rust driver (empty = run to the end of the call,
/// the old behaviour; a deopt of any other call, e.g. a hand running as a callee, is not affected); returns the previous registration
/// (give it to `resume_restore` when the hand is done).
pub fn resume_set(fr: &Frame, pcs: &[u32]) -> ResumeSave {
    RESUME.with(|r| std::mem::replace(&mut *r.borrow_mut(), ResumeSave { ra: fr.ra, sp: fr.sp, base: 0, len: 0, abs: pcs.to_vec(), rel: &[] }))
}
/// Relocatable hands (S4): the resume points are `fr.base + off` for the (static) offsets.
pub fn resume_set_rel(fr: &Frame, offs: &'static [u32]) -> ResumeSave {
    resume_set_rel_len(fr, offs, 0)
}
/// `resume_set_rel` plus the function's length `len`: a deopt interprets inside it (budgeted) until the next resume point.
pub fn resume_set_rel_len(fr: &Frame, offs: &'static [u32], len: u32) -> ResumeSave {
    RESUME.with(|r| std::mem::replace(&mut *r.borrow_mut(), ResumeSave { ra: fr.ra, sp: fr.sp, base: fr.base, len, abs: Vec::new(), rel: offs }))
}
pub fn resume_restore(prev: ResumeSave) {
    RESUME.with(|r| *r.borrow_mut() = prev);
}
fn resume_at(fr: &Frame, pc: u32) -> bool {
    RESUME.with(|r| {
        let r = r.borrow();
        r.ra == fr.ra && r.sp == fr.sp && (r.abs.contains(&pc) || r.rel.contains(&pc.wrapping_sub(r.base)))
    })
}

// ---------------- tracing (gate and capture only) ----------------

/// What a traced run saw.
#[derive(Default)]
pub struct Trace {
    /// RAM pages (4 KB) fetched from, loaded from or stored to (conservative: +-16 bytes around an access).
    pub pages: std::collections::BTreeSet<u32>,
    /// Loads/stores outside RAM and scratchpad: (address, is_store, value for stores, icount before it).
    pub mmio: Vec<(u32, u8, u64, u64)>,
}

thread_local! {
    /// S2g: (pc, ic) of the most recent `deopt` (the region gate reads it to see how far the Rust ran before it gave up).
    static LAST_DEOPT: std::cell::Cell<(u32, u64)> = const { std::cell::Cell::new((0, 0)) };
    /// S2g: how often `stub!(..)` deopted per block index (gate: a block that never ran to its end in Rust must be a declared stub).
    static STUBS: std::cell::RefCell<Vec<u64>> = const { std::cell::RefCell::new(Vec::new()) };
    static TRACE: std::cell::RefCell<Option<Trace>> = const { std::cell::RefCell::new(None) };
    static ON: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// S6: deopts on this thread (the parallel region gate tells its threads' deopts apart with it).
    static NDEOPT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// S6: how many `deopt`s ran on this thread so far.
pub fn deopts_here() -> u64 {
    NDEOPT.with(|n| n.get())
}

#[inline]
fn tracing() -> bool {
    ON.with(|o| o.get())
}

pub fn last_deopt() -> (u32, u64) {
    LAST_DEOPT.with(|d| d.get())
}

/// `stub!` of a generated gen.rs: block `n` of the region is a declared deopt stub.
pub fn stub_hit(n: usize) {
    STUBS.with(|s| {
        let mut s = s.borrow_mut();
        if s.len() <= n {
            s.resize(n + 1, 0);
        }
        s[n] += 1;
    });
}

pub fn stub_hits() -> Vec<u64> {
    STUBS.with(|s| s.borrow().clone())
}

/// (number of hardware accesses seen so far, hash of them): the region gate compares it at every region exit.
pub fn trace_mmio_sig() -> (usize, u64) {
    TRACE.with(|t| {
        let t = t.borrow();
        let Some(t) = t.as_ref() else { return (0, 0) };
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for m in &t.mmio {
            for x in [m.0 as u64, m.1 as u64, m.2, m.3] {
                h = (h ^ x).wrapping_mul(0x0000_0100_0000_01b3).rotate_left(23);
            }
        }
        (t.mmio.len(), h)
    })
}

pub fn trace_begin() {
    TRACE.with(|t| *t.borrow_mut() = Some(Trace::default()));
    ON.with(|o| o.set(true));
}

pub fn trace_end() -> Trace {
    ON.with(|o| o.set(false));
    TRACE.with(|t| t.borrow_mut().take()).unwrap_or_default()
}

/// RAM offset of a (possibly mirrored) address, None for scratchpad and hardware.
fn ram_off(a: u32) -> Option<u32> {
    let p = if (0x8000_0000..0xc000_0000).contains(&a) { a & 0x1fff_ffff } else { a };
    match p {
        0..=0x01ff_ffff => Some(p),
        0x2000_0000..=0x21ff_ffff | 0x3000_0000..=0x31ff_ffff => Some(p & 0x01ff_ffff),
        _ => None,
    }
}

/// Called before every interpreted instruction while tracing: pages touched and hardware accesses.
fn pre_step(c: &Cpu) {
    if !tracing() {
        return;
    }
    let pc = c.pc;
    let Some(po) = ram_off(pc) else { return };
    let op = u32::from_le_bytes(c.mem.ram[(po & !3) as usize..][..4].try_into().unwrap());
    pre_op(c, op, po, c.icount);
}

/// S2g: a hand (`hs_fast::mmio`) runs ONE hardware access inline instead of the interpreter: record it in the trace exactly as `pre_step` would have
/// (`ic_before` = the instruction count before the instruction; the registers in `c` must be current).
pub fn trace_hw(c: &Cpu, pc: u32, ic_before: u64) {
    if !tracing() {
        return;
    }
    let Some(po) = ram_off(pc) else { return };
    let op = u32::from_le_bytes(c.mem.ram[(po & !3) as usize..][..4].try_into().unwrap());
    pre_op(c, op, po, ic_before);
}

fn pre_op(c: &Cpu, op: u32, po: u32, ic: u64) {
    TRACE.with(|t| {
        let mut t = t.borrow_mut();
        let Some(t) = t.as_mut() else { return };
        t.pages.insert(po >> 12);
        let opc = op >> 26;
        let is_ld = matches!(opc, 0x1a | 0x1b | 0x1e | 0x20..=0x27 | 0x30 | 0x31 | 0x36 | 0x37);
        let is_st = matches!(opc, 0x1f | 0x28..=0x2e | 0x38 | 0x39 | 0x3e | 0x3f);
        if !(is_ld || is_st) {
            return;
        }
        let rs = (op >> 21 & 31) as usize;
        let rt = (op >> 16 & 31) as usize;
        let ea = (c.g(rs) as u32).wrapping_add(op as i16 as i32 as u32);
        match ram_off(ea) {
            Some(o) => {
                for d in [o.wrapping_sub(16), o, o + 31] {
                    if d < 0x0200_0000 {
                        t.pages.insert(d >> 12);
                    }
                }
            }
            None => {
                if ea & 0xf000_0000 == 0x7000_0000 {
                    return; // scratchpad: part of the case
                }
                let v = match opc {
                    0x39 => c.f[rt] as u64,
                    0x3e => c.vf[rt][0] as u64 | (c.vf[rt][1] as u64) << 32,
                    _ => c.g(rt),
                };
                if is_st && matches!(opc, 0x2b | 0x3f) && v & 0x100 != 0 {
                    dma_pages(c, ea, v as u32, &mut t.pages);
                }
                t.mmio.push((ea, is_st as u8, if is_st { v } else { 0 }, ic));
            }
        }
    });
}

/// One interpreter step with the gate's tracing (used by `hs_region`'s partial-hand driver).
pub fn pure_step(c: &mut Cpu) {
    pre_step(c);
    c.step();
    if c.rec_flush {
        c.rec_flushed();
    }
}

/// A store of a DMA channel CHCR with the start bit: add the RAM pages the transfer reads or writes (normal mode: MADR/QWC; chain mode: the
/// tag walk from TADR) so that the corpus replays DMA that moves RAM <-> scratchpad / VIF / GIF (lane hs S2d; was missing for scratchpad DMA).
fn dma_pages(c: &Cpu, ea: u32, chcr: u32, pages: &mut std::collections::BTreeSet<u32>) {
    let base = ea & !0xff;
    if !matches!(base & 0xffff_ff00, 0x1000_8000 | 0x1000_9000 | 0x1000_a000 | 0x1000_b000 | 0x1000_b400 | 0x1000_c000 | 0x1000_c400 | 0x1000_c800 | 0x1000_d000 | 0x1000_d400) || ea & 0xff != 0 {
        return;
    }
    let hw = |o: u32| c.mem.hw32(base + o);
    fn add(pages: &mut std::collections::BTreeSet<u32>, a: u32, n: u32) {
        // n bytes from RAM address a (+-16), pages as the access tracer does
        let a = a & 0x01ff_fff0;
        for d in [a.wrapping_sub(16) & 0x01ff_ffff, a, (a + n.max(16) + 15) & 0x01ff_ffff] {
            pages.insert(d >> 12);
        }
        let mut x = a & !0xfff;
        while x < (a + n).min(0x0200_0000) {
            pages.insert(x >> 12);
            x += 0x1000;
        }
    }
    let qwc = |q: u32| if q == 0 { 0x10000 } else { q };
    match (chcr >> 2) & 3 {
        0 => {
            let m = hw(0x10);
            if m & 0x8000_0000 == 0 {
                add(pages, m, qwc(hw(0x20) & 0xffff) * 16);
            }
        }
        1 => {
            let (mut tag, mut stack) = (hw(0x30), vec![]);
            for _ in 0..16384 {
                if tag & 0x8000_0000 != 0 {
                    break; // tags in the scratchpad: part of the case
                }
                let ta = tag & 0x01ff_fff0;
                add(pages, ta, 16);
                let w = u64::from_le_bytes(c.mem.ram[ta as usize..ta as usize + 8].try_into().unwrap());
                let (q, id, adr) = ((w & 0xffff) as u32, (w >> 28 & 7) as u32, (w >> 32) as u32);
                let data = |a: u32, pages: &mut std::collections::BTreeSet<u32>| {
                    if a & 0x8000_0000 == 0 && q > 0 {
                        for x in (a & 0x01ff_f000..(a & 0x01ff_fff0) + q * 16).step_by(0x1000) {
                            pages.insert(x >> 12);
                        }
                        pages.insert(((a & 0x01ff_fff0) + q * 16 + 15) >> 12);
                    }
                };
                match id {
                    0 => {
                        data(adr, pages);
                        break;
                    }
                    1 => {
                        data(ta + 16, pages);
                        tag = ta + 16 + q * 16;
                    }
                    2 => {
                        data(ta + 16, pages);
                        tag = adr;
                    }
                    3 | 4 => {
                        data(adr, pages);
                        tag = ta + 16;
                    }
                    5 => {
                        data(ta + 16, pages);
                        stack.push(ta + 16 + q * 16);
                        tag = adr;
                    }
                    6 => {
                        data(ta + 16, pages);
                        match stack.pop() {
                            Some(n) => tag = n,
                            None => break,
                        }
                    }
                    _ => {
                        data(ta + 16, pages);
                        break;
                    }
                }
            }
        }
        _ => {}
    }
}
