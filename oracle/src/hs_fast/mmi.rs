//! S5c: exact helpers for the EE MMI instructions of speed-style regions (lane hs). Cheat sheet: scratch_perf_hs/cut_s5c/HS_OPS.md.
//!
//! Ground truth = ee-core `Cpu::mmi` (core/src/cpu.rs; the recompiled code calls `c.mmi(op, pc, rs, rt, rd, sa)`). Every function here
//! is a pure `#[inline(always)]` function of the 128-bit register VALUES `a` = GPR[rs], `b` = GPR[rt] (the names of the interpreter)
//! and returns the value written to GPR[rd]; the caller writes `rd` (the full 128 bits; nothing when rd = $zero) and reads `$zero`
//! as 0u128. HI/LO are locals (`u128`) that the region loads from `c.hi` / `c.lo` before the first read and stores back at every
//! exit when it wrote them. SSE2 only (the lane builds without target features); no FMA (there is no float maths here).
//! The shifts take the immediate `sa` of the retail word as a const generic (psllw::<3>(rt) = `psllw rd, rt, 3`).
use std::arch::x86_64::*;

type X = __m128i;

#[inline(always)]
fn x(a: u128) -> X {
    unsafe { std::mem::transmute(a) }
}
#[inline(always)]
fn u(a: X) -> u128 {
    unsafe { std::mem::transmute(a) }
}
#[inline(always)]
fn sel(m: X, a: X, b: X) -> X {
    unsafe { _mm_or_si128(_mm_and_si128(m, a), _mm_andnot_si128(m, b)) }
}

// ---- MMI0 ----
/// paddw rd, rs, rt
#[inline(always)]
pub fn paddw(a: u128, b: u128) -> u128 {
    u(unsafe { _mm_add_epi32(x(a), x(b)) })
}
/// psubw rd, rs, rt
#[inline(always)]
pub fn psubw(a: u128, b: u128) -> u128 {
    u(unsafe { _mm_sub_epi32(x(a), x(b)) })
}
/// paddh rd, rs, rt
#[inline(always)]
pub fn paddh(a: u128, b: u128) -> u128 {
    u(unsafe { _mm_add_epi16(x(a), x(b)) })
}
/// psubh rd, rs, rt
#[inline(always)]
pub fn psubh(a: u128, b: u128) -> u128 {
    u(unsafe { _mm_sub_epi16(x(a), x(b)) })
}
/// paddb rd, rs, rt
#[inline(always)]
pub fn paddb(a: u128, b: u128) -> u128 {
    u(unsafe { _mm_add_epi8(x(a), x(b)) })
}
/// psubb rd, rs, rt
#[inline(always)]
pub fn psubb(a: u128, b: u128) -> u128 {
    u(unsafe { _mm_sub_epi8(x(a), x(b)) })
}
/// pcgtw rd, rs, rt (signed a > b)
#[inline(always)]
pub fn pcgtw(a: u128, b: u128) -> u128 {
    u(unsafe { _mm_cmpgt_epi32(x(a), x(b)) })
}
/// pcgth rd, rs, rt
#[inline(always)]
pub fn pcgth(a: u128, b: u128) -> u128 {
    u(unsafe { _mm_cmpgt_epi16(x(a), x(b)) })
}
/// pcgtb rd, rs, rt
#[inline(always)]
pub fn pcgtb(a: u128, b: u128) -> u128 {
    u(unsafe { _mm_cmpgt_epi8(x(a), x(b)) })
}
/// pmaxw rd, rs, rt: signed `if a > b { a } else { b }` per word.
#[inline(always)]
pub fn pmaxw(a: u128, b: u128) -> u128 {
    unsafe {
        let (a, b) = (x(a), x(b));
        u(sel(_mm_cmpgt_epi32(a, b), a, b))
    }
}
/// pmaxh rd, rs, rt
#[inline(always)]
pub fn pmaxh(a: u128, b: u128) -> u128 {
    u(unsafe { _mm_max_epi16(x(a), x(b)) })
}
/// pextlw rd, rs, rt: [b0, a0, b1, a1] (words, low = rt).
#[inline(always)]
pub fn pextlw(a: u128, b: u128) -> u128 {
    u(unsafe { _mm_unpacklo_epi32(x(b), x(a)) })
}
/// pextlh rd, rs, rt
#[inline(always)]
pub fn pextlh(a: u128, b: u128) -> u128 {
    u(unsafe { _mm_unpacklo_epi16(x(b), x(a)) })
}
/// pextlb rd, rs, rt
#[inline(always)]
pub fn pextlb(a: u128, b: u128) -> u128 {
    u(unsafe { _mm_unpacklo_epi8(x(b), x(a)) })
}
/// ppacw rd, rs, rt: [b0, b2, a0, a2] (words).
#[inline(always)]
pub fn ppacw(a: u128, b: u128) -> u128 {
    u(unsafe { _mm_castps_si128(_mm_shuffle_ps(_mm_castsi128_ps(x(b)), _mm_castsi128_ps(x(a)), 0b10_00_10_00)) })
}
/// ppach rd, rs, rt: the even halfwords of rt (low 64 bits) then of rs (high 64 bits).
#[inline(always)]
pub fn ppach(a: u128, b: u128) -> u128 {
    unsafe {
        let se = |v: X| _mm_srai_epi32::<16>(_mm_slli_epi32::<16>(v));
        u(_mm_packs_epi32(se(x(b)), se(x(a))))
    }
}
/// ppacb rd, rs, rt: the even bytes of rt (low 64 bits) then of rs (high 64 bits).
#[inline(always)]
pub fn ppacb(a: u128, b: u128) -> u128 {
    unsafe {
        let m = _mm_set1_epi16(0x00ff);
        u(_mm_packus_epi16(_mm_and_si128(x(b), m), _mm_and_si128(x(a), m)))
    }
}

