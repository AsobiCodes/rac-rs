//! EE FPU and VU float arithmetic with PCSX2's default behaviour:
//! round toward zero (division and square root round to nearest), denormals
//! read as zero and flush to zero, no inf/NaN (results clamp to +-FLT_MAX).
//! Every op runs in inline asm under its own MXCSR so the host float
//! environment is never changed outside the instruction.

use std::arch::asm;
use std::arch::x86_64::*;

/// Exceptions masked, DAZ, FTZ, round toward zero.
pub const CSR_CHOP: u32 = 0x1f80 | 0x0040 | 0x8000 | 0x6000;
/// Exceptions masked, DAZ, FTZ, round to nearest (PCSX2 "division rounding").
pub const CSR_NEAR: u32 = 0x1f80 | 0x0040 | 0x8000;

macro_rules! ss_op {
    ($name:ident, $ins:literal) => {
        #[inline(always)]
        pub fn $name(a: f32, b: f32, csr: u32) -> f32 {
            let mut x = a;
            let mut save: u32 = 0;
            unsafe {
                asm!(
                    "stmxcsr dword ptr [{s}]",
                    "ldmxcsr dword ptr [{c}]",
                    concat!($ins, " {x}, {y}"),
                    "ldmxcsr dword ptr [{s}]",
                    s = in(reg) &mut save as *mut u32,
                    c = in(reg) &csr as *const u32,
                    x = inout(xmm_reg) x,
                    y = in(xmm_reg) b,
                    options(nostack),
                );
            }
            x
        }
    };
}
ss_op!(addss, "addss");
ss_op!(subss, "subss");
ss_op!(mulss, "mulss");
ss_op!(divss, "divss");
ss_op!(sqrtss2, "sqrtss");

macro_rules! ps_op {
    ($name:ident, $ins:literal) => {
        #[inline(always)]
        pub fn $name(a: [f32; 4], b: [f32; 4], csr: u32) -> [f32; 4] {
            let mut x: __m128 = unsafe { std::mem::transmute(a) };
            let y: __m128 = unsafe { std::mem::transmute(b) };
            let mut save: u32 = 0;
            unsafe {
                asm!(
                    "stmxcsr dword ptr [{s}]",
                    "ldmxcsr dword ptr [{c}]",
                    concat!($ins, " {x}, {y}"),
                    "ldmxcsr dword ptr [{s}]",
                    s = in(reg) &mut save as *mut u32,
                    c = in(reg) &csr as *const u32,
                    x = inout(xmm_reg) x,
                    y = in(xmm_reg) y,
                    options(nostack),
                );
            }
            unsafe { std::mem::transmute(x) }
        }
    };
}
ps_op!(addps, "addps");
ps_op!(subps, "subps");
ps_op!(mulps, "mulps");

/// Holds MXCSR at `CSR_CHOP` for its lifetime (restores the previous value on drop), so that the `*_nc` ops
/// below, which leave MXCSR alone, can be used in a hot loop instead of switching it for every op.
pub struct ChopGuard {
    save: u32,
}

impl ChopGuard {
    #[inline]
    pub fn new() -> ChopGuard {
        let mut save: u32 = 0;
        let csr = CSR_CHOP;
        unsafe {
            asm!(
                "stmxcsr dword ptr [{s}]",
                "ldmxcsr dword ptr [{c}]",
                s = in(reg) &mut save as *mut u32,
                c = in(reg) &csr as *const u32,
                options(nostack),
            );
        }
        ChopGuard { save }
    }
}

impl Drop for ChopGuard {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            asm!("ldmxcsr dword ptr [{s}]", s = in(reg) &self.save as *const u32, options(nostack));
        }
    }
}

macro_rules! ps_op_nc {
    ($name:ident, $ins:literal) => {
        /// Needs a live `ChopGuard`.
        #[inline(always)]
        pub fn $name(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
            let mut x: __m128 = unsafe { std::mem::transmute(a) };
            let y: __m128 = unsafe { std::mem::transmute(b) };
            unsafe {
                asm!(concat!($ins, " {x}, {y}"), x = inout(xmm_reg) x, y = in(xmm_reg) y, options(nostack, nomem));
                std::mem::transmute(x)
            }
        }
    };
}
ps_op_nc!(addps_nc, "addps");
ps_op_nc!(subps_nc, "subps");
ps_op_nc!(mulps_nc, "mulps");

