//! R5900 interpreter: integer + MMI, COP0, FPU (COP1), VU0 macro mode (COP2).
//! Semantics follow PCSX2's interpreter (register upper halves, LWL/LWR
//! tables, divide-by-zero results); floats follow PCSX2 default rounding.

use crate::fpu::{self, CSR_CHOP};
use crate::mem::Mem;
use std::collections::HashMap;

const LO64: u128 = u64::MAX as u128;
const HI64: u128 = !LO64;

#[inline(always)]
pub fn sx32(v: u32) -> u64 {
    v as i32 as i64 as u64
}
#[inline(always)]
pub fn w4(x: u128) -> [u32; 4] {
    unsafe { std::mem::transmute(x) }
}
#[inline(always)]
pub fn uw4(x: [u32; 4]) -> u128 {
    unsafe { std::mem::transmute(x) }
}
#[inline(always)]
fn h8(x: u128) -> [u16; 8] {
    unsafe { std::mem::transmute(x) }
}
#[inline(always)]
fn uh8(x: [u16; 8]) -> u128 {
    unsafe { std::mem::transmute(x) }
}
fn mapw(a: u128, b: u128, f: impl Fn(u32, u32) -> u32) -> u128 {
    let (a, b) = (w4(a), w4(b));
    uw4([f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2]), f(a[3], b[3])])
}
fn maph(a: u128, b: u128, f: impl Fn(u16, u16) -> u16) -> u128 {
    let (a, b) = (h8(a), h8(b));
    let mut r = [0u16; 8];
    for i in 0..8 {
        r[i] = f(a[i], b[i]);
    }
    uh8(r)
}
fn mapb(a: u128, b: u128, f: impl Fn(u8, u8) -> u8) -> u128 {
    let (a, b) = (a.to_le_bytes(), b.to_le_bytes());
    let mut r = [0u8; 16];
    for i in 0..16 {
        r[i] = f(a[i], b[i]);
    }
    u128::from_le_bytes(r)
}
fn ssw(v: i64) -> u32 {
    v.clamp(i32::MIN as i64, i32::MAX as i64) as i32 as u32
}
fn ssh(v: i32) -> u16 {
    v.clamp(-32768, 32767) as i16 as u16
}
fn ssb(v: i32) -> u8 {
    v.clamp(-128, 127) as i8 as u8
}

pub const LWL_MASK: [u32; 4] = [0x00ff_ffff, 0x0000_ffff, 0x0000_00ff, 0];
pub const LWR_MASK: [u32; 4] = [0, 0xff00_0000, 0xffff_0000, 0xffff_ff00];
pub const SWL_MASK: [u32; 4] = [0xffff_ff00, 0xffff_0000, 0xff00_0000, 0];
pub const SWR_MASK: [u32; 4] = [0, 0x0000_00ff, 0x0000_ffff, 0x00ff_ffff];
pub const LDL_MASK: [u64; 8] = [
    0x00ff_ffff_ffff_ffff, 0x0000_ffff_ffff_ffff, 0x0000_00ff_ffff_ffff, 0x0000_0000_ffff_ffff,
    0x0000_0000_00ff_ffff, 0x0000_0000_0000_ffff, 0x0000_0000_0000_00ff, 0,
];
pub const LDR_MASK: [u64; 8] = [
    0, 0xff00_0000_0000_0000, 0xffff_0000_0000_0000, 0xffff_ff00_0000_0000,
    0xffff_ffff_0000_0000, 0xffff_ffff_ff00_0000, 0xffff_ffff_ffff_0000, 0xffff_ffff_ffff_ff00,
];
pub const SDL_MASK: [u64; 8] = [
    0xffff_ffff_ffff_ff00, 0xffff_ffff_ffff_0000, 0xffff_ffff_ff00_0000, 0xffff_ffff_0000_0000,
    0xffff_ff00_0000_0000, 0xffff_0000_0000_0000, 0xff00_0000_0000_0000, 0,
];
pub const SDR_MASK: [u64; 8] = [
    0, 0x0000_0000_0000_00ff, 0x0000_0000_0000_ffff, 0x0000_0000_00ff_ffff,
    0x0000_0000_ffff_ffff, 0x0000_00ff_ffff_ffff, 0x0000_ffff_ffff_ffff, 0x00ff_ffff_ffff_ffff,
];

// VU upper-op kinds.
pub const ADD: u8 = 0;
pub const SUB: u8 = 1;
pub const MADD: u8 = 2;
pub const MSUB: u8 = 3;
pub const MAX: u8 = 4;
pub const MIN: u8 = 5;
pub const MUL: u8 = 6;
pub const BC_KIND: [u8; 7] = [ADD, SUB, MADD, MSUB, MAX, MIN, MUL];
/// Special1/special2 28..=39: (kind, uses I instead of Q).
pub const QI: [(u8, bool); 12] = [
    (MUL, false), (MAX, true), (MUL, true), (MIN, true), (ADD, false), (MADD, false),
    (ADD, true), (MADD, true), (SUB, false), (MSUB, false), (SUB, true), (MSUB, true),
];
/// 40..=47 full-vector kinds (46 = OPMSUB/OPMULA handled separately).
pub const FULL: [u8; 8] = [ADD, MADD, MUL, MAX, SUB, MSUB, 255, MIN];

pub struct Stub {
    pub ret: u64,
    pub name: String,
    /// Overlay stubs: the first four code words of the stubbed function (from the level's overlay ELF); the stub
    /// applies only while RAM holds them, because the level number changes before the loader swaps the overlay.
    pub sig: Option<[u32; 4]>,
    /// Host hook (see `add_host_stub`): called with the CPU at the function's entry when the stub applies. Returning
    /// true means the host did the function's work (result registers set): the CPU returns to `ra`. Returning false
    /// leaves the function to run its own code.
    pub host: Option<fn(&mut Cpu) -> bool>,
}

pub struct Cpu {
    pub r: [u128; 32],
    pub hi: u128,
    pub lo: u128,
    pub sa: u32,
    pub pc: u32,
    pub npc: u32,
    pub cp0: [u32; 32],
    pub f: [u32; 32],
    pub facc: u32,
    pub fcr31: u32,
    pub vf: [[u32; 4]; 32],
    pub vacc: [u32; 4],
    pub vi: [u32; 32],
    pub mem: Mem,
    pub icount: u64,
    pub stop: Option<String>,
    pub stub_bits: Vec<u64>,
    pub stubs: HashMap<u32, Stub>,
    pub stub_hits: HashMap<u32, u64>,
    pub syscalls: HashMap<i32, u64>,
    pub sema_next: u32,
    pub div_csr: u32,
    pub ring: [u32; 64],
    pub ri: usize,
    pub vu_calls: HashMap<u32, u64>,
    /// SetVSyncFlag pointers (flag, csr), written at each vblank start.
    pub vsync: Option<(u32, u32)>,
    pub irqs: [u64; 2],
    pub vsync_armed: bool,
    /// Cold boot (`cold`): the kernel HLE that replaces the BIOS (or only hook rules); None for savestate runs.
    pub cold: Option<Box<crate::cold::Kernel>>,
    pub watch: std::collections::HashSet<u32>,
    pub watch_hits: std::collections::BTreeMap<u32, u64>,
    /// ORACLE_CALL: print args at entry and v0 at return of this function.
    pub trace_call: Option<u32>,
    pub trace_left: u32,
    trace_pend: Vec<(u32, u32, [u32; 5])>,
    pub snd: crate::snd989::driver::Snd989,
    /// The pad report `PAD_READ` returns (lib `Machine::set_pad`); None leaves the read to the game code.
    pub pad: Option<[u8; 18]>,
    /// IOP RAM (2 MB) from the savestate; empty when the state has none.
    pub iop: Vec<u8>,
    /// The game disc (2048-byte sectors) for sceCdRead.
    pub iso: Option<std::fs::File>,
    /// Recompiled functions (rec/), when enabled.
    pub rec: Option<Box<crate::rec::Table>>,
    /// The host's frame: run at each armed vblank wait (it raises the vblank); the game then goes on in place
    /// (rec/mod.rs `vblank_hook`). None: the code unwinds to a host loop at the wait.
    pub vblank_cb: Option<Box<dyn FnMut(&mut Cpu) + Send>>,
    /// The frame callback rebuilt the recompiled table: `vblank_hook` unwinds the Rust frames of the compiled
    /// callers, which may be another level's code now (all their state is in the Cpu; the host resumes at the wait).
    pub rec_unwind: bool,
    /// FlushCache ran (new code may be in RAM): the next dispatch re-validates the recompiled table (`rec_flushed`).
    pub rec_flush: bool,
    /// A host loop catches `RecRebuilt` (thread mode), so a rebuild may unwind the compiled frames.
    pub rec_catch: bool,
    /// VU0 microprogram counters: [starts, parks (wait loops entered), resumes (CTC2 kicks), pairs executed,
    /// pc where the last run stopped, how (0 = E bit, 1 = parked, 2 = pair cap)].
    pub vu0_stats: [u64; 6],
    /// Debug: (kind 1 = CTC2 / 2 = VCALLMS / 3 = park / 4 = end, reg or start pc, value, ee pc) of the VU0 handshake.
    pub vu0_trace: Option<Vec<(u8, u32, u32, u32)>>,
    /// The memory cards and mcserv's open files (mc.rs).
    pub mc: crate::mc::Mc,
    /// Native prog0 (vu0aot.rs).
    pub vu0_aot: crate::vu0aot::Vu0Aot,
}

