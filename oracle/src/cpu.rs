//! R5900 interpreter: integer + MMI, COP0, FPU (COP1), VU0 macro mode (COP2).
//! Semantics follow PCSX2's interpreter (register upper halves, LWL/LWR
//! tables, divide-by-zero results); floats follow PCSX2 default rounding.

use crate::fpu;
use crate::mem::Mem;
#[doc(hidden)]
pub use ee_core::cpu::*;
use std::collections::HashMap;


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

/// `#[repr(C)]`: the fields up to `mem` (and `Mem`'s first fields) are ee-core's `Cpu`, the view the generated
/// code works on (`core_mut`); lib.rs checks the offsets at compile time.
#[repr(C)]
pub struct Cpu {
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
    pub icount: u64,
    pub div_csr: u32,
    pub vsync_armed: bool,
    pub stop: Option<String>,
    /// Cold boot (`cold`): the kernel HLE that replaces the BIOS (or only hook rules); None for savestate runs.
    pub cold: Option<Box<crate::cold::Kernel>>,
    /// Recompiled functions (rec/), when enabled.
    pub rec: Option<Box<crate::rec::Table>>,
    /// [PC] (speed) Moved up from after `cp0` so ee-core's `Cpu` view can test it inline (`Cpu::hand`): `#[repr(C)]` order is
    /// layout, `rec/mod.rs` asserts the offset. One bit per code word of the 32 MB RAM: the pcs `stub` must see.
    pub stub_bits: Vec<u64>,
    pub mem: Mem,
    pub cp0: [u32; 32],
    pub stubs: HashMap<u32, Stub>,
    pub stub_hits: HashMap<u32, u64>,
    pub syscalls: HashMap<i32, u64>,
    pub sema_next: u32,
    pub ring: [u32; 64],
    pub ri: usize,
    pub vu_calls: HashMap<u32, u64>,
    /// SetVSyncFlag pointers (flag, csr), written at each vblank start.
    pub vsync: Option<(u32, u32)>,
    pub irqs: [u64; 2],
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
    /// Native one-joint skinning loop (skinff.rs).
    pub skinff: crate::skinff::SkinFf,
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
    /// ee-core's view of this CPU (the generated code's `Cpu`): the same memory, the first fields.
    #[inline(always)]
    pub fn core(&self) -> &ee_core::cpu::Cpu {
        unsafe { &*(self as *const Cpu as *const ee_core::cpu::Cpu) }
    }
    #[inline(always)]
    pub fn core_mut(&mut self) -> &mut ee_core::cpu::Cpu {
        unsafe { &mut *(self as *mut Cpu as *mut ee_core::cpu::Cpu) }
    }
    /// The whole CPU behind ee-core's view (`core_mut`; the generated code only ever has views of a whole Cpu).
    #[inline(always)]
    pub fn full(c: &mut ee_core::cpu::Cpu) -> &mut Cpu {
        unsafe { &mut *(c as *mut ee_core::cpu::Cpu as *mut Cpu) }
    }