/// Needs a live `ChopGuard`.
#[inline(always)]
pub fn mulss_nc(a: f32, b: f32) -> f32 {
    let mut x = a;
    unsafe {
        asm!("mulss {x}, {y}", x = inout(xmm_reg) x, y = in(xmm_reg) b, options(nostack, nomem));
    }
    x
}

/// Needs a live `ChopGuard`.
#[inline(always)]
pub fn cvtsi2ss_nc(v: i32) -> f32 {
    let mut x: f32 = 0.0;
    unsafe {
        asm!("cvtsi2ss {x}, {v:e}", x = inout(xmm_reg) x, v = in(reg) v, options(nostack, nomem));
    }
    x
}

#[inline(always)]
pub fn cvtsi2ss(v: i32, csr: u32) -> f32 {
    let mut x: f32 = 0.0;
    let mut save: u32 = 0;
    unsafe {
        asm!(
            "stmxcsr dword ptr [{s}]",
            "ldmxcsr dword ptr [{c}]",
            "cvtsi2ss {x}, {v:e}",
            "ldmxcsr dword ptr [{s}]",
            s = in(reg) &mut save as *mut u32,
            c = in(reg) &csr as *const u32,
            x = inout(xmm_reg) x,
            v = in(reg) v,
            options(nostack),
        );
    }
    x
}

const MAX: u32 = 0x7f7f_ffff;

/// Operand: exponent 255 reads as +-FLT_MAX (PS2 has no inf/NaN).
#[inline(always)]
pub fn fin(b: u32) -> f32 {
    if b & 0x7f80_0000 == 0x7f80_0000 {
        f32::from_bits((b & 0x8000_0000) | MAX)
    } else {
        f32::from_bits(b)
    }
}

/// Result: inf/NaN clamp to +-FLT_MAX keeping the sign bit.
#[inline(always)]
pub fn fout(r: f32) -> u32 {
    let b = r.to_bits();
    if b & 0x7f80_0000 == 0x7f80_0000 {
        (b & 0x8000_0000) | MAX
    } else {
        b
    }
}

/// Denormal-as-zero value for compares and min/max.
#[inline(always)]
pub fn fz(b: u32) -> f32 {
    if b & 0x7f80_0000 == 0 {
        f32::from_bits(b & 0x8000_0000)
    } else {
        fin(b)
    }
}