/// Architectural state for the hand-written functions' checks (`Cpu::snapshot` / `restore`): registers, RAM,
/// scratchpad and VU0 memory.
#[derive(Clone)]
pub struct Snap {
    pub r: [u128; 32],
    pub hi: u128,
    pub lo: u128,
    pub sa: u32,
    pub pc: u32,
    pub npc: u32,
    pub f: [u32; 32],
    pub facc: u32,
    pub fcr31: u32,
    pub vf: [[u32; 4]; 32],
    pub vacc: [u32; 4],
    pub vi: [u32; 32],
    pub ram: Vec<u8>,
    pub spr: Vec<u8>,
    pub vu0m: Vec<u8>,
    pub vu0d: Vec<u8>,
}

impl Cpu {
    pub fn snapshot(&self) -> Snap {
        Snap {
            r: self.r,
            hi: self.hi,
            lo: self.lo,
            sa: self.sa,
            pc: self.pc,
            npc: self.npc,
            f: self.f,
            facc: self.facc,
            fcr31: self.fcr31,
            vf: self.vf,
            vacc: self.vacc,
            vi: self.vi,
            ram: self.mem.ram.clone(),
            spr: self.mem.spr.clone(),
            vu0m: self.mem.vu0m.clone(),
            vu0d: self.mem.vu0d.clone(),
        }
    }

    pub fn restore(&mut self, s: &Snap) {
        (self.r, self.hi, self.lo, self.sa, self.pc, self.npc) = (s.r, s.hi, s.lo, s.sa, s.pc, s.npc);
        (self.f, self.facc, self.fcr31, self.vf, self.vacc, self.vi) = (s.f, s.facc, s.fcr31, s.vf, s.vacc, s.vi);
        self.mem.ram.copy_from_slice(&s.ram);
        self.mem.spr.copy_from_slice(&s.spr);
        self.mem.vu0m.copy_from_slice(&s.vu0m);
        self.mem.vu0m_changed();
        self.mem.vu0d.copy_from_slice(&s.vu0d);
    }

    pub fn new(mem: Mem) -> Cpu {
        Cpu {
            r: [0; 32],
            hi: 0,
            lo: 0,
            sa: 0,
            pc: 0,
            npc: 4,
            cp0: [0; 32],
            f: [0; 32],
            facc: 0,
            fcr31: 0,
            vf: [[0; 4]; 32],
            vacc: [0; 4],
            vi: [0; 32],
            mem,
            icount: 0,
            stop: None,
            stub_bits: vec![0; (crate::mem::RAM / 4) / 64],
            stubs: HashMap::new(),
            stub_hits: HashMap::new(),
            syscalls: HashMap::new(),
            sema_next: 0x100,
            div_csr: fpu::CSR_NEAR,
            ring: [0; 64],
            ri: 0,
            vu_calls: HashMap::new(),
            vsync: None,
            irqs: [0; 2],
            vsync_armed: false,
            cold: None,
            rec_unwind: false,
            rec_flush: false,
            rec_catch: false,
            watch: Default::default(),
            watch_hits: Default::default(),
            trace_call: None,
            trace_left: 0,
            trace_pend: Vec::new(),
            snd: Default::default(),
            pad: None,
            iop: Vec::new(),
            iso: None,
            rec: None,
            vblank_cb: None,
            vu0_stats: [0; 6],
            vu0_trace: None,
            mc: Default::default(),
            vu0_aot: crate::vu0aot::Vu0Aot::from_env(),
        }
    }

    /// Counts executions of `addr` (watch_hits) without changing behaviour.
    pub fn add_watch(&mut self, addr: u32) {
        let i = ((addr & 0x01ff_ffff) >> 2) as usize;
        self.stub_bits[i >> 6] |= 1 << (i & 63);
        self.watch.insert(addr);
    }

    pub fn add_stub(&mut self, addr: u32, ret: u64, name: &str) {
        let i = ((addr & 0x01ff_ffff) >> 2) as usize;
        self.stub_bits[i >> 6] |= 1 << (i & 63);
        self.stubs.insert(addr, Stub { ret, name: name.to_string(), sig: None, host: None });
    }

    /// A host hook at a function entry for code the interpreter runs (`Cpu::hand` is only consulted by recompiled
    /// code, so a function in an overlay without crates, like the front end, can only be replaced this way). It applies
    /// only while RAM holds `sig` (the function's first four code words), like an overlay stub.
    pub fn add_host_stub(&mut self, addr: u32, name: &str, sig: [u32; 4], f: fn(&mut Cpu) -> bool) {
        self.add_stub(addr, 0, name);
        let st = self.stubs.get_mut(&addr).unwrap();
        st.sig = Some(sig);
        st.host = Some(f);
    }

    #[inline(always)]
    pub fn g(&self, i: usize) -> u64 {
        self.r[i] as u64
    }
    #[inline(always)]
    pub fn s(&mut self, i: usize, v: u64) {
        if i != 0 {
            self.r[i] = (self.r[i] & HI64) | v as u128;
        }
    }
    #[inline(always)]
    fn q(&mut self, i: usize, v: u128) {
        if i != 0 {
            self.r[i] = v;
        }
    }
    #[inline(always)]
    pub fn set_lh(&mut self, lo: u64, hi: u64) {
        self.lo = (self.lo & HI64) | lo as u128;
        self.hi = (self.hi & HI64) | hi as u128;
    }
    fn set_lh1(&mut self, lo: u64, hi: u64) {
        self.lo = (self.lo & LO64) | ((lo as u128) << 64);
        self.hi = (self.hi & LO64) | ((hi as u128) << 64);
    }
    fn halt(&mut self, msg: String) {
        if self.stop.is_none() {
            self.stop = Some(msg);
        }
    }

    #[inline(always)]
    fn branch(&mut self, cond: bool, pc: u32, op: u32, likely: bool) {
        if cond {
            self.npc = pc.wrapping_add(4).wrapping_add(((op as i16 as i32) << 2) as u32);
        } else if likely {
            self.pc = self.npc;
            self.npc = self.npc.wrapping_add(4);
        }
    }

    pub fn run(&mut self, n: u64) {
        let mut k = n;
        while k > 0 && self.stop.is_none() {
            self.step();
            k -= 1;
        }
    }

    #[inline(always)]
    pub fn step(&mut self) {
        if self.icount >= self.mem.due {
            self.mem.mature(self.icount);
            if self.icount >= self.mem.tick {
                crate::cold::tick(self);
            }
        }
        if self.mem.irq_dirty {
            self.try_irq();
        }
        let pc = self.pc;
        let i = ((pc & 0x01ff_ffff) >> 2) as usize;
        if self.stub_bits[i >> 6] >> (i & 63) & 1 != 0 && self.stub(pc) {
            return;
        }
        self.ring[self.ri & 63] = pc;
        self.ri += 1;
        let op = self.mem.fetch(pc);
        self.pc = self.npc;
        self.npc = self.npc.wrapping_add(4);
        self.icount += 1;
        self.mem.now = self.icount;
        self.mem.pc = pc;
        self.exec(op, pc);
    }

