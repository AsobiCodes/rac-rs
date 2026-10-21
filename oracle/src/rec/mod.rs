//! Recompiled retail functions (recgen.py output) and their dispatcher.
//!
//! Each generated function runs its basic blocks straight from `Cpu` state (no Rust locals live across blocks), so
//! the dispatcher can enter at any block label. Calls between compiled functions are Rust calls; anything else runs
//! nested (`call_dyn`). A block runs inline when no interrupt or DMA stop is due inside it; otherwise it is
//! interpreted (`slow`), with interrupts serviced in place. Instruction counts match the interpreter's, so a run is
//! the same run either way. A function that finds the PC outside its own labels (the vblank wait, a handler that did
//! not come back) returns, and so do its callers, down to the dispatcher.
use crate::cpu::Cpu;
use crate::mem::Mem;

pub use ee_core::rec::Fn;

/// The generated sets (crates under oracle/rec, registered by ee-game), each with the level whose overlay it
/// compiles (`None`: boot code or unknown, always probed). Each is used only when all its functions match RAM,
/// because its functions call each other directly.
static SETS: std::sync::Mutex<Vec<(Option<u32>, &'static [Fn])>> = std::sync::Mutex::new(Vec::new());

/// Adds a generated set that is not tied to a level (once per set).
pub fn register(set: &'static [Fn]) {
    register_at(None, set);
}

/// Adds a generated set of level `level` (the level byte, 0x15ed84; once per set). `Table::build` probes the entries
/// of such a set (for the table to notice it matching) only while that level is the current one, or while the set is
/// in use.
pub fn register_level(level: u32, set: &'static [Fn]) {
    register_at(Some(level), set);
}

fn register_at(tag: Option<u32>, set: &'static [Fn]) {
    let mut s = SETS.lock().unwrap();
    if !s.iter().any(|x| std::ptr::eq(x.1, set)) {
        s.push((tag, set));
    }
}

/// A Rust function that replaces a retail one (a PC hook registered by rac-rs; the Eudora rewrites, crate ee-hand, are gone). It runs when the retail code at
/// `entry..end` matches `hash` and the function is entered at `entry` (from compiled code or the dispatcher), with
/// the arguments in the registers as retail passes them. It returns false to leave the call to the retail code
/// (state untouched). After it, the PC is the caller's return address and the instruction count advances by `est`
/// (the retail function's average own instructions, so interrupt and DMA timing stay close).
pub struct Hand {
    pub entry: u32,
    pub end: u32,
    pub hash: u64,
    pub est: u64,
    pub f: fn(&mut Cpu) -> bool,
}

static HANDS: std::sync::Mutex<Vec<&'static [Hand]>> = std::sync::Mutex::new(Vec::new());

/// Adds a set of hand-written functions (once per set). ORACLE_HAND=0 leaves them out of the table.
pub fn register_hand(set: &'static [Hand]) {
    let mut s = HANDS.lock().unwrap();
    if !s.iter().any(|x| std::ptr::eq(*x, set)) {
        s.push(set);
    }
}

/// The registered hand-written functions (tools).
pub fn hands() -> Vec<&'static Hand> {
    HANDS.lock().unwrap().iter().flat_map(|s| s.iter()).collect()
}

/// The registered sets (tools: profiles, the hand-written functions' checks).
pub fn sets() -> Vec<&'static [Fn]> {
    SETS.lock().unwrap().iter().map(|&(_, s)| s).collect()
}

/// Return address of `Cpu::call` (kernel area, never executed: the call ends when the PC gets there).
const CALL_RET: u32 = 0x10;

/// Level loader state (Eudora 0x236308, boot-area halfword).
pub const LOADER: u32 = 0x15ee48;

/// PC -> recompiled function, for the functions whose code matches RAM.
/// `#[repr(C)]`: `lo` and `hand_bits` first, ee-core's `rec::RecHot` (the `has_hand` test the generated code inlines).
#[repr(C)]
pub struct Table {
    lo: u32,
    /// Entries with a hand-written function (bit per word from `lo`), and the functions.
    hand_bits: Vec<u64>,
    map: Vec<u32>,
    fns: Vec<fn(&mut ee_core::cpu::Cpu)>,
    pub level: u32,
    /// Level loader state (0x15ee48) at build time: states 1..=6 replace the overlay, so no set is used then.
    pub loader: u16,
    /// The sets used, by registration index.
    sets: Vec<usize>,
    /// RAM words at every registered function's entry when built (address, word): a FlushCache that finds one
    /// changed rebuilds the table (`Cpu::rec_flushed`).
    probe: Vec<(u32, u32)>,
    hand: std::collections::HashMap<u32, (fn(&mut Cpu) -> bool, u64)>,
    /// (entry, end) of every function in the sets used, by entry (`gap_frame`).
    spans: Vec<(u32, u32)>,
}

