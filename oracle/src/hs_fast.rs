//! Speed style for speed hands (lane hs, S2c): helpers. See scratch_perf_hs/HS_RUNTIME.md "Speed style".
//!
//! Rules of the style
//! - Live-in registers are read once at entry into Rust locals (`lo(c, n)` = the low 64 bits; 128-bit values only where the
//!   function moves quadwords), live-outs are written back once per exit (`wlo`: a plain 64-bit store, no read-modify-write of the
//!   128-bit register) and in front of every call / `deopt`. `c.icount` is a local `ic` (`ic += T[BLOCK]`, constant-folded) and is
//!   stored to `c` only in front of a call, a deopt and at the exit.
//! - Memory: `Fm::p(addr, n)` = ONE range test for RAM (`addr < 32 MB - n`) or scratchpad, else None. None means "not plain memory"
//!   (MMIO, VU0 data mirror, kseg/uncached mirrors, ...): the hand spills its locals and `deopt`s at that instruction with the
//!   exact icount, so the retail slow path runs it. Alignment is not tested (the recompiled code does not either).
//! - Events: `ic + n > limit` before the first block of a segment (hs::due_within), reloaded after every call.
//! - Never FMA. VU0 macro ops: `vu0` module (SSE, bit-exact with core `fpu::vadd_c` etc.).
use crate::cpu::Cpu;

/// S5c: exact MMI helpers (hs_fast/mmi.rs) and more VU0 macro / COP2 helpers (hs_fast/vu0x.rs, re-exported in `vu0`).
pub mod mmi;
mod vu0x;

/// Plain-memory view of one hand run: raw base pointers of RAM and scratchpad (the Vecs never move while a hand runs).
#[derive(Clone, Copy)]
pub struct Fm {
    ram: *mut u8,
    spr: *mut u8,
}

const RAM_END: u32 = 0x0200_0000;

impl Fm {
    #[inline(always)]
    pub fn new(c: &mut Cpu) -> Fm {
        Fm { ram: c.mem.ram.as_mut_ptr(), spr: c.mem.spr.as_mut_ptr() }
    }
    /// Pointer to `n` plain bytes at `a` (RAM or scratchpad), else None.
    #[inline(always)]
    pub fn p(&self, a: u32, n: u32) -> Option<*mut u8> {
        #[cfg(feature = "hs_deny")]
        if deny_tick() {
            return None;
        }
        if a <= RAM_END - n {
            return Some(unsafe { self.ram.add(a as usize) });
        }
        if a >> 28 == 7 && (a & 0x0fff_ffff) <= 0x4000 - n {
            return Some(unsafe { self.spr.add((a & 0x0fff_ffff) as usize) });
        }
        None
    }
    #[inline(always)]
    pub fn r8(&self, a: u32) -> Option<u8> {
        self.p(a, 1).map(|p| unsafe { p.read() })
    }
    #[inline(always)]
    pub fn r16(&self, a: u32) -> Option<u16> {
        self.p(a, 2).map(|p| unsafe { (p as *const u16).read_unaligned() })
    }
    #[inline(always)]
    pub fn r32(&self, a: u32) -> Option<u32> {
        self.p(a, 4).map(|p| unsafe { (p as *const u32).read_unaligned() })
    }
    #[inline(always)]
    pub fn r64(&self, a: u32) -> Option<u64> {
        self.p(a, 8).map(|p| unsafe { (p as *const u64).read_unaligned() })
    }
    /// lq: the address is rounded down to 16.
    #[inline(always)]
    pub fn r128(&self, a: u32) -> Option<u128> {
        self.p(a & !15, 16).map(|p| unsafe { (p as *const u128).read_unaligned() })
    }
    #[inline(always)]
    pub fn w8(&self, a: u32, v: u8) -> bool {
        self.p(a, 1).map(|p| unsafe { p.write(v) }).is_some()
    }
    #[inline(always)]
    pub fn w16(&self, a: u32, v: u16) -> bool {
        self.p(a, 2).map(|p| unsafe { (p as *mut u16).write_unaligned(v) }).is_some()
    }
    #[inline(always)]
    pub fn w32(&self, a: u32, v: u32) -> bool {
        self.p(a, 4).map(|p| unsafe { (p as *mut u32).write_unaligned(v) }).is_some()
    }
    #[inline(always)]
    pub fn w64(&self, a: u32, v: u64) -> bool {
        self.p(a, 8).map(|p| unsafe { (p as *mut u64).write_unaligned(v) }).is_some()
    }
    /// sq: the address is rounded down to 16.
    #[inline(always)]
    pub fn w128(&self, a: u32, v: u128) -> bool {
        self.p(a & !15, 16).map(|p| unsafe { (p as *mut u128).write_unaligned(v) }).is_some()
    }
}

