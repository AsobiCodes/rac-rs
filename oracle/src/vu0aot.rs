//! Native (ahead-of-time translated) VU0 microprograms "prog0" (the skinned-vertex program, 2624 bytes =
//! decomp/vu1/prog0.bin, see `vu0_prog0.rs`) and "prog1" (four straight-line 4-vertex blocks, 1408 bytes =
//! decomp/vu1/prog1.bin, `vu0_prog1.rs`; resident on Kerwan, Eudora, Novalis, Hoven, Batalia ...), and "prog9hi" (prog9's
//! 752-byte piece at micro address 0xc80: a sine and a rotation-matrix builder, loaded next to prog0 or prog1 and run
//! about 2400 times per field on Blarg; `vu0_prog9hi.rs`), all generated from the disc at setup by tools/vu/vu0gen.py
//! (VU0_PROG=prog0 / prog1 / prog9hi, see tools/vugen_iso.py).
//!
//! The interpreter in `vu.rs` stays the reference. `Cpu::vu0_exec` (here) hands a start or a resume to the native
//! code only when the code bytes in VU0 micro memory equal prog0 (checked at every VCALLMS/VCALLMSR) and the start
//! address is one the native code knows (the program start, the three VCALLMSR entries and the eight wait loops);
//! anything else runs in the interpreter. The native code is the interpreter's own per-pair model (upper and lower
//! read the pre-pair state, upper wins a same-register write, VF results ready 4 cycles after issue, Q after 7 / 13,
//! LOI's I seen from the next pair, branch delay slots, the wait-loop park rule), unrolled over the pair-level
//! hazard state at translation time: every stall is a constant, so only the values are computed at run time.
//! The float arithmetic is the interpreter's (`fpu` helpers: operands with exponent 255 read as +-FLT_MAX, round
//! toward zero, denormals as zero, results clamp to +-FLT_MAX), written on whole 4-lane vectors.
//!
//! VU0AOT=0 turns it off, VU0AOT=diff runs the interpreter and the native code side by side on every prog0 / prog1 start
//! and resume and compares the whole VU0 state afterwards (`Vu0Aot` counters), the interpreter's result is kept.

use crate::cpu::Cpu;
use std::arch::asm;
use std::arch::x86_64::*;

pub type V = __m128i;

pub const MODE_OFF: u8 = 0;
pub const MODE_ON: u8 = 1;
pub const MODE_DIFF: u8 = 2;

/// Cap on pairs per run, as in the interpreter.
pub const PAIR_CAP: u64 = 5_000_000;

#[derive(Default)]
pub struct Vu0Aot {
    pub mode: u8,
    /// Micro memory generation (Mem::vu0m_gen) at which the code bytes were last verified as prog0 or prog1.
    pub ok_gen: u32,
    pub ok: bool,
    /// With `ok`: the bytes are prog1 (else prog0).
    pub prog1: bool,
    /// Micro memory from `vu0_prog9hi::BASE` holds prog9hi (checked with `ok`, independently of the low program).
    pub hi9: bool,
    /// `ok` / `ok_gen` hold a verdict (`vu0_aot_check` ran once).
    pub checked: bool,
    /// Runs by the native code / by the interpreter (prog0 loaded but start address unknown) / skipped as no-ops.
    pub n_native: u64,
    pub n_noop: u64,
    pub n_fallback: u64,
    /// Differential mode: runs compared, runs that differ, first differences (text).
    pub diff_runs: u64,
    pub diff_bad: u64,
    pub diff_msgs: Vec<String>,
    /// Native node execution counts (differential mode and coverage tools).
    pub cov: Vec<u32>,
    pub cov_on: bool,
    /// VU0AOT_NOSHORT=1 (test tool): run the wait-loop bodies natively instead of taking the no-op shortcut.
    pub noshort: bool,
    /// Profile (VU0AOT_PROF=1): rdtsc ticks and calls, index 0 = the whole vu0_exec, 1 = the native code,
    /// 2 = the interpreter.
    pub prof: bool,
    pub ticks: [u64; 4],
    pub calls: [u64; 4],
}