impl Table {
    pub fn build(mem: &Mem, level: u32) -> Table {
        // A rebuild follows every overlay load (FlushCache): the host learns which overlay is in RAM (`ver::on_rebuild`).
        crate::ver::rebuilt(&mem.ram);
        let (lo, hi) = (0x10_0000u32, 0x40_0000u32);
        let loader = u16::from_le_bytes([mem.ram[LOADER as usize], mem.ram[LOADER as usize + 1]]);
        let mut t = Table {
            lo,
            map: vec![0; ((hi - lo) / 4) as usize],
            fns: Vec::new(),
            level,
            loader,
            sets: Vec::new(),
            probe: Vec::new(),
            hand_bits: vec![0; ((hi - lo) / 4 / 64) as usize],
            hand: std::collections::HashMap::new(),
            spans: Vec::new(),
        };
        if (1..=6).contains(&loader) {
            return t;
        }
        // A set is used only when all its functions match RAM (they call each other directly).
        for (i, &(tag, set)) in SETS.lock().unwrap().iter().enumerate() {
            let used = !set.iter().any(|f| f.end > hi || f.entry < lo || hash(mem, f.entry, f.end) != f.hash);
            // Probe the entries of the sets in use, of the sets not tied to a level and of the current level's sets
            // (not matching yet). A set of another level that does not match is hashed but not probed: its
            // overlay words are live data of the loaded level and change every field.
            if used || tag.map_or(true, |l| l == level) {
                for f in set.iter().filter(|f| f.entry >= lo && f.end <= hi) {
                    for a in [f.entry, f.entry + 4] {
                        t.probe.push((a, rd(mem, a)));
                    }
                }
            }
            if !used {
                continue;
            }
            t.sets.push(i);
            for f in set.iter() {
                t.fns.push(f.f);
                let k = t.fns.len() as u32;
                for &l in f.labels {
                    t.map[((l - lo) / 4) as usize] = k;
                }
                t.spans.push((f.entry, f.end));
            }
        }
        t.spans.sort_unstable();
        if !std::env::var("ORACLE_HAND").is_ok_and(|v| v == "0") {
            // ORACLE_EST_SCALE=P (percent, default 100): a hand advances the instruction count by est*P/100 (timing
            // probe; unset leaves est as is).
            let scale = std::env::var("ORACLE_EST_SCALE").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(100);
            for h in hands() {
                if h.entry >= lo && h.end <= hi && hash(mem, h.entry, h.end) == h.hash {
                    let i = ((h.entry - lo) / 4) as usize;
                    t.hand_bits[i >> 6] |= 1 << (i & 63);
                    t.hand.insert(h.entry, (h.f, if scale == 100 { h.est } else { h.est * scale / 100 }));
                    // A hand in use also probes its entry words: changed code under a hand rebuilds the table.
                    for a in [h.entry, h.entry + 4] {
                        if a < h.end {
                            t.probe.push((a, rd(mem, a)));
                        }
                    }
                }
            }
        }
        // `stale` tests whether any probe differs: a sorted, deduplicated list gives the same answer with fewer
        // reads, in address order (sets of different levels share overlay addresses, so pairs repeat).
        t.probe.sort_unstable();
        t.probe.dedup();
        t
    }

    /// A hand-written function replaces the one at `entry`.
    #[inline(always)]
    pub fn has_hand(&self, entry: u32) -> bool {
        let i = (entry.wrapping_sub(self.lo) / 4) as usize;
        self.hand_bits.get(i >> 6).is_some_and(|w| w >> (i & 63) & 1 != 0)
    }

    /// How many hand-written functions are in use.
    pub fn hand_len(&self) -> usize {
        self.hand.len()
    }