#[cfg(feature = "hs_deny")]
thread_local! {
    static DENY: std::cell::Cell<(u32, u32)> = const { std::cell::Cell::new((0, 0)) };
}

/// S6c (gate builds, feature `hs_deny`): from now on every `n`-th `Fm` access of this thread is refused (None; `phase` shifts which
/// one), so the gate runs a hand's MMIO slow paths and deopts on plain memory and compares them with the reference. `n` = 0: off.
#[cfg(feature = "hs_deny")]
pub fn deny_set(n: u32, phase: u32) {
    DENY.with(|d| d.set((n, if n == 0 { 0 } else { phase % n })));
}

#[cfg(feature = "hs_deny")]
#[inline(always)]
fn deny_tick() -> bool {
    DENY.with(|d| {
        let (n, k) = d.get();
        if n == 0 {
            return false;
        }
        let hit = k + 1 >= n;
        d.set((n, if hit { 0 } else { k + 1 }));
        hit
    })
}

/// Low 64 bits of GPR `n`.
#[inline(always)]
pub fn lo(c: &Cpu, n: usize) -> u64 {
    c.r[n] as u64
}

/// Writes the low 64 bits of GPR `n` (the high half stays): one 64-bit store (little endian). `n` must not be 0.
#[inline(always)]
pub fn wlo(c: &mut Cpu, n: usize, v: u64) {
    debug_assert!(n != 0);
    unsafe { (c.r.as_mut_ptr().add(n) as *mut u64).write(v) }
}

