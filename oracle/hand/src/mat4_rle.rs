//! VU0 quad helpers (retail 0x1f19c8, 0x1f22e0, 0x1f23b8) and the bit-plane run-length packer (0x1f29b0).
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const AT: usize = 1;
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const T0: usize = 8;
const T1: usize = 9;
const T2: usize = 10;
const T3: usize = 11;
const T9: usize = 25;

/// `lui $at, 4`: the packer stops when its bit cursor reaches this count.
const SPAN_BITS: u32 = 0x4_0000;
/// `addiu $v1, $zero, 0x7f` on entry, then 0xff once a run has been closed.
const RUN_START: u32 = 0x7f;
const RUN_MAX: u32 = 0xff;

/// The 128 bits of a VU register as a GPR value (qmfc2).
fn vq_gpr(v: V) -> u128 {
    v.0.iter().enumerate().fold(0u128, |acc, (i, &l)| acc | (l as u128) << (32 * i))
}

/// 0x1f19c8 vec3_len_sq(v) -> f0: x*x + y*y + z*z in VU0 arithmetic (VF1 and a0 keep the squared quad).
pub fn vec3_len_sq(c: &mut Cpu) -> bool {
    let q = c.rv(c.arg(0));
    // vaddw.xyz vf2, vf0, vf0w: VF2.xyz = 0 + 1, w kept.
    let vf2 = c.vf(2).with_xyz(F::ONE, F::ONE, F::ONE);
    // vmul.xyz vf1, vf1, vf1
    let sq = q * q;
    let vf1 = q.with(sq, XYZ);
    // vadday.x ACC, vf1, vf1y: ACC.x = x + y
    let acc = c.acc().with(V::splat(vf1.x().vu_add(vf1.y())), X);
    // vmaddz.x vf1, vf2, vf1z: x = ACC.x + VF2.x * z
    let d = acc.x().vu_add(vf2.x().vu_mul(vf1.z()));
    let vf1 = vf1.with(V::splat(d), X);
    c.set_vf(1, vf1);
    c.set_vf(2, vf2);
    c.set_acc(acc);
    // qmfc2 $a0, vf1; mtc1 $a0, $f0 (low word = VF1.x)
    c.r[A0] = vq_gpr(vf1);
    c.fret(d);
    true
}

/// 0x1f22e0 mat4_scale(out, m, s): out = m * s on all 16 lanes (VU0 vmulx with s in VF5.x).
pub fn mat4_scale(c: &mut Cpu) -> bool {
    let out = c.arg(0);
    let m = c.arg(1);
    let s = c.farg(0);
    // mfc1 $at, $f12: the low 64 bits become the sign-extended word, the high 64 bits stay.
    let up = (c.r[AT] >> 64) as u64;
    let lo64 = (s.0 as i32 as i64) as u64;
    c.r[AT] = ((up as u128) << 64) | lo64 as u128;
    // qmtc2 $at, vf5: all 128 bits of $at go to VF5.
    let vf5 = V([lo64 as u32, (lo64 >> 32) as u32, up as u32, (up >> 32) as u32]);
    c.set_vf(5, vf5);
    let mut rows = [V([0; 4]); 4];
    for (i, row) in rows.iter_mut().enumerate() {
        // lqc2 vf(1+i), 0x10*i($a1); vmulx.xyzw vf(1+i), vf(1+i), vf5x
        *row = c.rv(m.wrapping_add(0x10 * i as u32)) * vf5.x();
    }
    for (i, row) in rows.iter().enumerate() {
        c.set_vf(1 + i, *row);
        c.wv(out.wrapping_add(0x10 * i as u32), *row);
    }
    true
}

