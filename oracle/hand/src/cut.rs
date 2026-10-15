//! Region fallback for partial hands (see scratch_cut/CUTTER.md). A partial hand is a table of regions of one retail
//! function, in address order; each region is either retail code (run here by the interpreter until the PC leaves the
//! region) or a Rust function. `regions` is the dispatcher: it runs whichever region holds the PC until the function
//! returns.
//!
//! The function has returned when the PC is the return address it was entered with and `sp` is the stack pointer it
//! was entered with (the same test fndiff and `Cpu::call_dyn` use; both are per invocation, kept on the Rust stack, so
//! a recursive function - one whose own code calls its entry, through `call_dyn` and the hand - has one dispatcher
//! per level). Anything else that leaves the table (a tail `j`, a branch past the profiled end, a Rust region that
//! returns an exit outside the function) is a transfer to code that is not in the table: the dispatcher runs it
//! (interpreted or compiled, hands included) until the same return condition and then returns.
use ee_oracle::cpu::Cpu;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Exit "pc" a Rust region returns when it executed the function's own `jr ra` (the dispatcher stops; `run_hand`
/// sets the PC to the caller's return address as for any hand).
pub const RETURN: u32 = u32::MAX;
/// The vblank wait (the host raises the vblank there).
pub const VBLANK_WAIT: u32 = 0x118cb0;

/// One region [lo, hi) of the function. `rust`: the Rust replacement `fn(c, entry_pc) -> exit_pc` (entry_pc is the
/// address the region was entered at: `lo`, or another entry listed by cutter.py); it returns the pc execution goes on
/// at (the next region's entry, a pc outside the function for a tail exit, or `RETURN`). `est`: the average number of
/// instructions the retail region executes (own, callees excluded), added to the instruction count after a Rust
/// region so interrupt/DMA timing stays near retail's (retail regions count their own).
///
/// `entries`: the pcs OTHER than `lo` where the contract (cutter.py `entries`, MULTI-ENTRY) lets execution enter the region
/// (`&[]` for an ordinary region). A Rust region may only be entered at `lo` or at one of these: any other pc is a hard error
/// (`c.stop` "region-entry", counted in the statistics, reported by the checker as `entry-illegal`), because the Rust code
/// and its contract know nothing about a mid-region entry. A retail region is entered anywhere (the interpreter runs it).
pub struct Reg {
    pub lo: u32,
    pub hi: u32,
    pub rust: Option<fn(&mut Cpu, u32) -> u32>,
    pub est: u64,
    pub entries: &'static [u32],
}

/// One invocation of a partial hand: what the function was entered with. All per-call state is here (no globals).
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub entry: u32,
    pub end: u32,
    /// Return address and stack pointer at entry: the function is done when the PC is `ra` and `sp` is `sp` again.
    pub ra: u32,
    pub sp: u32,
}

impl Frame {
    /// The function's own return has executed (the PC is not in a branch delay slot).
    #[inline]
    pub fn done(&self, c: &Cpu) -> bool {
        c.pc == self.ra && c.npc == self.ra.wrapping_add(4) && c.g(29) as u32 == self.sp
    }
}

/// A Rust region about to run (what a region hook sees).
pub struct RegionCall<'a> {
    pub fr: Frame,
    /// Region index in the table.
    pub k: usize,
    pub reg: &'a Reg,
    /// The address the region is entered at.
    pub pc: u32,
}

/// fndiff's region check: runs the region (retail and Rust, compares) and returns the exit pc, leaving the CPU in the
/// retail run's post-state. While it runs, the hook is off (nested dispatchers run plain).
pub type RegionHook = fn(&mut Cpu, &RegionCall) -> u32;

thread_local! {
    static HOOK: Cell<Option<RegionHook>> = const { Cell::new(None) };
}

/// Installs (or removes) the region hook of this thread (checker only).
pub fn set_region_hook(h: Option<RegionHook>) {
    HOOK.set(h);
}