    /// The same sets and hands are in use (test: `ORACLE_REC_CHECK`).
    pub fn same_use(&self, o: &Table) -> bool {
        self.sets == o.sets && self.hand.len() == o.hand.len() && self.hand.keys().all(|k| o.hand.contains_key(k))
    }

    /// Code at a function entry changed since the build.
    pub fn stale(&self, mem: &Mem) -> bool {
        self.probe.iter().any(|&(a, w)| rd(mem, a) != w)
    }

    pub fn len(&self) -> usize {
        self.fns.len()
    }

    #[inline(always)]
    pub fn get(&self, pc: u32) -> Option<fn(&mut ee_core::cpu::Cpu)> {
        let i = (pc.wrapping_sub(self.lo) / 4) as usize;
        match self.map.get(i) {
            Some(&k) if k != 0 && pc & 3 == 0 => Some(self.fns[k as usize - 1]),
            _ => None,
        }
    }

    /// When `pc` lies after a compiled function's end and before the next compiled entry (the census cut the
    /// function short, so the rest of its body or its epilogue is there): the stack frame its prologue allocates
    /// (`addiu`/`daddiu sp, sp, -N` among its first 12 words), else None.
    pub fn gap_frame(&self, pc: u32, mem: &Mem) -> Option<u32> {
        let k = self.spans.partition_point(|&(e, _)| e <= pc);
        let &(entry, end) = self.spans.get(k.checked_sub(1)?)?;
        if pc < end {
            return None;
        }
        (0..12).map(|i| rd(mem, entry + 4 * i)).find_map(|w| {
            (matches!(w >> 16, 0x27bd | 0x67bd) && w & 0x8000 != 0).then(|| 0x1_0000 - (w & 0xffff))
        })
    }
}

/// The level loader state word (`LOADER`).
pub fn loader(mem: &Mem) -> u16 {
    u16::from_le_bytes([mem.ram[LOADER as usize], mem.ram[LOADER as usize + 1]])
}

/// ORACLE_REC_LOG=1 (read once): every table rebuild after a FlushCache prints `rec_rebuild ICOUNT LEVEL LOADER` to stderr.
fn rebuild_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("ORACLE_REC_LOG").is_ok_and(|v| v == "1"))
}

/// ORACLE_REC_CHECK=1 (read once, test only): a FlushCache that skips the rebuild checks that a rebuild would not
/// have changed the sets or hands in use (panics otherwise).
fn rebuild_check() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("ORACLE_REC_CHECK").is_ok_and(|v| v == "1"))
}

fn rd(mem: &Mem, a: u32) -> u32 {
    u32::from_le_bytes(mem.ram[a as usize..a as usize + 4].try_into().unwrap())
}

pub fn hash(mem: &Mem, lo: u32, hi: u32) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for a in (lo..hi).step_by(4) {
        let w = u32::from_le_bytes(mem.ram[a as usize..a as usize + 4].try_into().unwrap());
        h = (h ^ w as u64).wrapping_mul(0x100_0000_01b3);
    }
    h
}

impl Cpu {
    /// One dispatch: a recompiled function from the current PC, else one interpreter step.
    #[inline]
    pub fn step_rec(&mut self) {
        self.step_rec_impl::<false>()
    }

    /// `n` dispatches in one loop with the interpreter's decoder inlined (no call per interpreted instruction).
    #[inline(never)]
    pub fn run_rec(&mut self, n: u32) {
        for _ in 0..n {
            self.step_rec_impl::<true>();
        }
    }

    #[inline(always)]
    fn step_rec_impl<const INL: bool>(&mut self) {
        if self.pc == crate::VBLANK_WAIT && self.vsync_armed && self.vblank_cb.is_some() {
            self.vblank_hook();
        }
        // Not in a delay slot (the interpreter finishes a branch it started).
        let f = match &self.rec {
            Some(t) if self.npc == self.pc.wrapping_add(4) => t.get(self.pc),
            _ => None,
        };
        if let Some(f) = f {
            let i = (((self.pc & 0x01ff_ffff) >> 2)) as usize;
            if self.stub_bits[i >> 6] >> (i & 63) & 1 == 0 {
                let (pc, n) = (self.pc, self.icount);
                f(self.core_mut());
                if self.pc != pc || self.icount != n {
                    return;
                }
            }
        }
        self.step_impl::<INL>();
        if self.rec_flush {
            self.rec_flushed();
        }
    }

