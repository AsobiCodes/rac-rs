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
/// compiles (`None`: boot code or unknown, always probed; `FRONT_LEVEL` and up: the front-end images). Each is used only
/// when all its functions match RAM, because its functions call each other directly.
static SETS: std::sync::Mutex<Vec<(Option<u32>, &'static [Fn])>> = std::sync::Mutex::new(Vec::new());

/// Adds a generated set that is not tied to a level (once per set).
pub fn register(set: &'static [Fn]) {
    register_at(None, set);
}

/// Adds a generated set of level `level` (the level byte, 0x15ed84; once per set), or of a front-end image
/// (`FRONT_LEVEL`, `FRONT_LEVEL + 1`). `Table::build` probes the entries of such a set only while it is in use; while it
/// does not match, it watches the one function that keeps it from matching (`Table::pending`, see there), for the
/// current level's sets and for the front-end sets (whatever the level byte).
pub fn register_level(level: u32, set: &'static [Fn]) {
    register_at(Some(level), set);
}

/// The first pseudo-level of the front-end code (tools/recgen_iso.py FRONT_A = 19: the boot ELF's own front end, FRONT_B = 20:
/// frontbin, which the game loads after Quit Game). The level byte is only ever 0..18, so these sets belong to no level: the
/// title runs with the level byte 0.
pub const FRONT_LEVEL: u32 = 19;

fn is_front(tag: Option<u32>) -> bool {
    tag.is_some_and(|l| l >= FRONT_LEVEL)
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

static RELOC_HANDS: std::sync::Mutex<Vec<&'static crate::hs_reloc::RelocHand>> = std::sync::Mutex::new(Vec::new());

/// Adds a relocatable speed hand (S2i, hs_reloc.rs; once per hand). `Table::build` installs it at the entry of EVERY recompiled function
/// (any level, either disc) whose code words equal its template with the relocated fields masked. ORACLE_HAND=0 leaves them out.
pub fn register_reloc_hand(h: &'static crate::hs_reloc::RelocHand) {
    let mut s = RELOC_HANDS.lock().unwrap();
    if !s.iter().any(|x| std::ptr::eq(*x, h)) {
        s.push(h);
    }
}

/// S4: are the relocatable speed hands (built-in ones in `hs_hands::reloc_all`) installed? `RAC_HS=0` turns them off (A/B against
/// the recompiled code); registered relocatable hands (PC hooks) are installed regardless; ORACLE_HAND=0 turns off every hand incl. PC hooks). Read once per process.
pub fn speed_hands_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    match HS_FORCE.load(std::sync::atomic::Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => *ON.get_or_init(|| !std::env::var("RAC_HS").is_ok_and(|v| v == "0")),
    }
}
static HS_FORCE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
/// Tools (hsbench): overrides RAC_HS for the following `Table::build` calls (None = back to the environment).
pub fn speed_hands_force(on: Option<bool>) {
    HS_FORCE.store(match on { None => 0, Some(true) => 1, Some(false) => 2 }, std::sync::atomic::Ordering::Relaxed);
}

/// The registered relocatable speed hands (tools).
pub fn reloc_hands() -> Vec<&'static crate::hs_reloc::RelocHand> {
    RELOC_HANDS.lock().unwrap().clone()
}

/// The registered hand-written functions (tools).
pub fn hands() -> Vec<&'static Hand> {
    HANDS.lock().unwrap().iter().flat_map(|s| s.iter()).collect()
}

/// The registered sets (tools: profiles, the hand-written functions' checks).
pub fn sets() -> Vec<&'static [Fn]> {
    SETS.lock().unwrap().iter().map(|&(_, s)| s).collect()
}

/// The registered sets of the boot code and of the levels, without the front-end images. rac-rs's mod code guard
/// (omni/codeguard.rs) refuses mod writes over these; the front end was never compiled before, so a mod may still patch it
/// (`Cpu::rec_front_written` then drops the compiled copy).
pub fn level_sets() -> Vec<&'static [Fn]> {
    SETS.lock().unwrap().iter().filter(|&&(t, _)| !is_front(t)).map(|&(_, s)| s).collect()
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
    /// For each set that does not match and that the table watches (the current level's, and the front-end images'
    /// whatever the level byte): (entry, end, hash) of the first function that fails. The set can match only after that
    /// function does, so `stale` is true when its hash fits RAM now.
    pending: Vec<(u32, u32, u64)>,
    /// The functions of the front-end sets in use: (entry, the largest end up to this entry), sorted by entry (`front_overlaps`).
    front: Vec<(u32, u32)>,
    /// The front-end sets in use, by registration index (`check_diff`).
    front_sets: Vec<usize>,
    hand: std::collections::HashMap<u32, (fn(&mut Cpu) -> bool, u64)>,
    /// The same hands as a short list for `run_hand` (a linear scan of a few entries beats a SipHash lookup: ~14 ns per hand call).
    hlin: Vec<(u32, fn(&mut Cpu) -> bool, u64)>,
    /// (entry, end) of every function in the sets used, by entry (`gap_frame`).
    spans: Vec<(u32, u32)>,
}