/// Counters of one region of one partial hand (all invocations, all threads).
#[derive(Default)]
struct Cnt {
    insns: AtomicU64,
    retail_runs: AtomicU64,
    rust_runs: AtomicU64,
    /// Rust region entered at a pc that is not a contract entry (hard error, M5).
    illegal: AtomicU64,
}

/// The statistics table of one partial hand, keyed by its entry: one counter set per region (any number of regions)
/// and the deepest nesting of the hand in itself (self-recursion) seen on one thread.
struct Table {
    entry: u32,
    end: u32,
    regs: Vec<(u32, u32, bool)>,
    /// The extra entry pcs of each region (`Reg::entries`).
    ents: Vec<Vec<u32>>,
    cnt: Vec<Cnt>,
    max_depth: AtomicU32,
}

static TABLES: Mutex<Vec<Arc<Table>>> = Mutex::new(Vec::new());

fn find_table(entry: u32, end: u32, regs: &[Reg]) -> Arc<Table> {
    let shape: Vec<(u32, u32, bool)> = regs.iter().map(|r| (r.lo, r.hi, r.rust.is_some())).collect();
    let ents: Vec<Vec<u32>> = regs.iter().map(|r| r.entries.to_vec()).collect();
    let mut t = TABLES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(x) = t.iter().find(|x| x.entry == entry && x.end == end && x.regs == shape && x.ents == ents) {
        return x.clone();
    }
    // A hand rebuilt with another cut replaces the old table.
    let n = Arc::new(Table { entry, end, regs: shape, ents, cnt: regs.iter().map(|_| Cnt::default()).collect(), max_depth: AtomicU32::new(0) });
    t.retain(|x| x.entry != entry);
    t.push(n.clone());
    n
}

/// Per-thread cache of the table of each hand, keyed by entry and validated by the address and length of the (static)
/// region array the hand passes, so a call costs one hash lookup and no allocation, lock or shape comparison.
struct Slot {
    ptr: usize,
    len: usize,
    t: Arc<Table>,
    depth: u32,
}

thread_local! {
    static CACHE: RefCell<HashMap<u32, Slot>> = RefCell::new(HashMap::new());
}

/// One invocation of the dispatcher as the statistics see it: counts its nesting depth on this thread.
struct Inv {
    t: Arc<Table>,
}

impl Inv {
    fn enter(entry: u32, end: u32, regs: &[Reg]) -> Inv {
        let (ptr, len) = (regs.as_ptr() as usize, regs.len());
        CACHE.with_borrow_mut(|m| {
            let slot = match m.get_mut(&entry) {
                Some(s) if s.ptr == ptr && s.len == len => s,
                _ => {
                    let depth = m.get(&entry).map_or(0, |s| s.depth);
                    m.insert(entry, Slot { ptr, len, t: find_table(entry, end, regs), depth });
                    m.get_mut(&entry).unwrap()
                }
            };
            slot.depth += 1;
            slot.t.max_depth.fetch_max(slot.depth, Ordering::Relaxed);
            Inv { t: slot.t.clone() }
        })
    }
}

impl Drop for Inv {
    fn drop(&mut self) {
        let _ = CACHE.try_with(|m| {
            if let Some(s) = m.borrow_mut().get_mut(&self.t.entry) {
                s.depth = s.depth.saturating_sub(1);
            }
        });
    }
}

/// True when `entry` is a registered partial hand that has run (checker: which hands have regions).
pub fn is_cut(entry: u32) -> bool {
    TABLES.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|t| t.entry == entry)
}

/// The region table of a partial hand that has run: (lo, hi, is_rust) per region.
pub fn table(entry: u32) -> Vec<(u32, u32, bool)> {
    TABLES.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|t| t.entry == entry).map(|t| t.regs.clone()).unwrap_or_default()
}