    /// Takes INT0 (INTC) / INT1 (DMAC) through the BIOS vector at 0x80000200
    /// when Status allows it. The kernel in RAM dispatches to the handlers.
    pub(crate) fn try_irq(&mut self) {
        let (i0, i1) = self.mem.irq_lines();
        self.cp0[13] = (self.cp0[13] & !0xc00) | (i0 as u32) << 10 | (i1 as u32) << 11;
        if !i0 && !i1 {
            self.mem.irq_dirty = false;
            return;
        }
        let st = self.cp0[12];
        if st & 1 == 0 || st & 0x1_0000 == 0 || st & 6 != 0 || self.cp0[13] & st & 0xc00 == 0 {
            return;
        }
        self.irqs[if i0 && st & 0x400 != 0 { 0 } else { 1 }] += 1;
        let bd = self.npc != self.pc.wrapping_add(4);
        self.cp0[14] = if bd { self.pc.wrapping_sub(4) } else { self.pc };
        self.cp0[13] = (self.cp0[13] & !0x8000_007c) | if bd { 0x8000_0000 } else { 0 };
        self.cp0[12] |= 2;
        self.pc = 0x8000_0200;
        self.npc = 0x8000_0204;
    }

    /// Vblank start: SetVSyncFlag outputs and INTC VBLANK_S.
    pub fn vblank(&mut self) {
        self.vsync_armed = false;
        // ORACLE_ICOUNT_SKEW=N (default 0): the instruction count (COP0 Count, timers, DMA/interrupt clock) jumps
        // by N once per field (timing probe).
        static SKEW: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
        let skew = *SKEW.get_or_init(|| std::env::var("ORACLE_ICOUNT_SKEW").ok().and_then(|v| v.parse().ok()).unwrap_or(0));
        if skew != 0 {
            self.icount += skew;
            self.mem.now = self.icount;
        }
        if std::env::var("ORACLE_DBG").is_ok() { eprintln!("vblank {:?} pc {:08x} icount {}", self.vsync, self.pc, self.icount); }
        // One-shot, as the BIOS VBLANK_S handler (0x80001660..0x800016a0): it writes the flag and the CSR once and
        // clears the registration.
        if let Some((f, c)) = self.vsync.take() {
            self.mem.wr32(f, 1);
            self.mem.wr64(c, 0x200a);
        }
        // the IOP sound timer runs on the machine's time, not on EE code (989snd field clock, stage 10a)
        self.snd.field();
        let s = self.mem.hw32(crate::mem::INTC_STAT);
        self.mem.set_hw32(crate::mem::INTC_STAT, s | 4);
        self.mem.irq_dirty = true;
        // T1 counts BUSCLK/256 (the game's mode 0x82): one PAL field is 147456000 / 256 / 50 = 11520 counts, and the
        // 16-bit counter wraps. A game frame that spans vblanks reads that much more (the long-frame catch-up test).
        // NTSC (the USA disc): 9610 counts a field (`ver::t1_field`).
        self.mem.rcnt1 = (self.mem.rcnt1 + crate::ver::t1_field()) & 0xffff;
    }

    fn stub(&mut self, pc: u32) -> bool {
        if self.cold.is_some() && crate::cold::stub(self, pc) {
            return true;
        }
        if self.watch.contains(&pc) {
            *self.watch_hits.entry(pc).or_insert(0) += 1;
        }
        if self.trace_call.is_some() {
            let sp = self.g(29) as u32;
            if let Some(k) = self.trace_pend.iter().rposition(|&(ra, s, _)| ra == pc && s == sp) {
                let (_, _, a) = self.trace_pend.remove(k);
                if self.trace_left > 0 {
                    self.trace_left -= 1;
                    eprintln!(
                        "call {:08x}({:08x} {:08x} {:08x} {:08x}) from {:08x} -> {:08x} [{}]",
                        self.trace_call.unwrap(), a[0], a[1], a[2], a[3], a[4], self.g(2) as u32, self.icount
                    );
                }
            }
            if Some(pc) == self.trace_call {
                let ra = self.g(31) as u32;
                let a = [self.g(4) as u32, self.g(5) as u32, self.g(6) as u32, self.g(7) as u32, ra.wrapping_sub(8)];
                self.trace_pend.push((ra, sp, a));
                self.add_watch(ra);
            }
        }
        let Some(st) = self.stubs.get(&pc) else { return false };
        if let Some(sig) = st.sig {
            let w = |k: usize| u32::from_le_bytes(self.mem.ram[pc as usize + 4 * k..][..4].try_into().unwrap());
            if (0..4).any(|k| w(k) != sig[k]) {
                return false;
            }
        }
        let (ret, host) = (st.ret, st.host);
        if let Some(f) = host {
            if !f(self) {
                return false;
            }
            *self.stub_hits.entry(pc).or_insert(0) += 1;
            let ra = self.g(31) as u32;
            self.pc = ra;
            self.npc = ra.wrapping_add(4);
            return true;
        }
        *self.stub_hits.entry(pc).or_insert(0) += 1;
        if pc == 0x11b1c8 {
            self.sif_call_rpc();
        }
        if pc == 0x11aff8 {
            self.sif_bind_rpc();
        }
        if pc == crate::PAD_READ {
            if let Some(p) = self.pad {
                let a = self.g(5) as u32;
                for (k, b) in p.iter().enumerate() {
                    self.mem.wr8(a + k as u32, *b);
                }
            }
        }
        self.s(2, ret);
        let ra = self.g(31) as u32;
        self.pc = ra;
        self.npc = ra.wrapping_add(4);
        true
    }

