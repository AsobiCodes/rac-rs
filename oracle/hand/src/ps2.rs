//! PS2 arithmetic and register access for the hand-written functions. Floats compute as the EE FPU and VU0 do
//! (round toward zero, denormals read as zero, no inf/NaN: `ee_oracle::fpu`), so a rewrite reads like ordinary Rust
//! and stays bit-exact with the retail code it replaces.
use ee_oracle::cpu::Cpu;
use ee_oracle::fpu;
use std::ops::{Add, Div, Mul, Neg, Sub};
use std::sync::atomic::AtomicU64;

/// Nested hand functions (`h::run`) that declined where no retail entry could take the call over (the call did
/// nothing). fndiff reports a change of this counter as BAD [nested-decline].
pub static LOST_DECLINES: AtomicU64 = AtomicU64::new(0);

/// An EE FPU single (COP1: ADD/SUB without guard bits, MUL rounds toward zero, DIV and SQRT to nearest).
#[derive(Clone, Copy, Debug, Default)]
pub struct F(pub u32);

impl F {
    pub const ZERO: F = F(0);
    pub const ONE: F = F(0x3f80_0000);

    pub fn new(x: f32) -> F {
        F(x.to_bits())
    }
    /// The value as the hardware compares it (denormals read as zero, exponent 255 as +-FLT_MAX).
    pub fn get(self) -> f32 {
        fpu::fz(self.0)
    }
    /// ABS.S
    pub fn abs(self) -> F {
        F(self.0 & 0x7fff_ffff)
    }
    /// SQRT.S
    pub fn sqrt(self) -> F {
        F(fpu::sqrt(self.0, fpu::CSR_NEAR))
    }
    /// RSQRT.S: self / sqrt(|b|).
    pub fn rsqrt(self, b: F) -> F {
        F(fpu::rsqrt(self.0, b.0, fpu::CSR_NEAR))
    }
    /// MAX.S
    pub fn max(self, b: F) -> F {
        F(fpu::max(self.0, b.0))
    }
    /// MIN.S
    pub fn min(self, b: F) -> F {
        F(fpu::min(self.0, b.0))
    }
    /// CVT.W.S: toward zero, clamped to the i32 range.
    pub fn to_int(self) -> i32 {
        fpu::cvt_w(self.0) as i32
    }
    /// CVT.S.W
    pub fn from_int(i: i32) -> F {
        F(fpu::cvt_s(i as u32))
    }
    /// The sign bit (what `bltz` on the MFC1'd word tests).
    pub fn neg_bit(self) -> bool {
        self.0 & 0x8000_0000 != 0
    }
    /// One VU lane's ADD (no guard-bit step, unlike `+`).
    pub fn vu_add(self, b: F) -> F {
        F(fpu::fout(fpu::addss(fpu::fin(self.0), fpu::fin(b.0), fpu::CSR_CHOP)))
    }
    /// One VU lane's SUB.
    pub fn vu_sub(self, b: F) -> F {
        F(fpu::fout(fpu::subss(fpu::fin(self.0), fpu::fin(b.0), fpu::CSR_CHOP)))
    }
    /// One VU lane's MUL (same rounding as `*`).
    pub fn vu_mul(self, b: F) -> F {
        F(fpu::mul(self.0, b.0))
    }
    /// VDIV Q = self / b (rounded toward zero, unlike the FPU's DIV.S).
    pub fn q_div(self, b: F) -> F {
        F(fpu::div(self.0, b.0, fpu::CSR_CHOP))
    }
    /// VSQRT Q = sqrt(self).
    pub fn q_sqrt(self) -> F {
        F(fpu::sqrt(self.0, fpu::CSR_CHOP))
    }
    /// VRSQRT Q = self / sqrt(b).
    pub fn q_rsqrt(self, b: F) -> F {
        F(fpu::rsqrt(self.0, b.0, fpu::CSR_CHOP))
    }
}