/// The extra entry pcs (`Reg::entries`) of each region of a partial hand that has run, in table order.
pub fn table_entries(entry: u32) -> Vec<Vec<u32>> {
    TABLES.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|t| t.entry == entry).map(|t| t.ents.clone()).unwrap_or_default()
}

/// The most invocations of the partial hand `entry` active at once on one thread (1: never nested; 0: never ran). More
/// than 1 is self-recursion, or, in rec mode, a retail callee that returns by `Cpu::tail` (a `jr` to a register other
/// than ra after clobbering ra, e.g. 0x1f24c0): tail runs the caller's continuation inside the callee's frame, so the
/// next call of the hand starts before this one has returned (the base system's behaviour with or without the hand;
/// the Machine host parks inside that nest across vblanks).
pub fn max_depth(entry: u32) -> u32 {
    TABLES.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|t| t.entry == entry).map_or(0, |t| t.max_depth.load(Ordering::Relaxed))
}

/// Statistics of one region of one partial hand.
#[derive(Debug, Clone)]
pub struct RegionStat {
    pub entry: u32,
    pub region: usize,
    pub lo: u32,
    pub hi: u32,
    pub rust: bool,
    /// Retail instructions executed in the region (own, callees excluded), retail runs, Rust runs.
    pub insns: u64,
    pub retail_runs: u64,
    pub rust_runs: u64,
    /// Entries of this (Rust) region at a pc that is not a contract entry.
    pub illegal: u64,
}

/// Statistics per (partial hand, region) that ran at least once.
pub fn stats() -> Vec<RegionStat> {
    let t = TABLES.lock().unwrap_or_else(|e| e.into_inner());
    let mut out = Vec::new();
    for tb in t.iter() {
        for (k, (cnt, r)) in tb.cnt.iter().zip(&tb.regs).enumerate() {
            let (retail_runs, rust_runs) = (cnt.retail_runs.load(Ordering::Relaxed), cnt.rust_runs.load(Ordering::Relaxed));
            let illegal = cnt.illegal.load(Ordering::Relaxed);
            if retail_runs + rust_runs + illegal > 0 {
                out.push(RegionStat { entry: tb.entry, region: k, lo: r.0, hi: r.1, rust: r.2, insns: cnt.insns.load(Ordering::Relaxed), retail_runs, rust_runs, illegal });
            }
        }
    }
    out.sort_by_key(|s| (s.entry, s.region));
    out
}

/// The dispatcher: the hand body of a partial hand. The CPU holds the call's registers (its PC may be anywhere: a
/// caller hand can run this through `h::run`); the function starts at `entry`.
pub fn regions(c: &mut Cpu, entry: u32, end: u32, regs: &[Reg]) -> bool {
    #[cfg(feature = "cutfin_probe")]
    probe(entry);
    let fr = Frame { entry, end, ra: c.g(31) as u32, sp: c.g(29) as u32 };
    let inv = Inv::enter(entry, end, regs);
    let cnt = &inv.t.cnt;
    let mut pc = entry;
    c.pc = pc;
    c.npc = pc.wrapping_add(4);
    let mut first = true;
    loop {
        if c.stop.is_some() {
            return true;
        }
        if unwinding(c) {
            return false;
        }
        // The function's own return (a retail `jr ra` and its delay slot have run, or a Rust region said RETURN).
        if pc == RETURN {
            c.pc = fr.ra;
            c.npc = fr.ra.wrapping_add(4);
            return true;
        }
        if !first && fr.done(c) {
            return true;
        }
        first = false;
        let Some(k) = regs.iter().position(|r| pc >= r.lo && pc < r.hi) else {
            if pc >= entry && pc < end {
                c.stop = Some(format!("cut: pc {pc:#x} is in no region of {entry:#x}"));
                return true;
            }
            return transfer(c, &fr);
        };
        let r = &regs[k];
        match r.rust {
            Some(_) => {
                // M5: a Rust region is entered only where its contract has an entry.
                if pc != r.lo && !r.entries.contains(&pc) {
                    cnt[k].illegal.fetch_add(1, Ordering::Relaxed);
                    c.stop = Some(format!(
                        "cut: region-entry: Rust region R{k} [{:#x}..{:#x}) of {entry:#x} entered at {pc:#x}, which is not a contract entry (entries: {:#x} {:x?})",
                        r.lo, r.hi, r.lo, r.entries
                    ));
                    return true;
                }
                let call = RegionCall { fr, k, reg: r, pc };
                pc = match HOOK.get() {
                    Some(h) => {
                        HOOK.set(None);
                        let e = h(c, &call);
                        HOOK.set(Some(h));
                        e
                    }
                    None => {
                        cnt[k].rust_runs.fetch_add(1, Ordering::Relaxed);
                        rust_region(c, &call)
                    }
                };
                if pc != RETURN {
                    c.pc = pc;
                    c.npc = pc.wrapping_add(4);
                }
            }
            None => {
                cnt[k].retail_runs.fetch_add(1, Ordering::Relaxed);
                c.pc = pc;
                c.npc = pc.wrapping_add(4);
                let n = run_retail(c, &fr, r.lo, r.hi, None);
                cnt[k].insns.fetch_add(n, Ordering::Relaxed);
                pc = c.pc;
            }
        }
    }
}

