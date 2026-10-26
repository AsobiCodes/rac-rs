//! S5c: more exact VU0 macro-mode helpers for speed-style regions (lane hs), merged into `hs_fast::vu0` by `pub use`.
//! Cheat sheet: scratch_perf_hs/cut_s5c/HS_OPS.md. Ground truth = ee-core `Cpu::vu_special1/2`, `vu_macro_upper` (vu.rs `vu_upper_g`),
//! `cop2` (oracle cpu.rs); the recompiled code calls exactly these with the retail word as a constant.
//!
//! Conventions (as the S2e/S2f kit): a `V` local holds the ARCHITECTURAL 4 words of a VF register / ACC. Arithmetic (add, sub, mul,
//! madd, msub, itof input excepted) takes PREPARED operands (`prep()`); add/sub/mul/madd/msub/itof/opmula/opmsub RETURN prepared
//! words. max, min, move, mr32, abs, ftoi, qmtc2 and lqc2 results are raw words: `prep()` them before arithmetic. A live `Chop` guard is
//! required for add/sub/mul/madd/msub/opmula/opmsub/itof/vdiv_g/vsqrt_g/vrsqrt_g (not for max/min/ftoi/clipw/vdiv/vsqrt/vrsqrt).
use super::vu0::{st_of_mac, Sticky, V};
use crate::cpu::Cpu;
use std::arch::x86_64::*;

#[inline(always)]
unsafe fn sel(m: __m128i, a: __m128i, b: __m128i) -> __m128i {
    _mm_or_si128(_mm_and_si128(m, a), _mm_andnot_si128(m, b))
}