/// 0x1f23b8 mat4_add(out, a, b): out = a + b, 4 rows (VU0 vadd.xyzw).
pub fn mat4_add(c: &mut Cpu) -> bool {
    let out = c.arg(0);
    let a = c.arg(1);
    let b = c.arg(2);
    // lqc2 vf1..4 from a, vf5..8 from b
    let mut ra = [V([0; 4]); 4];
    let mut rb = [V([0; 4]); 4];
    for i in 0..4 {
        ra[i] = c.rv(a.wrapping_add(0x10 * i as u32));
        rb[i] = c.rv(b.wrapping_add(0x10 * i as u32));
    }
    let mut sum = [V([0; 4]); 4];
    for i in 0..4 {
        // vadd.xyzw vf(1+i), vf(1+i), vf(5+i)
        sum[i] = ra[i] + rb[i];
        c.set_vf(1 + i, sum[i]);
        c.set_vf(5 + i, rb[i]);
    }
    for (i, row) in sum.iter().enumerate() {
        c.wv(out.wrapping_add(0x10 * i as u32), *row);
    }
    true
}

/// 0x1f29b0 rle_bits_pack(dst, n, pairs, plane) -> v0: writes run lengths of the bit plane into dst[0..n].
/// `pairs` holds (skip, count) byte pairs: skip bits are passed, then count bits are read from `plane`.
/// Returns the bytes written, or -1 when the output fills before the span ends.
pub fn rle_bits_pack(c: &mut Cpu) -> bool {
    let out0 = c.arg(0);
    let end = out0.wrapping_add(c.arg(1));
    let mut pairs = c.arg(2);
    let plane = c.arg(3);
    let mut dst = out0;
    let mut t0: u32 = 0;
    let mut t1: u32 = 1;
    let mut t2 = c.r[T2] as u32;
    let mut t3: u32 = 0;
    let mut t9 = c.r[T9] as u32;
    let mut v0: u32 = 0;
    let mut v1: u32 = RUN_START;
    let mut at: u32 = 0;
    let overflow = loop {
        // lbu $at, ($a2) runs on every pass (delay slot of the bnez).
        let skip = c.r8(pairs) as u32;
        at = skip;
        if v0 == 0 {
            v0 = c.r8(pairs.wrapping_add(1)) as u32;
            t0 = t0.wrapping_add(skip);
            let span_done = t0 == SPAN_BITS;
            pairs = pairs.wrapping_add(2);
            if span_done {
                break false;
            }
            continue;
        }
        // Read one bit of the plane at bit cursor t0.
        t9 = plane.wrapping_add(t0 >> 3);
        t2 = t0 & 7;
        t0 = t0.wrapping_add(1);
        let byte = c.r8(t9) as u32;
        v0 -= 1;
        let bit = (byte >> t2) & 1;
        at = bit;
        let changed = bit != t1;
        t1 = bit;
        if changed {
            // The run ends: store its length and start a new one.
            c.w8(dst, t3 as u8);
            dst = dst.wrapping_add(1);
            v1 = RUN_MAX;
            if dst == end {
                break true;
            }
            t3 = 1;
        } else if t3 != v1 {
            t3 += 1;
        } else {
            // A run of 0xff (or 0x7f) fills: store it as a 2-byte step.
            c.w8(dst, t3 as u8);
            dst = dst.wrapping_add(2);
            t3 = 1;
            v1 = RUN_MAX;
            at = end.wrapping_sub(dst);
            if dst == end {
                break true;
            }
            c.w8(dst.wrapping_sub(1), 0);
        }
    };
    if overflow {
        v0 = u32::MAX;
    } else {
        if t3 != 0 {
            c.w8(dst, t3 as u8);
            dst = dst.wrapping_add(1);
        }
        // lb $at, ($t6); sll $at, $at, 1; addi $at, $at, 1; sb $at, ($t6)
        let first = c.r8(out0) as i8 as i32 as u32;
        let marked = (first << 1).wrapping_add(1);
        c.w8(out0, marked as u8);
        at = marked;
        v0 = dst.wrapping_sub(out0);
    }
    c.s(A0, dst as u64);
    c.s(A1, end as u64);
    c.s(A2, pairs as u64);
    c.s(T0, t0 as u64);
    c.s(T1, t1 as u64);
    c.s(T2, t2 as u64);
    c.s(T3, t3 as u64);
    c.s(T9, t9 as u64);
    c.s(V1, v1 as u64);
    c.s(AT, at as u64);
    c.s(V0, v0 as u64);
    c.ret(v0);
    true
}
