//! Native (ahead-of-time translated) VU1 microprograms: dispatch and the SSE helpers the generated code uses.
//!
//! `tools/vu1aot/vu1gen.py` writes `vu1_progN.rs` (one per retail program) and `vu1aot_gen.rs` (the table). A run goes
//! native when the micro memory starts with exactly a known program (checked again after every MPG upload) and the
//! entry address is one the generator covered; everything else runs on the interpreter in `vu1.rs`. The generated code
//! reproduces the interpreter bit for bit, see the header of `vu1.rs`.
//!
//! Helpers mirror the interpreter's scalar float functions lane by lane under the run's MXCSR (DAZ, FTZ, round toward
//! zero): `cl!` = `fin` (exponent 255 -> sign | FLT_MAX); add/sub/mul need no output clamp because round toward zero
//! never overflows to infinity and clamped operands never make NaN; `vmax!`/`vmin!` compare the clamped values (DAZ
//! flushes denormals like `fz`) and return the raw operand bits like `fmax`/`fmin`.
use crate::gsfull::vu1::{mac_of, st_of, Path1, Pipe, Vu1};
use std::arch::x86_64::*;

#[path = "vu1aot_gen.rs"]
mod progs;

/// `vism` value: no MAC flag landed in this run yet (vi16 / vi17 still hold the run's start values).
pub(crate) const NOMAC: u32 = 0x100;
/// Spreads 3 bits to bit positions 0, 2, 4 (CLIP judgement bits).
pub(crate) const SPREAD3: [u32; 8] = [0, 1, 4, 5, 16, 17, 20, 21];

pub(crate) enum Exec {
    /// The run reached its end natively; the cycle count.
    Done(u64),
    /// The native code stopped at a point it does not cover; the interpreter continues from this state.
    Bail(Box<Pipe>),
    /// Unknown code or entry: nothing was executed.
    NotCovered,
}

pub(crate) struct Native {
    pub id: u8,
    pub code: &'static [u64],
    pub run: unsafe fn(&mut Vu1, usize, u32, u32, &mut dyn Path1) -> Exec,
}

/// Which native program (if any) matches the micro memory, valid for one micro memory generation.
#[derive(Default)]
pub(crate) struct Cache {
    mgen: u64,
    prog: Option<usize>,
    sse41: Option<bool>,
}

/// The id of the native program that matches the micro memory (None: interpreter).
pub fn native_prog(vu: &mut Vu1) -> Option<u8> {
    refresh(vu);
    vu.aot_cache.prog.map(|k| progs::PROGS[k].id)
}

fn refresh(vu: &mut Vu1) {
    let g = vu.micro_gen();
    if vu.aot_cache.mgen == g {
        return;
    }
    let sse = *vu.aot_cache.sse41.get_or_insert_with(|| std::arch::is_x86_feature_detected!("sse4.1"));
    let m = vu.micro();
    let mut best: Option<usize> = None;
    if sse {
        for (k, p) in progs::PROGS.iter().enumerate() {
            if m.len() >= p.code.len() && m[..p.code.len()] == *p.code && best.is_none_or(|b| progs::PROGS[b].code.len() < p.code.len()) {
                best = Some(k);
            }
        }
    }
    vu.aot_cache.mgen = g;
    vu.aot_cache.prog = best;
}

/// Runs natively if the code and the entry are covered. Called under the run's MXCSR.
pub(crate) fn exec(vu: &mut Vu1, start: usize, top: u32, itop: u32, sink: &mut dyn Path1) -> Exec {
    refresh(vu);
    let Some(k) = vu.aot_cache.prog else { return Exec::NotCovered };
    if vu.data.len() != 1024 {
        return Exec::NotCovered;
    }
    // SAFETY: SSE4.1 was detected in `refresh`; the generated code indexes data memory with `& 0x3ff` (1024 qwords).
    unsafe { (progs::PROGS[k].run)(vu, start, top, itop, sink) }
}

/// vi17 as the interpreter would hold it now.
#[inline]
pub(crate) fn mat17(b17: u32, visv: __m128, vism: u32) -> u32 {
    if vism == NOMAC { b17 } else { mac_of(unsafe { std::mem::transmute::<__m128, [u32; 4]>(visv) }, vism) }
}

/// vi16 as the interpreter would hold it now: base bits, Z/S of the last landed MAC flag, sticky Z/S of all landed.
#[inline]
pub(crate) fn mat16(b16: u32, visv: __m128, vism: u32, zacc: __m128i, sacc: __m128i) -> u32 {
    let low = if vism == NOMAC { b16 & 0xf } else { st_of(mac_of(unsafe { std::mem::transmute::<__m128, [u32; 4]>(visv) }, vism)) };
    // SAFETY: SSE2 is baseline on x86_64.
    let (z, s) = unsafe {
        (
            (_mm_movemask_ps(_mm_castsi128_ps(_mm_cmpeq_epi32(zacc, _mm_setzero_si128()))) != 0) as u32,
            (_mm_movemask_ps(_mm_castsi128_ps(sacc)) != 0) as u32,
        )
    };
    (b16 & !0xf) | low | (z << 6) | (s << 7)
}