/// Runs a Rust region as the dispatcher does (the region function, then its instruction-count estimate) and returns
/// its exit pc (`RETURN` is the function's own return).
pub fn rust_region(c: &mut Cpu, call: &RegionCall) -> u32 {
    let f = call.reg.rust.expect("cut: rust_region on a retail region");
    let exit = f(c, call.pc);
    c.icount += call.reg.est;
    c.mem.now = c.icount;
    exit
}

/// Runs the retail instructions of a region as the dispatcher does (from `call.pc` until the PC leaves the region or
/// the function returns) and returns the exit pc, the CPU left there. `trace` sees the CPU before each own
/// instruction. Not counted in the statistics.
pub fn retail_region(c: &mut Cpu, call: &RegionCall, trace: Option<&mut dyn FnMut(&Cpu)>) -> u32 {
    c.pc = call.pc;
    c.npc = call.pc.wrapping_add(4);
    run_retail(c, &call.fr, call.reg.lo, call.reg.hi, trace);
    c.pc
}

/// The PC left the function's code (a tail `j`, a branch past the profiled end, a Rust region's tail exit): the code
/// there is not in the table, so run it as the rest of this function until the function's own return condition holds,
/// as `Cpu::tail` does for compiled code (compiled functions and hands included).
fn transfer(c: &mut Cpu, fr: &Frame) -> bool {
    loop {
        if c.stop.is_some() || fr.done(c) {
            return true;
        }
        if unwinding(c) {
            return false;
        }
        c.step_rec();
    }
}

/// The host has no frame callback and the PC is at the vblank wait: compiled code unwinds its Rust frames there (the host
/// loop raises the vblank and resumes at the PC). The dispatcher does the same: it returns false with the PC left at the
/// wait (`run_hand` then leaves the PC alone), the rest of the function runs as plain retail after the vblank.
#[inline]
fn unwinding(c: &Cpu) -> bool {
    c.pc == VBLANK_WAIT && c.vblank_cb.is_none()
}

fn is_link(op: u32) -> bool {
    let (o, rt, f) = (op >> 26, (op >> 16) & 31, op & 63);
    o == 3 || (o == 0 && f == 9) || (o == 1 && (0x10..=0x13).contains(&rt))
}

/// PROOFS ONLY: CUT_OWN=N selects a hidden-retail negative control of the proof modules (1: a plain `run_retail` inside the Rust row).
#[cfg(feature = "proofs")]
pub fn own_ctl() -> u32 {
    static N: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *N.get_or_init(|| std::env::var("CUT_OWN").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(0))
}