    #[doc(hidden)]
    pub fn exec(&mut self, op: u32, pc: u32) {
        let rs = ((op >> 21) & 31) as usize;
        let rt = ((op >> 16) & 31) as usize;
        let rd = ((op >> 11) & 31) as usize;
        let sa = (op >> 6) & 31;
        let imm = op as u16 as i16 as i64 as u64;
        let immu = (op & 0xffff) as u64;
        let ea = (self.g(rs) as u32).wrapping_add(imm as u32);
        match op >> 26 {
            0 => self.special(op, pc, rs, rt, rd, sa),
            1 => {
                let v = self.g(rs) as i64;
                match rt {
                    0 => self.branch(v < 0, pc, op, false),
                    1 => self.branch(v >= 0, pc, op, false),
                    2 => self.branch(v < 0, pc, op, true),
                    3 => self.branch(v >= 0, pc, op, true),
                    8..=14 => {}
                    16..=19 => {
                        self.s(31, pc.wrapping_add(8) as u64);
                        let c = if rt & 1 == 0 { v < 0 } else { v >= 0 };
                        self.branch(c, pc, op, rt >= 18);
                    }
                    24 => self.sa = ((self.g(rs) as u32) & 0xf) ^ (immu as u32 & 0xf),
                    25 => self.sa = (((self.g(rs) as u32) & 7) ^ (immu as u32 & 7)) << 1,
                    _ => self.halt(format!("unknown REGIMM {op:08x} at {pc:08x}")),
                }
            }
            2 => self.npc = (self.pc & 0xf000_0000) | ((op & 0x03ff_ffff) << 2),
            3 => {
                self.s(31, pc.wrapping_add(8) as u64);
                self.npc = (self.pc & 0xf000_0000) | ((op & 0x03ff_ffff) << 2);
            }
            4 => self.branch(self.g(rs) == self.g(rt), pc, op, false),
            5 => self.branch(self.g(rs) != self.g(rt), pc, op, false),
            6 => self.branch(self.g(rs) as i64 <= 0, pc, op, false),
            7 => self.branch(self.g(rs) as i64 > 0, pc, op, false),
            8 | 9 => self.s(rt, sx32((self.g(rs) as u32).wrapping_add(imm as u32))),
            10 => self.s(rt, ((self.g(rs) as i64) < imm as i64) as u64),
            11 => self.s(rt, (self.g(rs) < imm) as u64),
            12 => self.s(rt, self.g(rs) & immu),
            13 => self.s(rt, self.g(rs) | immu),
            14 => self.s(rt, self.g(rs) ^ immu),
            15 => self.s(rt, sx32((op & 0xffff) << 16)),
            16 => self.cop0(op, pc, rs, rt, rd),
            17 => self.cop1(op, pc, rs, rt, rd, sa as usize),
            18 => self.cop2(op, pc, rs, rt, rd),
            20 => self.branch(self.g(rs) == self.g(rt), pc, op, true),
            21 => self.branch(self.g(rs) != self.g(rt), pc, op, true),
            22 => self.branch(self.g(rs) as i64 <= 0, pc, op, true),
            23 => self.branch(self.g(rs) as i64 > 0, pc, op, true),
            24 | 25 => self.s(rt, self.g(rs).wrapping_add(imm)),
            26 => {
                let sh = (ea & 7) as usize;
                let m = self.mem.rd64(ea & !7);
                let v = (self.g(rt) & LDL_MASK[sh]) | (m << (56 - sh * 8));
                self.s(rt, v);
            }
            27 => {
                let sh = (ea & 7) as usize;
                let m = self.mem.rd64(ea & !7);
                let v = (self.g(rt) & LDR_MASK[sh]) | (m >> (sh * 8));
                self.s(rt, v);
            }
            28 => self.mmi(op, pc, rs, rt, rd, sa),
            30 => {
                let v = self.mem.rd128(ea & !15);
                self.q(rt, v);
            }
            31 => self.mem.wr128(ea & !15, self.r[rt]),
            32 => {
                let v = self.mem.rd8(ea) as i8 as i64 as u64;
                self.s(rt, v);
            }
            33 => {
                let v = self.mem.rd16(ea) as i16 as i64 as u64;
                self.s(rt, v);
            }
            34 => {
                let sh = (ea & 3) as usize;
                let m = self.mem.rd32(ea & !3);
                let v = ((self.g(rt) as u32) & LWL_MASK[sh]) | (m << (24 - sh * 8));
                self.s(rt, sx32(v));
            }
            35 => {
                let v = sx32(self.mem.rd32(ea));
                self.s(rt, v);
            }
            36 => {
                let v = self.mem.rd8(ea) as u64;
                self.s(rt, v);
            }
            37 => {
                let v = self.mem.rd16(ea) as u64;
                self.s(rt, v);
            }
            38 => {
                let sh = (ea & 3) as usize;
                let m = self.mem.rd32(ea & !3);
                let v = ((self.g(rt) as u32) & LWR_MASK[sh]) | (m >> (sh * 8));
                if sh == 0 {
                    self.s(rt, sx32(v));
                } else {
                    let o = self.g(rt);
                    self.s(rt, (o & !0xffff_ffff) | v as u64);
                }
            }
            39 => {
                let v = self.mem.rd32(ea) as u64;
                self.s(rt, v);
            }
            40 => self.mem.wr8(ea, self.g(rt) as u8),
            41 => self.mem.wr16(ea, self.g(rt) as u16),
            42 => {
                let sh = (ea & 3) as usize;
                let m = self.mem.rd32(ea & !3);
                let v = ((self.g(rt) as u32) >> (24 - sh * 8)) | (m & SWL_MASK[sh]);
                self.mem.wr32(ea & !3, v);
            }
            43 => self.mem.wr32(ea, self.g(rt) as u32),
            44 => {
                let sh = (ea & 7) as usize;
                let m = self.mem.rd64(ea & !7);
                let v = (self.g(rt) >> (56 - sh * 8)) | (m & SDL_MASK[sh]);
                self.mem.wr64(ea & !7, v);
            }
            45 => {
                let sh = (ea & 7) as usize;
                let m = self.mem.rd64(ea & !7);
                let v = (self.g(rt) << (sh * 8)) | (m & SDR_MASK[sh]);
                self.mem.wr64(ea & !7, v);
            }
            46 => {
                let sh = (ea & 3) as usize;
                let m = self.mem.rd32(ea & !3);
                let v = ((self.g(rt) as u32) << (sh * 8)) | (m & SWR_MASK[sh]);
                self.mem.wr32(ea & !3, v);
            }
            47 | 51 => {}
            49 => self.f[rt] = self.mem.rd32(ea),
            54 => {
                let v = w4(self.mem.rd128(ea & !15));
                if rt != 0 {
                    self.vf[rt] = v;
                }
            }
            55 => {
                let v = self.mem.rd64(ea);
                self.s(rt, v);
            }
            57 => self.mem.wr32(ea, self.f[rt]),
            62 => self.mem.wr128(ea & !15, uw4(self.vf[rt])),
            63 => self.mem.wr64(ea, self.g(rt)),
            _ => self.halt(format!("unknown op {op:08x} at {pc:08x}")),
        }
    }

    fn special(&mut self, op: u32, pc: u32, rs: usize, rt: usize, rd: usize, sa: u32) {
        let a = self.g(rs);
        let b = self.g(rt);
        match op & 0x3f {
            0 => self.s(rd, sx32((b as u32) << sa)),
            2 => self.s(rd, sx32((b as u32) >> sa)),
            3 => self.s(rd, sx32(((b as u32 as i32) >> sa) as u32)),
            4 => self.s(rd, sx32((b as u32) << (a & 31))),
            6 => self.s(rd, sx32((b as u32) >> (a & 31))),
            7 => self.s(rd, sx32(((b as u32 as i32) >> (a & 31)) as u32)),
            8 => self.npc = a as u32,
            9 => {
                self.s(rd, pc.wrapping_add(8) as u64);
                self.npc = a as u32;
            }
            10 => {
                if b == 0 {
                    self.s(rd, a)
                }
            }
            11 => {
                if b != 0 {
                    self.s(rd, a)
                }
            }
            12 => self.syscall(pc),
            13 => self.halt(format!("break at {pc:08x}")),
            15 => {}
            16 => self.s(rd, self.hi as u64),
            17 => self.hi = (self.hi & HI64) | a as u128,
            18 => self.s(rd, self.lo as u64),
            19 => self.lo = (self.lo & HI64) | a as u128,
            20 => self.s(rd, b << (a & 63)),
            22 => self.s(rd, b >> (a & 63)),
            23 => self.s(rd, ((b as i64) >> (a & 63)) as u64),
            24 => {
                let p = (a as u32 as i32 as i64) * (b as u32 as i32 as i64);
                self.set_lh(sx32(p as u32), sx32((p >> 32) as u32));
                self.s(rd, sx32(p as u32));
            }
            25 => {
                let p = (a as u32 as u64) * (b as u32 as u64);
                self.set_lh(sx32(p as u32), sx32((p >> 32) as u32));
                self.s(rd, sx32(p as u32));
            }
            26 => {
                let (l, h) = div32(a as u32, b as u32);
                self.set_lh(l, h);
            }
            27 => {
                let (l, h) = divu32(a as u32, b as u32);
                self.set_lh(l, h);
            }
            32 | 33 => self.s(rd, sx32((a as u32).wrapping_add(b as u32))),
            34 | 35 => self.s(rd, sx32((a as u32).wrapping_sub(b as u32))),
            36 => self.s(rd, a & b),
            37 => self.s(rd, a | b),
            38 => self.s(rd, a ^ b),
            39 => self.s(rd, !(a | b)),
            40 => self.s(rd, self.sa as u64),
            41 => self.sa = a as u32,
            42 => self.s(rd, ((a as i64) < (b as i64)) as u64),
            43 => self.s(rd, (a < b) as u64),
            44 | 45 => self.s(rd, a.wrapping_add(b)),
            46 | 47 => self.s(rd, a.wrapping_sub(b)),
            48..=54 => {
                let t = match op & 0x3f {
                    48 => a as i64 >= b as i64,
                    49 => a >= b,
                    50 => (a as i64) < b as i64,
                    51 => a < b,
                    52 => a == b,
                    54 => a != b,
                    _ => false,
                };
                if t {
                    self.halt(format!("trap {op:08x} at {pc:08x}"));
                }
            }
            56 => self.s(rd, b << sa),
            58 => self.s(rd, b >> sa),
            59 => self.s(rd, ((b as i64) >> sa) as u64),
            60 => self.s(rd, b << (sa + 32)),
            62 => self.s(rd, b >> (sa + 32)),
            63 => self.s(rd, ((b as i64) >> (sa + 32)) as u64),
            _ => self.halt(format!("unknown SPECIAL {op:08x} at {pc:08x}")),
        }
    }

