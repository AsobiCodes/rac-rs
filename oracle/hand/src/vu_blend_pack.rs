//! VU0 blend, pack and basis helpers (batch b219): the pack of a vec4 into bytes, the quaternion normalised lerp,
//! the 4-vector basis from a vector, and the sphere fade store.

use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Return address of the `jal sphere_visible` inside 0x1f2970 (ra after the call).
const FADE_RA_AFTER_CALL: u64 = 0x1f2980;
/// The fade clamp limit (the `addi $at, $zero, 0x80` of 0x1f2970).
const FADE_MAX: u32 = 0x80;

/// VU broadcast operand: lane `l` of `v` in all four lanes.
fn bc(v: V, l: usize) -> V {
    V::splat(v.lane(l))
}

/// 0x1f1cd8 pack_vec4_bytes(vec) -> v0: VFTOI0 of the vec4 at a0, then the low byte of x, y, z, w packed into v0.
pub fn pack_vec4_bytes(c: &mut Cpu) -> bool {
    let src = c.arg(0);
    let v = c.rv(src).ftoi(0);
    c.set_vf(1, v);
    // QMFC2 $at, vf1: the low dword holds x and y.
    c.s(1, v.0[0] as u64 | (v.0[1] as u64) << 32);
    // PPACH then PPACB keep the low byte of each word, x in byte 0.
    let packed = (0..4).fold(0u32, |acc, i| acc | ((v.0[i] & 0xff) << (8 * i)));
    c.ret(packed);
    true
}

/// 0x1f2440 quat_nlerp(out, q1, q2, t): normalised lerp of two quaternions, flipping q2 when q1.q2 < 0.
pub fn quat_nlerp(c: &mut Cpu) -> bool {
    let (out, qa, qb) = (c.arg(0), c.arg(1), c.arg(2));
    let t = c.farg(0);
    // MFC1 $at, $f12 sign-extends the word into the low dword; the high dword of $at is left as it was.
    let at_lo = t.0 as i32 as i64 as u64;
    let at_hi = (c.r[1] >> 64) as u64;
    let at = at_lo as u128 | (at_hi as u128) << 64;
    // QMTC2 $at, vf3 (lanes x, y, z, w from the low to the high word).
    let mut vf3 = V([at as u32, (at >> 32) as u32, (at >> 64) as u32, (at >> 96) as u32]);
    let mut vf1 = c.rv(qa);
    let mut vf2 = c.rv(qb);
    let mut vf7 = c.vf(7);
    let mut acc = c.acc();

    vf3 = vf3.with(V::ZERO_W1 - bc(vf3, 0), W);
    vf7 = vf7.with(V::ZERO_W1 + bc(V::ZERO_W1, 3), XYZ);
    vf1 = vf1 * bc(vf3, 3);
    vf2 = vf2 * bc(vf3, 0);
    let mut vf4 = vf1 + vf2;
    let mut vf6 = vf1 * vf2;
    let mut vf5 = vf4 * vf4;
    acc = acc.with(vf6 + bc(vf6, 0), Y);
    acc = acc.with(V::madd(acc, vf7, bc(vf6, 2)), Y);
    vf6 = vf6.with(V::madd(acc, vf7, bc(vf6, 3)), Y);
    acc = acc.with(vf5 + bc(vf5, 1), X);
    acc = acc.with(V::madd(acc, vf7, bc(vf5, 2)), X);
    vf5 = vf5.with(V::madd(acc, vf7, bc(vf5, 3)), X);
    // QMFC2 $t1, vf6; BGEZ $t1: the sign of the low dword is the sign of vf6.y. Negative: flip q2.
    if vf6.y().0 & 0x8000_0000 != 0 {
        vf4 = vf1 - vf2;
        vf5 = vf4 * vf4;
        acc = acc.with(vf5 + bc(vf5, 1), X);
        acc = acc.with(V::madd(acc, vf7, bc(vf5, 2)), X);
        vf5 = vf5.with(V::madd(acc, vf7, bc(vf5, 3)), X);
    }
    // VRSQRT Q, vf0.w, vf5.x; VMULQ: normalise vf4 by Q.
    let q = V::ZERO_W1.w().q_rsqrt(vf5.x());
    vf1 = vf4 * q;
    c.wv(out, vf1);
    c.s(1, at_lo);
    c.s(9, vf6.0[0] as u64 | (vf6.0[1] as u64) << 32);
    c.set_vf(1, vf1);
    c.set_vf(2, vf2);
    c.set_vf(3, vf3);
    c.set_vf(4, vf4);
    c.set_vf(5, vf5);
    c.set_vf(6, vf6);
    c.set_vf(7, vf7);
    c.set_acc(acc);
    true
}

