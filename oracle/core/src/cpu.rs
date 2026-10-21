//! The CPU state the generated code works on (a view of ee-oracle's `Cpu`: its first fields, same layout) and the
//! operations it inlines: register access, VU0 macro ops, MMI. Moved verbatim from ee-oracle cpu.rs.

use crate::fpu::{self, CSR_CHOP};
use crate::mem::Mem;

#[doc(hidden)]
pub const LO64: u128 = u64::MAX as u128;
pub const HI64: u128 = !LO64;

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
pub fn h8(x: u128) -> [u16; 8] {
    unsafe { std::mem::transmute(x) }
}
#[inline(always)]
pub fn uh8(x: [u16; 8]) -> u128 {
    unsafe { std::mem::transmute(x) }
}
pub fn mapw(a: u128, b: u128, f: impl Fn(u32, u32) -> u32) -> u128 {
    let (a, b) = (w4(a), w4(b));
    uw4([f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2]), f(a[3], b[3])])
}
pub fn maph(a: u128, b: u128, f: impl Fn(u16, u16) -> u16) -> u128 {
    let (a, b) = (h8(a), h8(b));
    let mut r = [0u16; 8];
    for i in 0..8 {
        r[i] = f(a[i], b[i]);
    }
    uh8(r)
}
pub fn mapb(a: u128, b: u128, f: impl Fn(u8, u8) -> u8) -> u128 {
    let (a, b) = (a.to_le_bytes(), b.to_le_bytes());
    let mut r = [0u8; 16];
    for i in 0..16 {
        r[i] = f(a[i], b[i]);
    }
    u128::from_le_bytes(r)
}
pub fn ssw(v: i64) -> u32 {
    v.clamp(i32::MIN as i64, i32::MAX as i64) as i32 as u32
}
pub fn ssh(v: i32) -> u16 {
    v.clamp(-32768, 32767) as i16 as u16
}
pub fn ssb(v: i32) -> u8 {
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

/// The first fields of ee-oracle's `Cpu`, in its order (both `#[repr(C)]`; ee-oracle checks every offset at
/// compile time). Only ever a view: ee-oracle passes `&mut` of its own Cpu (`Cpu::core_mut`), never one of these.
#[repr(C)]
#[non_exhaustive]
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
    /// ee-oracle: `Option<Box<cold::Kernel>>` (only tested for presence here).
    pub cold: Option<std::ptr::NonNull<u8>>,
    /// ee-oracle: `Option<Box<rec::Table>>`, whose first fields are `RecHot`.
    pub rec: Option<std::ptr::NonNull<crate::rec::RecHot>>,
    /// ee-oracle's `stub_bits` (one bit per code word of the RAM, index `(pc & 0x1ff_ffff) >> 2`): `hand` tests it inline.
    pub stub_bits: Vec<u64>,
    pub mem: Mem,
}

// The slow paths: ee-oracle defines these (#[no_mangle], rec/mod.rs), on the whole Cpu behind the view.
extern "Rust" {
    fn ee_core_cold_entry(c: &mut Cpu, entry: u32) -> bool;
    fn ee_core_run_hand(c: &mut Cpu, entry: u32) -> bool;
    fn ee_core_poll_slow(c: &mut Cpu, n: u64) -> bool;
    fn ee_core_hw(c: &mut Cpu, op: u32, pc: u32, n: u64, next: u32);
    fn ee_core_slow(c: &mut Cpu, last: u32, brl: u32) -> bool;
    fn ee_core_call_dyn(c: &mut Cpu, t: u32, ret: u32);
    fn ee_core_tail(c: &mut Cpu);
    fn ee_core_vblank_hook(c: &mut Cpu) -> bool;
    fn ee_core_exec(c: &mut Cpu, op: u32, pc: u32);
    fn ee_core_cop2(c: &mut Cpu, op: u32, pc: u32, rs: usize, rt: usize, rd: usize);
    fn ee_core_vcallms(c: &mut Cpu, a: u32, pc: u32);
    fn ee_core_ff(c: &mut Cpu, kind: u32, pc: u32) -> bool;
}

impl Cpu {
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
    pub fn q(&mut self, i: usize, v: u128) {
        if i != 0 {
            self.r[i] = v;
        }
    }