    fn cop0(&mut self, op: u32, pc: u32, rs: usize, rt: usize, rd: usize) {
        match rs {
            0 => {
                let v = match rd {
                    9 => self.icount as u32,
                    13 => {
                        let (i0, i1) = self.mem.irq_lines();
                        (self.cp0[13] & !0xc00) | (i0 as u32) << 10 | (i1 as u32) << 11
                    }
                    25 => 0,
                    _ => self.cp0[rd],
                };
                self.s(rt, sx32(v));
            }
            4 => {
                if rd != 25 {
                    self.cp0[rd] = self.g(rt) as u32
                }
            }
            8 => {
                // CPCOND0: DMA always idle.
                let c = true;
                match rt {
                    0 => self.branch(!c, pc, op, false),
                    1 => self.branch(c, pc, op, false),
                    2 => self.branch(!c, pc, op, true),
                    _ => self.branch(c, pc, op, true),
                }
            }
            16 => match op & 0x3f {
                0x38 => self.cp0[12] |= 0x1_0000,
                0x39 => self.cp0[12] &= !0x1_0000,
                0x18 => {
                    let t = if self.cp0[12] & 4 != 0 {
                        self.cp0[12] &= !4;
                        self.cp0[30]
                    } else {
                        self.cp0[12] &= !2;
                        self.cp0[14]
                    };
                    self.pc = t;
                    self.npc = t.wrapping_add(4);
                    self.mem.irq_dirty = true;
                }
                _ => {}
            },
            _ => self.halt(format!("unknown COP0 {op:08x} at {pc:08x}")),
        }
    }

    fn cop1(&mut self, op: u32, pc: u32, rs: usize, ft: usize, fs: usize, fd: usize) {
        match rs {
            0 => self.s(ft, sx32(self.f[fs])),
            2 => {
                let v = match fs {
                    0 => 0x2e30,
                    31 => self.fcr31,
                    _ => 0,
                };
                self.s(ft, sx32(v));
            }
            4 => self.f[fs] = self.g(ft) as u32,
            6 => {
                if fs == 31 {
                    self.fcr31 = self.g(ft) as u32
                }
            }
            8 => {
                let c = self.fcr31 & (1 << 23) != 0;
                match ft {
                    0 => self.branch(!c, pc, op, false),
                    1 => self.branch(c, pc, op, false),
                    2 => self.branch(!c, pc, op, true),
                    _ => self.branch(c, pc, op, true),
                }
            }
            16 => {
                let a = self.f[fs];
                let b = self.f[ft];
                match op & 0x3f {
                    0 => self.f[fd] = fpu::add(a, b),
                    1 => self.f[fd] = fpu::sub(a, b),
                    2 => self.f[fd] = fpu::mul(a, b),
                    3 => self.f[fd] = fpu::div(a, b, self.div_csr),
                    4 => self.f[fd] = fpu::sqrt(b, self.div_csr),
                    5 => self.f[fd] = a & 0x7fff_ffff,
                    6 => self.f[fd] = a,
                    7 => self.f[fd] = a ^ 0x8000_0000,
                    22 => self.f[fd] = fpu::rsqrt(a, b, self.div_csr),
                    24 => self.facc = fpu::add(a, b),
                    25 => self.facc = fpu::sub(a, b),
                    26 => self.facc = fpu::mul(a, b),
                    28 => self.f[fd] = fpu::add(self.facc, fpu::mul(a, b)),
                    29 => self.f[fd] = fpu::sub(self.facc, fpu::mul(a, b)),
                    30 => self.facc = fpu::add(self.facc, fpu::mul(a, b)),
                    31 => self.facc = fpu::sub(self.facc, fpu::mul(a, b)),
                    36 => self.f[fd] = fpu::cvt_w(a),
                    40 => self.f[fd] = fpu::max(a, b),
                    41 => self.f[fd] = fpu::min(a, b),
                    48 | 50 | 52 | 54 => {
                        let (x, y) = (fpu::fz(a), fpu::fz(b));
                        let c = match op & 0x3f {
                            48 => false,
                            50 => x == y,
                            52 => x < y,
                            _ => x <= y,
                        };
                        if c {
                            self.fcr31 |= 1 << 23
                        } else {
                            self.fcr31 &= !(1 << 23)
                        }
                    }
                    _ => self.halt(format!("unknown FPU.S {op:08x} at {pc:08x}")),
                }
            }
            20 if op & 0x3f == 32 => self.f[fd] = fpu::cvt_s(self.f[fs]),
            _ => self.halt(format!("unknown COP1 {op:08x} at {pc:08x}")),
        }
    }

    // ---------------- VU0 macro mode ----------------

    /// Macro-mode upper op: same arithmetic and flags as micro mode (vu.rs
    /// vu_upper), committed at once. MAC/status/clip flags are what CFC2
    /// reads (e.g. the collision range checks in seg_collide 0x1efbf0). [PC] (speed) Inlined into the recompiled
    /// code's `vu_special1` / `vu_special2` calls, where `op` is a constant: the decode in `vu_upper` folds away
    /// (CLIP / ITOF / FTOI / ABS / OPMULA ran the whole decoder per op, ~6% of the EE thread on Blarg).
    #[inline(always)]
    fn vu_macro_upper(&mut self, op: u32, pc: u32) {
        let u = match self.vu_upper(op, self.vi[18]) {
            Ok(u) => u,
            Err(e) => return self.halt(format!("{e} at {pc:08x}")),
        };
        if u.dst == 32 {
            self.awr(u.val, u.mask);
        } else if u.dst < 32 {
            self.vwr(u.dst, u.val, u.mask);
        }
        if let Some(m) = u.mac {
            let st = ((m & 0xf != 0) as u32) | (((m & 0xf0 != 0) as u32) << 1);
            self.vi[17] = m;
            self.vi[16] = (self.vi[16] & !0xf) | st | (st << 6);
        }
        if let Some(c) = u.clip {
            self.vi[18] = c;
        }
    }

    /// A VU0 macro upper result (recompiled code): written to VF `dst` (32 = ACC, above = none) under `mask`;
    /// with `mac`, the MAC flags and status. Same writes as `vu_macro_upper`.
    #[inline(always)]
    #[doc(hidden)]
    pub fn vu_out(&mut self, dst: usize, v: [u32; 4], mask: u32, mac: bool) {
        if dst == 32 {
            self.awr(v, mask);
        } else if dst < 32 {
            self.vwr(dst, v, mask);
        }
        if mac {
            let m = crate::vu::mac_of(v, mask);
            let st = ((m & 0xf != 0) as u32) | (((m & 0xf0 != 0) as u32) << 1);
            self.vi[17] = m;
            self.vi[16] = (self.vi[16] & !0xf) | st | (st << 6);
        }
    }

    #[inline(always)]
    fn vwr(&mut self, d: usize, v: [u32; 4], dest: u32) {
        if d == 0 {
            return;
        }
        for i in 0..4 {
            if dest & (8 >> i) != 0 {
                self.vf[d][i] = v[i];
            }
        }
    }
    #[inline(always)]
    fn awr(&mut self, v: [u32; 4], dest: u32) {
        for i in 0..4 {
            if dest & (8 >> i) != 0 {
                self.vacc[i] = v[i];
            }
        }
    }
    #[inline(always)]
    pub fn fmac(&self, kind: u8, s: [u32; 4], t: [u32; 4]) -> [u32; 4] {
        self.fmac_g::<false>(kind, s, t)
    }
    /// `fmac` with FAST = true: no MXCSR switching per op (a `fpu::ChopGuard` must be alive).
    #[inline(always)]
    pub fn fmac_g<const FAST: bool>(&self, kind: u8, s: [u32; 4], t: [u32; 4]) -> [u32; 4] {
        match kind {
            ADD => fpu::vadd_g::<FAST>(s, t),
            SUB => fpu::vsub_g::<FAST>(s, t),
            MUL => fpu::vmul_g::<FAST>(s, t),
            MADD => fpu::vadd_g::<FAST>(self.vacc, fpu::vmul_g::<FAST>(s, t)),
            MSUB => fpu::vsub_g::<FAST>(self.vacc, fpu::vmul_g::<FAST>(s, t)),
            MAX => [fpu::max(s[0], t[0]), fpu::max(s[1], t[1]), fpu::max(s[2], t[2]), fpu::max(s[3], t[3])],
            _ => [fpu::min(s[0], t[0]), fpu::min(s[1], t[1]), fpu::min(s[2], t[2]), fpu::min(s[3], t[3])],
        }
    }
    fn vu_mem_addr(&self, vi: usize, lane: usize) -> usize {
        ((self.vi[vi] as usize * 16) & 0xff0) + lane * 4
    }
    fn vu_rd(&self, a: usize) -> u32 {
        u32::from_le_bytes(self.mem.vu0d[a..a + 4].try_into().unwrap())
    }
    fn vu_wr(&mut self, a: usize, v: u32) {
        self.mem.vu0d[a..a + 4].copy_from_slice(&v.to_le_bytes());
    }
    pub fn set_vi(&mut self, i: usize, v: u32) {
        if i != 0 {
            self.vi[i] = if i < 16 { v & 0xffff } else { v };
        }
    }

