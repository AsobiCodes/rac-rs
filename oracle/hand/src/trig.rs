//! Fast trigonometry (VU0 microprograms at 0xc80 / 0xc90 and the EE-side atan2; twins in every overlay).
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const VU_PI: F = F(0x4049_0fda);
const VU_HALF_PI: F = F(0x3fc9_0fda);
/// sin Taylor terms x^3 .. x^9 (VU0 micro 0xcf0..0xd10).
const SIN_C: [F; 4] = [F(0xbe2a_aaa4), F(0x3c08_873e), F(0xb94f_b21d), F(0x362e_9c14)];

/// VU0 micro 0xc90: sine of x folded into [-pi/2, pi/2] (min(x, pi - x), then max with -pi - x), odd polynomial
/// to x^9 summed in the accumulator.
pub fn vu_sin(x: F) -> F {
    let (pi, npi) = (F::ZERO.vu_add(VU_PI), F::ZERO.vu_add(-VU_PI));
    let x = x.min(pi.vu_sub(x)).max(npi.vu_sub(x));
    let x2 = x.vu_mul(x);
    let x3 = x.vu_mul(x2);
    let x5 = x3.vu_mul(x2);
    let x7 = x5.vu_mul(x2);
    let x9 = x7.vu_mul(x2);
    let mut acc = x.vu_mul(F::ONE);
    for (p, k) in [x3, x5, x7, x9].into_iter().zip(SIN_C) {
        acc = acc.vu_add(p.vu_mul(k));
    }
    acc
}

/// VU0 micro 0xc80: cos x = sin(x + pi/2).
pub fn vu_cos(x: F) -> F {
    vu_sin(x.vu_add(VU_HALF_PI))
}