impl Table {
    pub fn build(mem: &Mem, level: u32) -> Table {
        // A rebuild follows every overlay load (FlushCache): the host learns which overlay is in RAM (`ver::on_rebuild`).
        crate::ver::rebuilt(&mem.ram);
        let sets = SETS.lock().unwrap().clone();
        Table::build_sets(mem, level, &sets)
    }

    /// `build` over the given sets (tests build over their own).
    fn build_sets(mem: &Mem, level: u32, sets: &[(Option<u32>, &'static [Fn])]) -> Table {
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
            pending: Vec::new(),
            front: Vec::new(),
            front_sets: Vec::new(),
            hand_bits: vec![0; ((hi - lo) / 4 / 64) as usize],
            hand: std::collections::HashMap::new(),
            hlin: Vec::new(),
            spans: Vec::new(),
        };
        if (1..=6).contains(&loader) {
            return t;
        }
        // A set is used only when all its functions match RAM (they call each other directly).
        for (i, &(tag, set)) in sets.iter().enumerate() {
            let bad = set.iter().find(|f| f.end > hi || f.entry < lo || hash(mem, f.entry, f.end) != f.hash);
            let used = bad.is_none();
            // Probe the entries of the sets in use and of the sets not tied to a level (boot code: a rebuild when it is
            // loaded or changes). A set of a level that does not match is hashed but not probed: its address range is
            // live data when that overlay is not in RAM (the level byte is 0 at the title, with the front end there)
            // and changes every field. It can only start to match when the first function that fails now matches, so the
            // current level's sets keep that one function (`pending`) and `stale` hashes it: no rebuild unless it fits.
            // The front-end sets (tags FRONT_LEVEL and up) do the same whatever the level byte: no level byte ever names
            // them, and a set that was found not matching gets no entry probe, so without `pending` a front end that
            // finishes loading later (frontbin after Quit Game) would only be picked up by an unrelated rebuild.
            if used || tag.is_none() {
                for f in set.iter().filter(|f| f.entry >= lo && f.end <= hi) {
                    for a in [f.entry, f.entry + 4] {
                        t.probe.push((a, rd(mem, a)));
                    }
                }
            } else if let Some(b) = bad.filter(|b| (tag == Some(level) || is_front(tag)) && b.entry >= lo && b.end <= hi) {
                t.pending.push((b.entry, b.end, b.hash));
            }
            if !used {
                continue;
            }
            t.sets.push(i);
            if is_front(tag) {
                t.front_sets.push(i);
            }
            for f in set.iter() {
                t.fns.push(f.f);
                let k = t.fns.len() as u32;
                for &l in f.labels {
                    t.map[((l - lo) / 4) as usize] = k;
                }
                t.spans.push((f.entry, f.end));
                if is_front(tag) {
                    t.front.push((f.entry, f.end));
                }
            }
        }
        t.spans.sort_unstable();
        t.front.sort_unstable();
        let mut reach = 0;
        for s in &mut t.front {
            reach = reach.max(s.1);
            s.1 = reach;
        }
        if !std::env::var("ORACLE_HAND").is_ok_and(|v| v == "0") {
            // ORACLE_EST_SCALE=P (percent, default 100): a hand advances the instruction count by est*P/100 (timing
            // probe; unset leaves est as is).
            let scale = std::env::var("ORACLE_EST_SCALE").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(100);
            for h in hands() {
                if h.entry >= lo && h.end <= hi && hash(mem, h.entry, h.end) == h.hash {
                    let i = ((h.entry - lo) / 4) as usize;
                    t.hand_bits[i >> 6] |= 1 << (i & 63);
                    let est = if scale == 100 || h.est == crate::hs::EXACT { h.est } else { h.est * scale / 100 };
                    t.hand.insert(h.entry, (h.f, est));
                    t.hlin.retain(|x| x.0 != h.entry);
                    t.hlin.push((h.entry, h.f, est));
                    // A hand in use also probes its entry words: changed code under a hand rebuilds the table.
                    for a in [h.entry, h.entry + 4] {
                        if a < h.end {
                            t.probe.push((a, rd(mem, a)));
                        }
                    }
                }
            }
        }
        // Relocatable speed hands (S2i): every recompiled function of the sets in use whose code matches the template is a hand entry.
        // RAC_HS=0 leaves out only the built-in speed hands; the REGISTERED ones (PC hooks, mods) are installed anyway.
        if !std::env::var("ORACLE_HAND").is_ok_and(|v| v == "0") {
            let spans = t.spans.clone();
            // (hand, built-in speed hand): the registered ones (mod hook points, PC hooks) never bind into the front-end sets. The
            // front end was interpreted before it was compiled and a hand is only consulted at a compiled function's entry, so
            // no registered hand could ever run there; a hook point (OmniLoader `rac1.*`, Arsenal Mastery) whose function also
            // exists in the front end would otherwise start to fire at the title. The built-in speed hands are exact, so they
            // may bind anywhere.
            let mut all: Vec<_> = if speed_hands_on() { crate::hs_hands::reloc_all().into_iter().map(|h| (h, true)).collect() } else { Vec::new() };
            for rh in reloc_hands() {
                if !all.iter().any(|x| std::ptr::eq(x.0, rh)) {
                    all.push((rh, false));
                }
            }
            let mut n_hs = 0u32;
            for (rh, builtin) in all {
                let r = (rh.reloc)();
                for &(e, en) in &spans {
                    if !builtin && t.front.binary_search_by_key(&e, |x| x.0).is_ok() {
                        continue;
                    }
                    if e >= lo && en <= hi && en - e == r.len && r.matches(mem, e) {
                        let i = ((e - lo) / 4) as usize;
                        t.hand_bits[i >> 6] |= 1 << (i & 63);
                        t.hand.insert(e, (rh.f, rh.est));
                        t.hlin.retain(|x| x.0 != e);
                        t.hlin.push((e, rh.f, rh.est));
                        for a in [e, e + 4] {
                            t.probe.push((a, rd(mem, a)));
                        }
                        n_hs += 1;
                    }
                }
            }
            if n_hs > 0 {
                // One line per table build, so a run log shows the speed hands are live (RAC_HS=0 turns them off).
                eprintln!("rec: {n_hs} speed hand(s) installed (RAC_HS=0 = built-in off, PC hooks stay)");
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

    /// A front-end set is in use: the front end (title, menus, or the frontbin after Quit Game) is in RAM and runs compiled.
    pub fn front_in_use(&self) -> bool {
        !self.front.is_empty()
    }

    /// Whether `addr..addr+len` overlaps a function of a front-end set in use (overlapping and nested functions included).
    pub fn front_overlaps(&self, addr: u32, len: u32) -> bool {
        let end = addr.saturating_add(len);
        let k = self.front.partition_point(|&(e, _)| e < end);
        k.checked_sub(1).and_then(|i| self.front.get(i)).is_some_and(|&(_, en)| en > addr)
    }

    /// A skipped rebuild that matters (test: `ORACLE_REC_CHECK`), or None; `o` is a full rebuild at the same level and loader.
    /// Reported: a set or hand `o` uses that this table lacks (a pickup `stale` missed), and a front-end set this table keeps that `o`
    /// drops (compiled front-end code over RAM that changed behind unchanged entry words; the front end ran interpreted before this
    /// lane). Not reported: a boot or level set this table keeps although its body no longer matches RAM while its entry words do
    /// (Quit Game from a level: heap data lands on the dead overlay's last function). The probes of a set in use are the two entry
    /// words of every function, as in dev s137, whose table keeps such a set up to the same rebuild (Quit Game replay: the first
    /// rebuild after the boot one is at icount 257680928 in both). Nothing calls into the dead overlay.
    pub fn check_diff(&self, o: &Table) -> Option<String> {
        let add_s: Vec<_> = o.sets.iter().filter(|s| !self.sets.contains(s)).collect();
        let add_h: Vec<_> = o.hand.keys().filter(|k| !self.hand.contains_key(k)).map(|k| format!("{k:#x}")).collect();
        let lost_f: Vec<_> = self.front_sets.iter().filter(|s| !o.sets.contains(s)).collect();
        if add_s.is_empty() && add_h.is_empty() && lost_f.is_empty() {
            return None;
        }
        Some(format!("sets it would add {add_s:?}, hands it would add {add_h:?}, front-end sets it would drop {lost_f:?}"))
    }

    /// Code at a function entry changed since the build, or a set that did not match can now (its first failing function fits).
    pub fn stale(&self, mem: &Mem) -> bool {
        self.probe.iter().any(|&(a, w)| rd(mem, a) != w) || self.pending.iter().any(|&(e, en, h)| hash(mem, e, en) == h)
    }

    /// Diagnostics (ORACLE_REC_LOG=1): the probes that differ, with the owner of the first few.
    fn stale_report(&self, mem: &Mem) -> String {
        let bad: Vec<&(u32, u32)> = self.probe.iter().filter(|&&(a, w)| rd(mem, a) != w).collect();
        let npend = self.pending.iter().filter(|&&(e, en, h)| hash(mem, e, en) == h).count();
        let mut s = format!(" nprobe={} npending={} ndiff={} pending_fit={} used={:?} nhand={}", self.probe.len(), self.pending.len(), bad.len(), npend, self.sets, self.hand.len());
        let sets = SETS.lock().unwrap();
        for &&(a, w) in bad.iter().take(3) {
            let mut own = String::from("hand/none");
            for (i, &(tag, set)) in sets.iter().enumerate() {
                if set.iter().any(|f| f.entry == a || f.entry + 4 == a) {
                    own += &format!(" set{}(tag={:?},used={})", i, tag, self.sets.contains(&i));
                }
            }
            if hands().iter().any(|h| h.entry == a || h.entry + 4 == a) {
                own += " REGHAND";
            }
            s += &format!(" [{:#x} {:#010x}->{:#010x} {}]", a, w, rd(mem, a), own);
        }
        s
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
        let Some(&(_, f, est)) = self.rec.as_ref().and_then(|t| t.hlin.iter().find(|h| h.0 == entry)) else { return false };
        // A speed hand (lane hs, hs.rs) counts its own instructions and sets the PC itself.
        if est == crate::hs::EXACT {
            return f(self);
        }
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
                if n.level == t.level && n.loader == t.loader {
                    if let Some(d) = t.check_diff(&n) {
                        panic!("rec_check: skipped rebuild would change the table at icount {} level {} loader {}: {}", self.icount, lv, loader(&self.mem), d);
                    }
                }
            }
            return;
        }
        if rebuild_log() {
            eprintln!("rec_rebuild {} {} {}{}", self.icount, self.mem.ram[crate::LEVEL as usize], loader(&self.mem), t.stale_report(&self.mem));
        }
        let n = Table::build(&self.mem, self.mem.ram[crate::LEVEL as usize] as u32);
        let changed = n.sets != t.sets;
        self.rec = Some(Box::new(n));
        if changed && self.rec_catch {
            std::panic::resume_unwind(Box::new(crate::RecRebuilt));
        }
    }

    /// A mod wrote `len` bytes at `addr` (rac-rs omni/host.rs, after the write). When that is code of the compiled front end in use,
    /// the table is rebuilt at once: the front end was interpreted before, so a patch there always took effect, and a compiled
    /// copy would ignore a write that no function entry probe sees. The rebuilt table finds the patched set no longer matching RAM
    /// and leaves it out (the interpreter runs the patched code, as before the front end was compiled); the compiled frames unwind
    /// at the next vblank like after a level load. Anywhere else this is a range check.
    pub fn rec_front_written(&mut self, addr: u32, len: u32) {
        if !self.rec.as_ref().is_some_and(|t| t.front_overlaps(addr, len)) {
            return;
        }
        let lv = self.mem.ram[crate::LEVEL as usize] as u32;
        self.rec = Some(Box::new(Table::build(&self.mem, lv)));
        self.rec_unwind = true;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hs_reloc::{Reloc, RelocHand};
    use std::sync::OnceLock;

    fn nop(_: &mut ee_core::cpu::Cpu) {}
    const BASE: u32 = 0x30_0000;

    fn fnv(words: &[u32]) -> u64 {
        words.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &w| (h ^ w as u64).wrapping_mul(0x100_0000_01b3))
    }

    fn put(mem: &mut Mem, a: u32, words: &[u32]) {
        for (i, w) in words.iter().enumerate() {
            let o = a as usize + 4 * i;
            mem.ram[o..o + 4].copy_from_slice(&w.to_le_bytes());
        }
    }

    /// A set of functions laid out one after the other from `entry` (no disc: the code words are made up).
    fn set_of(entry: u32, funcs: &[&[u32]]) -> &'static [Fn] {
        let mut a = entry;
        let v: Vec<Fn> = funcs
            .iter()
            .map(|w| {
                let f = Fn { entry: a, end: a + 4 * w.len() as u32, hash: fnv(w), labels: Box::leak(vec![a].into_boxed_slice()), f: nop };
                a = f.end;
                f
            })
            .collect();
        Box::leak(v.into_boxed_slice())
    }

    const F1: [u32; 4] = [0x1111_0001, 0x1111_0002, 0x1111_0003, 0x1111_0004];
    const F2: [u32; 2] = [0x2222_0005, 0x2222_0006];
    const F3: [u32; 3] = [0x3333_0007, 0x3333_0008, 0x3333_0009];

    /// A front-end set (tag FRONT_LEVEL + 1) has a `pending` entry whatever the level byte, so a front end that finishes
    /// loading later (frontbin after Quit Game) is picked up by the next FlushCache; a level set of another level has none.
    #[test]
    fn a_front_end_set_is_watched_whatever_the_level_byte() {
        let mut mem = Mem::new();
        let fe = set_of(BASE, &[&F1, &F2]);
        let lv = set_of(BASE + 0x1000, &[&F2]);
        let sets = [(Some(FRONT_LEVEL + 1), fe), (Some(5), lv)];
        let t = Table::build_sets(&mem, 0, &sets);
        assert_eq!((t.pending.len(), t.len(), t.sets.len()), (1, 0, 0), "level byte 0: only the front-end set is watched");
        assert_eq!(Table::build_sets(&mem, 5, &sets).pending.len(), 2, "level 5: its own set too");
        assert!(!t.stale(&mem));
        // The first function lands: the set is stale, the rebuilt table watches the second function.
        put(&mut mem, BASE, &F1);
        assert!(t.stale(&mem));
        let t = Table::build_sets(&mem, 0, &sets);
        assert_eq!((t.pending.len(), t.sets.len()), (1, 0));
        assert_eq!(t.pending[0].0, BASE + 16, "the second function is the one that fails now");
        assert!(!t.stale(&mem));
        // The last one lands: stale again, and the rebuilt table uses the set and no longer watches it.
        put(&mut mem, BASE + 16, &F2);
        assert!(t.stale(&mem));
        let t = Table::build_sets(&mem, 0, &sets);
        assert_eq!((t.pending.len(), t.sets.clone(), t.len()), (0, vec![0], 2));
        assert!(t.front_in_use() && !t.stale(&mem));
    }

    /// A failing function outside the compiled range is never watched (that set can never match).
    #[test]
    fn a_failing_function_outside_the_code_range_is_not_watched() {
        let mem = Mem::new();
        let t = Table::build_sets(&mem, 0, &[(Some(FRONT_LEVEL), set_of(0x8_0000, &[&F1]))]);
        assert!(t.pending.is_empty() && !t.stale(&mem));
    }

    /// `stale_report` (ORACLE_REC_LOG=1) counts the watched functions and the ones that fit now.
    #[test]
    fn the_stale_report_counts_the_pending_functions_that_fit() {
        let mut mem = Mem::new();
        let t = Table::build_sets(&mem, 0, &[(Some(FRONT_LEVEL), set_of(BASE, &[&F1, &F2]))]);
        assert!(t.stale_report(&mem).contains("npending=1 ndiff=0 pending_fit=0"), "{}", t.stale_report(&mem));
        put(&mut mem, BASE, &F1);
        assert!(t.stale_report(&mem).contains("npending=1 ndiff=0 pending_fit=1"), "{}", t.stale_report(&mem));
    }

    /// `check_diff` (ORACLE_REC_CHECK): a level set kept although its last function changed behind unchanged entry words is
    /// the entry-probe design of dev s137 and is not reported; the same for a front-end set is (compiled front-end code is new),
    /// and so is a set a full rebuild would add.
    #[test]
    fn the_rebuild_check_reports_a_lost_pickup_or_a_stale_front_end_set_but_not_a_stale_level_set() {
        let mut mem = Mem::new();
        put(&mut mem, BASE, &F1);
        put(&mut mem, BASE + 16, &F3);
        let lv = set_of(BASE, &[&F1, &F3]);
        let sets = [(Some(0), lv)];
        let t = Table::build_sets(&mem, 0, &sets);
        assert_eq!(t.sets, vec![0]);
        assert!(t.check_diff(&Table::build_sets(&mem, 0, &sets)).is_none());
        // The heap lands on the last word of the last function (three words: the probes are its first two): not seen.
        put(&mut mem, BASE + 24, &[0xdead_beef]);
        assert!(!t.stale(&mem), "the probes see only the entry words");
        let n = Table::build_sets(&mem, 0, &sets);
        assert!(n.sets.is_empty(), "a full rebuild drops the set");
        assert!(t.check_diff(&n).is_none(), "kept like dev s137 does: not reported");
        // The same for a front-end set is reported.
        put(&mut mem, BASE + 24, &F3[2..]);
        let fe = [(Some(FRONT_LEVEL), lv)];
        let t = Table::build_sets(&mem, 0, &fe);
        assert_eq!((t.sets.clone(), t.front_sets.clone()), (vec![0], vec![0]));
        put(&mut mem, BASE + 24, &[0xdead_beef]);
        assert!(!t.stale(&mem));
        let d = t.check_diff(&Table::build_sets(&mem, 0, &fe)).expect("a stale front-end set is reported");
        assert!(d.contains("front-end sets it would drop [0]"), "{d}");
        // A set the table does not use but a full rebuild would: a pickup that was missed.
        put(&mut mem, BASE + 24, &F3[2..]);
        let d = Table::build_sets(&mem, 0, &fe);
        let miss = Table::build_sets(&Mem::new(), 0, &fe).check_diff(&d).expect("a missed pickup is reported");
        assert!(miss.contains("sets it would add [0]"), "{miss}");
    }

    fn reloc() -> &'static Reloc {
        static R: OnceLock<Reloc> = OnceLock::new();
        R.get_or_init(|| Reloc::parse(&format!("reloc test_hook entry {BASE:#x} len {:#x}\nmhash {:#x}\n", 4 * F2.len(), fnv(&F2))).unwrap())
    }