impl V {
    #[inline(always)]
    pub fn zero() -> V {
        V(unsafe { _mm_setzero_si128() })
    }
    /// All four lanes = `x` (raw word: the Q register `vi[22]`, I `vi[21]`, a scalar). `prep()` it before arithmetic.
    #[inline(always)]
    pub fn splat(x: u32) -> V {
        V(unsafe { _mm_set1_epi32(x as i32) })
    }
    /// Lane `L` (0 = x .. 3 = w) as a raw word (vdiv / vsqrt / vrsqrt operands: `fsf`/`ftf` of the retail word).
    #[inline(always)]
    pub fn lane<const L: u32>(self) -> u32 {
        unsafe {
            match L {
                0 => _mm_cvtsi128_si32(self.0) as u32,
                1 => _mm_cvtsi128_si32(_mm_shuffle_epi32::<0x55>(self.0)) as u32,
                2 => _mm_cvtsi128_si32(_mm_shuffle_epi32::<0xaa>(self.0)) as u32,
                _ => _mm_cvtsi128_si32(_mm_shuffle_epi32::<0xff>(self.0)) as u32,
            }
        }
    }
    /// MSUB: `self` (ACC, prepared) - a * b (two ops, like the recompiled code).
    #[inline(always)]
    pub fn msub(self, a: V, b: V) -> V {
        self.sub(a.mul(b))
    }
    /// VMAX (core `fpu::max` per lane: `if fz(a) > fz(b) { a } else { b }`) on the ARCHITECTURAL words (raw or prepared, whatever the
    /// registers hold; for the bc forms `o` is the broadcast of the architectural ft lane). The result lane is one of the two RAW words.
    #[inline(always)]
    pub fn max(self, o: V) -> V {
        unsafe {
            let m = _mm_castps_si128(_mm_cmpgt_ps(_mm_castsi128_ps(self.prep().0), _mm_castsi128_ps(o.prep().0)));
            V(sel(m, self.0, o.0))
        }
    }
    /// VMINI (core `fpu::min`: `if fz(a) < fz(b) { a } else { b }`), same conventions as `max`.
    #[inline(always)]
    pub fn min(self, o: V) -> V {
        unsafe {
            let m = _mm_castps_si128(_mm_cmplt_ps(_mm_castsi128_ps(self.prep().0), _mm_castsi128_ps(o.prep().0)));
            V(sel(m, self.0, o.0))
        }
    }
    /// VMR32 (lanes [y, z, w, x]); raw words.
    #[inline(always)]
    pub fn rot32(self) -> V {
        V(unsafe { _mm_shuffle_epi32::<0x39>(self.0) })
    }
    /// VOPMULA / VOPMSUB product, PREPARED fs and ft: [fs.y*ft.z, fs.z*ft.x, fs.x*ft.y, (lane w: unused, mask it out)].
    /// VOPMULA.xyz: `acc = fs.opmul(ft)` (ACC.xyz, flags dest 0xe / lanes 7); VOPMSUB.xyz fd: `fd.xyz = acc.sub(fs.opmul(ft))`.
    #[inline(always)]
    pub fn opmul(self, t: V) -> V {
        unsafe { V(_mm_shuffle_epi32::<0xc9>(self.0)).mul(V(_mm_shuffle_epi32::<0xd2>(t.0))) }
    }
    /// VITOF0 / 4 / 12 / 15 (K = 0, 4, 12, 15) on the ARCHITECTURAL integer words (all four lanes): int -> float rounded toward zero, scaled by
    /// 2^-K (exact). Needs a live `Chop` (MXCSR round toward zero). Result prepared.
    #[inline(always)]
    pub fn itof<const K: u32>(self) -> V {
        unsafe {
            let mut f: __m128i = self.0;
            std::arch::asm!("cvtdq2ps {x}, {x}", x = inout(xmm_reg) f, options(nostack, nomem));
            if K != 0 {
                let sc = _mm_castps_si128(_mm_set1_ps(match K {
                    4 => 0.0625,
                    12 => 1.0 / 4096.0,
                    _ => 1.0 / 32768.0,
                }));
                std::arch::asm!("mulps {x}, {y}", x = inout(xmm_reg) f, y = in(xmm_reg) sc, options(nostack, nomem));
            }
            V(f)
        }
    }
    /// VFTOI0 / 4 / 12 / 15 (K = 0, 4, 12, 15), all four lanes: truncate fs * 2^K toward zero, saturate. Raw or prepared input;
    /// core `fpu::vftoi(v, 2^K)`. Result is a raw integer word.
    #[inline(always)]
    pub fn ftoi<const K: u32>(self) -> V {
        unsafe {
            let f = _mm_castsi128_ps(self.0);
            let f = match K {
                0 => f,
                4 => _mm_mul_ps(f, _mm_set1_ps(16.0)),
                12 => _mm_mul_ps(f, _mm_set1_ps(4096.0)),
                _ => _mm_mul_ps(f, _mm_set1_ps(32768.0)),
            };
            let r = _mm_cvttps_epi32(f);
            let ovf = _mm_cmpeq_epi32(r, _mm_set1_epi32(i32::MIN));
            let pos = _mm_cmpgt_epi32(self.0, _mm_set1_epi32(-1)); // sign bit of the operand clear
            V(_mm_xor_si128(r, _mm_and_si128(ovf, pos)))
        }
    }
    /// VCLIPW.xyz fs, ft: the new clip word (vi[18]) from the previous one: `prev << 6` (24 bits) with, for lanes x, y, z of `self` (fs), bit 2l set
    /// when lane > |ft.w| and bit 2l+1 when lane < -|ft.w| (DAZ / exponent-255 reads as the VU). The retail dest bits are ignored by the hardware op.
    #[inline(always)]
    pub fn clipw(self, t: V, prev: u32) -> u32 {
        const SPREAD: [u32; 8] = [0, 1, 4, 5, 16, 17, 20, 21];
        unsafe {
            let ps = _mm_castsi128_ps(self.prep().0);
            let w = _mm_and_ps(_mm_castsi128_ps(t.prep().bcw().0), _mm_castsi128_ps(_mm_set1_epi32(0x7fff_ffff)));
            let nw = _mm_xor_ps(w, _mm_castsi128_ps(_mm_set1_epi32(i32::MIN)));
            let gt = (_mm_movemask_ps(_mm_cmpgt_ps(ps, w)) & 7) as usize;
            let lt = (_mm_movemask_ps(_mm_cmplt_ps(ps, nw)) & 7) as usize;
            ((prev << 6) & 0xff_ffff) | SPREAD[gt] | (SPREAD[lt] << 1)
        }
    }
}

/// The lane mask of the `Sticky` / `blend` / `neg` helpers (x = 1 .. w = 8) for a retail dest field (x = 8 .. w = 1), and back.
#[inline(always)]
pub const fn lanes(dest: u32) -> u32 {
    ((dest & 8) >> 3) | ((dest & 4) >> 1) | ((dest & 2) << 1) | ((dest & 1) << 3)
}

/// vi[16] at the exit: status of the LAST flag op `m` (its `V::mac(dest)`) with the sticky bits 6-7 of every flag op (`sk`, only if
/// `ST`; entry value `e16`). The same word as `Cpu::vu_macro_upper` leaves after the sequence.
#[inline(always)]
pub fn status16(e16: u32, m: u32, sk: Sticky) -> u32 {
    (e16 & !0xf) | st_of_mac(m) | (sk.bits() << 6)
}