    /// Compiled function entry: runs the hand-written replacement instead, when there is one (true: done, the PC
    /// is the return address).
    #[inline(always)]
    pub fn hand(&mut self, entry: u32) -> bool {
        // Cold boot: hook rules (cold_hooks.txt) sit at function entries; compiled callers reach this prologue directly, not `stub`.
        if self.cold.is_some() && self.pc == entry && self.cold_entry(entry) {
            return true;
        }
        match &self.rec {
            Some(t) if self.pc == entry && t.has_hand(entry) => self.run_hand(entry),
            _ => false,
        }
    }

    #[inline(never)]
    #[doc(hidden)]
    pub fn cold_entry(&mut self, entry: u32) -> bool {
        let i = ((entry & 0x01ff_ffff) >> 2) as usize;
        self.stub_bits[i >> 6] >> (i & 63) & 1 != 0 && crate::cold::stub(self, entry)
    }

    #[inline(never)]
    #[doc(hidden)]
    pub fn run_hand(&mut self, entry: u32) -> bool {
        let Some(&(f, est)) = self.rec.as_ref().and_then(|t| t.hand.get(&entry)) else { return false };
        let ra = self.g(31) as u32;
        if !f(self) {
            return false;
        }
        self.icount += est;
        self.mem.now = self.icount;
        self.pc = ra;
        self.npc = ra.wrapping_add(4);
        true
    }

    /// From a hand-written function: calls the retail function at `t` (arguments already in the registers) and
    /// returns when it does. `ra` is clobbered, as a retail call would.
    pub fn call(&mut self, t: u32) {
        self.s(31, CALL_RET as u64);
        self.call_dyn(t, CALL_RET);
    }

    /// After a FlushCache: rebuild the table if code at a function entry changed. When the sets in use change and
    /// a host loop catches it, the compiled frames unwind (they may be the replaced code); the PC is the instruction
    /// after the syscall, so the host goes on interpreting from there.
    #[inline(never)]
    #[doc(hidden)]
    pub fn rec_flushed(&mut self) {
        self.rec_flush = false;
        let Some(t) = self.rec.as_ref() else { return };
        if !t.stale(&self.mem) {
            if rebuild_check() {
                let lv = self.mem.ram[crate::LEVEL as usize] as u32;
                let n = Table::build(&self.mem, lv);
                // A level or loader change not yet seen is the vblank callback's rebuild (lib.rs), same as before this change.
                if n.level == t.level && n.loader == t.loader && !t.same_use(&n) {
                    panic!("rec_check: skipped rebuild would change the table at icount {} level {} loader {}", self.icount, lv, loader(&self.mem));
                }
            }
            return;
        }
        if rebuild_log() {
            eprintln!("rec_rebuild {} {} {}", self.icount, self.mem.ram[crate::LEVEL as usize], loader(&self.mem));
        }
        let n = Table::build(&self.mem, self.mem.ram[crate::LEVEL as usize] as u32);
        let changed = n.sets != t.sets;
        self.rec = Some(Box::new(n));
        if changed && self.rec_catch {
            std::panic::resume_unwind(Box::new(crate::RecRebuilt));
        }
    }

    /// At the vblank wait: an armed wait runs the frame callback (which raises the vblank) and the game goes on;
    /// armed without a callback, false (the caller unwinds to a host loop that raises it).
    #[inline(never)]
    #[doc(hidden)]
    pub fn vblank_hook(&mut self) -> bool {
        if !self.vsync_armed {
            return true;
        }
        if self.mem.tick != u64::MAX {
            self.mem.rearm_tick(self.icount);
        }
        match self.vblank_cb.take() {
            Some(mut f) => {
                f(self);
                if self.vblank_cb.is_none() {
                    self.vblank_cb = Some(f);
                }
                if std::mem::take(&mut self.rec_unwind) {
                    std::panic::resume_unwind(Box::new(crate::RecRebuilt));
                }
                true
            }
            None => false,
        }
    }

    /// At the vblank wait with no frame callback: compiled code unwinds to the host loop.
    #[inline]
    fn unwind_wait(&self) -> bool {
        self.pc == crate::VBLANK_WAIT && self.vblank_cb.is_none()
    }