impl Vu0Aot {
    pub fn from_env() -> Vu0Aot {
        let mode = match std::env::var("VU0AOT").as_deref() {
            Ok("0") | Ok("off") => MODE_OFF,
            Ok("diff") => MODE_DIFF,
            _ => MODE_ON,
        };
        // The native code uses PMINSD (SSE4.1; every x86-64 CPU since 2008): without it the interpreter runs.
        let mode = if std::is_x86_feature_detected!("sse4.1") { mode } else { MODE_OFF };
        let prof = std::env::var("VU0AOT_PROF").is_ok();
        let mut a = Vu0Aot { mode, prof, ..Default::default() };
        a.cov = vec![0; crate::vu0_prog0::NODES.max(crate::vu0_prog1::NODES).max(crate::vu0_prog9hi::NODES)];
        a.cov_on = mode == MODE_DIFF || std::env::var("VU0AOT_COV").is_ok();
        a.noshort = std::env::var("VU0AOT_NOSHORT").is_ok();
        a
    }
}

#[inline(always)]
pub fn rdtsc() -> u64 {
    unsafe { _rdtsc() }
}

// ---------------------------------------------------------------------------------------------------------------
// Vector helpers. All float ops are inline asm like `fpu::*_nc`: they need the MXCSR of a live `fpu::ChopGuard`
// (round toward zero, DAZ, FTZ), and the compiler must not fold or move them across it.

#[inline(always)]
pub unsafe fn ldv(p: *const u32) -> V {
    _mm_loadu_si128(p as *const V)
}
#[inline(always)]
pub unsafe fn stv(p: *mut u32, v: V) {
    _mm_storeu_si128(p as *mut V, v)
}
#[inline(always)]
pub fn splat(x: u32) -> V {
    unsafe { _mm_set1_epi32(x as i32) }
}
/// m ? a : b, per bit.
#[inline(always)]
pub fn sel(m: V, a: V, b: V) -> V {
    unsafe { _mm_or_si128(_mm_and_si128(m, a), _mm_andnot_si128(m, b)) }
}
/// Lane mask of the dest bits M (bit 3 = x = lane 0).
#[inline(always)]
pub fn lanes<const M: u32>() -> V {
    unsafe {
        _mm_set_epi32(
            if M & 1 != 0 { -1 } else { 0 },
            if M & 2 != 0 { -1 } else { 0 },
            if M & 4 != 0 { -1 } else { 0 },
            if M & 8 != 0 { -1 } else { 0 },
        )
    }
}
/// `new` in the lanes of M, `old` elsewhere.
#[inline(always)]
pub fn blendm<const M: u32>(old: V, new: V) -> V {
    if M == 15 {
        new
    } else if M == 0 {
        old
    } else {
        sel(lanes::<M>(), new, old)
    }
}
/// MR32: lane l = lane l + 1 of v (x <- y, y <- z, z <- w, w <- x).
#[inline(always)]
pub fn mr32(v: V) -> V {
    unsafe { _mm_shuffle_epi32::<0x39>(v) }
}
/// Broadcast of lane N (0 = x).
#[inline(always)]
pub fn bc<const N: u32>(v: V) -> V {
    unsafe {
        match N {
            0 => _mm_shuffle_epi32::<0x00>(v),
            1 => _mm_shuffle_epi32::<0x55>(v),
            2 => _mm_shuffle_epi32::<0xAA>(v),
            _ => _mm_shuffle_epi32::<0xFF>(v),
        }
    }
}
#[inline(always)]
pub fn lane_of(v: V, n: usize) -> u32 {
    let mut a = [0u32; 4];
    unsafe { _mm_storeu_si128(a.as_mut_ptr() as *mut V, v) };
    a[n]
}

/// `fpu::fin` / `fpu::fout` on 4 lanes: exponent 255 -> sign | FLT_MAX. With a = |v| as a non-negative integer, the
/// exponent-255 lanes are exactly those above 0x7f7fffff, so the result is sign | min(a, 0x7f7fffff) (PMINSD, SSE4.1;
/// `vu0_aot_check` switches the native code off on a CPU without it).
#[inline(always)]
pub fn clamp4(v: V) -> V {
    unsafe {
        let sign = _mm_set1_epi32(i32::MIN);
        let mut m = _mm_andnot_si128(sign, v);
        asm!("pminsd {m}, {c}", m = inout(xmm_reg) m, c = in(xmm_reg) _mm_set1_epi32(0x7f7f_ffff), options(nostack, nomem, pure));
        _mm_or_si128(_mm_and_si128(v, sign), m)
    }
}
/// `fpu::fz` on 4 lanes: denormal -> signed zero, else `clamp4`.
#[inline(always)]
pub fn fz4(v: V) -> V {
    unsafe {
        let e = _mm_set1_epi32(0x7f80_0000);
        let z = _mm_cmpeq_epi32(_mm_and_si128(v, e), _mm_setzero_si128());
        sel(z, _mm_and_si128(v, _mm_set1_epi32(i32::MIN)), clamp4(v))
    }
}