/// The EE FPU has no guard bits: before ADD/SUB, the bits of the smaller
/// operand that its alignment shift would push past one guard bit are
/// cleared (as PCSX2's EE FPU_ADD_SUB does; the VUs add plainly).
#[inline]
fn guard(a: u32, b: u32) -> (u32, u32) {
    let diff = ((a >> 23) & 0xff) as i32 - ((b >> 23) & 0xff) as i32;
    match diff {
        25.. => (a, b & 0x8000_0000),
        1..=24 => (a, b & (u32::MAX << (diff - 1))),
        -24..=-1 => (a & (u32::MAX << (-diff - 1)), b),
        ..=-25 => (a & 0x8000_0000, b),
        _ => (a, b),
    }
}
// [PC] (speed) The EE FPU ops below ran each instruction between a stmxcsr / ldmxcsr pair (round toward zero, DAZ, FTZ),
// which stalls the pipeline; recompiled game code issues hundreds of thousands of them per field. ADD / SUB / MUL /
// CVT.S now take lane 0 of the MXCSR-free vector ops (the same operation: `fout(op(fin(a), fin(b), CSR_CHOP))`), DIV /
// SQRT the f64 forms (`div_f64` / `sqrt_f64`). The asm forms stay as the reference (`*_ref`, tests).
#[inline]
pub fn add(a: u32, b: u32) -> u32 {
    let (a, b) = guard(a, b);
    vadd([a, 0, 0, 0], [b, 0, 0, 0])[0]
}
#[inline]
pub fn sub(a: u32, b: u32) -> u32 {
    let (a, b) = guard(a, b);
    vsub([a, 0, 0, 0], [b, 0, 0, 0])[0]
}
#[inline]
pub fn mul(a: u32, b: u32) -> u32 {
    vmul([a, 0, 0, 0], [b, 0, 0, 0])[0]
}
#[inline]
pub fn div(a: u32, b: u32, csr: u32) -> u32 {
    if b & 0x7f80_0000 == 0 {
        return ((a ^ b) & 0x8000_0000) | MAX;
    }
    match csr {
        CSR_NEAR => div_f64::<false>(a, b),
        CSR_CHOP => div_f64::<true>(a, b),
        _ => fout(divss(fin(a), fin(b), csr)),
    }
}
#[inline]
pub fn sqrt(b: u32, csr: u32) -> u32 {
    let v = b & 0x7fff_ffff;
    if v & 0x7f80_0000 == 0 {
        return 0;
    }
    match csr {
        CSR_NEAR => sqrt_f64::<false>(v),
        CSR_CHOP => sqrt_f64::<true>(v),
        _ => fout(sqrtss2(0.0, fin(v), csr)),
    }
}
/// The f32 result of an operation whose exact value `q` (finite, |q| < 2^255) is known to f64 precision (one rounding):
/// the f32 rounding of q under DAZ / FTZ, round to nearest (CHOP = false: the host's MXCSR, which recompiled code runs
/// under) or toward zero (CHOP = true: rounded, then stepped one ulp toward zero where above the exact value). For a
/// quotient or square root of f32 operands the f64 rounding never reaches or crosses an f32 value or a midpoint between
/// two (their distance from both is at least 2^-50 relative, f64 keeps 53 bits), so this is the directly rounded value.
/// Tiny results are signed zeros (FTZ; SSE detects tininess before rounding, so the test is on the exact value: an f32
/// quotient never lies within 2^-47 relative of a power of two without equalling it); overflow is +-FLT_MAX.
#[inline(always)]
fn round_f64<const CHOP: bool>(q: f64) -> u32 {
    if q.abs() < f32::MIN_POSITIVE as f64 {
        return (q.to_bits() >> 32) as u32 & 0x8000_0000;
    }
    let r = q as f32;
    let mut bits = r.to_bits();
    if CHOP && (r.abs() as f64) > q.abs() {
        bits -= 1;
    }
    fout(f32::from_bits(bits))
}
/// `fout(divss(fin(a), fin(b), CSR_NEAR / CSR_CHOP))` for b with a non-zero exponent, without MXCSR switching.
#[inline(always)]
pub fn div_f64<const CHOP: bool>(a: u32, b: u32) -> u32 {
    // DAZ: a denormal dividend is a signed zero.
    let a = if a & 0x7f80_0000 == 0 { a & 0x8000_0000 } else { a };
    round_f64::<CHOP>(fin(a) as f64 / fin(b) as f64)
}
/// `fout(sqrtss(fin(v), CSR_NEAR / CSR_CHOP))` for a positive v with a non-zero exponent, without MXCSR switching.
#[inline(always)]
pub fn sqrt_f64<const CHOP: bool>(v: u32) -> u32 {
    round_f64::<CHOP>((fin(v) as f64).sqrt())
}
#[cfg(test)]
pub fn add_ref(a: u32, b: u32) -> u32 {
    let (a, b) = guard(a, b);
    fout(addss(fin(a), fin(b), CSR_CHOP))
}
#[cfg(test)]
pub fn sub_ref(a: u32, b: u32) -> u32 {
    let (a, b) = guard(a, b);
    fout(subss(fin(a), fin(b), CSR_CHOP))
}
#[cfg(test)]
pub fn mul_ref(a: u32, b: u32) -> u32 {
    fout(mulss(fin(a), fin(b), CSR_CHOP))
}
#[cfg(test)]
pub fn div_ref(a: u32, b: u32, csr: u32) -> u32 {
    if b & 0x7f80_0000 == 0 {
        return ((a ^ b) & 0x8000_0000) | MAX;
    }
    fout(divss(fin(a), fin(b), csr))
}
#[cfg(test)]
pub fn sqrt_ref(b: u32, csr: u32) -> u32 {
    let v = b & 0x7fff_ffff;
    if v & 0x7f80_0000 == 0 {
        return 0;
    }
    fout(sqrtss2(0.0, fin(v), csr))
}
#[cfg(test)]
pub fn cvt_s_ref(w: u32) -> u32 {
    fout(cvtsi2ss(w as i32, CSR_CHOP))
}
/// `div` / `sqrt` / `rsqrt` with `csr` = CSR_CHOP while a `ChopGuard` holds MXCSR there: the same results without the
/// MXCSR switching of each op.
#[inline(always)]
fn divss_nc(a: f32, b: f32) -> f32 {
    let mut x = a;
    unsafe {
        asm!("divss {x}, {y}", x = inout(xmm_reg) x, y = in(xmm_reg) b, options(nostack, nomem));
    }
    x
}
#[inline(always)]
fn sqrtss_nc(a: f32) -> f32 {
    let mut x = 0.0f32;
    unsafe {
        asm!("sqrtss {x}, {y}", x = inout(xmm_reg) x, y = in(xmm_reg) a, options(nostack, nomem));
    }
    x
}
/// Needs a live `ChopGuard`; equals `div(a, b, CSR_CHOP)`.
#[inline]
pub fn div_nc(a: u32, b: u32) -> u32 {
    if b & 0x7f80_0000 == 0 {
        return ((a ^ b) & 0x8000_0000) | MAX;
    }
    fout(divss_nc(fin(a), fin(b)))
}
/// Needs a live `ChopGuard`; equals `sqrt(b, CSR_CHOP)`.
#[inline]
pub fn sqrt_nc(b: u32) -> u32 {
    let v = b & 0x7fff_ffff;
    if v & 0x7f80_0000 == 0 {
        return 0;
    }
    fout(sqrtss_nc(fin(v)))
}
/// Needs a live `ChopGuard`; equals `rsqrt(a, b, CSR_CHOP)`.
#[inline]
pub fn rsqrt_nc(a: u32, b: u32) -> u32 {
    div_nc(a, sqrt_nc(b))
}
/// EE RSQRT.S / VU RSQRT: a / sqrt(|b|).
#[inline]
pub fn rsqrt(a: u32, b: u32, csr: u32) -> u32 {
    let s = sqrt(b, csr);
    div(a, s, csr)
}
#[inline]
pub fn cvt_w(b: u32) -> u32 {
    if b & 0x7f80_0000 <= 0x4e80_0000 {
        (fin(b) as i32) as u32
    } else if b & 0x8000_0000 != 0 {
        0x8000_0000
    } else {
        0x7fff_ffff
    }
}
#[inline]
pub fn cvt_s(w: u32) -> u32 {
    vitof([w, 0, 0, 0], 1.0)[0]
}
#[inline]
pub fn max(a: u32, b: u32) -> u32 {
    if fz(a) > fz(b) { a } else { b }
}
#[inline]
pub fn min(a: u32, b: u32) -> u32 {
    if fz(a) < fz(b) { a } else { b }
}