    /// Block entry: whether the next `n` instructions can run without the interpreter's per-instruction checks
    /// (no DMA stop matures and no interrupt is taken among them). A due interrupt is serviced first.
    #[inline(always)]
    #[doc(hidden)]
    pub fn poll(&mut self, n: u64) -> bool {
        if self.icount + n <= self.mem.limit {
            return true;
        }
        self.poll_slow(n)
    }

    /// False also when an interrupt did not return here (the caller checks the PC).
    #[inline(never)]
    fn poll_slow(&mut self, n: u64) -> bool {
        self.npc = self.pc.wrapping_add(4);
        loop {
            if self.icount >= self.mem.due {
                self.mem.mature(self.icount);
                if self.icount >= self.mem.tick {
                    crate::cold::tick(self);
                }
            }
            if self.mem.irq_dirty {
                let pc = self.pc;
                self.try_irq();
                if self.pc != pc {
                    if !self.service() {
                        return false;
                    }
                    continue;
                }
            }
            return self.icount + n <= self.mem.due;
        }
    }

    /// A load or store that is not plain memory, run by the interpreter as instruction number `n` (the PC after
    /// it is `next`).
    #[cold]
    #[inline(never)]
    #[doc(hidden)]
    pub fn hw(&mut self, op: u32, pc: u32, n: u64, next: u32) {
        self.icount = n;
        self.mem.now = n;
        self.mem.pc = pc;
        self.pc = next;
        self.npc = next.wrapping_add(4);
        self.exec(op, pc);
        // [PC] (speed) An access that marked the interrupt lines dirty (every DMA kick sets its D_STAT bit) sent the rest
        // of the block to the interpreter, whose next step only re-tests the lines (`try_irq`: both low -> Cause IP
        // bits clear, dirty off). Done here when both are low, the block goes on compiled. A stop that matures or a
        // tick before that step marks the lines dirty again itself, and `try_irq` rewrites the IP bits before any use.
        if self.mem.irq_dirty {
            let (i0, i1) = self.mem.irq_lines();
            if !i0 && !i1 {
                self.cp0[13] &= !0xc00;
                self.mem.clear_dirty();
            }
        }
    }

    /// After a hardware access inside a block: whether the `n` instructions left still run without checks.
    #[inline(always)]
    #[doc(hidden)]
    pub fn go(&self, n: u64) -> bool {
        self.icount + n <= self.mem.limit
    }

    /// Interprets from the current state until the instruction at `last` has run (or the likely branch at `brl`
    /// skipped its delay slot), servicing interrupts in place. False when one did not come back.
    #[inline(never)]
    #[doc(hidden)]
    pub fn slow(&mut self, last: u32, brl: u32) -> bool {
        loop {
            if self.stop.is_some() {
                return false;
            }
            let p = self.pc;
            let n = self.irqs[0] + self.irqs[1];
            self.step_impl::<true>();
            if self.rec_flush {
                self.rec_flushed();
            }
            if self.irqs[0] + self.irqs[1] != n {
                if !self.service() {
                    return false;
                }
                continue;
            }
            if p == last || (p == brl && self.pc == brl.wrapping_add(8)) {
                return true;
            }
        }
    }

    /// Runs the interrupt just taken until it returns (ERET to EPC).
    fn service(&mut self) -> bool {
        let epc = self.cp0[14];
        while !(self.pc == epc && self.cp0[12] & 2 == 0) {
            if self.stop.is_some() || self.unwind_wait() {
                return false;
            }
            self.step_rec_impl::<true>();
        }
        true
    }

    /// A call to `t` that returns to `ret` (state already linked): the compiled function holding the label, else
    /// the dispatcher until the PC is back at `ret` with the caller's stack pointer, or reaches the vblank wait.
    #[inline(never)]
    #[doc(hidden)]
    pub fn call_dyn(&mut self, t: u32, ret: u32) {
        self.call_dyn_frame(t, ret, 0);
    }