impl Add for F {
    type Output = F;
    fn add(self, o: F) -> F {
        F(fpu::add(self.0, o.0))
    }
}
impl Sub for F {
    type Output = F;
    fn sub(self, o: F) -> F {
        F(fpu::sub(self.0, o.0))
    }
}
impl Mul for F {
    type Output = F;
    fn mul(self, o: F) -> F {
        F(fpu::mul(self.0, o.0))
    }
}
impl Div for F {
    type Output = F;
    fn div(self, o: F) -> F {
        F(fpu::div(self.0, o.0, fpu::CSR_NEAR))
    }
}
impl Neg for F {
    type Output = F;
    fn neg(self) -> F {
        F(self.0 ^ 0x8000_0000)
    }
}
/// C.EQ.S (denormals as zero).
impl PartialEq for F {
    fn eq(&self, o: &F) -> bool {
        self.get() == o.get()
    }
}
/// C.LT.S / C.LE.S
impl PartialOrd for F {
    fn partial_cmp(&self, o: &F) -> Option<std::cmp::Ordering> {
        self.get().partial_cmp(&o.get())
    }
}

/// VU destination masks (the `.xyzw` suffix).
pub const X: u32 = 8;
pub const Y: u32 = 4;
pub const Z: u32 = 2;
pub const W: u32 = 1;
pub const XYZ: u32 = X | Y | Z;

/// A VU0 vector (VF lanes x, y, z, w; the VU adds plainly, without the FPU's guard-bit step).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct V(pub [u32; 4]);

impl V {
    /// VF0: (0, 0, 0, 1).
    pub const ZERO_W1: V = V([0, 0, 0, 0x3f80_0000]);

    pub fn splat(f: F) -> V {
        V([f.0; 4])
    }
    pub fn x(self) -> F {
        F(self.0[0])
    }
    pub fn y(self) -> F {
        F(self.0[1])
    }
    pub fn z(self) -> F {
        F(self.0[2])
    }
    pub fn w(self) -> F {
        F(self.0[3])
    }
    pub fn lane(self, i: usize) -> F {
        F(self.0[i])
    }
    /// The xyz lanes replaced (w kept).
    pub fn with_xyz(self, x: F, y: F, z: F) -> V {
        V([x.0, y.0, z.0, self.0[3]])
    }
    /// `o`'s lanes where `dest` is set (a masked VU write over `self`).
    pub fn with(self, o: V, dest: u32) -> V {
        let mut v = self;
        for i in 0..4 {
            if dest & (8 >> i) != 0 {
                v.0[i] = o.0[i];
            }
        }
        v
    }
    /// VMADD: acc + a * b, each rounded.
    pub fn madd(acc: V, a: V, b: V) -> V {
        acc + a * b
    }
    /// VMSUB: acc - a * b, each rounded.
    pub fn msub(acc: V, a: V, b: V) -> V {
        acc - a * b
    }
    pub fn max(self, o: V) -> V {
        V(std::array::from_fn(|i| fpu::max(self.0[i], o.0[i])))
    }
    pub fn min(self, o: V) -> V {
        V(std::array::from_fn(|i| fpu::min(self.0[i], o.0[i])))
    }
    /// VFTOI0 / 4 / 12 / 15 (`frac` = 0, 4, 12, 15 fraction bits): scaled toward zero, then to int (clamped).
    pub fn ftoi(self, frac: u32) -> V {
        let sc = (1u32 << frac) as f32;
        V(self.0.map(|l| fpu::cvt_w(fpu::fout(fpu::mulss(fpu::fin(l), sc, fpu::CSR_CHOP)))))
    }
    /// VITOF0 / 4 / 12 / 15: int lanes to floats, scaled down by 2^frac.
    pub fn itof(self, frac: u32) -> V {
        let sc = 1.0 / (1u32 << frac) as f32;
        V(self.0.map(|l| fpu::fout(fpu::mulss(fpu::cvtsi2ss(l as i32, fpu::CSR_CHOP), sc, fpu::CSR_CHOP))))
    }
}

impl Add for V {
    type Output = V;
    fn add(self, o: V) -> V {
        V(fpu::vadd(self.0, o.0))
    }
}
impl Sub for V {
    type Output = V;
    fn sub(self, o: V) -> V {
        V(fpu::vsub(self.0, o.0))
    }
}
impl Mul for V {
    type Output = V;
    fn mul(self, o: V) -> V {
        V(fpu::vmul(self.0, o.0))
    }
}
/// The broadcast forms (VMULx and so on).
impl Mul<F> for V {
    type Output = V;
    fn mul(self, o: F) -> V {
        self * V::splat(o)
    }
}

