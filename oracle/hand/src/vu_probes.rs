//! VU0 helpers of the 0x1f1a50..0x1f1cd8 block: a sign test on a difference and sum, a capped length, a reflection
//! and a byte-to-float pack.
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// The four lanes as one 128-bit GPR value (lane 0 in the low word).
fn quad(v: V) -> u128 {
    v.0.iter().enumerate().fold(0u128, |q, (i, &l)| q | (l as u128) << (32 * i))
}

/// 0x1f1a50 diff_dot_sign(a, b) -> 1 when the VU0 test value of (a - b) and (a + b) is negative, else 0.
pub fn diff_dot_sign(c: &mut Cpu) -> bool {
    let a = c.rv(c.arg(0));
    let b = c.rv(c.arg(1));
    c.set_vf(1, a);
    c.set_vf(2, b);
    let vf0 = c.vf(0);
    let mut vf3 = c.vf(3).with(a - b, XYZ);
    let mut vf4 = c.vf(4).with(a + b, W);
    vf3 = vf3.with(vf3 * vf3, XYZ);
    vf4 = vf4.with(vf4 * vf4, W);
    let vf5 = c.vf(5).with(vf0 + V::splat(vf0.w()), XYZ);
    let mut acc = c.acc();
    acc = acc.with(vf3 + V::splat(vf3.x()), Y);
    acc = acc.with(V::madd(acc, vf5, V::splat(vf3.z())), Y);
    vf3 = vf3.with(V::msub(acc, vf5, V::splat(vf4.w())), Y);
    c.set_vf(3, vf3);
    c.set_vf(4, vf4);
    c.set_vf(5, vf5);
    c.set_acc(acc);
    c.ret(vf3.y().neg_bit() as u32);
    true
}

/// 0x1f1b98 vec_cap_length_xy(out, v, cap) -> 1: when |v.xy| >= cap (and nonzero), writes v with xy scaled to length
/// cap to out; 0 leaves out untouched. The caller's float cap arrives in f12.
pub fn vec_cap_length_xy(c: &mut Cpu) -> bool {
    let cap = c.farg(0);
    let v = c.rv(c.arg(1));
    c.set_vf(1, v);
    let vf0 = c.vf(0);
    let mut vf2 = c.vf(2).with(v * v, X | Y);
    vf2 = vf2.with(vf2 + V::splat(vf2.y()), X);
    let len = vf2.x().q_sqrt();
    let hi = if cap.neg_bit() { 0xffff_ffff } else { 0 };
    let upper = (c.r[1] >> 64) as u64;
    let vf3 = V([cap.0, hi, upper as u32, (upper >> 32) as u32]);
    c.set_vf(3, vf3);
    vf2 = vf2.with(vf0 + V::splat(len), X);
    let vf4 = c.vf(4).with(vf3 - V::splat(len), X);
    let at = (vf2.x().0 as u64) | (vf2.y().0 as u64) << 32;
    let v0 = (vf4.x().0 as u64) << 32;
    c.set_vf(2, vf2);
    c.set_vf(4, vf4);
    c.r[1] = quad(vf2);
    if at == 0 || (v0 as i64) > 0 {
        c.ret(0);
        return true;
    }
    let q = vf3.x().q_div(vf2.x());
    let vf1 = v.with(v * V::splat(q), X | Y);
    c.set_vf(1, vf1);
    c.wv(c.arg(0), vf1);
    c.ret(1);
    true
}

/// 0x1f1bf8 reflect_vec(out, a, n) -> none: out = a reflected in the plane of unit-ized n, when a.n <= 0 (else a copy
/// of a, as the retail code leaves it).
pub fn reflect_vec(c: &mut Cpu) -> bool {
    let a = c.rv(c.arg(1));
    let b = c.rv(c.arg(2));
    let vf0 = c.vf(0);
    c.set_vf(1, a);
    c.set_vf(2, b);
    let mut vf1 = a.with(vf0 - a, XYZ);
    let mut vf2 = b;
    let vf3 = c.vf(3).with(vf0 + V::splat(vf0.w()), XYZ);
    let mut vf4 = c.vf(4).with(b * b, XYZ);
    let mut acc = c.acc();
    acc = acc.with(vf4 + V::splat(vf4.y()), X);
    vf4 = vf4.with(V::madd(acc, vf3, V::splat(vf4.z())), X);
    let q = vf0.w().q_rsqrt(vf4.x());
    vf2 = vf2.with(vf2 * V::splat(q), XYZ);
    vf4 = vf4.with(vf1 * vf2, XYZ);
    acc = acc.with(vf4 + V::splat(vf4.y()), X);
    vf4 = vf4.with(V::madd(acc, vf3, V::splat(vf4.z())), X);
    vf2 = vf2.with(vf2 * V::splat(vf4.x()), XYZ);
    let at = (vf4.x().0 as u64) << 32;
    if vf4.x().neg_bit() {
        c.wv(c.arg(0), a);
        c.r[1] = quad(a);
    } else {
        vf2 = vf2.with(vf2 - vf1, XYZ);
        vf2 = vf2.with(vf2 + vf2, XYZ);
        vf1 = vf1.with(vf1 + vf2, XYZ);
        c.wv(c.arg(0), vf1);
        c.r[1] = (quad(vf4) & !(u64::MAX as u128)) | at as u128;
    }
    c.set_vf(1, vf1);
    c.set_vf(2, vf2);
    c.set_vf(3, vf3);
    c.set_vf(4, vf4);
    c.set_acc(acc);
    true
}

/// 0x1f1cc0 pack_bytes_f32(out, value) -> none: the low four bytes of value as four floats to the quad at out.
pub fn pack_bytes_f32(c: &mut Cpu) -> bool {
    let v = c.arg(1);
    let bytes = V(std::array::from_fn(|i| (v >> (8 * i)) & 0xff));
    let vf1 = bytes.itof(0);
    c.set_vf(1, vf1);
    c.wv(c.arg(0), vf1);
    c.r[5] = quad(bytes);
    true
}