    /// `call_dyn`, where the return may also come with the stack pointer raised by `frame` (the callee pops a
    /// frame that was allocated before the call).
    fn call_dyn_frame(&mut self, t: u32, ret: u32, frame: u32) {
        self.pc = t;
        self.npc = t.wrapping_add(4);
        if let Some(f) = self.rec.as_ref().and_then(|r| r.get(t)) {
            let i = ((t & 0x01ff_ffff) >> 2) as usize;
            if self.stub_bits[i >> 6] >> (i & 63) & 1 == 0 {
                f(self.core_mut());
                return;
            }
        }
        let sp = self.r[29] as u32;
        let up = sp.wrapping_add(frame);
        while !(self.pc == ret && (self.r[29] as u32 == sp || self.r[29] as u32 == up)) {
            if self.stop.is_some() || self.unwind_wait() {
                return;
            }
            self.step_rec_impl::<true>();
        }
    }

    /// The PC left the function's labels: a tail jump into other code continues there (returning to the current
    /// `ra`); the vblank wait, kernel code (a handler's exit) and a stop unwind. Code after a compiled function's
    /// end (`Table::gap_frame`: the rest of a body the census cut short, often just its epilogue) runs in that
    /// function's frame, so it returns to `ra` with the stack pointer raised by the frame: without that second
    /// test the nested run missed the return and went on into the caller's code (into CALL_RET under a hand's
    /// `Cpu::call`).
    #[inline(never)]
    #[doc(hidden)]
    pub fn tail(&mut self) {
        let pc = self.pc;
        if pc >= 0x8000_0000 || self.unwind_wait() || self.stop.is_some() {
            return;
        }
        let ra = self.g(31) as u32;
        let npc = self.npc;
        if npc != pc.wrapping_add(4) {
            return;
        }
        let frame = self.rec.as_ref().and_then(|t| t.gap_frame(pc, &self.mem)).unwrap_or(0);
        self.call_dyn_frame(pc, ra, frame);
    }

    /// At the head of a vblank-start poll, `lw rt, off(rs)`; `andi rt, rt, 4`; 3 nops; `beq rt, zero, head`; nop: sceGsSyncV
    /// FUN_00118bd0 +0x40 (`crate::POLL_HEAD`), which spins there while no vsync callback is registered: after the PAL 50/60
    /// Hz switch on the title menu, in level loads and in levels whose start does not register it again (e.g. Load Game).
    /// Called by compiled code (loop hook kind 1, recgen_iso.py `poll_head`) and by the interpreter (`Cpu::stub`: during a
    /// level load no compiled set is in use).
    /// [PC] (speed) Every iteration that starts at least 7 instructions before the next event (`mem.due`: a VIF1 stop or
    /// the cold-boot field tick) loads the same INTC_STAT (it changes only at events, in interrupts and by EE stores) and
    /// does nothing else, so all but the last of them are skipped: the count advances 7 per iteration and rt keeps what
    /// each one leaves (stat & 4 = 0). The last one runs as before (it sets `mem.now` / `mem.pc` itself), so the loop leaves
    /// at the same instruction count. With the lines marked dirty (a line held up but masked) every iteration re-tests them
    /// (`try_irq`: the same Cause IP bits each time) and takes nothing while Status, which the loop does not change, masks
    /// them. Not in savestate runs (no field timer: their vblank comes from the host, not from `due`).
    pub fn poll_ff(&mut self, pc: u32) {
        const N: u64 = 7;
        if self.mem.tick == u64::MAX || self.npc != pc.wrapping_add(4) {
            return;
        }
        if self.mem.irq_dirty {
            let (i0, i1) = self.mem.irq_lines();
            if Self::irq_taken(self.cp0[12], i0, i1) {
                return;
            }
        }
        let k = self.mem.due.saturating_sub(self.icount) / N;
        if k < 2 {
            return;
        }
        let w = rd(&self.mem, pc);
        let (rs, rt) = ((w >> 21 & 31) as usize, (w >> 16 & 31) as usize);
        let at = |i: u32| rd(&self.mem, pc + 4 * i);
        if w >> 26 != 0x23 || rt == 0 || rs == rt || at(1) != 0x3000_0004 | (rt as u32) << 21 | (rt as u32) << 16
            || at(2) | at(3) | at(4) | at(6) != 0 || at(5) != 0x1000_fffa | (rt as u32) << 21
        {
            return;
        }
        let a = (self.g(rs) as u32).wrapping_add(w as i16 as i32 as u32);
        if a != crate::mem::INTC_STAT || self.mem.hw32(crate::mem::INTC_STAT) & 4 != 0 {
            return;
        }
        self.icount += (k - 1) * N;
        self.s(rt, 0);
    }
}