/// Registers and memory as the retail code sees them.
pub trait Ee {
    /// Integer argument `i` (a0..a3, then t0..t3).
    fn arg(&self, i: usize) -> u32;
    /// Float argument `i` (f12..f19).
    fn farg(&self, i: usize) -> F;
    /// Sets integer argument `i` for a `call` (sign-extended).
    fn set_arg(&mut self, i: usize, v: u32);
    /// Sets float argument `i` for a `call`.
    fn set_farg(&mut self, i: usize, f: F);
    /// The integer result of a `call` (v0).
    fn result(&self) -> u32;
    /// The float result of a `call` (f0).
    fn fresult(&self) -> F;
    /// Integer result in v0 (sign-extended, as LW / ADDIU leave it).
    fn ret(&mut self, v: u32);
    /// Float result in f0.
    fn fret(&mut self, f: F);
    fn r8(&mut self, a: u32) -> u8;
    fn r16(&mut self, a: u32) -> u16;
    fn r32(&mut self, a: u32) -> u32;
    fn rf(&mut self, a: u32) -> F;
    /// LQC2 (the address's low four bits are ignored).
    fn rv(&mut self, a: u32) -> V;
    fn w8(&mut self, a: u32, v: u8);
    fn w16(&mut self, a: u32, v: u16);
    fn w32(&mut self, a: u32, v: u32);
    fn wf(&mut self, a: u32, f: F);
    /// SQC2
    fn wv(&mut self, a: u32, v: V);
    /// VU0 register VFn (VF0 is (0, 0, 0, 1)).
    fn vf(&self, n: usize) -> V;
    /// Sets VFn (all four lanes; VF0 stays as it is). Leave every VF register retail leaves written with
    /// retail's value: later retail code reads VU temporaries, and fndiff counts any VF difference as BAD [vf].
    fn set_vf(&mut self, n: usize, v: V);
    /// The VU0 accumulator ACC.
    fn acc(&self) -> V;
    /// Sets ACC (as retail's last VMULA/VMADDA/... leaves it; fndiff counts a difference as BAD [acc]).
    fn set_acc(&mut self, v: V);
}

impl Ee for Cpu {
    fn arg(&self, i: usize) -> u32 {
        self.g(4 + i) as u32
    }
    fn farg(&self, i: usize) -> F {
        F(self.f[12 + i])
    }
    fn set_arg(&mut self, i: usize, v: u32) {
        self.s(4 + i, v as i32 as i64 as u64);
    }
    fn set_farg(&mut self, i: usize, f: F) {
        self.f[12 + i] = f.0;
    }
    fn result(&self) -> u32 {
        self.g(2) as u32
    }
    fn fresult(&self) -> F {
        F(self.f[0])
    }
    fn ret(&mut self, v: u32) {
        self.s(2, v as i32 as i64 as u64);
    }
    fn fret(&mut self, f: F) {
        self.f[0] = f.0;
    }
    fn r8(&mut self, a: u32) -> u8 {
        self.mem.rd8(a)
    }
    fn r16(&mut self, a: u32) -> u16 {
        self.mem.rd16(a)
    }
    fn r32(&mut self, a: u32) -> u32 {
        self.mem.rd32(a)
    }
    fn rf(&mut self, a: u32) -> F {
        F(self.mem.rd32(a))
    }
    fn rv(&mut self, a: u32) -> V {
        let q = self.mem.rd128(a & !15);
        V(std::array::from_fn(|i| (q >> (32 * i)) as u32))
    }
    fn w8(&mut self, a: u32, v: u8) {
        self.mem.wr8(a, v)
    }
    fn w16(&mut self, a: u32, v: u16) {
        self.mem.wr16(a, v)
    }
    fn w32(&mut self, a: u32, v: u32) {
        self.mem.wr32(a, v)
    }
    fn wf(&mut self, a: u32, f: F) {
        self.mem.wr32(a, f.0)
    }
    fn wv(&mut self, a: u32, v: V) {
        let q = v.0.iter().enumerate().fold(0u128, |q, (i, &l)| q | (l as u128) << (32 * i));
        self.mem.wr128(a & !15, q)
    }
    fn vf(&self, n: usize) -> V {
        V(self.vf[n])
    }
    fn set_vf(&mut self, n: usize, v: V) {
        if n != 0 {
            self.vf[n] = v.0;
        }
    }
    fn acc(&self) -> V {
        V(self.vacc)
    }
    fn set_acc(&mut self, v: V) {
        self.vacc = v.0;
    }
}