// ---- VU lanes ----
#[inline(always)]
pub fn vin(v: [u32; 4]) -> [f32; 4] {
    [fin(v[0]), fin(v[1]), fin(v[2]), fin(v[3])]
}
#[inline(always)]
pub fn vout(v: [f32; 4]) -> [u32; 4] {
    [fout(v[0]), fout(v[1]), fout(v[2]), fout(v[3])]
}
// [PC] (speed) `vadd` / `vsub` / `vmul` without MXCSR switching: the same results as
// `vout(addps(vin(a), vin(b), CSR_CHOP))` etc. (round toward zero, DAZ, FTZ, exponent-255 operands as +-FLT_MAX,
// overflow to +-FLT_MAX), computed under the host's round-to-nearest MXCSR (also right under a live `ChopGuard`).
// The sum is rounded to nearest and stepped one ulp toward zero when its exact error (Fast2Sum) has the opposite
// sign; the product is exact in f64 and stepped the same way after rounding to f32. The two ldmxcsr per op (each
// stalls the pipeline) were most of the cost of a recompiled COP2 macro op (17.4 -> 8.5 ns per op; checked bit
// for bit against the asm ops on random lanes with exponent edges, scratch_fps/mxb).
#[inline(always)]
unsafe fn sel(m: __m128i, a: __m128i, b: __m128i) -> __m128i {
    _mm_or_si128(_mm_and_si128(m, a), _mm_andnot_si128(m, b))
}

/// Operand as the ops read it: exponent 255 -> +-FLT_MAX (`fin`), exponent 0 -> +-0 (DAZ).
#[inline(always)]
unsafe fn vprep(v: [u32; 4]) -> __m128i {
    let v = _mm_loadu_si128(v.as_ptr() as *const __m128i);
    let e = _mm_and_si128(v, _mm_set1_epi32(0x7f80_0000));
    let s = _mm_and_si128(v, _mm_set1_epi32(0x8000_0000u32 as i32));
    let v = sel(_mm_cmpeq_epi32(e, _mm_set1_epi32(0x7f80_0000)), _mm_or_si128(s, _mm_set1_epi32(MAX as i32)), v);
    sel(_mm_cmpeq_epi32(e, _mm_setzero_si128()), s, v)
}