    #[doc(hidden)]
    pub fn cop2(&mut self, op: u32, pc: u32, rs: usize, rt: usize, rd: usize) {
        match rs {
            1 => self.q(rt, uw4(self.vf[rd])),
            2 => {
                // VPU_STAT (vi29): the busy bit while a microprogram waits for the EE (the parked pc stays hidden).
                let v = match rd {
                    20 => self.vi[20] & 0x7f_ffff,
                    29 => self.vi[29] & 0xffff,
                    _ => self.vi[rd],
                };
                self.s(rt, sx32(v));
            }
            5 => {
                if rd != 0 {
                    self.vf[rd] = w4(self.r[rt]);
                }
            }
            6 => {
                let v = self.g(rt) as u32;
                match rd {
                    0 | 17 => {}
                    // TPC and VPU_STAT are read-only.
                    26 | 29 => {}
                    20 => self.vi[20] = 0x3f80_0000 | (v & 0x7f_ffff),
                    28 => {}
                    31 => {
                        *self.syscalls.entry(-0x1000).or_insert(0) += 1;
                    }
                    _ => {
                        self.set_vi(rd, v);
                        if let Some(t) = self.vu0_trace.as_mut() {
                            t.push((1, rd as u32, v, pc));
                        }
                        // A microprogram parked in a wait loop re-tests its condition (the EE's handshake).
                        if self.vu0_parked() {
                            if let Err(e) = self.vu0_resume() {
                                self.halt(format!("{e} (CTC2 vi{rd} at {pc:08x})"));
                            }
                        }
                    }
                }
            }
            8 => {
                // VU0 busy only while a microprogram waits for the EE: BC2F taken when idle, BC2T when busy.
                let c = self.vu0_parked();
                match rt {
                    0 => self.branch(!c, pc, op, false),
                    1 => self.branch(c, pc, op, false),
                    2 => self.branch(!c, pc, op, true),
                    _ => self.branch(c, pc, op, true),
                }
            }
            16..=31 => {
                if op & 0x3f >= 60 {
                    self.vu_special2(op, pc)
                } else {
                    self.vu_special1(op, pc)
                }
            }
            _ => self.halt(format!("unknown COP2 {op:08x} at {pc:08x}")),
        }
    }

    #[doc(hidden)]
    #[inline(always)]
    pub fn vu_special1(&mut self, op: u32, pc: u32) {
        let ft = ((op >> 16) & 31) as usize;
        let fs = ((op >> 11) & 31) as usize;
        let fd = ((op >> 6) & 31) as usize;
        let f = (op & 0x3f) as usize;
        match f {
            0..=47 => self.vu_macro_upper(op, pc),
            48 => self.set_vi(fd, self.vi[fs].wrapping_add(self.vi[ft])),
            49 => self.set_vi(fd, self.vi[fs].wrapping_sub(self.vi[ft])),
            50 => {
                let im = ((((op >> 6) & 31) as i32) << 27 >> 27) as u32;
                self.set_vi(ft, self.vi[fs].wrapping_add(im));
            }
            52 => self.set_vi(fd, self.vi[fs] & self.vi[ft]),
            53 => self.set_vi(fd, self.vi[fs] | self.vi[ft]),
            56 | 57 => {
                let a = if f == 56 { (op >> 6) & 0x7fff } else { self.vi[27] };
                *self.vu_calls.entry(a * 8).or_insert(0) += 1;
                if let Err(e) = self.vu0_micro(a * 8) {
                    self.halt(format!("{e} (VCALLMS from {pc:08x})"));
                }
            }
            _ => self.halt(format!("unknown VU special1 {op:08x} at {pc:08x}")),
        }
    }

    #[doc(hidden)]
    #[inline(always)]
    pub fn vu_special2(&mut self, op: u32, pc: u32) {
        let dest = (op >> 21) & 15;
        let ft = ((op >> 16) & 31) as usize;
        let fs = ((op >> 11) & 31) as usize;
        let f = ((op & 3) | ((op >> 4) & 0x7c)) as usize;
        let s = self.vf[fs];
        let fsf = ((op >> 21) & 3) as usize;
        let ftf = ((op >> 23) & 3) as usize;
        match f {
            0..=46 => self.vu_macro_upper(op, pc),
            47 | 59 => {}
            48 => self.vwr(ft, s, dest),
            49 => self.vwr(ft, [s[1], s[2], s[3], s[0]], dest),
            52 | 54 => {
                let is = fs;
                if f == 54 {
                    self.set_vi(is, self.vi[is].wrapping_sub(1));
                }
                let a = self.vu_mem_addr(is, 0);
                let v = [self.vu_rd(a), self.vu_rd(a + 4), self.vu_rd(a + 8), self.vu_rd(a + 12)];
                self.vwr(ft, v, dest);
                if f == 52 {
                    self.set_vi(is, self.vi[is].wrapping_add(1));
                }
            }
            53 | 55 => {
                let it = ft;
                if f == 55 {
                    self.set_vi(it, self.vi[it].wrapping_sub(1));
                }
                let a = self.vu_mem_addr(it, 0);
                for i in 0..4 {
                    if dest & (8 >> i) != 0 {
                        self.vu_wr(a + i * 4, s[i]);
                    }
                }
                if f == 53 {
                    self.set_vi(it, self.vi[it].wrapping_add(1));
                }
            }
            56 => self.vi[22] = fpu::div(s[fsf], self.vf[ft][ftf], CSR_CHOP),
            57 => self.vi[22] = fpu::sqrt(self.vf[ft][ftf], CSR_CHOP),
            58 => self.vi[22] = fpu::rsqrt(s[fsf], self.vf[ft][ftf], CSR_CHOP),
            60 => self.set_vi(ft, s[fsf] & 0xffff),
            61 => {
                let v = self.vi[fs] as u16 as i16 as i32 as u32;
                self.vwr(ft, [v; 4], dest);
            }
            62 => {
                let lane = (0..4).find(|i| dest & (8 >> i) != 0).unwrap_or(0);
                let a = self.vu_mem_addr(fs, lane);
                let v = self.vu_rd(a) & 0xffff;
                self.set_vi(ft, v);
            }
            63 => {
                let v = self.vi[ft] & 0xffff;
                for i in 0..4 {
                    if dest & (8 >> i) != 0 {
                        let a = self.vu_mem_addr(fs, i);
                        self.vu_wr(a, v);
                    }
                }
            }
            64 | 65 => {
                if f == 64 {
                    let r = self.vi[20];
                    let x = (r >> 4) & 1;
                    let y = (r >> 22) & 1;
                    let r = (r << 1) ^ x ^ y;
                    self.vi[20] = (r & 0x7f_ffff) | 0x3f80_0000;
                }
                let r = self.vi[20];
                self.vwr(ft, [r; 4], dest);
            }
            66 => self.vi[20] = 0x3f80_0000 | (s[fsf] & 0x7f_ffff),
            67 => self.vi[20] = 0x3f80_0000 | ((self.vi[20] ^ s[fsf]) & 0x7f_ffff),
            _ => self.halt(format!("unknown VU special2 {op:08x} at {pc:08x}")),
        }
    }

    // ---------------- MMI ----------------