impl Mem {
    // The plain-memory fast path lives in ee-core (mem.rs there), where the generated code inlines it.
    #[inline(always)]
    fn core_mem(&mut self) -> &mut ee_core::mem::Mem {
        unsafe { &mut *(self as *mut Mem as *mut ee_core::mem::Mem) }
    }
    #[inline(always)] #[doc(hidden)] pub fn fast(&mut self, a: u32, n: usize) -> bool { self.core_mem().fast(a, n) }
    #[inline(always)] #[doc(hidden)] pub fn frd8(&mut self, a: u32) -> Option<u8> { self.core_mem().frd8(a) }
    #[inline(always)] #[doc(hidden)] pub fn frd16(&mut self, a: u32) -> Option<u16> { self.core_mem().frd16(a) }
    #[inline(always)] #[doc(hidden)] pub fn frd32(&mut self, a: u32) -> Option<u32> { self.core_mem().frd32(a) }
    #[inline(always)] #[doc(hidden)] pub fn frd64(&mut self, a: u32) -> Option<u64> { self.core_mem().frd64(a) }
    #[inline(always)] #[doc(hidden)] pub fn frd128(&mut self, a: u32) -> Option<u128> { self.core_mem().frd128(a) }
    #[inline(always)] #[doc(hidden)] pub fn fwr8(&mut self, a: u32, v: u8) -> bool { self.core_mem().fwr8(a, v) }
    #[inline(always)] #[doc(hidden)] pub fn fwr16(&mut self, a: u32, v: u16) -> bool { self.core_mem().fwr16(a, v) }
    #[inline(always)] #[doc(hidden)] pub fn fwr32(&mut self, a: u32, v: u32) -> bool { self.core_mem().fwr32(a, v) }
    #[inline(always)] #[doc(hidden)] pub fn fwr64(&mut self, a: u32, v: u64) -> bool { self.core_mem().fwr64(a, v) }
    #[inline(always)] #[doc(hidden)] pub fn fwr128(&mut self, a: u32, v: u128) -> bool { self.core_mem().fwr128(a, v) }
}

// ---------------- the interface (ee-core declares these; the generated code calls them through its `Cpu` view) ----------------

const _: () = {
    use std::mem::offset_of;
    type C = ee_core::cpu::Cpu;
    type M = ee_core::mem::Mem;
    type H = ee_core::rec::RecHot;
    assert!(offset_of!(Cpu, r) == offset_of!(C, r));
    assert!(offset_of!(Cpu, hi) == offset_of!(C, hi));
    assert!(offset_of!(Cpu, lo) == offset_of!(C, lo));
    assert!(offset_of!(Cpu, sa) == offset_of!(C, sa));
    assert!(offset_of!(Cpu, pc) == offset_of!(C, pc));
    assert!(offset_of!(Cpu, npc) == offset_of!(C, npc));
    assert!(offset_of!(Cpu, f) == offset_of!(C, f));
    assert!(offset_of!(Cpu, facc) == offset_of!(C, facc));
    assert!(offset_of!(Cpu, fcr31) == offset_of!(C, fcr31));
    assert!(offset_of!(Cpu, vf) == offset_of!(C, vf));
    assert!(offset_of!(Cpu, vacc) == offset_of!(C, vacc));
    assert!(offset_of!(Cpu, vi) == offset_of!(C, vi));
    assert!(offset_of!(Cpu, icount) == offset_of!(C, icount));
    assert!(offset_of!(Cpu, div_csr) == offset_of!(C, div_csr));
    assert!(offset_of!(Cpu, vsync_armed) == offset_of!(C, vsync_armed));
    assert!(offset_of!(Cpu, stop) == offset_of!(C, stop));
    assert!(offset_of!(Cpu, cold) == offset_of!(C, cold));
    assert!(offset_of!(Cpu, rec) == offset_of!(C, rec));
    assert!(offset_of!(Cpu, stub_bits) == offset_of!(C, stub_bits));
    assert!(offset_of!(Cpu, mem) == offset_of!(C, mem));
    assert!(offset_of!(Mem, ram) == offset_of!(M, ram));
    assert!(offset_of!(Mem, spr) == offset_of!(M, spr));
    assert!(offset_of!(Mem, vu0d) == offset_of!(M, vu0d));
    assert!(offset_of!(Mem, watch_vu) == offset_of!(M, watch_vu));
    assert!(offset_of!(Mem, irq_dirty) == offset_of!(M, irq_dirty));
    assert!(offset_of!(Mem, due) == offset_of!(M, due));
    assert!(offset_of!(Mem, limit) == offset_of!(M, limit));
    assert!(offset_of!(Table, lo) == offset_of!(H, lo));
    assert!(offset_of!(Table, hand_bits) == offset_of!(H, hand_bits));
    assert!(std::mem::size_of::<Option<Box<crate::cold::Kernel>>>() == std::mem::size_of::<Option<std::ptr::NonNull<u8>>>());
    assert!(std::mem::size_of::<Option<Box<Table>>>() == std::mem::size_of::<Option<std::ptr::NonNull<H>>>());
    assert!(std::mem::size_of::<C>() <= std::mem::size_of::<Cpu>() && std::mem::size_of::<M>() <= std::mem::size_of::<Mem>());
};