/// `fin` on 4 lanes: exponent 255 -> sign | FLT_MAX.
macro_rules! cl {
    ($v:expr) => {{
        let i = ::std::arch::x86_64::_mm_castps_si128($v);
        ::std::arch::x86_64::_mm_castsi128_ps(::std::arch::x86_64::_mm_or_si128(
            ::std::arch::x86_64::_mm_and_si128(i, ::std::arch::x86_64::_mm_set1_epi32(i32::MIN)),
            ::std::arch::x86_64::_mm_min_epi32(
                ::std::arch::x86_64::_mm_and_si128(i, ::std::arch::x86_64::_mm_set1_epi32(0x7fff_ffff)),
                ::std::arch::x86_64::_mm_set1_epi32(0x7f7f_ffff),
            ),
        ))
    }};
}
/// `fmax`: the raw bits of a where fz(a) > fz(b), else of b.
macro_rules! vmax {
    ($a:expr, $b:expr) => {{
        let (a, b) = ($a, $b);
        ::std::arch::x86_64::_mm_blendv_ps(b, a, ::std::arch::x86_64::_mm_cmpgt_ps(cl!(a), cl!(b)))
    }};
}
/// `fmin`: the raw bits of a where fz(a) < fz(b), else of b.
macro_rules! vmin {
    ($a:expr, $b:expr) => {{
        let (a, b) = ($a, $b);
        ::std::arch::x86_64::_mm_blendv_ps(b, a, ::std::arch::x86_64::_mm_cmplt_ps(cl!(a), cl!(b)))
    }};
}
/// ITOF0/4/12/15.
macro_rules! itof {
    ($v:expr, $sc:expr) => {{
        let r = ::std::arch::x86_64::_mm_cvtepi32_ps(::std::arch::x86_64::_mm_castps_si128($v));
        match $sc {
            0 => r,
            1 => ::std::arch::x86_64::_mm_mul_ps(r, ::std::arch::x86_64::_mm_set1_ps(0.0625)),
            2 => ::std::arch::x86_64::_mm_mul_ps(r, ::std::arch::x86_64::_mm_set1_ps(1.0 / 4096.0)),
            _ => ::std::arch::x86_64::_mm_mul_ps(r, ::std::arch::x86_64::_mm_set1_ps(1.0 / 32768.0)),
        }
    }};
}
/// FTOI0/4/12/15: truncation, out of range -> 0x80000000 (negative) / 0x7fffffff (positive).
macro_rules! ftoi {
    ($v:expr, $sc:expr) => {{
        let c = cl!($v);
        let t = match $sc {
            0 => c,
            1 => ::std::arch::x86_64::_mm_mul_ps(c, ::std::arch::x86_64::_mm_set1_ps(16.0)),
            2 => ::std::arch::x86_64::_mm_mul_ps(c, ::std::arch::x86_64::_mm_set1_ps(4096.0)),
            _ => ::std::arch::x86_64::_mm_mul_ps(c, ::std::arch::x86_64::_mm_set1_ps(32768.0)),
        };
        let r = ::std::arch::x86_64::_mm_cvttps_epi32(t);
        let ovf = ::std::arch::x86_64::_mm_cmpgt_epi32(
            ::std::arch::x86_64::_mm_castps_si128(t),
            ::std::arch::x86_64::_mm_set1_epi32(0x4eff_ffff),
        );
        ::std::arch::x86_64::_mm_castsi128_ps(::std::arch::x86_64::_mm_xor_si128(r, ovf))
    }};
}
macro_rules! absm {
    () => {
        ::std::arch::x86_64::_mm_castsi128_ps(::std::arch::x86_64::_mm_set1_epi32(0x7fff_ffff))
    };
}
macro_rules! splat {
    ($x:expr) => {
        ::std::arch::x86_64::_mm_castsi128_ps(::std::arch::x86_64::_mm_set1_epi32(($x) as i32))
    };
}
macro_rules! lane {
    ($v:expr, $k:literal) => {
        (::std::arch::x86_64::_mm_extract_ps::<$k>($v) as u32)
    };
}
macro_rules! arr {
    ($v:expr) => {
        ::std::mem::transmute::<::std::arch::x86_64::__m128, [u32; 4]>($v)
    };
}
macro_rules! ldr {
    ($a:expr) => {
        ::std::arch::x86_64::_mm_loadu_ps(($a).as_ptr() as *const f32)
    };
}
macro_rules! ld {
    ($d:expr, $a:expr) => {
        ::std::arch::x86_64::_mm_loadu_ps(($d).add($a) as *const f32)
    };
}
macro_rules! st {
    ($d:expr, $a:expr, $v:expr) => {
        ::std::arch::x86_64::_mm_storeu_ps(($d).add($a) as *mut f32, $v)
    };
}
/// CLIP: judgement of s.xyz against |t.w|, shifted onto the previous judgements (24 bits).
macro_rules! clip {
    ($s:expr, $t:expr, $prev:expr) => {{
        let s = cl!($s);
        let t = $t;
        let w = ::std::arch::x86_64::_mm_and_ps(cl!(::std::arch::x86_64::_mm_shuffle_ps::<0xff>(t, t)), absm!());
        let nw = ::std::arch::x86_64::_mm_xor_ps(w, ::std::arch::x86_64::_mm_castsi128_ps(::std::arch::x86_64::_mm_set1_epi32(i32::MIN)));
        let gt = (::std::arch::x86_64::_mm_movemask_ps(::std::arch::x86_64::_mm_cmpgt_ps(s, w)) & 7) as usize;
        let lt = (::std::arch::x86_64::_mm_movemask_ps(::std::arch::x86_64::_mm_cmplt_ps(s, nw)) & 7) as usize;
        (($prev << 6) & 0xff_ffff) | $crate::gsfull::vu1aot::SPREAD3[gt] | ($crate::gsfull::vu1aot::SPREAD3[lt] << 1)
    }};
}
pub(crate) use {absm, arr, cl, clip, ftoi, itof, lane, ld, ldr, splat, st, vmax, vmin};