#[inline(always)]
unsafe fn vstore(v: __m128i) -> [u32; 4] {
    let mut o = [0u32; 4];
    _mm_storeu_si128(o.as_mut_ptr() as *mut __m128i, v);
    o
}

/// Sum of prepared lanes (finite, normal or zero).
#[inline(always)]
unsafe fn vadd_prepped(a: __m128i, b: __m128i) -> [u32; 4] {
    vstore(vadd_pv(a, b))
}

/// `vadd_prepped` as a vector (the result is itself prepared: `vprep` leaves it unchanged).
#[inline(always)]
unsafe fn vadd_pv(a: __m128i, b: __m128i) -> __m128i {
    let mag = _mm_set1_epi32(0x7fff_ffff);
    // Fast2Sum needs |x| >= |y| (finite magnitudes compare as integers).
    let big = _mm_cmpgt_epi32(_mm_and_si128(b, mag), _mm_and_si128(a, mag));
    let x = _mm_castsi128_ps(sel(big, b, a));
    let y = _mm_castsi128_ps(sel(big, a, b));
    let s = _mm_add_ps(x, y);
    let err = _mm_sub_ps(y, _mm_sub_ps(s, x));
    let si = _mm_castps_si128(s);
    let ei = _mm_castps_si128(err);
    // |s| above the exact sum (error non-zero, opposite sign): one step toward zero.
    let nz = _mm_andnot_si128(_mm_cmpeq_epi32(_mm_and_si128(ei, mag), _mm_setzero_si128()), _mm_set1_epi32(-1));
    let opp = _mm_srai_epi32(_mm_xor_si128(si, ei), 31);
    let r = _mm_add_epi32(si, _mm_and_si128(nz, opp));
    let e = _mm_and_si128(r, _mm_set1_epi32(0x7f80_0000));
    let sign = _mm_and_si128(r, _mm_set1_epi32(0x8000_0000u32 as i32));
    // Overflow (inf): +-FLT_MAX; a denormal result: +-0.
    let r = sel(_mm_cmpeq_epi32(e, _mm_set1_epi32(0x7f80_0000)), _mm_or_si128(sign, _mm_set1_epi32(MAX as i32)), r);
    sel(_mm_cmpeq_epi32(e, _mm_setzero_si128()), sign, r)
}

#[inline]
pub fn vadd(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    unsafe { vadd_prepped(vprep(a), vprep(b)) }
}
#[inline]
pub fn vsub(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    unsafe { vadd_prepped(vprep(a), _mm_xor_si128(vprep(b), _mm_set1_epi32(0x8000_0000u32 as i32))) }
}
#[inline]
pub fn vmul(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    unsafe { vstore(vmul_pv(vprep(a), vprep(b))) }
}

/// Product of prepared lanes; the result is prepared too (FTZ below the smallest normal, overflow to +-FLT_MAX).
#[inline(always)]
unsafe fn vmul_pv(a: __m128i, b: __m128i) -> __m128i {
    {
        let (a, b) = (_mm_castsi128_ps(a), _mm_castsi128_ps(b));
        // Exact in f64: 24 x 24 significand bits fit in 53, the exponents in its range.
        let plo = _mm_mul_pd(_mm_cvtps_pd(a), _mm_cvtps_pd(b));
        let phi = _mm_mul_pd(_mm_cvtps_pd(_mm_movehl_ps(a, a)), _mm_cvtps_pd(_mm_movehl_ps(b, b)));
        let r = _mm_movelh_ps(_mm_cvtpd_ps(plo), _mm_cvtpd_ps(phi));
        let abs = _mm_castsi128_pd(_mm_set1_epi64x(0x7fff_ffff_ffff_ffff));
        let (aplo, aphi) = (_mm_and_pd(plo, abs), _mm_and_pd(phi, abs));
        // |rounded| above |exact| (also inf on overflow): one step toward zero (inf - 1 ulp = FLT_MAX).
        let ul = _mm_castpd_ps(_mm_cmpgt_pd(_mm_and_pd(_mm_cvtps_pd(r), abs), aplo));
        let uh = _mm_castpd_ps(_mm_cmpgt_pd(_mm_and_pd(_mm_cvtps_pd(_mm_movehl_ps(r, r)), abs), aphi));
        let up = _mm_castps_si128(_mm_shuffle_ps(ul, uh, 0b10_00_10_00));
        // An exact product below the smallest normal: +-0 (FTZ).
        let tmin = _mm_set1_pd(f32::MIN_POSITIVE as f64);
        let tl = _mm_castpd_ps(_mm_cmplt_pd(aplo, tmin));
        let th = _mm_castpd_ps(_mm_cmplt_pd(aphi, tmin));
        let tiny = _mm_castps_si128(_mm_shuffle_ps(tl, th, 0b10_00_10_00));
        let ri = _mm_add_epi32(_mm_castps_si128(r), up);
        sel(tiny, _mm_and_si128(_mm_castps_si128(r), _mm_set1_epi32(0x8000_0000u32 as i32)), ri)
    }
}