    #[doc(hidden)]
    // Inlined: recompiled code calls these with constant operands, so each call site folds to its one operation
    // instead of a decode at run time (same code, same results).
    #[inline(always)]
    pub fn mmi(&mut self, op: u32, pc: u32, rs: usize, rt: usize, rd: usize, sa: u32) {
        let (a, b) = (self.r[rs], self.r[rt]);
        match op & 0x3f {
            0 | 1 | 32 | 33 => {
                let up = op & 0x3f >= 32;
                let sh = if up { 64 } else { 0 };
                let acc = ((self.lo >> sh) as u32 as u64) | (((self.hi >> sh) as u32 as u64) << 32);
                let (x, y) = (if up { w4(a)[2] } else { a as u32 }, if up { w4(b)[2] } else { b as u32 });
                let p = if op & 1 == 0 {
                    (acc as i64).wrapping_add((x as i32 as i64) * (y as i32 as i64)) as u64
                } else {
                    acc.wrapping_add((x as u64) * (y as u64))
                };
                let (l, h) = (sx32(p as u32), sx32((p >> 32) as u32));
                if up {
                    self.set_lh1(l, h)
                } else {
                    self.set_lh(l, h)
                }
                self.s(rd, l);
            }
            4 => {
                if rd != 0 {
                    let w = w4(a);
                    let mut r = w4(self.r[rd]);
                    for n in 0..2 {
                        let v = w[n] as i32;
                        let v = if v < 0 { !v } else { v };
                        r[n] = if v == 0 { 31 } else { v.leading_zeros() - 1 };
                    }
                    self.r[rd] = uw4(r);
                }
            }
            8 => self.mmi0(op, pc, a, b, rd, sa),
            9 => self.mmi2(op, pc, a, b, rs, rd, sa),
            16 => self.s(rd, (self.hi >> 64) as u64),
            17 => self.hi = (self.hi & LO64) | ((a as u64 as u128) << 64),
            18 => self.s(rd, (self.lo >> 64) as u64),
            19 => self.lo = (self.lo & LO64) | ((a as u64 as u128) << 64),
            24 => {
                let p = (a as u32 as i32 as i64) * (b as u32 as i32 as i64);
                self.set_lh1(sx32(p as u32), sx32((p >> 32) as u32));
                self.s(rd, sx32(p as u32));
            }
            25 => {
                let p = (a as u32 as u64) * (b as u32 as u64);
                self.set_lh1(sx32(p as u32), sx32((p >> 32) as u32));
                self.s(rd, sx32(p as u32));
            }
            26 => {
                let (l, h) = div32(a as u32, b as u32);
                self.set_lh1(l, h);
            }
            27 => {
                let (l, h) = divu32(a as u32, b as u32);
                self.set_lh1(l, h);
            }
            40 => self.mmi1(op, pc, a, b, rd, sa),
            41 => self.mmi3(op, pc, a, b, rs, rd, sa),
            48 => {
                let (l, h) = (w4(self.lo), w4(self.hi));
                let v = match sa {
                    0 => uw4([l[0], h[0], l[2], h[2]]),
                    1 => uw4([l[1], h[1], l[3], h[3]]),
                    2 => {
                        let x = ((h[0] as u64) << 32 | l[0] as u64) as i64;
                        let y = ((h[2] as u64) << 32 | l[2] as u64) as i64;
                        (sx32(ssw(x)) as u128) | ((sx32(ssw(y)) as u128) << 64)
                    }
                    3 => {
                        let (l, h) = (h8(self.lo), h8(self.hi));
                        uh8([l[0], l[2], h[0], h[2], l[4], l[6], h[4], h[6]])
                    }
                    4 => {
                        let (l, h) = (w4(self.lo), w4(self.hi));
                        let c = |v: u32| ssh(v as i32);
                        uh8([c(l[0]), c(l[1]), c(h[0]), c(h[1]), c(l[2]), c(l[3]), c(h[2]), c(h[3])])
                    }
                    _ => {
                        self.halt(format!("unknown PMFHL {op:08x} at {pc:08x}"));
                        return;
                    }
                };
                self.q(rd, v);
            }
            49 => {
                if sa == 0 {
                    let w = w4(a);
                    let (mut l, mut h) = (w4(self.lo), w4(self.hi));
                    l[0] = w[0];
                    h[0] = w[1];
                    l[2] = w[2];
                    h[2] = w[3];
                    self.lo = uw4(l);
                    self.hi = uw4(h);
                } else {
                    self.halt(format!("unknown PMTHL {op:08x} at {pc:08x}"));
                }
            }
            52 => self.q(rd, maph(b, 0, |x, _| x << (sa & 15))),
            54 => self.q(rd, maph(b, 0, |x, _| x >> (sa & 15))),
            55 => self.q(rd, maph(b, 0, |x, _| ((x as i16) >> (sa & 15)) as u16)),
            60 => self.q(rd, mapw(b, 0, |x, _| x << sa)),
            62 => self.q(rd, mapw(b, 0, |x, _| x >> sa)),
            63 => self.q(rd, mapw(b, 0, |x, _| ((x as i32) >> sa) as u32)),
            _ => self.halt(format!("unknown MMI {op:08x} at {pc:08x}")),
        }
    }

    #[inline(always)]
    fn mmi0(&mut self, op: u32, pc: u32, a: u128, b: u128, rd: usize, sa: u32) {
        let v = match sa {
            0 => mapw(a, b, |x, y| x.wrapping_add(y)),
            1 => mapw(a, b, |x, y| x.wrapping_sub(y)),
            2 => mapw(a, b, |x, y| if (x as i32) > (y as i32) { !0 } else { 0 }),
            3 => mapw(a, b, |x, y| if (x as i32) > (y as i32) { x } else { y }),
            4 => maph(a, b, |x, y| x.wrapping_add(y)),
            5 => maph(a, b, |x, y| x.wrapping_sub(y)),
            6 => maph(a, b, |x, y| if (x as i16) > (y as i16) { !0 } else { 0 }),
            7 => maph(a, b, |x, y| if (x as i16) > (y as i16) { x } else { y }),
            8 => mapb(a, b, |x, y| x.wrapping_add(y)),
            9 => mapb(a, b, |x, y| x.wrapping_sub(y)),
            10 => mapb(a, b, |x, y| if (x as i8) > (y as i8) { !0 } else { 0 }),
            16 => mapw(a, b, |x, y| ssw(x as i32 as i64 + y as i32 as i64)),
            17 => mapw(a, b, |x, y| ssw(x as i32 as i64 - y as i32 as i64)),
            18 => {
                let (x, y) = (w4(a), w4(b));
                uw4([y[0], x[0], y[1], x[1]])
            }
            19 => {
                let (x, y) = (w4(a), w4(b));
                uw4([y[0], y[2], x[0], x[2]])
            }
            20 => maph(a, b, |x, y| ssh(x as i16 as i32 + y as i16 as i32)),
            21 => maph(a, b, |x, y| ssh(x as i16 as i32 - y as i16 as i32)),
            22 => {
                let (x, y) = (h8(a), h8(b));
                uh8([y[0], x[0], y[1], x[1], y[2], x[2], y[3], x[3]])
            }
            23 => {
                let (x, y) = (h8(a), h8(b));
                uh8([y[0], y[2], y[4], y[6], x[0], x[2], x[4], x[6]])
            }
            24 => mapb(a, b, |x, y| ssb(x as i8 as i32 + y as i8 as i32)),
            25 => mapb(a, b, |x, y| ssb(x as i8 as i32 - y as i8 as i32)),
            26 => {
                let (x, y) = (a.to_le_bytes(), b.to_le_bytes());
                let mut r = [0u8; 16];
                for i in 0..8 {
                    r[2 * i] = y[i];
                    r[2 * i + 1] = x[i];
                }
                u128::from_le_bytes(r)
            }
            27 => {
                let (x, y) = (a.to_le_bytes(), b.to_le_bytes());
                let mut r = [0u8; 16];
                for i in 0..8 {
                    r[i] = y[2 * i];
                    r[8 + i] = x[2 * i];
                }
                u128::from_le_bytes(r)
            }
            30 => mapw(b, 0, |x, _| (x & 0x1f) << 3 | (x >> 5 & 0x1f) << 11 | (x >> 10 & 0x1f) << 19 | (x >> 15 & 1) << 31),
            31 => mapw(b, 0, |x, _| x >> 3 & 0x1f | (x >> 11 & 0x1f) << 5 | (x >> 19 & 0x1f) << 10 | (x >> 31) << 15),
            _ => {
                self.halt(format!("unknown MMI0 {op:08x} at {pc:08x}"));
                return;
            }
        };
        self.q(rd, v);
    }