// ---- MMI1 ----
/// pceqw rd, rs, rt
#[inline(always)]
pub fn pceqw(a: u128, b: u128) -> u128 {
    u(unsafe { _mm_cmpeq_epi32(x(a), x(b)) })
}
/// pceqh rd, rs, rt
#[inline(always)]
pub fn pceqh(a: u128, b: u128) -> u128 {
    u(unsafe { _mm_cmpeq_epi16(x(a), x(b)) })
}
/// pceqb rd, rs, rt
#[inline(always)]
pub fn pceqb(a: u128, b: u128) -> u128 {
    u(unsafe { _mm_cmpeq_epi8(x(a), x(b)) })
}
/// pminw rd, rs, rt: signed `if a < b { a } else { b }` per word.
#[inline(always)]
pub fn pminw(a: u128, b: u128) -> u128 {
    unsafe {
        let (a, b) = (x(a), x(b));
        u(sel(_mm_cmplt_epi32(a, b), a, b))
    }
}
/// pminh rd, rs, rt
#[inline(always)]
pub fn pminh(a: u128, b: u128) -> u128 {
    u(unsafe { _mm_min_epi16(x(a), x(b)) })
}
/// pextuw rd, rs, rt: [b2, a2, b3, a3].
#[inline(always)]
pub fn pextuw(a: u128, b: u128) -> u128 {
    u(unsafe { _mm_unpackhi_epi32(x(b), x(a)) })
}
/// pextuh rd, rs, rt
#[inline(always)]
pub fn pextuh(a: u128, b: u128) -> u128 {
    u(unsafe { _mm_unpackhi_epi16(x(b), x(a)) })
}
/// pextub rd, rs, rt
#[inline(always)]
pub fn pextub(a: u128, b: u128) -> u128 {
    u(unsafe { _mm_unpackhi_epi8(x(b), x(a)) })
}

