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
#[inline]
pub fn add(a: u32, b: u32) -> u32 {
    let (a, b) = guard(a, b);
    fout(addss(fin(a), fin(b), CSR_CHOP))
}
#[inline]
pub fn sub(a: u32, b: u32) -> u32 {
    let (a, b) = guard(a, b);
    fout(subss(fin(a), fin(b), CSR_CHOP))
}
#[inline]
pub fn mul(a: u32, b: u32) -> u32 {
    fout(mulss(fin(a), fin(b), CSR_CHOP))
}
#[inline]
pub fn div(a: u32, b: u32, csr: u32) -> u32 {
    if b & 0x7f80_0000 == 0 {
        return ((a ^ b) & 0x8000_0000) | MAX;
    }
    fout(divss(fin(a), fin(b), csr))
}
#[inline]
pub fn sqrt(b: u32, csr: u32) -> u32 {
    let v = b & 0x7fff_ffff;
    if v & 0x7f80_0000 == 0 {
        return 0;
    }
    fout(sqrtss2(0.0, fin(v), csr))
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
    fout(cvtsi2ss(w as i32, CSR_CHOP))
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
    vstore(sel(_mm_cmpeq_epi32(e, _mm_setzero_si128()), sign, r))
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
    unsafe {
        let (a, b) = (_mm_castsi128_ps(vprep(a)), _mm_castsi128_ps(vprep(b)));
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
        vstore(sel(tiny, _mm_and_si128(_mm_castps_si128(r), _mm_set1_epi32(0x8000_0000u32 as i32)), ri))
    }
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