    fn hook(_: &mut Cpu) -> bool {
        false
    }

    /// A registered relocatable hand (mod hook point, PC hook) binds into a level's set but not into a front-end set with the
    /// same code; the front end's compiled functions answer `front_overlaps` for mod writes.
    #[test]
    fn registered_hands_do_not_bind_into_the_front_end() {
        static H: RelocHand = RelocHand { name: "test_hook", reloc, est: 0, f: hook };
        register_reloc_hand(&H);
        let mut mem = Mem::new();
        put(&mut mem, BASE, &F2);
        put(&mut mem, BASE + 0x1000, &F2);
        let sets = [(Some(0), set_of(BASE, &[&F2])), (Some(FRONT_LEVEL), set_of(BASE + 0x1000, &[&F2]))];
        let t = Table::build_sets(&mem, 0, &sets);
        assert_eq!((t.sets.clone(), t.hand_len()), (vec![0, 1], 1));
        assert!(t.has_hand(BASE) && !t.has_hand(BASE + 0x1000));
        let (a, b) = (BASE + 0x1000, BASE + 0x1000 + 4 * F2.len() as u32);
        assert!(t.front_overlaps(a, 4) && t.front_overlaps(b - 4, 8) && t.front_overlaps(a - 2, 4));
        assert!(!t.front_overlaps(a - 4, 4) && !t.front_overlaps(b, 4) && !t.front_overlaps(BASE, 8));
    }

    /// A mod write over compiled front-end code rebuilds the table at once; any other write leaves it alone.
    #[test]
    fn a_mod_write_into_the_compiled_front_end_rebuilds_the_table() {
        let mut mem = Mem::new();
        put(&mut mem, BASE, &F1);
        let sets = [(Some(FRONT_LEVEL), set_of(BASE, &[&F1]))];
        let mut c = Cpu::new(mem);
        c.rec = Some(Box::new(Table::build_sets(&c.mem, 0, &sets)));
        assert!(c.rec.as_ref().unwrap().front_in_use());
        c.rec_front_written(BASE + 0x100, 4);
        assert!(!c.rec_unwind && c.rec.as_ref().unwrap().front_in_use(), "outside the front end: nothing");
        c.rec_front_written(BASE + 8, 4);
        assert!(c.rec_unwind, "inside a compiled front-end function: rebuilt, compiled frames unwind");
        assert!(!c.rec.as_ref().unwrap().front_in_use(), "the rebuilt table (the global sets) does not hold the made-up set");
    }
}