// ---- MMI2 / MMI3 ----
/// pand rd, rs, rt
#[inline(always)]
pub fn pand(a: u128, b: u128) -> u128 {
    a & b
}
/// pxor rd, rs, rt
#[inline(always)]
pub fn pxor(a: u128, b: u128) -> u128 {
    a ^ b
}
/// por rd, rs, rt
#[inline(always)]
pub fn por(a: u128, b: u128) -> u128 {
    a | b
}
/// pnor rd, rs, rt
#[inline(always)]
pub fn pnor(a: u128, b: u128) -> u128 {
    !(a | b)
}
/// pcpyld rd, rs, rt: low 64 bits of rt, then low 64 bits of rs in the high half.
#[inline(always)]
pub fn pcpyld(a: u128, b: u128) -> u128 {
    (b & (u64::MAX as u128)) | (a << 64)
}
/// pcpyud rd, rs, rt: high 64 bits of rs in the low half, high 64 bits of rt in the high half.
#[inline(always)]
pub fn pcpyud(a: u128, b: u128) -> u128 {
    (a >> 64) | (b & !(u64::MAX as u128))
}
/// pcpyh rd, rt: halfword 0 of rt x4 (low 64 bits), halfword 4 x4 (high 64 bits). (rs is ignored.)
#[inline(always)]
pub fn pcpyh(b: u128) -> u128 {
    u(unsafe { _mm_shufflehi_epi16::<0>(_mm_shufflelo_epi16::<0>(x(b))) })
}
/// prot3w rd, rt: words [b1, b2, b0, b3].
#[inline(always)]
pub fn prot3w(b: u128) -> u128 {
    u(unsafe { _mm_shuffle_epi32::<0b11_00_10_01>(x(b)) })
}
/// pexew rd, rt: words [b2, b1, b0, b3]
#[inline(always)]
pub fn pexew(b: u128) -> u128 {
    u(unsafe { _mm_shuffle_epi32::<0b11_00_01_10>(x(b)) })
}

// ---- shifts by an immediate (rd = shift(rt, sa)) ----
/// psllw rd, rt, SA (SA 0..=31)
#[inline(always)]
pub fn psllw<const SA: i32>(b: u128) -> u128 {
    u(unsafe { _mm_sll_epi32(x(b), _mm_cvtsi32_si128(SA)) })
}
/// psrlw rd, rt, SA
#[inline(always)]
pub fn psrlw<const SA: i32>(b: u128) -> u128 {
    u(unsafe { _mm_srl_epi32(x(b), _mm_cvtsi32_si128(SA)) })
}
/// psraw rd, rt, SA
#[inline(always)]
pub fn psraw<const SA: i32>(b: u128) -> u128 {
    u(unsafe { _mm_sra_epi32(x(b), _mm_cvtsi32_si128(SA)) })
}
/// psllh rd, rt, SA (the interpreter shifts by `sa & 15`: the helper masks too)
#[inline(always)]
pub fn psllh<const SA: i32>(b: u128) -> u128 {
    u(unsafe { _mm_sll_epi16(x(b), _mm_cvtsi32_si128(SA & 15)) })
}
/// psrlh rd, rt, SA
#[inline(always)]
pub fn psrlh<const SA: i32>(b: u128) -> u128 {
    u(unsafe { _mm_srl_epi16(x(b), _mm_cvtsi32_si128(SA & 15)) })
}
/// psrah rd, rt, SA
#[inline(always)]
pub fn psrah<const SA: i32>(b: u128) -> u128 {
    u(unsafe { _mm_sra_epi16(x(b), _mm_cvtsi32_si128(SA & 15)) })
}