/// VDIV Q, fs.fsf, ft.ftf -> the new Q (vi[22]) from the two ARCHITECTURAL words (`v.lane::<L>()`). Division rounds as `fpu::div(.., CSR_CHOP)`
/// (f64 route, no MXCSR use). No flags, no stall (core writes Q at once; VWAITQ / VNOP are no-ops).
#[inline(always)]
pub fn vdiv(a: u32, b: u32) -> u32 {
    ee_core::fpu::div(a, b, ee_core::fpu::CSR_CHOP)
}
/// VSQRT Q, ft.ftf
#[inline(always)]
pub fn vsqrt(b: u32) -> u32 {
    ee_core::fpu::sqrt(b, ee_core::fpu::CSR_CHOP)
}
/// VRSQRT Q, fs.fsf, ft.ftf
#[inline(always)]
pub fn vrsqrt(a: u32, b: u32) -> u32 {
    ee_core::fpu::rsqrt(a, b, ee_core::fpu::CSR_CHOP)
}
/// The same three with the divss / sqrtss instruction under a LIVE `Chop` (same results, fewer instructions).
#[inline(always)]
pub fn vdiv_g(a: u32, b: u32) -> u32 {
    ee_core::fpu::div_nc(a, b)
}
#[inline(always)]
pub fn vsqrt_g(b: u32) -> u32 {
    ee_core::fpu::sqrt_nc(b)
}
#[inline(always)]
pub fn vrsqrt_g(a: u32, b: u32) -> u32 {
    ee_core::fpu::rsqrt_nc(a, b)
}

/// VSQI.DEST fs, (vit++): stores the lanes of `v` (= VF[fs], architectural) selected by `DEST` (x = 8 .. w = 1) to VU0 data memory at
/// `(vi[it] * 16) & 0xff0` and increments `vi[it]` (16 bits; vi[0] never changes). Reads / writes `c.vi[it]` and `c.mem.vu0d` directly:
/// VU0 data memory is not `Fm` memory, so every vsqi goes through this helper. (VSQD would decrement first: not needed by the 6 functions.)
#[inline(always)]
pub fn sqi<const DEST: u32>(c: &mut Cpu, it: usize, v: V) {
    let a = (c.vi[it] as usize * 16) & 0xff0;
    if DEST == 0xf {
        c.mem.vu0d[a..a + 16].copy_from_slice(&v.to_u128().to_le_bytes());
    } else {
        let w = v.st();
        for i in 0..4 {
            if DEST & (8 >> i) != 0 {
                c.mem.vu0d[a + i * 4..a + i * 4 + 4].copy_from_slice(&w[i].to_le_bytes());
            }
        }
    }
    if it != 0 {
        c.vi[it] = if it < 16 { c.vi[it].wrapping_add(1) & 0xffff } else { c.vi[it].wrapping_add(1) };
    }
}

/// CTC2 rt, vi<RD> for a plain VI register (1..=15, 19, 21, 23..=25, 27; NOT 0, 16, 17, 18, 20, 22, 26, 28, 29, 31: those are special or
/// locals of the speed style): the value goes to `c.vi[RD]` (16 bits for RD < 16, else 32) when no microprogram is parked in VPU_STAT and
/// no VU0 trace runs (`vu0_resume` / the trace record are the only other effects of `Cpu::cop2` rs = 6). Returns false WITHOUT doing anything
/// when it must run as retail: the region then flushes and runs `c.cop2(op, pc, 6, rt, RD)` or deopts at this instruction.
#[inline(always)]
pub fn ctc2<const RD: usize>(c: &mut Cpu, v: u32) -> bool {
    const { assert!(RD >= 1 && RD <= 27 && !matches!(RD, 16 | 17 | 18 | 20 | 22 | 26)) };
    if c.vi[29] & 1 != 0 || c.vu0_trace.is_some() {
        return false;
    }
    c.vi[RD] = if RD < 16 { v & 0xffff } else { v };
    true
}

/// VCALLMS imm15 / VCALLMSR vi27: the recompiled code's own call (`Cpu::vu_special1`). The microprogram reads and writes ANY VF, ACC, VI
/// (incl. Q, MAC, status, clip), VU0 data memory and micro memory: the region must have written back every VU0 local first, and must re-read
/// every VU0 local afterwards (see HS_OPS.md "vcallms protocol"). It does not touch `c.icount`.
#[inline(always)]
pub fn callms(c: &mut Cpu, op: u32, pc: u32) {
    c.vu_special1(op, pc)
}