macro_rules! ps_asm {
    ($name:ident, $ins:literal) => {
        #[inline(always)]
        fn $name(a: V, b: V) -> V {
            unsafe {
                let mut x = _mm_castsi128_ps(a);
                asm!(concat!($ins, " {x}, {y}"), x = inout(xmm_reg) x, y = in(xmm_reg) _mm_castsi128_ps(b), options(nostack, nomem));
                _mm_castps_si128(x)
            }
        }
    };
}
ps_asm!(addps, "addps");
ps_asm!(subps, "subps");
ps_asm!(mulps, "mulps");

/// Operand clamp (`fpu::fin`) unless the generator proved the operand lanes clean (vuclean.py): NEED = true clamps.
#[inline(always)]
fn cl<const NEED: bool>(v: V) -> V {
    if NEED {
        clamp4(v)
    } else {
        v
    }
}
// The ops `fpu::vadd_g::<true>` etc. are `fout(op(fin(a), fin(b)))`. MXCSR rounds toward zero here, so an overflow
// gives the largest finite number (never Inf) and finite operands never give NaN: `fout` is the identity and only
// the operands are clamped (NS / NT / NA: the S, T, ACC operand needs its `fin`).
#[inline(always)]
pub fn add4<const NS: bool, const NT: bool>(a: V, b: V) -> V {
    addps(cl::<NS>(a), cl::<NT>(b))
}
#[inline(always)]
pub fn sub4<const NS: bool, const NT: bool>(a: V, b: V) -> V {
    subps(cl::<NS>(a), cl::<NT>(b))
}
#[inline(always)]
pub fn mul4<const NS: bool, const NT: bool>(a: V, b: V) -> V {
    mulps(cl::<NS>(a), cl::<NT>(b))
}
/// MADD: acc + s*t (the product is a finite result, as in `vmadd_g`).
#[inline(always)]
pub fn madd4<const NA: bool, const NS: bool, const NT: bool>(acc: V, s: V, t: V) -> V {
    addps(cl::<NA>(acc), mul4::<NS, NT>(s, t))
}
#[inline(always)]
pub fn msub4<const NA: bool, const NS: bool, const NT: bool>(acc: V, s: V, t: V) -> V {
    subps(cl::<NA>(acc), mul4::<NS, NT>(s, t))
}
/// `fpu::max` / `fpu::min` per lane (the raw operand bits are returned).
#[inline(always)]
pub fn max4(a: V, b: V) -> V {
    unsafe {
        let g = _mm_castps_si128(_mm_cmpgt_ps(_mm_castsi128_ps(fz4(a)), _mm_castsi128_ps(fz4(b))));
        sel(g, a, b)
    }
}
#[inline(always)]
pub fn min4(a: V, b: V) -> V {
    unsafe {
        let g = _mm_castps_si128(_mm_cmplt_ps(_mm_castsi128_ps(fz4(a)), _mm_castsi128_ps(fz4(b))));
        sel(g, a, b)
    }
}
#[inline(always)]
pub fn abs4(a: V) -> V {
    unsafe { _mm_and_si128(a, _mm_set1_epi32(0x7fff_ffff)) }
}
/// ITOF0/4/12/15: `fout(mulss_nc(cvtsi2ss_nc(s as i32), sc))` per lane. CVTDQ2PS rounds like CVTSI2SS (MXCSR, toward zero
/// here); asm so the compiler never reasons about the rounding mode. The scales are <= 1, so the product is finite and
/// `fout` is the identity.
#[inline(always)]
pub fn itof4<const K: u32>(s: V) -> V {
    let sc = [1.0f32, 0.0625, 1.0 / 4096.0, 1.0 / 32768.0][K as usize];
    unsafe {
        let mut x = _mm_castsi128_ps(s);
        asm!("cvtdq2ps {x}, {x}", x = inout(xmm_reg) x, options(nostack, nomem, pure));
        mulps(_mm_castps_si128(x), _mm_castps_si128(_mm_set1_ps(sc)))
    }
}
/// FTOI0/4/12/15: `cvt_w(fout(mulss_nc(fin(s), sc)))` per lane. The product is finite (rounding toward zero never
/// gives Inf). |p| < 2^31 (exponent field <= 0x4e800000, as `cvt_w` tests) truncates exactly in CVTTPS2DQ; above that
/// it gives 0x80000000 and the positive lanes saturate to 0x7fffffff (0x80000000 ^ 0xffffffff).
#[inline(always)]
pub fn ftoi4<const K: u32>(s: V) -> V {
    let sc = [1.0f32, 16.0, 4096.0, 32768.0][K as usize];
    unsafe {
        let p = mulps(clamp4(s), _mm_castps_si128(_mm_set1_ps(sc)));
        let r = _mm_cvttps_epi32(_mm_castsi128_ps(p));
        let ov = _mm_cmpgt_epi32(_mm_and_si128(p, _mm_set1_epi32(0x7f80_0000)), _mm_set1_epi32(0x4e80_0000));
        let pos = _mm_cmpgt_epi32(p, _mm_set1_epi32(-1));
        _mm_xor_si128(r, _mm_and_si128(ov, pos))
    }
}