/// PROOFS ONLY (cargo feature `proofs`; a worker copy does not ship it): `run_retail` for a Rust row of a proof module that is meant to
/// stand in for a Rust region (the negative controls and the multi-entry stand-ins of CUTTER.md 11/12). A plain `run_retail` from inside a Rust
/// region is hidden retail: the region check counts the interpreted own-code instructions and fails `[own-retail]` (12.11).
#[cfg(feature = "proofs")]
pub fn run_retail_proof(c: &mut Cpu, fr: &Frame, lo: u32, hi: u32, trace: Option<&mut dyn FnMut(&Cpu)>) -> u64 {
    c.own_off += 1;
    let n = run_retail(c, fr, lo, hi, trace);
    c.own_off -= 1;
    n
}

/// Runs the retail instructions of the region [lo, hi) of the function `fr` from the current PC until the PC leaves
/// the region or the function returns (PC = its return address with its entry `sp`), and returns the number of own
/// instructions executed. The exit is `c.pc`. Calls made inside the region (jal/jalr/bal) run as retail calls
/// (compiled code or a hand when the callee has one, else the interpreter); a jump out of the function is NOT
/// followed here (the dispatcher runs it). Interrupts are taken and serviced in place. `trace` sees the CPU before
/// each own instruction.
pub fn run_retail(c: &mut Cpu, fr: &Frame, lo: u32, hi: u32, mut trace: Option<&mut dyn FnMut(&Cpu)>) -> u64 {
    let mut n = 0u64;
    let mut link: Option<u32> = None;
    loop {
        let pc = c.pc;
        if c.stop.is_some() {
            break;
        }
        let in_slot = c.npc != pc.wrapping_add(4);
        if !in_slot && (fr.done(c) || pc < lo || pc >= hi) {
            break;
        }
        let op = c.mem.fetch(pc);
        let own = pc >= lo && pc < hi;
        if !in_slot && is_link(op) {
            link = Some(pc.wrapping_add(8));
        }
        if own {
            if let Some(t) = trace.as_mut() {
                t(c);
            }
            n += 1;
        }
        let irq0 = c.irqs[0] + c.irqs[1];
        c.step();
        if c.rec_flush {
            c.rec_flushed();
        }
        if c.irqs[0] + c.irqs[1] != irq0 {
            // An interrupt was taken (the instruction did not run): service it to ERET, then go on.
            n -= own as u64;
            let epc = c.cp0[14];
            while !(c.pc == epc && c.cp0[12] & 2 == 0) {
                if c.stop.is_some() || unwinding(c) {
                    break;
                }
                c.step_rec();
            }
            link = None;
            continue;
        }
        if link.is_some() && !in_slot {
            // The link instruction itself ran. A branch-and-link that was not taken has no delay slot pending.
            if c.npc == c.pc.wrapping_add(4) {
                link = None;
            }
            continue;
        }
        if let (Some(ret), true) = (link, c.npc == c.pc.wrapping_add(4)) {
            // The delay slot ran: the call. Its callee runs (hands, compiled code) and returns here.
            link = None;
            if c.pc != ret {
                c.call_dyn(c.pc, ret);
                if c.pc != ret && c.stop.is_none() {
                    if unwinding(c) {
                        break; // the callee unwound to the vblank wait: leave the PC there (the dispatcher propagates)
                    }
                    c.stop = Some(format!("cut: callee did not return to {ret:#x} (pc {:#x})", c.pc));
                }
            }
        }
    }
    n
}

/// cutfinish.py ramdiff probe (cargo feature `cutfin_probe`, off in every other build): says once per thread that the hand of ENTRY was reached.
#[cfg(feature = "cutfin_probe")]
fn probe(entry: u32) {
    thread_local! {
        static SEEN: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    }
    SEEN.with(|s| {
        let mut s = s.borrow_mut();
        if !s.contains(&entry) {
            s.push(entry);
            eprintln!("cutfin: hand {entry:#x} reached");
        }
    });
}