    #[inline(always)]
    fn mmi1(&mut self, op: u32, pc: u32, a: u128, b: u128, rd: usize, sa: u32) {
        let v = match sa {
            1 => mapw(0, b, |_, y| if y == 0x8000_0000 { 0x7fff_ffff } else { (y as i32).unsigned_abs() }),
            2 => mapw(a, b, |x, y| if x == y { !0 } else { 0 }),
            3 => mapw(a, b, |x, y| if (x as i32) < (y as i32) { x } else { y }),
            5 => maph(0, b, |_, y| if y == 0x8000 { 0x7fff } else { (y as i16).unsigned_abs() }),
            6 => maph(a, b, |x, y| if x == y { !0 } else { 0 }),
            7 => maph(a, b, |x, y| if (x as i16) < (y as i16) { x } else { y }),
            10 => mapb(a, b, |x, y| if x == y { !0 } else { 0 }),
            16 => mapw(a, b, |x, y| x.saturating_add(y)),
            17 => mapw(a, b, |x, y| x.saturating_sub(y)),
            18 => {
                let (x, y) = (w4(a), w4(b));
                uw4([y[2], x[2], y[3], x[3]])
            }
            20 => maph(a, b, |x, y| x.saturating_add(y)),
            21 => maph(a, b, |x, y| x.saturating_sub(y)),
            22 => {
                let (x, y) = (h8(a), h8(b));
                uh8([y[4], x[4], y[5], x[5], y[6], x[6], y[7], x[7]])
            }
            24 => mapb(a, b, |x, y| x.saturating_add(y)),
            25 => mapb(a, b, |x, y| x.saturating_sub(y)),
            26 => {
                let (x, y) = (a.to_le_bytes(), b.to_le_bytes());
                let mut r = [0u8; 16];
                for i in 0..8 {
                    r[2 * i] = y[8 + i];
                    r[2 * i + 1] = x[8 + i];
                }
                u128::from_le_bytes(r)
            }
            27 => {
                let s = (self.sa & 15) * 8;
                if s == 0 {
                    b
                } else {
                    (b >> s) | (a << (128 - s))
                }
            }
            _ => {
                self.halt(format!("unknown MMI1 {op:08x} at {pc:08x}"));
                return;
            }
        };
        self.q(rd, v);
    }

    #[inline(always)]
    fn mmi2(&mut self, op: u32, pc: u32, a: u128, b: u128, rs: usize, rd: usize, sa: u32) {
        let _ = rs;
        let v = match sa {
            0 | 4 | 12 => {
                let (x, y) = (w4(a), w4(b));
                let (l, h) = (w4(self.lo), w4(self.hi));
                let mut out = [0u64; 2];
                let (mut nl, mut nh) = ([0u64; 2], [0u64; 2]);
                for n in 0..2 {
                    let p = (x[2 * n] as i32 as i64) * (y[2 * n] as i32 as i64);
                    let acc = ((h[2 * n] as u64) << 32 | l[2 * n] as u64) as i64;
                    let r = match sa {
                        0 => acc.wrapping_add(p),
                        4 => acc.wrapping_sub(p),
                        _ => p,
                    };
                    out[n] = r as u64;
                    nl[n] = sx32(r as u32);
                    nh[n] = sx32((r >> 32) as u32);
                }
                self.lo = nl[0] as u128 | (nl[1] as u128) << 64;
                self.hi = nh[0] as u128 | (nh[1] as u128) << 64;
                out[0] as u128 | (out[1] as u128) << 64
            }
            2 | 3 => {
                let (x, y) = (w4(a), w4(b));
                let f = |v: u32, s: u32| if sa == 2 { v << (s & 31) } else { v >> (s & 31) };
                sx32(f(y[0], x[0])) as u128 | (sx32(f(y[2], x[2])) as u128) << 64
            }
            8 => self.hi,
            9 => self.lo,
            10 => {
                let (x, y) = (h8(a), h8(b));
                uh8([y[0], x[4], y[1], x[5], y[2], x[6], y[3], x[7]])
            }
            13 => {
                let (x, y) = (w4(a), w4(b));
                let (mut nl, mut nh) = ([0u64; 2], [0u64; 2]);
                for n in 0..2 {
                    let (q, r) = div32(x[2 * n], y[2 * n]);
                    nl[n] = q;
                    nh[n] = r;
                }
                self.lo = nl[0] as u128 | (nl[1] as u128) << 64;
                self.hi = nh[0] as u128 | (nh[1] as u128) << 64;
                return;
            }
            14 => (b & LO64) | (a << 64),
            16 | 28 => {
                let (x, y) = (h8(a), h8(b));
                let (mut l, mut h) = (w4(self.lo), w4(self.hi));
                let p = |i: usize| (x[i] as i16 as i32).wrapping_mul(y[i] as i16 as i32) as u32;
                let acc = sa == 16;
                let add = |o: u32, v: u32| if acc { o.wrapping_add(v) } else { v };
                l[0] = add(l[0], p(0));
                l[1] = add(l[1], p(1));
                h[0] = add(h[0], p(2));
                h[1] = add(h[1], p(3));
                l[2] = add(l[2], p(4));
                l[3] = add(l[3], p(5));
                h[2] = add(h[2], p(6));
                h[3] = add(h[3], p(7));
                self.lo = uw4(l);
                self.hi = uw4(h);
                uw4([l[0], h[0], l[2], h[2]])
            }
            18 => a & b,
            19 => a ^ b,
            26 => {
                let y = h8(b);
                uh8([y[2], y[1], y[0], y[3], y[6], y[5], y[4], y[7]])
            }
            27 => {
                let y = h8(b);
                uh8([y[3], y[2], y[1], y[0], y[7], y[6], y[5], y[4]])
            }
            30 => {
                let y = w4(b);
                uw4([y[2], y[1], y[0], y[3]])
            }
            31 => {
                let y = w4(b);
                uw4([y[1], y[2], y[0], y[3]])
            }
            _ => {
                self.halt(format!("unknown MMI2 {op:08x} at {pc:08x}"));
                return;
            }
        };
        self.q(rd, v);
    }

    #[inline(always)]
    fn mmi3(&mut self, op: u32, pc: u32, a: u128, b: u128, rs: usize, rd: usize, sa: u32) {
        let _ = rs;
        let v = match sa {
            0 | 12 => {
                let (x, y) = (w4(a), w4(b));
                let (l, h) = (w4(self.lo), w4(self.hi));
                let mut out = [0u64; 2];
                let (mut nl, mut nh) = ([0u64; 2], [0u64; 2]);
                for n in 0..2 {
                    let p = (x[2 * n] as u64) * (y[2 * n] as u64);
                    let acc = (h[2 * n] as u64) << 32 | l[2 * n] as u64;
                    let r = if sa == 0 { acc.wrapping_add(p) } else { p };
                    out[n] = r;
                    nl[n] = sx32(r as u32);
                    nh[n] = sx32((r >> 32) as u32);
                }
                self.lo = nl[0] as u128 | (nl[1] as u128) << 64;
                self.hi = nh[0] as u128 | (nh[1] as u128) << 64;
                out[0] as u128 | (out[1] as u128) << 64
            }
            3 => {
                let (x, y) = (w4(a), w4(b));
                let f = |v: u32, s: u32| ((v as i32) >> (s & 31)) as u32;
                sx32(f(y[0], x[0])) as u128 | (sx32(f(y[2], x[2])) as u128) << 64
            }
            8 => {
                self.hi = a;
                return;
            }
            9 => {
                self.lo = a;
                return;
            }
            10 => {
                let (x, y) = (h8(a), h8(b));
                uh8([y[0], x[0], y[2], x[2], y[4], x[4], y[6], x[6]])
            }
            13 => {
                let (x, y) = (w4(a), w4(b));
                let (mut nl, mut nh) = ([0u64; 2], [0u64; 2]);
                for n in 0..2 {
                    let (q, r) = divu32(x[2 * n], y[2 * n]);
                    nl[n] = q;
                    nh[n] = r;
                }
                self.lo = nl[0] as u128 | (nl[1] as u128) << 64;
                self.hi = nh[0] as u128 | (nh[1] as u128) << 64;
                return;
            }
            14 => (a >> 64) | (b & HI64),
            18 => a | b,
            19 => !(a | b),
            26 => {
                let y = h8(b);
                uh8([y[0], y[2], y[1], y[3], y[4], y[6], y[5], y[7]])
            }
            27 => {
                let y = h8(b);
                uh8([y[0], y[0], y[0], y[0], y[4], y[4], y[4], y[4]])
            }
            30 => {
                let y = w4(b);
                uw4([y[0], y[2], y[1], y[3]])
            }
            _ => {
                self.halt(format!("unknown MMI3 {op:08x} at {pc:08x}"));
                return;
            }
        };
        self.q(rd, v);
    }
}

#[inline]
pub fn div32(a: u32, b: u32) -> (u64, u64) {
    let (a, b) = (a as i32, b as i32);
    if a == i32::MIN && b == -1 {
        (sx32(0x8000_0000), 0)
    } else if b != 0 {
        (sx32(a.wrapping_div(b) as u32), sx32(a.wrapping_rem(b) as u32))
    } else {
        (if a < 0 { 1 } else { u64::MAX }, sx32(a as u32))
    }
}

#[inline]
pub fn divu32(a: u32, b: u32) -> (u64, u64) {
    if b != 0 {
        (sx32(a / b), sx32(a % b))
    } else {
        (u64::MAX, sx32(a))
    }
}