#[inline(always)]
pub fn sx32(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// Spill + deopt in one: `fastdeopt!(c, fr, pc, ic; spill statements)`.
#[macro_export]
macro_rules! fastdeopt {
    ($c:expr, $fr:expr, $pc:expr, $ic:expr; $($spill:stmt;)*) => {{
        $($spill;)*
        return $crate::hs::deopt($c, &$fr, $pc, $ic);
    }};
}

/// S2f/S2g: ONE hardware (MMIO / non-plain) load or store run inline, exactly as the recompiled code does it (rec `sq`/`hq`: `Cpu::hw` = the interpreter
/// runs that one instruction with icount `ic`, then `go(rem)` = no event due within the `rem` instructions that follow). The hand has WRITTEN BACK every
/// live local first (`flush!()`: the access reads/writes `c`) and reloads what the access changed (a load's destination) afterwards. Workers use the
/// `ld8!..ld128!` / `st8!..st128!` macros of gen.rs, which do all of this in one line.
/// `pc` = the instruction's address (not a delay slot, not a branch; its word is read from the code in RAM), `ic` = the instruction count AFTER it
/// (block start + index + 1), `rem` = instructions after it until the hand's next due check (rec: the rest of the block; a hand: the longest path to its next
/// `due_within`/exit). Returns true when no event became due: the hand goes on in Rust; false: `return deopt(c, fr, pc + 4, c.icount)` WITHOUT another flush
/// (retail resumes at the next instruction and services the event at the exact instruction; the stitched driver returns to Rust at the next region entry).
/// Under the gate's tracing the access is recorded in the MMIO sequence like an interpreted one.
#[inline(never)]
pub fn mmio(c: &mut Cpu, pc: u32, ic: u64, rem: u64) -> bool {
    let op = c.mem.frd32(pc).unwrap_or(0);
    crate::hs::trace_hw(c, pc, ic.wrapping_sub(1));
    c.hw(op, pc, ic, pc.wrapping_add(4));
    c.icount + rem <= c.mem.limit
}

// ---------------------------------------------------------------------------------------------------------------------
// VU0 macro-op helpers (S2e). `V` = one VF register / ACC in an xmm local. Safe API (region code may not write `unsafe`).
// Bit-exact with core `fpu::vadd_g/vsub_g/vmul_g::<true>` (S2f: add/sub/mul are ONE addps/subps/mulps under a `Chop` guard = MXCSR CSR_CHOP, the
// recompiled code's own form; add_pv/mul_pv below are the older exact-without-guard copies, unused). No FMA. Rules:
//  * `prep()` a value that came from memory / an old register once; results of add/sub/mul/madd are already prepared (prep is a no-op on them).
//    Loop-invariant operands: prep once at region entry. Never feed raw data to add/sub/mul/madd.
//  * Lane masks `M` are lane bits (x=1, y=2, z=4, w=8). All four lanes are computed; `blend::<M>` keeps the other lanes of the old value.
//  * Flags: the MAC/status of the LAST flag-setting op are computed once at the exit (`V::mac(dest)`, PS2 dest layout x=8..w=1, xy = 0xc);
//    the sticky status bits 6-7 only when not already both set at region entry (`Sticky`).
pub mod vu0 {
    use std::arch::x86_64::*;

    const MAXB: i32 = 0x7f7f_ffff;

    #[derive(Clone, Copy)]
    pub struct V(pub __m128i);

    #[inline(always)]
    unsafe fn sel(m: __m128i, a: __m128i, b: __m128i) -> __m128i {
        _mm_or_si128(_mm_and_si128(m, a), _mm_andnot_si128(m, b))
    }
    #[inline(always)]
    unsafe fn prep_raw(v: __m128i) -> __m128i {
        let e = _mm_and_si128(v, _mm_set1_epi32(0x7f80_0000));
        let s = _mm_and_si128(v, _mm_set1_epi32(0x8000_0000u32 as i32));
        let v = sel(_mm_cmpeq_epi32(e, _mm_set1_epi32(0x7f80_0000)), _mm_or_si128(s, _mm_set1_epi32(MAXB)), v);
        sel(_mm_cmpeq_epi32(e, _mm_setzero_si128()), s, v)
    }
    #[allow(dead_code)]
    #[inline(always)]
    unsafe fn add_pv(a: __m128i, b: __m128i) -> __m128i {
        let mag = _mm_set1_epi32(0x7fff_ffff);
        let big = _mm_cmpgt_epi32(_mm_and_si128(b, mag), _mm_and_si128(a, mag));
        let x = _mm_castsi128_ps(sel(big, b, a));
        let y = _mm_castsi128_ps(sel(big, a, b));
        let s = _mm_add_ps(x, y);
        let err = _mm_sub_ps(y, _mm_sub_ps(s, x));
        let si = _mm_castps_si128(s);
        let ei = _mm_castps_si128(err);
        let nz = _mm_andnot_si128(_mm_cmpeq_epi32(_mm_and_si128(ei, mag), _mm_setzero_si128()), _mm_set1_epi32(-1));
        let opp = _mm_srai_epi32(_mm_xor_si128(si, ei), 31);
        let r = _mm_add_epi32(si, _mm_and_si128(nz, opp));
        let e = _mm_and_si128(r, _mm_set1_epi32(0x7f80_0000));
        let sign = _mm_and_si128(r, _mm_set1_epi32(0x8000_0000u32 as i32));
        let r = sel(_mm_cmpeq_epi32(e, _mm_set1_epi32(0x7f80_0000)), _mm_or_si128(sign, _mm_set1_epi32(MAXB)), r);
        sel(_mm_cmpeq_epi32(e, _mm_setzero_si128()), sign, r)
    }
    #[allow(dead_code)]
    #[inline(always)]
    unsafe fn mul_pv(a: __m128i, b: __m128i) -> __m128i {
        let (a, b) = (_mm_castsi128_ps(a), _mm_castsi128_ps(b));
        let plo = _mm_mul_pd(_mm_cvtps_pd(a), _mm_cvtps_pd(b));
        let phi = _mm_mul_pd(_mm_cvtps_pd(_mm_movehl_ps(a, a)), _mm_cvtps_pd(_mm_movehl_ps(b, b)));
        let r = _mm_movelh_ps(_mm_cvtpd_ps(plo), _mm_cvtpd_ps(phi));
        let abs = _mm_castsi128_pd(_mm_set1_epi64x(0x7fff_ffff_ffff_ffff));
        let (aplo, aphi) = (_mm_and_pd(plo, abs), _mm_and_pd(phi, abs));
        let ul = _mm_castpd_ps(_mm_cmpgt_pd(_mm_and_pd(_mm_cvtps_pd(r), abs), aplo));
        let uh = _mm_castpd_ps(_mm_cmpgt_pd(_mm_and_pd(_mm_cvtps_pd(_mm_movehl_ps(r, r)), abs), aphi));
        let up = _mm_castps_si128(_mm_shuffle_ps(ul, uh, 0b10_00_10_00));
        let tmin = _mm_set1_pd(f32::MIN_POSITIVE as f64);
        let tl = _mm_castpd_ps(_mm_cmplt_pd(aplo, tmin));
        let th = _mm_castpd_ps(_mm_cmplt_pd(aphi, tmin));
        let tiny = _mm_castps_si128(_mm_shuffle_ps(tl, th, 0b10_00_10_00));
        let ri = _mm_add_epi32(_mm_castps_si128(r), up);
        sel(tiny, _mm_and_si128(_mm_castps_si128(r), _mm_set1_epi32(0x8000_0000u32 as i32)), ri)
    }
    #[inline(always)]
    fn lane_mask<const M: u32>() -> __m128i {
        let l = |b: u32| if M & b != 0 { -1 } else { 0 };
        unsafe { _mm_set_epi32(l(8), l(4), l(2), l(1)) }
    }

    /// One ssse op without MXCSR switching (needs a live `Chop`). Same asm shape as core `fpu::ps_op_nc`.
    macro_rules! nc_op {
        ($ins:literal, $a:expr, $b:expr) => {{
            let mut x: __m128i = $a;
            unsafe {
                std::arch::asm!(concat!($ins, " {x}, {y}"), x = inout(xmm_reg) x, y = in(xmm_reg) $b, options(nostack, nomem));
            }
            x
        }};
    }

    /// Holds MXCSR at CSR_CHOP (round toward zero, DAZ, FTZ, exceptions masked) for the V add / sub / mul / madd ops: one per region visit
    /// (or stitched call). `end()` restores MXCSR (call it before a deopt / exit; idempotent, also done on drop).
    pub struct Chop(Option<ee_core::fpu::ChopGuard>);
    impl Chop {
        #[inline(always)]
        pub fn new() -> Chop {
            Chop(Some(ee_core::fpu::ChopGuard::new()))
        }
        #[inline(always)]
        pub fn end(&mut self) {
            self.0 = None;
        }
        /// Re-arms the guard after an `end()` (a region that did an inline MMIO access in the middle: `flush!()` ended it).
        #[inline(always)]
        pub fn begin(&mut self) {
            if self.0.is_none() {
                self.0 = Some(ee_core::fpu::ChopGuard::new());
            }
        }
    }

    impl V {
        /// Raw bits of a VF register / ACC.
        #[inline(always)]
        pub fn ld(a: [u32; 4]) -> V {
            V(unsafe { _mm_loadu_si128(a.as_ptr() as *const __m128i) })
        }
        #[inline(always)]
        pub fn from_u128(x: u128) -> V {
            V(unsafe { _mm_loadu_si128(&x as *const u128 as *const __m128i) })
        }
        #[inline(always)]
        pub fn to_u128(self) -> u128 {
            let mut o = 0u128;
            unsafe { _mm_storeu_si128(&mut o as *mut u128 as *mut __m128i, self.0) };
            o
        }
        #[inline(always)]
        pub fn st(self) -> [u32; 4] {
            let mut o = [0u32; 4];
            unsafe { _mm_storeu_si128(o.as_mut_ptr() as *mut __m128i, self.0) };
            o
        }
        /// Operand as the ops read it (exponent 255 -> +-FLT_MAX, exponent 0 -> +-0). Idempotent.
        #[inline(always)]
        pub fn prep(self) -> V {
            V(unsafe { prep_raw(self.0) })
        }
        #[inline(always)]
        pub fn bcx(self) -> V {
            V(unsafe { _mm_shuffle_epi32::<0x00>(self.0) })
        }
        #[inline(always)]
        pub fn bcy(self) -> V {
            V(unsafe { _mm_shuffle_epi32::<0x55>(self.0) })
        }
        #[inline(always)]
        pub fn bcz(self) -> V {
            V(unsafe { _mm_shuffle_epi32::<0xaa>(self.0) })
        }
        #[inline(always)]
        pub fn bcw(self) -> V {
            V(unsafe { _mm_shuffle_epi32::<0xff>(self.0) })
        }
        /// VADD / VSUB / VMUL on PREPARED operands (lane-wise, all four lanes). Results are prepared. S2f: ONE addps / subps / mulps
        /// under MXCSR = CSR_CHOP, exactly the instruction core `fpu::vadd_g::<true>` etc. runs for recompiled code (`vout(addps_nc(vin(a), vin(b)))`;
        /// `vin` is the prep, `vout` is the identity for prepared operands: round toward zero never overflows to inf and NaN needs inf).
        /// A live `Chop` guard is REQUIRED (else the result is wrong; the gate would show it).
        #[inline(always)]
        pub fn add(self, o: V) -> V {
            V(nc_op!("addps", self.0, o.0))
        }
        #[inline(always)]
        pub fn sub(self, o: V) -> V {
            V(nc_op!("subps", self.0, o.0))
        }
        #[inline(always)]
        pub fn mul(self, o: V) -> V {
            V(nc_op!("mulps", self.0, o.0))
        }
        /// MADD: `self` (ACC, prepared) + a * b. (MSUB: `acc.sub(a.mul(b))`.) Two ops, like the recompiled code.
        #[inline(always)]
        pub fn madd(self, a: V, b: V) -> V {
            self.add(a.mul(b))
        }
        /// Flip the sign of the lanes in `M` (turns an add into a subtract in those lanes: x - y = x + (-y)).
        #[inline(always)]
        pub fn neg<const M: u32>(self) -> V {
            V(unsafe { _mm_xor_si128(self.0, _mm_and_si128(lane_mask::<M>(), _mm_set1_epi32(0x8000_0000u32 as i32))) })
        }
        /// VABS on the lanes in `M`, the others unchanged (raw bit clear, no prep).
        #[inline(always)]
        pub fn abs<const M: u32>(self) -> V {
            V(unsafe { _mm_and_si128(self.0, _mm_or_si128(_mm_andnot_si128(lane_mask::<M>(), _mm_set1_epi32(-1)), _mm_and_si128(lane_mask::<M>(), _mm_set1_epi32(0x7fff_ffff)))) })
        }
        /// Lanes in `M` from `new`, the others from `self` (= the old register value).
        #[inline(always)]
        pub fn blend<const M: u32>(self, new: V) -> V {
            V(unsafe { sel(lane_mask::<M>(), new.0, self.0) })
        }
        /// VFTOI0 on a PREPARED value: truncate toward zero, saturate (same words as core `fpu::vftoi(v, 1.0)`); all four lanes.
        #[inline(always)]
        pub fn ftoi0(self) -> V {
            unsafe {
                let r = _mm_cvttps_epi32(_mm_castsi128_ps(self.0));
                let ovf = _mm_cmpeq_epi32(r, _mm_set1_epi32(i32::MIN));
                let pos = _mm_cmpgt_epi32(self.0, _mm_set1_epi32(-1)); // sign bit clear
                V(_mm_xor_si128(r, _mm_and_si128(ovf, pos)))
            }
        }
        /// Sign bits of the four lanes (bit l = lane l).
        #[inline(always)]
        pub fn signs(self) -> u32 {
            unsafe { _mm_movemask_ps(_mm_castsi128_ps(self.0)) as u32 }
        }
        /// Low 64 bits (lanes x, y).
        #[inline(always)]
        pub fn lo64(self) -> u64 {
            unsafe { _mm_cvtsi128_si64(self.0) as u64 }
        }
        #[inline(always)]
        pub fn hi64(self) -> u64 {
            unsafe { _mm_cvtsi128_si64(_mm_unpackhi_epi64(self.0, self.0)) as u64 }
        }
        /// MAC flags of a result under `dest` (PS2 dest layout x=8 .. w=1): core `vu::mac_of`.
        #[inline(always)]
        pub fn mac(self, dest: u32) -> u32 {
            ee_core::vu::mac_of(self.st(), dest)
        }
    }

    /// Sticky accumulator for the status bits 6-7 (only needed when they are not both set at region entry): OR of the zero / sign
    /// state of every flag-setting result over its lanes `M`. `bits()` = the 2-bit status (zero | sign << 1) to shift into bits 6-7.
    #[derive(Clone, Copy)]
    pub struct Sticky(__m128i, __m128i);
    impl Sticky {
        #[inline(always)]
        pub fn new() -> Sticky {
            unsafe { Sticky(_mm_setzero_si128(), _mm_setzero_si128()) }
        }
        #[inline(always)]
        pub fn acc<const M: u32>(&mut self, v: V) {
            unsafe {
                let z = _mm_cmpeq_epi32(_mm_and_si128(v.0, _mm_set1_epi32(0x7fff_ffff)), _mm_setzero_si128());
                self.0 = _mm_or_si128(self.0, _mm_and_si128(z, lane_mask::<M>()));
                self.1 = _mm_or_si128(self.1, _mm_and_si128(v.0, lane_mask::<M>()));
            }
        }
        #[inline(always)]
        pub fn bits(self) -> u32 {
            unsafe {
                let z = _mm_movemask_ps(_mm_castsi128_ps(self.0)) != 0;
                let s = _mm_movemask_ps(_mm_castsi128_ps(self.1)) != 0;
                (z as u32) | ((s as u32) << 1)
            }
        }
    }

    /// Status bits 0-1 a result sets, from its MAC flags (as `Cpu::vu_out`).
    #[inline(always)]
    pub fn st_of_mac(m: u32) -> u32 {
        ((m & 0xf != 0) as u32) | (((m & 0xf0 != 0) as u32) << 1)
    }

    pub use super::vu0x::*;
}

/// A window of `N` plain bytes (RAM or scratchpad) at an address: constant-offset reads without per-access range tests.
#[derive(Clone, Copy)]
pub struct Win<const N: u32>(*mut u8);

impl Fm {
    /// `N` bytes at `a` in one range test, else None (then deopt at the block start).
    #[inline(always)]
    pub fn win<const N: u32>(&self, a: u32) -> Option<Win<N>> {
        self.p(a, N).map(Win)
    }
}
impl<const N: u32> Win<N> {
    #[inline(always)]
    pub fn r8<const O: u32>(&self) -> u8 {
        assert!(O + 1 <= N);
        unsafe { self.0.add(O as usize).read() }
    }
    #[inline(always)]
    pub fn r16<const O: u32>(&self) -> u16 {
        assert!(O + 2 <= N);
        unsafe { (self.0.add(O as usize) as *const u16).read_unaligned() }
    }
    #[inline(always)]
    pub fn r32<const O: u32>(&self) -> u32 {
        assert!(O + 4 <= N);
        unsafe { (self.0.add(O as usize) as *const u32).read_unaligned() }
    }
}