/// MAC flag lane masks of a result (`mac_of`): (zero lanes, sign lanes), each lane-ordered (bit 0 = x) and cut to
/// the lanes of `lm` (the dest bits reversed: bit 0 = x).
#[inline(always)]
pub fn zs_masks(v: V, lm: u32) -> (u32, u32) {
    unsafe {
        let z = _mm_movemask_ps(_mm_castsi128_ps(_mm_cmpeq_epi32(_mm_and_si128(v, _mm_set1_epi32(0x7fff_ffff)), _mm_setzero_si128()))) as u32;
        let s = _mm_movemask_ps(_mm_castsi128_ps(v)) as u32;
        (z & lm, s & lm)
    }
}
/// Lane-ordered mask (bit l = lane l) -> the MAC register layout (lane l at bit 3 - l).
pub const REV4: [u32; 16] = [0, 8, 4, 12, 2, 10, 6, 14, 1, 9, 5, 13, 3, 11, 7, 15];

// ---------------------------------------------------------------------------------------------------------------

impl Cpu {
    /// Runs the microprogram at `pc` (a start, or the loop head it is parked at) to its E bit or to the next wait
    /// loop that only the EE can end. The native code takes prog0 runs; everything else is `vu0_exec_interp`.
    pub(crate) fn vu0_exec(&mut self, pc: usize) -> Result<u64, String> {
        if self.vu0_aot.prof {
            let t0 = rdtsc();
            let r = self.vu0_exec_dispatch(pc);
            self.vu0_aot.ticks[0] += rdtsc().wrapping_sub(t0);
            self.vu0_aot.calls[0] += 1;
            return r;
        }
        self.vu0_exec_dispatch(pc)
    }

    fn vu0_exec_interp_prof(&mut self, pc: usize) -> Result<u64, String> {
        if self.vu0_aot.prof {
            let t0 = rdtsc();
            let r = self.vu0_exec_interp(pc);
            self.vu0_aot.ticks[2] += rdtsc().wrapping_sub(t0);
            self.vu0_aot.calls[2] += 1;
            return r;
        }
        self.vu0_exec_interp(pc)
    }

    fn vu0_exec_dispatch(&mut self, pc: usize) -> Result<u64, String> {
        let a = &self.vu0_aot;
        // A start in the prog9hi piece (its flow stays above BASE, the low programs' below it) needs only that piece.
        let hi = pc >= crate::vu0_prog9hi::BASE;
        if a.mode == MODE_OFF || !a.checked || a.ok_gen != self.mem.vu0m_gen || !(if hi { a.hi9 } else { a.ok }) {
            return self.vu0_exec_interp_prof(pc);
        }
        let entry = if hi {
            crate::vu0_prog9hi::entry_node(pc)
        } else if a.prog1 {
            crate::vu0_prog1::entry_node(pc)
        } else {
            crate::vu0_prog0::entry_node(pc)
        };
        let node = match entry {
            Some(n) => n,
            None => {
                self.vu0_aot.n_fallback += 1;
                return self.vu0_exec_interp_prof(pc);
            }
        };
        self.vu0_exec_aot_mode(pc, node)
    }