    #[inline(always)]
    pub fn set_lh(&mut self, lo: u64, hi: u64) {
        self.lo = (self.lo & HI64) | lo as u128;
        self.hi = (self.hi & HI64) | hi as u128;
    }

    pub fn set_lh1(&mut self, lo: u64, hi: u64) {
        self.lo = (self.lo & LO64) | ((lo as u128) << 64);
        self.hi = (self.hi & LO64) | ((hi as u128) << 64);
    }

    pub fn halt(&mut self, msg: String) {
        if self.stop.is_none() {
            self.stop = Some(msg);
        }
    }

    /// Macro-mode upper op: same arithmetic and flags as micro mode (vu.rs
    /// vu_upper), committed at once. MAC/status/clip flags are what CFC2
    /// reads (e.g. the collision range checks in seg_collide 0x1efbf0). [PC] (speed) Inlined into the recompiled
    /// code's `vu_special1` / `vu_special2` calls, where `op` is a constant: the decode in `vu_upper` folds away
    /// (CLIP / ITOF / FTOI / ABS / OPMULA ran the whole decoder per op, ~6% of the EE thread on Blarg).
    #[inline(always)]
    pub fn vu_macro_upper(&mut self, op: u32, pc: u32) {
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
    pub fn vwr(&mut self, d: usize, v: [u32; 4], dest: u32) {
        if d == 0 {
            return;
        }
        self.vf[d] = blend4(self.vf[d], v, dest);
    }

    #[inline(always)]
    pub fn awr(&mut self, v: [u32; 4], dest: u32) {
        self.vacc = blend4(self.vacc, v, dest);
    }

    /// [PC] (speed) `vu_out` for recompiled COP2 FMAC ops. `F` = 1: MAC and status flags as `vu_out` with `mac`;
    /// 0: no flags (MAX / MIN); 2: only the sticky status bits (6, 7), for a result whose MAC flags and status bits 0-3
    /// the recompiler knows are overwritten by a later FMAC op of the same block before anything can read them.
    #[inline(always)]
    pub fn vu_out_f<const F: u8>(&mut self, dst: usize, v: [u32; 4], mask: u32) {
        if dst == 32 {
            self.awr(v, mask);
        } else if dst < 32 {
            self.vwr(dst, v, mask);
        }
        if F == 1 {
            let m = crate::vu::mac_of(v, mask);
            let st = ((m & 0xf != 0) as u32) | (((m & 0xf0 != 0) as u32) << 1);
            self.vi[17] = m;
            self.vi[16] = (self.vi[16] & !0xf) | st | (st << 6);
        } else if F == 2 {
            self.vi[16] |= crate::vu::sticky_of(v, mask) << 6;
        }
    }

    /// `fmac` with operands known to be prepared (`fpu::vadd_c`): PS = fs, PT = ft (or its broadcast lane), PA = ACC.
    #[inline(always)]
    pub fn fmac_c<const PS: bool, const PT: bool, const PA: bool>(&self, kind: u8, s: [u32; 4], t: [u32; 4]) -> [u32; 4] {
        match kind {
            ADD => fpu::vadd_c::<PS, PT>(s, t),
            SUB => fpu::vsub_c::<PS, PT>(s, t),
            MUL => fpu::vmul_c::<PS, PT>(s, t),
            MADD => fpu::vmadd_c::<PA, PS, PT>(self.vacc, s, t, false),
            MSUB => fpu::vmadd_c::<PA, PS, PT>(self.vacc, s, t, true),
            _ => self.fmac_g::<false>(kind, s, t),
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

    pub fn vu_mem_addr(&self, vi: usize, lane: usize) -> usize {
        ((self.vi[vi] as usize * 16) & 0xff0) + lane * 4
    }

    pub fn vu_rd(&self, a: usize) -> u32 {
        u32::from_le_bytes(self.mem.vu0d[a..a + 4].try_into().unwrap())
    }

    pub fn vu_wr(&mut self, a: usize, v: u32) {
        self.mem.vu0d[a..a + 4].copy_from_slice(&v.to_le_bytes());
    }

    pub fn set_vi(&mut self, i: usize, v: u32) {
        if i != 0 {
            self.vi[i] = if i < 16 { v & 0xffff } else { v };
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
                unsafe { ee_core_vcallms(self, a, pc) }
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

    // ---------------- recompiled-code runtime (rec/mod.rs in ee-oracle) ----------------

    /// Compiled function entry: runs the hand-written replacement instead, when there is one (true: done, the PC
    /// is the return address).
    #[inline(always)]
    pub fn hand(&mut self, entry: u32) -> bool {
        // Cold boot: hook rules (cold_hooks.txt) sit at function entries; compiled callers reach this prologue directly, not `stub`.
        // [PC] (speed) The stub bit is tested here; the out-of-line call (which tests the same bit first) happens only when it is set.
        if self.cold.is_some() && self.pc == entry && {
            let i = ((entry & 0x01ff_ffff) >> 2) as usize;
            self.stub_bits.get(i >> 6).is_some_and(|w| w >> (i & 63) & 1 != 0)
        } && unsafe { ee_core_cold_entry(self, entry) }
        {
            return true;
        }
        match self.rec {
            Some(t) if self.pc == entry && unsafe { t.as_ref() }.has_hand(entry) => unsafe { ee_core_run_hand(self, entry) },
            _ => false,
        }
    }

    /// Block entry: whether the next `n` instructions can run without the interpreter's per-instruction checks
    /// (no DMA stop matures and no interrupt is taken among them). A due interrupt is serviced first.
    #[inline(always)]
    #[doc(hidden)]
    pub fn poll(&mut self, n: u64) -> bool {
        if self.icount + n <= self.mem.limit {
            return true;
        }
        unsafe { ee_core_poll_slow(self, n) }
    }

    /// After a hardware access inside a block: whether the `n` instructions left still run without checks.
    #[inline(always)]
    #[doc(hidden)]
    pub fn go(&self, n: u64) -> bool {
        self.icount + n <= self.mem.limit
    }

    #[inline(always)] #[doc(hidden)] pub fn hw(&mut self, op: u32, pc: u32, n: u64, next: u32) { unsafe { ee_core_hw(self, op, pc, n, next) } }
    #[inline(always)] #[doc(hidden)] pub fn slow(&mut self, last: u32, brl: u32) -> bool { unsafe { ee_core_slow(self, last, brl) } }
    #[inline(always)] #[doc(hidden)] pub fn call_dyn(&mut self, t: u32, ret: u32) { unsafe { ee_core_call_dyn(self, t, ret) } }
    #[inline(always)] #[doc(hidden)] pub fn tail(&mut self) { unsafe { ee_core_tail(self) } }
    #[inline(always)] #[doc(hidden)] pub fn vblank_hook(&mut self) -> bool { unsafe { ee_core_vblank_hook(self) } }
    #[inline(always)] #[doc(hidden)] pub fn exec(&mut self, op: u32, pc: u32) { unsafe { ee_core_exec(self, op, pc) } }
    #[inline(always)] #[doc(hidden)] pub fn cop2(&mut self, op: u32, pc: u32, rs: usize, rt: usize, rd: usize) { unsafe { ee_core_cop2(self, op, pc, rs, rt, rd) } }
    /// A loop with a native fast path (kind 0: the one-joint skinning loop head): true when it ran, pc set.
    #[inline(always)] #[doc(hidden)] pub fn ff(&mut self, kind: u32, pc: u32) -> bool { unsafe { ee_core_ff(self, kind, pc) } }
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

/// The lanes of `new` selected by a VU dest mask (x = 8 .. w = 1), the others from `old`: one 16-byte store, so a
/// following full-width load of the register is forwarded from it (two narrower stores would stall that load).
#[inline(always)]
pub fn blend4(old: [u32; 4], new: [u32; 4], dest: u32) -> [u32; 4] {
    if dest & 15 == 15 {
        return new;
    }
    use std::arch::x86_64::*;
    unsafe {
        let m = _mm_set_epi32(-((dest & 1) as i32), -((dest >> 1 & 1) as i32), -((dest >> 2 & 1) as i32), -((dest >> 3 & 1) as i32));
        let o = _mm_loadu_si128(old.as_ptr() as *const __m128i);
        let n = _mm_loadu_si128(new.as_ptr() as *const __m128i);
        let mut r = [0u32; 4];
        _mm_storeu_si128(r.as_mut_ptr() as *mut __m128i, _mm_or_si128(_mm_and_si128(m, n), _mm_andnot_si128(m, o)));
        r
    }
}
