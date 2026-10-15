//! Packed 64-bit record words (retail 0x242b98, 0x242bb8, 0x242be8) and a VU0 gap-length routine (0x242cf8).
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Offset of the 64-bit word these routines read and write inside the record.
const WORD_OFF: u32 = 0x38;
/// `lui $at, 0x3a80`: 2^-10 as f32 bits, loaded into VF3 by qmtc2.
const GAP_SCALE: F = F(0x3a80_0000);

const AT: usize = 1;
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;

fn r64(c: &mut Cpu, a: u32) -> u64 {
    let lo = c.r32(a) as u64;
    let hi = c.r32(a.wrapping_add(4)) as u64;
    lo | (hi << 32)
}

fn w64(c: &mut Cpu, a: u32, v: u64) {
    c.w32(a, v as u32);
    c.w32(a.wrapping_add(4), (v >> 32) as u32);
}

/// The 128 bits of a VU register as a GPR value (qmfc2).
fn quad(v: V) -> u128 {
    v.0.iter().enumerate().fold(0u128, |q, (i, &l)| q | (l as u128) << (32 * i))
}

/// 0x242b98 pack_word_low(rec, a1, a2, a3) -> none: rec+0x38 = (a1 << 32) | a2 | (a3 << 8) | (t0 << 16). Leaves v0 = that
/// word and a1, a3, t0 shifted as retail does.
pub fn pack_word_low(c: &mut Cpu) -> bool {
    const T0: usize = 8;
    let rec = c.arg(0);
    let a1 = c.g(A1) << 32;
    let a2 = c.g(A2);
    let a3 = c.g(A3) << 8;
    let t0 = c.g(T0) << 16;
    let v = a1 | a2 | a3 | t0;
    w64(c, rec.wrapping_add(WORD_OFF), v);
    c.s(V0, v);
    c.s(A1, a1);
    c.s(A3, a3);
    c.s(T0, t0);
    true
}

/// 0x242bb8 pack_word_high(rec, a1, a2, a3) -> none: rec+0x38 = (old low word) | a1 << 32 | a2 << 40 | a3 << 48.
/// Leaves v0 = that word and a1, a2, a3 shifted as retail does.
pub fn pack_word_high(c: &mut Cpu) -> bool {
    let rec = c.arg(0);
    let low = r64(c, rec.wrapping_add(WORD_OFF)) & 0xffff_ffff;
    let a1 = c.g(A1) << 32;
    let a2 = c.g(A2) << 40;
    let a3 = c.g(A3) << 48;
    let v = low | a1 | a2 | a3;
    w64(c, rec.wrapping_add(WORD_OFF), v);
    c.s(V0, v);
    c.s(A1, a1);
    c.s(A2, a2);
    c.s(A3, a3);
    true
}

/// 0x242be8 unpack_bytes(rec, out1, out2, out3) -> none: stores bits 32-39, 40-47 and 48-55 of rec+0x38 as words
/// to *out1, *out2, *out3.
pub fn unpack_bytes(c: &mut Cpu) -> bool {
    let rec = c.arg(0);
    let (o1, o2, o3) = (c.arg(1), c.arg(2), c.arg(3));
    let x = r64(c, rec.wrapping_add(WORD_OFF));
    let b1 = (x >> 32) & 0xff;
    let b2 = (x >> 40) & 0xff;
    let b3 = (x >> 48) & 0xff;
    c.w32(o1, b1 as u32);
    c.w32(o2, b2 as u32);
    c.w32(o3, b3 as u32);
    c.s(A0, x);
    c.s(AT, b1);
    c.s(V0, b2);
    c.s(V1, b3);
    true
}

/// 0x242cf8 vu_gap_len(a, b) -> f0: (|a.xyz - b.xyz| - (a.w + b.w)) * 2^-10, in VU0 arithmetic (a, b are quads).
pub fn vu_gap_len(c: &mut Cpu) -> bool {
    let a = c.rv(c.arg(0));
    let b = c.rv(c.arg(1));
    // vaddw.xyz vf3, vf0, vf0w: VF3.xyz = 0 + 1, w kept.
    let vf3 = c.vf(3).with_xyz(F::ONE, F::ONE, F::ONE);
    // vsub.xyz vf4, vf1, vf2; vmul.xyz vf4, vf4, vf4
    let sq = a - b;
    let sq = sq * sq;
    let vf4 = c.vf(4).with(sq, XYZ);
    // vadday.x ACC, vf4, vf4y: ACC.x = x + y
    let acc = c.acc().with(V::splat(sq.x().vu_add(sq.y())), X);
    // vmaddz.x vf4, vf3, vf4z: x = ACC.x + VF3.x * z
    let d2 = acc.x().vu_add(vf3.x().vu_mul(sq.z()));
    let vf4 = vf4.with(V::splat(d2), X);
    // vsqrt Q, vf4x
    let len = d2.q_sqrt();
    // vadd.w vf1, vf1, vf2
    let vf1 = a.with(V::splat(a.w().vu_add(b.w())), W);
    // qmtc2 $at, vf3: low 64 bits of $at = 0x3a800000 (lui); the upper 64 bits are the old $at (lui keeps them).
    let up = (c.r[AT] >> 64) as u64;
    let vf3 = V([GAP_SCALE.0, 0, up as u32, (up >> 32) as u32]);
    // vmulw.x vf1, vf3, vf1w
    let vf1 = vf1.with(V::splat(GAP_SCALE.vu_mul(vf1.w())), X);
    // vmulq.x vf3, vf3, Q
    let vf3 = vf3.with(V::splat(GAP_SCALE.vu_mul(len)), X);
    // vsub.x vf1, vf3, vf1
    let vf1 = vf1.with(V::splat(vf3.x().vu_sub(vf1.x())), X);
    c.set_vf(1, vf1);
    c.set_vf(2, b);
    c.set_vf(3, vf3);
    c.set_vf(4, vf4);
    c.set_acc(acc);
    // qmfc2 $at, vf1; mtc1 $at, f0
    c.r[AT] = quad(vf1);
    c.fret(vf1.x());
    true
}