    // Register helpers, VU macro ops and MMI live in ee-core (cpu.rs there), where the generated code inlines them.
    #[inline(always)] pub fn g(&self, i: usize) -> u64 { self.core().g(i) }
    #[inline(always)] pub fn s(&mut self, i: usize, v: u64) { self.core_mut().s(i, v) }
    #[inline(always)] fn q(&mut self, i: usize, v: u128) { self.core_mut().q(i, v) }
    #[inline(always)] pub fn set_lh(&mut self, lo: u64, hi: u64) { self.core_mut().set_lh(lo, hi) }
    #[inline(always)] pub(crate) fn halt(&mut self, msg: String) { self.core_mut().halt(msg) }
    #[inline(always)] #[doc(hidden)] pub fn vu_out(&mut self, dst: usize, v: [u32; 4], mask: u32, mac: bool) { self.core_mut().vu_out(dst, v, mask, mac) }
    #[inline(always)] pub fn fmac(&self, kind: u8, s: [u32; 4], t: [u32; 4]) -> [u32; 4] { self.core().fmac(kind, s, t) }
    #[inline(always)] pub fn fmac_g<const FAST: bool>(&self, kind: u8, s: [u32; 4], t: [u32; 4]) -> [u32; 4] { self.core().fmac_g::<FAST>(kind, s, t) }
    #[inline(always)] pub fn set_vi(&mut self, i: usize, v: u32) { self.core_mut().set_vi(i, v) }
    #[inline(always)] #[doc(hidden)] pub fn vu_special1(&mut self, op: u32, pc: u32) { self.core_mut().vu_special1(op, pc) }
    #[inline(always)] #[doc(hidden)] pub fn vu_special2(&mut self, op: u32, pc: u32) { self.core_mut().vu_special2(op, pc) }
    #[inline(always)] #[doc(hidden)] pub fn mmi(&mut self, op: u32, pc: u32, rs: usize, rt: usize, rd: usize, sa: u32) { self.core_mut().mmi(op, pc, rs, rt, rd, sa) }

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
            skinff: Default::default(),
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
        self.step_impl::<false>()
    }

    /// One interpreter step; `INL`: the decoder is inlined into the caller (the dispatcher's own loop, `run_rec`).
    #[inline(always)]
    pub(crate) fn step_impl<const INL: bool>(&mut self) {
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
        if INL {
            self.exec_body(op, pc)
        } else {
            self.exec(op, pc)
        }
    }

    /// Takes INT0 (INTC) / INT1 (DMAC) through the BIOS vector at 0x80000200
    /// when Status allows it. The kernel in RAM dispatches to the handlers.
    pub(crate) fn try_irq(&mut self) {
        let (i0, i1) = self.mem.irq_lines();
        self.cp0[13] = (self.cp0[13] & !0xc00) | (i0 as u32) << 10 | (i1 as u32) << 11;
        if !i0 && !i1 {
            self.mem.clear_dirty();
            return;
        }
        let st = self.cp0[12];
        if !Self::irq_taken(st, i0, i1) {
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

    /// Whether `try_irq` takes an interrupt with Status `st` and these lines (INT0, INT1): IE and EIE set, EXL and ERL
    /// clear, and an asserted line's IM bit set.
    #[inline(always)]
    pub(crate) fn irq_taken(st: u32, i0: bool, i1: bool) -> bool {
        st & 1 != 0 && st & 0x1_0000 != 0 && st & 6 == 0 && ((i0 as u32) << 10 | (i1 as u32) << 11) & st != 0
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
        self.mem.mark_dirty();
        // T1 counts BUSCLK/256 (the game's mode 0x82): one PAL field is 147456000 / 256 / 50 = 11520 counts, and the
        // 16-bit counter wraps. A game frame that spans vblanks reads that much more (the long-frame catch-up test).
        // NTSC (the USA disc): 9610 counts a field (`ver::t1_field`).
        self.mem.rcnt1 = (self.mem.rcnt1 + crate::ver::t1_field()) & 0xffff;
    }

    fn stub(&mut self, pc: u32) -> bool {
        if self.cold.is_some() && crate::cold::stub(self, pc) {
            return true;
        }
        if pc == crate::POLL_HEAD {
            self.poll_ff(pc);
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
            if crate::mem::stats() {
                *self.stub_hits.entry(pc).or_insert(0) += 1;
            }
            let ra = self.g(31) as u32;
            self.pc = ra;
            self.npc = ra.wrapping_add(4);
            return true;
        }
        if crate::mem::stats() {
            *self.stub_hits.entry(pc).or_insert(0) += 1;
        }
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
    #[inline(never)]
    pub fn exec(&mut self, op: u32, pc: u32) {
        self.exec_body(op, pc)
    }

    #[inline(always)]
    fn exec_body(&mut self, op: u32, pc: u32) {
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
                    _ => self.bad_insn("REGIMM", op, pc),
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
            28 => self.mmi_interp(op, pc, rs, rt, rd, sa),
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
            _ => self.bad_insn("op", op, pc),
        }
    }

    // [PC] (speed) The interpreter's rare or big paths live in functions of their own, so the dispatcher loop (`run_rec`,
    // which inlines `exec_body`) and `cop2` stay small: MMI and the VU0 macro decoders are inlined into compiled code only
    // (inlining them into the loop measured slower).
    #[inline(never)]
    fn mmi_interp(&mut self, op: u32, pc: u32, rs: usize, rt: usize, rd: usize, sa: u32) {
        self.mmi(op, pc, rs, rt, rd, sa)
    }

    #[inline(never)]
    fn cop2_macro(&mut self, op: u32, pc: u32) {
        if op & 0x3f >= 60 {
            self.vu_special2(op, pc)
        } else {
            self.vu_special1(op, pc)
        }
    }

    #[cold]
    #[inline(never)]
    fn bad_insn(&mut self, kind: &str, op: u32, pc: u32) {
        self.halt(format!("unknown {kind} {op:08x} at {pc:08x}"));
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

    #[inline(never)]
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
                    self.mem.mark_dirty();
                }
                _ => {}
            },
            _ => self.halt(format!("unknown COP0 {op:08x} at {pc:08x}")),
        }
    }

    #[inline(never)]
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

    #[doc(hidden)]
    #[inline(never)]
    fn ctc2_vi(&mut self, rd: usize, v: u32, pc: u32) {
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

    #[inline(never)]
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
                    _ => self.ctc2_vi(rd, v, pc),
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
            16..=31 => self.cop2_macro(op, pc),
            _ => self.bad_insn("COP2", op, pc),
        }
    }

    // ---------------- MMI ----------------

}
