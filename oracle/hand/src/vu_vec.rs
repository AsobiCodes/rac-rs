//! VU0 vector helpers: the retail micro sequences written out lane by lane (VF, ACC and $at end as retail leaves them).
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// The four lanes of a 128-bit value (lane 0 in the low word).
fn lanes(q: u128) -> V {
    V(std::array::from_fn(|i| (q >> (32 * i)) as u32))
}

/// A 128-bit value holding the VU lanes (the inverse of `lanes`).
fn quad(v: V) -> u128 {
    v.0.iter().enumerate().fold(0u128, |q, (i, &l)| q | (l as u128) << (32 * i))
}

/// $at after `mfc1 $at, $f`: the low 32 bits sign-extended to 64, the upper 64 bits kept.
fn mfc1_at(c: &Cpu, bits: u32) -> u128 {
    (c.r[1] & !(u64::MAX as u128)) | (bits as i32 as i64 as u64) as u128
}

/// 0x1f1870 vec_lerp_xyz(out, a, b, t): out = (a.xyz + (b.xyz - a.xyz) * t, a.w).
pub fn vec_lerp_xyz(c: &mut Cpu) -> bool {
    let t = c.farg(0);
    let at = mfc1_at(c, t.0);
    let a = c.rv(c.arg(1));
    let b = c.rv(c.arg(2));
    let d = b.with(b - a, XYZ);
    let d = d.with(d * t, XYZ);
    let r = a.with(a + d, XYZ);
    c.set_vf(1, r);
    c.set_vf(2, d);
    c.set_vf(3, lanes(at));
    c.r[1] = at;
    c.wv(c.arg(0), r);
    true
}

/// 0x1f19e8 dist3(a, b) -> f0: |a.xyz - b.xyz| (VU0 square, ACC.x sum, vsqrt; result also in the VF1 x lane).
pub fn dist3(c: &mut Cpu) -> bool {
    let a = c.rv(c.arg(0));
    let b = c.rv(c.arg(1));
    let vf3 = c.vf(3).with(V::splat(F::ZERO.vu_add(F::ONE)), XYZ);
    let d = a.with(a - b, XYZ);
    let sq = d.with(d * d, XYZ);
    let acc = c.acc().with(sq + V::splat(sq.y()), X);
    let sum = sq.with(V::madd(acc, vf3, V::splat(sq.z())), X);
    let root = F::ZERO.vu_add(sum.x().q_sqrt());
    let res = sum.with(V::splat(root), X);
    c.set_vf(1, res);
    c.set_vf(2, b);
    c.set_vf(3, vf3);
    c.set_acc(acc);
    c.r[4] = quad(res);
    c.fret(res.x());
    true
}

/// 0x1f1a98 vec_set_length(out, v, len): out = (v.xyz * len / |v.xyz|, v.w); a zero xyz gives (0, 0, 0, v.w).
pub fn vec_set_length(c: &mut Cpu) -> bool {
    let v = c.rv(c.arg(1));
    let len = c.farg(0);
    let vf3_ones = c.vf(3).with(V::splat(F::ZERO.vu_add(F::ONE)), XYZ);
    let sq = c.vf(2).with(v * v, XYZ);
    let acc = c.acc().with(sq + V::splat(sq.y()), X);
    let sum_v = sq.with(V::madd(acc, vf3_ones, V::splat(sq.z())), X);
    let sum = sum_v.x();
    let at = mfc1_at(c, len.0);
    let q = len.q_rsqrt(sum);
    let res = if sum.0 == 0 {
        v.with(V::splat(F::ZERO), XYZ)
    } else {
        v.with(v * q, XYZ)
    };
    c.set_vf(1, res);
    c.set_vf(2, sum_v);
    c.set_vf(3, lanes(at));
    c.set_acc(acc);
    c.r[1] = quad(sum_v);
    c.s(1, (sum.0 as u64) << 32);
    c.wv(c.arg(0), res);
    true
}

/// 0x1f1c90 vec4_mul_mat4(out, v, m): out = v * m (row vector times the four rows of m), ACC left as the last step.
pub fn vec4_mul_mat4(c: &mut Cpu) -> bool {
    let v = c.rv(c.arg(1));
    let m0 = c.arg(2);
    let m = [c.rv(m0), c.rv(m0 + 0x10), c.rv(m0 + 0x20), c.rv(m0 + 0x30)];
    let acc = m[0] * v.x();
    let acc = V::madd(acc, m[1], V::splat(v.y()));
    let acc = V::madd(acc, m[2], V::splat(v.z()));
    let out = V::madd(acc, m[3], V::splat(v.w()));
    c.set_vf(1, m[0]);
    c.set_vf(2, m[1]);
    c.set_vf(3, m[2]);
    c.set_vf(4, m[3]);
    c.set_vf(5, v);
    c.set_vf(6, out);
    c.set_acc(acc);
    c.wv(c.arg(0), out);
    true
}