/// 0x1f25f8 basis_from_vec(vec, out) -> (): four VU0 vectors built from the vec4 at a0, stored to a1.
pub fn basis_from_vec(c: &mut Cpu) -> bool {
    let (src, dst) = (c.arg(0), c.arg(1));
    let v0 = V::ZERO_W1;
    let mut r = [V::default(); 18];
    for (n, slot) in r.iter_mut().enumerate().skip(9) {
        *slot = c.vf(n);
    }
    r[8] = c.rv(src);
    r[14] = v0 * bc(v0, 0);
    r[15] = v0 * bc(v0, 0);
    r[16] = V([v0.0[1], v0.0[2], v0.0[3], v0.0[0]]);
    r[17] = v0;
    r[14] = r[14].with(r[14] + bc(v0, 3), X);
    r[15] = r[15].with(r[15] + bc(v0, 3), Y);
    r[9] = r[8] + r[8];
    r[10] = r[10].with(r[9] * bc(r[8], 3), XYZ);
    r[11] = r[11].with(r[9] * bc(r[8], 0), XYZ);
    r[12] = r[12].with(r[9] * bc(r[8], 1), Y | Z);
    r[13] = r[13].with(r[9] * bc(r[8], 2), Z);
    r[15] = r[15].with(v0 + bc(r[10], 2), X);
    r[16] = r[16].with(v0 - bc(r[10], 1), X);
    r[16] = r[16].with(v0 + bc(r[10], 0), Y);
    r[14] = r[14].with(r[14] - bc(r[12], 1), X);
    r[15] = r[15].with(r[15] - bc(r[11], 0), Y);
    r[16] = r[16].with(r[16] - bc(r[11], 0), Z);
    r[14] = r[14].with(r[11] - bc(r[10], 2), Y);
    r[14] = r[14].with(r[11] + bc(r[10], 1), Z);
    r[15] = r[15].with(r[12] - bc(r[10], 0), Z);
    r[15] = r[15].with(r[15] + bc(r[11], 1), X);
    r[16] = r[16].with(r[16] + bc(r[11], 2), X);
    r[16] = r[16].with(r[16] + bc(r[12], 2), Y);
    r[14] = r[14].with(r[14] - bc(r[13], 2), X);
    r[15] = r[15].with(r[15] - bc(r[13], 2), Y);
    r[16] = r[16].with(r[16] - bc(r[12], 1), Z);
    c.wv(dst, r[14]);
    c.wv(dst + 0x10, r[15]);
    c.wv(dst + 0x20, r[16]);
    c.wv(dst + 0x30, r[17]);
    for (n, v) in r.iter().enumerate().skip(8) {
        c.set_vf(n, *v);
    }
    true
}

/// 0x1f2970 sphere_fade_store(sphere, out) -> (): sphere_visible (frustum) test, then the fade level
/// min((int)y >> 7, 128) stored to a1, or 0 when the sphere is outside (v0 < 0).
pub fn sphere_fade_store(c: &mut Cpu) -> bool {
    let (sphere, dst, ra) = (c.arg(0), c.arg(1), c.g(31));
    let bias = c.farg(0);
    let (v0, _) = crate::h::call_keep(c, 0x1f2878, &[sphere], &[bias]);
    // VFTOI0.y vf3, vf3 (runs in the delay slot on both paths).
    let vf3 = c.vf(3);
    let vf3i = vf3.with(vf3.ftoi(0), Y);
    c.set_vf(3, vf3i);
    c.s(24, dst as u64);
    c.s(25, ra);
    c.s(31, FADE_RA_AFTER_CALL);
    c.ret(v0);
    if (v0 as i32) < 0 {
        c.w32(dst, 0);
    } else {
        // QMFC2 $v1, vf3; DSRL32 by 7: the y lane shifted right 7; PMAXW 0, PMINW 0x80.
        let shifted = vf3i.y().0 >> 7;
        let level = shifted.min(FADE_MAX);
        c.s(1, FADE_MAX as u64);
        c.s(3, level as u64);
        c.w32(dst, level);
    }
    true
}