// ---- HI / LO ----
/// pmulth rd, rs, rt -> (rd value, new LO, new HI): the 8 signed 16x16 products p0..p7 (halfword pairs of rs, rt);
/// LO = [p0, p1, p4, p5], HI = [p2, p3, p6, p7] (words), rd = [p0, p2, p4, p6]. Overwrites ALL of HI and LO (no old value is read).
#[inline(always)]
pub fn pmulth(a: u128, b: u128) -> (u128, u128, u128) {
    unsafe {
        let (a, b) = (x(a), x(b));
        let (l, h) = (_mm_mullo_epi16(a, b), _mm_mulhi_epi16(a, b));
        let p03 = _mm_unpacklo_epi16(l, h);
        let p47 = _mm_unpackhi_epi16(l, h);
        let rd = _mm_castps_si128(_mm_shuffle_ps(_mm_castsi128_ps(p03), _mm_castsi128_ps(p47), 0b10_00_10_00));
        (u(rd), u(_mm_unpacklo_epi64(p03, p47)), u(_mm_unpackhi_epi64(p03, p47)))
    }
}
/// pmfhl.SA rd (SA = the sa field: 0 lw, 1 uw, 2 slw, 3 lh, 4 sh) from the LO / HI locals.
#[inline(always)]
pub fn pmfhl<const SA: u32>(lo: u128, hi: u128) -> u128 {
    unsafe {
        let (l, h) = (x(lo), x(hi));
        match SA {
            0 => u(_mm_unpacklo_epi32(_mm_shuffle_epi32::<0b10_00_10_00>(l), _mm_shuffle_epi32::<0b10_00_10_00>(h))),
            1 => u(_mm_unpacklo_epi32(_mm_shuffle_epi32::<0b11_01_11_01>(l), _mm_shuffle_epi32::<0b11_01_11_01>(h))),
            2 => {
                // saturate (HI:LO of each 64-bit half) to 32 bits, sign-extended to 64
                let f = |l: u32, h: u32| {
                    let v = ((h as u64) << 32 | l as u64) as i64;
                    v.clamp(i32::MIN as i64, i32::MAX as i64) as i32 as i64 as u64
                };
                let (lw, hw) = (ee_core::cpu::w4(lo), ee_core::cpu::w4(hi));
                f(lw[0], hw[0]) as u128 | (f(lw[2], hw[2]) as u128) << 64
            }
            3 => {
                // halfwords [l0, l2, h0, h2, l4, l6, h4, h6]: even halfwords of LO and HI
                let se = |v: X| _mm_srai_epi32::<16>(_mm_slli_epi32::<16>(v));
                let p = _mm_packs_epi32(se(l), se(h)); // [l0 l1 l2 l3 h0 h1 h2 h3] words' low halves = even halfwords
                u(_mm_shuffle_epi32::<0b11_01_10_00>(p))
            }
            _ => {
                // signed saturation of the 8 words to halfwords, order [l0, l1, h0, h1, l2, l3, h2, h3]
                let p = _mm_packs_epi32(l, h);
                u(_mm_shuffle_epi32::<0b11_01_10_00>(p))
            }
        }
    }
}
/// pext5 rd, $zero, rt (MMI0 sa 30): each word's 1-5-5-5 colour (bits 15, 14..10, 9..5, 4..0) to 8-bit channels
/// (x << 3 per channel, alpha bit to bit 31). Scalar, as ee-core.
#[inline(always)]
pub fn pext5(b: u128) -> u128 {
    let f = |x: u32| (x & 0x1f) << 3 | (x >> 5 & 0x1f) << 11 | (x >> 10 & 0x1f) << 19 | (x >> 15 & 1) << 31;
    let w = ee_core::cpu::w4(b);
    ee_core::cpu::uw4([f(w[0]), f(w[1]), f(w[2]), f(w[3])])
}
/// padduw rd, rs, rt (MMI1 sa 16): unsigned saturating add of the four words.
#[inline(always)]
pub fn padduw(a: u128, b: u128) -> u128 {
    let (x, y) = (ee_core::cpu::w4(a), ee_core::cpu::w4(b));
    ee_core::cpu::uw4([x[0].saturating_add(y[0]), x[1].saturating_add(y[1]), x[2].saturating_add(y[2]), x[3].saturating_add(y[3])])
}
/// pmultuw rd, rs, rt (MMI3 sa 12) -> (rd value, new LO, new HI): unsigned 32x32 products of words 0 and 2;
/// rd = [p0, p1] (64-bit halves), LO = [sx32(p0 low), sx32(p1 low)], HI = [sx32(p0 high), sx32(p1 high)].
/// Overwrites ALL of HI and LO (no old value is read).
#[inline(always)]
pub fn pmultuw(a: u128, b: u128) -> (u128, u128, u128) {
    let (x, y) = (ee_core::cpu::w4(a), ee_core::cpu::w4(b));
    let sx = |v: u32| v as i32 as i64 as u64;
    let p0 = (x[0] as u64) * (y[0] as u64);
    let p1 = (x[2] as u64) * (y[2] as u64);
    (
        p0 as u128 | (p1 as u128) << 64,
        sx(p0 as u32) as u128 | (sx(p1 as u32) as u128) << 64,
        sx((p0 >> 32) as u32) as u128 | (sx((p1 >> 32) as u32) as u128) << 64,
    )
}
/// mflo / mfhi rd (SPECIAL): the low 64 bits.
#[inline(always)]
pub fn mflo(lo: u128) -> u64 {
    lo as u64
}
#[inline(always)]
pub fn mfhi(hi: u128) -> u64 {
    hi as u64
}