// [PC] (speed) The COP2 FMAC ops with operands the recompiler knows are already prepared (`P*` = true: every lane the
// op writes was last written by an ADD/SUB/MUL/MADD/MSUB result, which `vprep` leaves unchanged), so the per-operand
// `vprep` is skipped. Lanes the op does not write may differ from `vadd` etc.; the caller discards them (dest mask, MAC
// flags of the written lanes only). Bit for bit the same written lanes as `vadd` / `vsub` / `vmul` and MADD / MSUB.
#[inline(always)]
unsafe fn vp<const P: bool>(v: [u32; 4]) -> __m128i {
    if P { _mm_loadu_si128(v.as_ptr() as *const __m128i) } else { vprep(v) }
}
#[inline(always)]
pub fn vadd_c<const PA: bool, const PB: bool>(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    unsafe { vstore(vadd_pv(vp::<PA>(a), vp::<PB>(b))) }
}
#[inline(always)]
pub fn vsub_c<const PA: bool, const PB: bool>(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    unsafe { vstore(vadd_pv(vp::<PA>(a), _mm_xor_si128(vp::<PB>(b), _mm_set1_epi32(0x8000_0000u32 as i32)))) }
}
#[inline(always)]
pub fn vmul_c<const PA: bool, const PB: bool>(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    unsafe { vstore(vmul_pv(vp::<PA>(a), vp::<PB>(b))) }
}
/// MADD (`neg` = false) / MSUB: acc +- s * t, the product already prepared.
#[inline(always)]
pub fn vmadd_c<const PACC: bool, const PA: bool, const PB: bool>(acc: [u32; 4], a: [u32; 4], b: [u32; 4], neg: bool) -> [u32; 4] {
    unsafe {
        let p = vmul_pv(vp::<PA>(a), vp::<PB>(b));
        let p = if neg { _mm_xor_si128(p, _mm_set1_epi32(0x8000_0000u32 as i32)) } else { p };
        vstore(vadd_pv(vp::<PACC>(acc), p))
    }
}
/// [PC] (speed) ITOF0/4/12/15 without MXCSR switching: the same bits as `fout(mulss(cvtsi2ss(x, CSR_CHOP), sc, CSR_CHOP))`
/// per lane (`sc` = 1, 1/16, 1/4096, 1/32768), which took four stmxcsr / ldmxcsr round trips per lane. The integer is
/// exact in f64; rounded to f32 (any MXCSR rounding) and stepped one ulp toward zero where that is above the exact value,
/// it is the round-toward-zero conversion. The power-of-two scale is then exact (results are 0 or at least 2^-15: no
/// denormal, no overflow), so `fout` changes nothing.
#[inline(always)]
pub fn vitof(v: [u32; 4], sc: f32) -> [u32; 4] {
    unsafe {
        let x = _mm_loadu_si128(v.as_ptr() as *const __m128i);
        let lo = _mm_cvtepi32_pd(x);
        let hi = _mm_cvtepi32_pd(_mm_shuffle_epi32::<0x0e>(x));
        let r = _mm_movelh_ps(_mm_cvtpd_ps(lo), _mm_cvtpd_ps(hi));
        let abs = _mm_castsi128_pd(_mm_set1_epi64x(0x7fff_ffff_ffff_ffff));
        let ul = _mm_castpd_ps(_mm_cmpgt_pd(_mm_and_pd(_mm_cvtps_pd(r), abs), _mm_and_pd(lo, abs)));
        let uh = _mm_castpd_ps(_mm_cmpgt_pd(_mm_and_pd(_mm_cvtps_pd(_mm_movehl_ps(r, r)), abs), _mm_and_pd(hi, abs)));
        let up = _mm_castps_si128(_mm_shuffle_ps(ul, uh, 0b10_00_10_00));
        let t = _mm_castsi128_ps(_mm_add_epi32(_mm_castps_si128(r), up));
        vstore(_mm_castps_si128(_mm_mul_ps(t, _mm_set1_ps(sc))))
    }
}
/// [PC] (speed) FTOI0/4/12/15 without MXCSR switching: the same words as `cvt_w(fout(mulss(fin(s), sc, CSR_CHOP)))` per
/// lane (`sc` = 1, 16, 4096, 32768). The power-of-two product is exact for normal results under any rounding; a denormal
/// operand or product truncates to 0 either way, and an overflow (FLT_MAX under round-toward-zero, inf otherwise, both
/// +-FLT_MAX after `fout`) saturates in `cvt_w` either way.
#[inline(always)]
pub fn vftoi(v: [u32; 4], sc: f32) -> [u32; 4] {
    let f = |b: u32| cvt_w(fout(fin(b) * sc));
    [f(v[0]), f(v[1]), f(v[2]), f(v[3])]
}
/// The same with FAST = true: no MXCSR switching, a `ChopGuard` must be alive.
#[inline(always)]
pub fn vadd_g<const FAST: bool>(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    if FAST { vout(addps_nc(vin(a), vin(b))) } else { vadd(a, b) }
}
#[inline(always)]
pub fn vsub_g<const FAST: bool>(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    if FAST { vout(subps_nc(vin(a), vin(b))) } else { vsub(a, b) }
}
#[inline(always)]
pub fn vmul_g<const FAST: bool>(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    if FAST { vout(mulps_nc(vin(a), vin(b))) } else { vmul(a, b) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rnd(s: &mut u64) -> u32 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        (*s >> 16) as u32
    }
    /// Random words biased to exponent edges (0, 1, 254, 255, near the overflow and underflow of products and quotients),
    /// denormals, short significands (exact results) and integers near 2^24 and the i32 limits.
    fn word(s: &mut u64) -> u32 {
        let r = rnd(s);
        let e = [0u32, 1, 2, 23, 24, 25, 100, 126, 127, 128, 150, 151, 200, 230, 253, 254, 255][(rnd(s) % 17) as usize] << 23;
        match rnd(s) % 6 {
            0 => r,
            1 => (r & 0x807f_ffff) | e,
            2 => (r & 0x807f_0000) | e,
            3 => ((1u32 << (r % 32)) as i32).wrapping_add((rnd(s) % 512) as i32 - 256) as u32,
            4 => [0, 1, 0x7fff_ffff, 0x8000_0000, 0x8000_0001, 0x00ff_ffff, 0x0080_0000, 0x7f7f_ffff, 0xff7f_ffff, 0x3f80_0000][(r % 10) as usize],
            _ => (r & 0x8000_0000) | (r % 0x0100_0000) | e,
        }
    }

    #[test]
    fn scalar_ops_match_mxcsr_reference() {
        let mut s = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..4_000_000 {
            let (a, b) = (word(&mut s), word(&mut s));
            assert_eq!(add(a, b), add_ref(a, b), "add {a:08x} {b:08x}");
            assert_eq!(sub(a, b), sub_ref(a, b), "sub {a:08x} {b:08x}");
            assert_eq!(mul(a, b), mul_ref(a, b), "mul {a:08x} {b:08x}");
            for csr in [CSR_NEAR, CSR_CHOP] {
                assert_eq!(div(a, b, csr), div_ref(a, b, csr), "div {a:08x} {b:08x} csr {csr:x}");
                assert_eq!(sqrt(b, csr), sqrt_ref(b, csr), "sqrt {b:08x} csr {csr:x}");
                assert_eq!(rsqrt(a, b, csr), div_ref(a, sqrt_ref(b, csr), csr), "rsqrt {a:08x} {b:08x} csr {csr:x}");
            }
            assert_eq!(cvt_s(a), cvt_s_ref(a), "cvt.s {a:08x}");
        }
    }
}