    fn vu0_exec_aot_mode(&mut self, pc: usize, node: u32) -> Result<u64, String> {
        if self.vu0_aot.mode == MODE_DIFF {
            return self.vu0_exec_diff(pc, node);
        }
        self.vu0_exec_native(pc, node)
    }

    /// The native code (or the shortcut for a wait loop whose condition still holds).
    pub(crate) fn vu0_exec_native(&mut self, pc: usize, node: u32) -> Result<u64, String> {
        // A start or resume at a wait loop head whose branch is not taken: the interpreter runs the loop head, its
        // delay slot and the back branch, and parks again at the same address. Nothing else changes.
        let hi = pc >= crate::vu0_prog9hi::BASE;
        let park = if hi {
            crate::vu0_prog9hi::park_reg(pc)
        } else if self.vu0_aot.prog1 {
            crate::vu0_prog1::park_reg(pc)
        } else {
            crate::vu0_prog0::park_reg(pc)
        };
        if let Some(r) = park {
            if !self.vu0_aot.noshort && (self.vi[r] as u16 as i16) != (self.vi[0] as u16 as i16) {
                self.vu0_aot.n_noop += 1;
                self.vu0_stats[1] += 1;
                self.vu0_stats[3] += 3;
                self.vu0_stats[4] = pc as u64;
                self.vu0_stats[5] = 1;
                self.vi[29] = 1 | (pc as u32) << 16;
                if let Some(t) = self.vu0_trace.as_mut() {
                    t.push((3, pc as u32, 3, 2));
                }
                return Ok(2);
            }
        }
        self.vu0_aot.n_native += 1;
        let prof = self.vu0_aot.prof;
        let t0 = if prof { rdtsc() } else { 0 };
        let _chop = crate::fpu::ChopGuard::new();
        let r = match (hi, self.vu0_aot.prog1, self.vu0_aot.cov_on) {
            (true, _, true) => self.vu0_prog9hi_run::<true>(node),
            (true, _, false) => self.vu0_prog9hi_run::<false>(node),
            (false, false, true) => self.vu0_prog0_run::<true>(node),
            (false, false, false) => self.vu0_prog0_run::<false>(node),
            (false, true, true) => self.vu0_prog1_run::<true>(node),
            (false, true, false) => self.vu0_prog1_run::<false>(node),
        };
        if prof {
            self.vu0_aot.ticks[1] += rdtsc().wrapping_sub(t0);
            self.vu0_aot.calls[1] += 1;
        }
        let (cyc, pairs, last_pc, parked) = r?;
        self.vu0_stats[3] += pairs;
        self.vu0_stats[4] = last_pc as u64;
        self.vu0_stats[5] = parked as u64;
        if let Some(t) = self.vu0_trace.as_mut() {
            t.push((if parked { 3 } else { 4 }, last_pc as u32, pairs as u32, cyc as u32));
        }
        if parked {
            self.vu0_stats[1] += 1;
            self.vi[29] = 1 | (last_pc as u32) << 16;
        }
        Ok(cyc)
    }

    fn vu0_state(&self) -> VuState {
        VuState { vf: self.vf, vacc: self.vacc, vi: self.vi, d: self.mem.vu0d.clone(), stats: self.vu0_stats }
    }
    fn vu0_set_state(&mut self, s: &VuState) {
        self.vf = s.vf;
        self.vacc = s.vacc;
        self.vi = s.vi;
        self.mem.vu0d.copy_from_slice(&s.d);
        self.vu0_stats = s.stats;
    }