#[no_mangle] #[doc(hidden)] pub fn ee_core_cold_entry(c: &mut ee_core::cpu::Cpu, entry: u32) -> bool { Cpu::full(c).cold_entry(entry) }
#[no_mangle] #[doc(hidden)] pub fn ee_core_run_hand(c: &mut ee_core::cpu::Cpu, entry: u32) -> bool { Cpu::full(c).run_hand(entry) }
#[no_mangle] #[doc(hidden)] pub fn ee_core_poll_slow(c: &mut ee_core::cpu::Cpu, n: u64) -> bool { Cpu::full(c).poll_slow(n) }
#[no_mangle] #[doc(hidden)] pub fn ee_core_hw(c: &mut ee_core::cpu::Cpu, op: u32, pc: u32, n: u64, next: u32) { Cpu::full(c).hw(op, pc, n, next) }
#[no_mangle] #[doc(hidden)] pub fn ee_core_slow(c: &mut ee_core::cpu::Cpu, last: u32, brl: u32) -> bool { Cpu::full(c).slow(last, brl) }
#[no_mangle] #[doc(hidden)] pub fn ee_core_call_dyn(c: &mut ee_core::cpu::Cpu, t: u32, ret: u32) { Cpu::full(c).call_dyn(t, ret) }
#[no_mangle] #[doc(hidden)] pub fn ee_core_tail(c: &mut ee_core::cpu::Cpu) { Cpu::full(c).tail() }
#[no_mangle] #[doc(hidden)] pub fn ee_core_vblank_hook(c: &mut ee_core::cpu::Cpu) -> bool { Cpu::full(c).vblank_hook() }
#[no_mangle] #[doc(hidden)] pub fn ee_core_exec(c: &mut ee_core::cpu::Cpu, op: u32, pc: u32) { Cpu::full(c).exec(op, pc) }
/// Loop fast paths from compiled code (ee-core `ff`): kind 0 the one-joint skinning loop (true: ran), kind 1 the vblank-start
/// poll (`Cpu::poll_ff`; false: the compiled loop goes on).
#[no_mangle]
#[doc(hidden)]
pub fn ee_core_ff(c: &mut ee_core::cpu::Cpu, kind: u32, pc: u32) -> bool {
    match kind {
        0 => Cpu::full(c).skin_ff(pc),
        1 => {
            Cpu::full(c).poll_ff(pc);
            false
        }
        _ => false,
    }
}
#[no_mangle] #[doc(hidden)] pub fn ee_core_cop2(c: &mut ee_core::cpu::Cpu, op: u32, pc: u32, rs: usize, rt: usize, rd: usize) { Cpu::full(c).cop2(op, pc, rs, rt, rd) }
/// VCALLMS / VCALLMSR from compiled code (ee-core `vu_special1`).
#[no_mangle]
#[doc(hidden)]
pub fn ee_core_vcallms(c: &mut ee_core::cpu::Cpu, a: u32, pc: u32) {
    let c = Cpu::full(c);
    if crate::mem::stats() {
        *c.vu_calls.entry(a * 8).or_insert(0) += 1;
    }
    if let Err(e) = c.vu0_micro(a * 8) {
        c.halt(format!("{e} (VCALLMS from {pc:08x})"));
    }
}