/// Sign-extends a 32-bit value into a 64-bit GPR (as mfc1 / lui / addiu do).
fn sx32(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// The four lanes of a VU register as the 128-bit quadword a GPR holds (lane 0 lowest).
fn from_quad(q: u128) -> V {
    V(std::array::from_fn(|i| (q >> (32 * i)) as u32))
}

fn quad(v: V) -> u128 {
    v.0.iter().enumerate().fold(0u128, |q, (i, &l)| q | (l as u128) << (32 * i))
}

const LANE_X: [bool; 4] = [true, false, false, false];

/// Writes the masked lanes of VFn; the others keep their value.
fn put(c: &mut Cpu, n: usize, mask: [bool; 4], val: [F; 4]) {
    let mut cur = c.vf(n).0;
    for i in 0..4 {
        if mask[i] {
            cur[i] = val[i].0;
        }
    }
    c.set_vf(n, V(cur));
}

fn lanes(c: &Cpu, n: usize) -> [F; 4] {
    c.vf(n).0.map(F)
}

/// Retail sequence `mfc1 $at, f12; qmtc2 $at, vf1; vcallms entry; qmfc2 $at, vf1; mtc1 $at, f0`: the VU0 micro
/// program at `entry` (0xc80 cos, 0xc90 sin) runs on the VU interpreter, so VF / ACC are left as retail leaves them.
fn vu_micro_call(c: &mut Cpu, entry: u32) -> F {
    c.s(1, sx32(c.f[12]));
    c.set_vf(1, from_quad(c.r[1]));
    let _ = c.vu0_micro(entry);
    c.r[1] = quad(c.vf(1));
    let r = F(c.r[1] as u32);
    c.fret(r);
    r
}

/// 0x1f1d80 fast_cos(x)
pub fn fast_cos(c: &mut Cpu) -> bool {
    vu_micro_call(c, 0xc80);
    true
}

/// 0x1f1d98 fast_sin(x)
pub fn fast_sin(c: &mut Cpu) -> bool {
    vu_micro_call(c, 0xc90);
    true
}

/// 0x1f1e48 fast_atan2(a, b): t = (min - max) / (min + max) of |a|, |b|; atan(t) + pi/4 from an odd polynomial to
/// t^15 (coefficients 0x1c2520), then scaled and offset per octant (table 0x1c2540, indexed by the signs of
/// |a| - |b|, b and a). Both zero gives 0.
pub fn fast_atan2(c: &mut Cpu) -> bool {
    let (a, b) = (c.farg(0), c.farg(1));
    let d = a.abs() - b.abs();
    let (lo, hi) = if d.neg_bit() { (a.abs(), b.abs()) } else { (b.abs(), a.abs()) };
    if hi <= F::ZERO {
        c.fret(F::ZERO);
        return true;
    }
    let t = (lo - hi) / (lo + hi);
    let oct = 0x1c2540 + 8 * d.neg_bit() as u32 + 16 * b.neg_bit() as u32 + 32 * a.neg_bit() as u32;
    let (scale, base) = (c.rf(oct), c.rf(oct + 4));
    // The VU lanes hold t as x, 0 + t as y and z, 1 * t as w.
    let (ty, tw) = (F::ZERO.vu_add(t), F::ONE.vu_mul(t));
    let t2 = [t.vu_mul(t), ty.vu_mul(ty), ty.vu_mul(ty), tw.vu_mul(tw)];
    let t4: [F; 4] = std::array::from_fn(|i| t2[i].vu_mul(t2[i]));
    let t8: [F; 4] = std::array::from_fn(|i| t4[i].vu_mul(t4[i]));
    let lo4 = [t, ty.vu_mul(t2[1]), ty.vu_mul(t4[2]), tw.vu_mul(t4[3]).vu_mul(t2[3])];
    let hi4: [F; 4] = std::array::from_fn(|i| lo4[i].vu_mul(t8[i]));
    let (k1, k2) = (c.rv(0x1c2520), c.rv(0x1c2530));
    let mut sum = lo4[0].vu_mul(k1.x());
    for i in 1..8 {
        let (p, k) = if i < 4 { (lo4[i], k1.lane(i)) } else { (hi4[i - 4], k2.lane(i - 4)) };
        sum = sum.vu_add(p.vu_mul(k));
    }
    let r = (F(0x3f49_0fdb) + sum) * scale + base;
    c.fret(r);
    true
}

/// 0x1f24c0 axis_quat(out, axis, angle): the quaternion of a rotation by -angle about axis 0 / 1 / 2 (x / y / z),
/// from sqrt(2 + |2 cos|) and -sin over it.
pub fn axis_quat(c: &mut Cpu) -> bool {
    let axis = c.g(5);
    let out = c.arg(0);
    // Retail scratch: t6 = angle bits, t7 = return address.
    c.s(14, sx32(c.f[12]));
    c.s(15, c.g(31));
    fast_sin(c);
    let neg_sin = -c.fresult();
    c.fret(neg_sin);
    // The jal's delay slot (mfc1 t0, f0) runs before fast_cos: t0 is -sin.
    c.s(8, sx32(neg_sin.0));
    fast_cos(c);
    let c2 = c.fresult() + c.fresult();
    c.fret(c2);
    c.s(7, sx32(c2.0));
    // qmtc2 a3, vf1 ; qmtc2 t0, vf2 ; lui v0, 0x4000 ; lui v1, 0x3f00 ; qmtc2 v0, vf3 ; qmtc2 v1, vf5
    c.set_vf(1, from_quad(c.r[7]));
    c.set_vf(2, from_quad(c.r[8]));
    // dsll32 a3, a3, 0: the sign of 2 cos is then bit 63 (bltz below).
    c.s(7, (c2.0 as u64) << 32);
    c.s(2, 0x4000_0000);
    c.s(3, 0x3f00_0000);
    c.set_vf(3, from_quad(c.r[2]));
    c.set_vf(5, from_quad(c.r[3]));
    // vabs.x vf1, vf1
    let v1 = lanes(c, 1);
    put(c, 1, LANE_X, [v1[0].abs(), v1[1], v1[2], v1[3]]);
    // vmulx.xyzw vf4, vf0, vf0x ; vaddx.xyzw vf5, vf4, vf5x
    let vf0 = lanes(c, 0);
    let mut v4 = vf0.map(|a| a.vu_mul(vf0[0]));
    let v5x = lanes(c, 5)[0];
    let v5 = v4.map(|a| a.vu_add(v5x));
    // vadd.x vf1, vf1, vf3 ; vsqrt Q, vf1x
    let v3x = lanes(c, 3)[0];
    let x1 = lanes(c, 1)[0].vu_add(v3x);
    put(c, 1, LANE_X, [x1, F::ZERO, F::ZERO, F::ZERO]);
    let mut q = x1.q_sqrt();
    let v2x = lanes(c, 2)[0];
    // vaddq.x vf6, vf0, Q (both paths, with the sqrt in Q)
    let x6 = vf0[0].vu_add(q);
    if c2.neg_bit() {
        // vaddq.x vf6, vf0, Q ; vmulq.x vf4, vf5, Q ; vdiv Q, vf2x, vf6x ; vaddq.w vf4, vf4, Q
        v4[0] = v5[0].vu_mul(q);
        q = v2x.q_div(x6);
        v4[3] = v4[3].vu_add(q);
    } else {
        // vaddq.x vf6, vf0, Q ; vmulq.w vf4, vf5, Q ; vdiv Q, vf2x, vf6x ; vaddq.x vf4, vf4, Q
        v4[3] = v5[3].vu_mul(q);
        q = v2x.q_div(x6);
        v4[0] = v4[0].vu_add(q);
    }
    put(c, 6, LANE_X, [x6, F::ZERO, F::ZERO, F::ZERO]);
    // The tail for the axis: vaddx.y (axis 1) or vaddx.z (other), then vadd.x vf4, vf0, vf0.
    if axis != 0 {
        let k = if axis == 1 { 1 } else { 2 };
        v4[k] = v4[k].vu_add(v4[0]);
        v4[0] = vf0[0].vu_add(vf0[0]);
    }
    c.set_vf(4, V(v4.map(|f| f.0)));
    c.set_vf(5, V(v5.map(|f| f.0)));
    c.s(5, axis.wrapping_sub(1));
    c.wv(out, V(v4.map(|f| f.0)));
    // Retail leaves 2 cos in f0.
    c.fret(c2);
    true
}