    /// Differential mode: interpreter and native code from the same state; the state must agree afterwards.
    fn vu0_exec_diff(&mut self, pc: usize, node: u32) -> Result<u64, String> {
        let s0 = self.vu0_state();
        let tr = self.vu0_trace.as_ref().map(|t| t.len());
        let ri = self.vu0_exec_interp(pc);
        let si = self.vu0_state();
        if let (Some(t), Some(n)) = (self.vu0_trace.as_mut(), tr) {
            t.truncate(n);
        }
        self.vu0_set_state(&s0);
        let ra = self.vu0_exec_native(pc, node);
        let sa = self.vu0_state();
        if let (Some(t), Some(n)) = (self.vu0_trace.as_mut(), tr) {
            t.truncate(n);
        }
        self.vu0_aot.diff_runs += 1;
        let mut bad = String::new();
        match (&ri, &ra) {
            (Ok(a), Ok(b)) if a == b => {}
            (Err(_), Err(_)) => {}
            _ => bad += &format!("result interp {ri:?} native {ra:?}; "),
        }
        for r in 0..32 {
            if si.vf[r] != sa.vf[r] {
                bad += &format!("vf{r} interp {:08x?} native {:08x?}; ", si.vf[r], sa.vf[r]);
            }
            if si.vi[r] != sa.vi[r] {
                bad += &format!("vi{r} interp {:08x} native {:08x}; ", si.vi[r], sa.vi[r]);
            }
        }
        if si.vacc != sa.vacc {
            bad += &format!("ACC interp {:08x?} native {:08x?}; ", si.vacc, sa.vacc);
        }
        if si.d != sa.d {
            let k = (0..si.d.len()).find(|&k| si.d[k] != sa.d[k]).unwrap();
            bad += &format!("vu0 data mem differs first at {k:#x} (interp {:02x} native {:02x}); ", si.d[k], sa.d[k]);
        }
        if si.stats != sa.stats {
            bad += &format!("stats interp {:?} native {:?}; ", si.stats, sa.stats);
        }
        if !bad.is_empty() {
            self.vu0_aot.diff_bad += 1;
            if self.vu0_aot.diff_msgs.len() < 8 {
                let m = format!("VU0AOT DIFF at run {} (start pc {pc:04x}, EE pc {:08x}): {bad}", self.vu0_aot.diff_runs, self.pc);
                eprintln!("{m}");
                self.vu0_aot.diff_msgs.push(m);
            }
        }
        self.vu0_set_state(&si);
        ri
    }

    /// Verifies at a VCALLMS that micro memory holds prog0 or prog1 (every byte of it), and prog9hi above them. The
    /// native code is used while `vu0m_gen` (bumped by every VIF0 MPG / EE write to micro memory / state restore)
    /// stays as it was.
    pub(crate) fn vu0_aot_check(&mut self) {
        if self.vu0_aot.mode == MODE_OFF {
            return;
        }
        // Micro memory unchanged since the last verdict (every writer bumps `vu0m_gen`): the compare of the 2624
        // code bytes would give the same answer (it ran ~3000 times per field, at every VCALLMS).
        if self.vu0_aot.checked && self.vu0_aot.ok_gen == self.mem.vu0m_gen {
            return;
        }
        self.vu0_aot.checked = true;
        // (An empty program is the stub tools/vugen_iso.py writes for a disc whose copy differs: it never matches.)
        let holds = |p: &[u8]| !p.is_empty() && self.mem.vu0m.len() >= p.len() && self.mem.vu0m[..p.len()] == p[..];
        let ok0 = self.mem.vu0d.len() == 0x1000 && holds(&crate::vu0_prog0::PROG0);
        let ok1 = !ok0 && self.mem.vu0d.len() == 0x1000 && holds(&crate::vu0_prog1::PROG1);
        self.vu0_aot.ok = ok0 || ok1;
        self.vu0_aot.prog1 = ok1;
        let (b, p9) = (crate::vu0_prog9hi::BASE, &crate::vu0_prog9hi::PROG9HI);
        self.vu0_aot.hi9 = self.mem.vu0d.len() == 0x1000 && !p9.is_empty() && self.mem.vu0m.len() >= b + p9.len()
            && self.mem.vu0m[b..b + p9.len()] == p9[..];
        self.vu0_aot.ok_gen = self.mem.vu0m_gen;
    }
}

pub struct VuState {
    vf: [[u32; 4]; 32],
    vacc: [u32; 4],
    vi: [u32; 32],
    d: Vec<u8>,
    stats: [u64; 6],
}
