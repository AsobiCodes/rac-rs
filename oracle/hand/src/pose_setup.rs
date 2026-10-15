//! Pose struct rebuild: basis vectors and slots of the struct at obj->0x70, built from a stack buffer.
use crate::ps2::*;
use crate::h::{self, SQ};
use ee_oracle::cpu::Cpu;

/// Retail frame size of 0x2e76a0 (addiu sp,sp,-0xa0).
const FRAME: u32 = 0xa0;
/// Retail helper 0x2e7530, called with the stack buffer as a0 and a1 = 0.
const HELPER: u32 = 0x2e7530;
/// Constant vector block in the overlay (s3 = 0x166f10; the rows are read from RAM at call time).
const TABLE: u32 = 0x166f10;
/// Offset of the first constant row (s4 = TABLE + 0x30).
const ROW_A: u32 = 0x30;
/// Offset of the second constant row (s3 after `addiu s3,s3,0x20`).
const ROW_B: u32 = 0x20;
/// Retail vector helpers: dot3(a, b) -> f0, out = a.xyz +/- b.xyz, out = v.xyz * s, zero_vec4(dst).
const DOT3: u32 = 0x1f1930;
const VEC_ADD: u32 = 0x1f1810;
const VEC_SUB: u32 = 0x1f1840;
const VEC_SCALE: u32 = 0x1f1898;
const ZERO_VEC4: u32 = 0x1f17f8;
/// IEEE bits of 1.0f.
const ONE_BITS: u32 = 0x3f80_0000;

/// 0x2e76a0 pose_setup(obj): rebuilds the basis vectors and slots of the pose struct at obj->0x70 using a stack buffer
/// (dot3 and vector scale/add/sub steps), then clears the flags at +0xc4..+0xc7 and sets the scale word at +0xc8.
pub fn pose_setup(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    // Saved s0-s5 and ra, as the retail prologue stores them at sp+0x30..0x90.
    let saves = [
        (16, 0x30, SQ),
        (17, 0x40, SQ),
        (18, 0x50, SQ),
        (19, 0x60, SQ),
        (20, 0x70, SQ),
        (21, 0x80, SQ),
        (31, 0x90, SQ),
    ];
    h::frame(c, FRAME, &saves, |c, sp| setup_body(c, obj, sp));
    true
}

/// The body of `pose_setup` with the frame at `sp`; every call and store follows the retail order.
fn setup_body(c: &mut Cpu, obj: u32, sp: u32) {
    let s2 = c.r32(obj + 0x70);
    let s1 = s2 + 0x40;
    let s5 = s2 + 0x130;
    let s4 = TABLE + ROW_A;
    let s0 = sp + 0x10;

    // dot = dot3(buf, row A); buf2 = row A * dot; s1 = buf - buf2.
    h::call_keep(c, HELPER, &[sp, 0], &[]); // retail: move $a1,$zero before the jal
    let (_, dot) = h::call_keep(c, DOT3, &[sp, s4], &[]);
    h::call_keep(c, VEC_SCALE, &[s0, s4], &[dot]);
    h::call_keep(c, VEC_SUB, &[s1, sp, s0], &[]);
    // Copy buf2 to s2+0x50, then s1 += that copy.
    let quad = c.mem.rd128(s0);
    c.mem.wr128(s2 + 0x50, quad);
    c.r[2] = quad;
    h::call_keep(c, VEC_ADD, &[s1, s1, s2 + 0x50], &[]);

    // buf3 = row B * s1.xyz-scale (float at s1+0xb0); two sums into the slots at +0x80 and +0xd0.
    let row_b = TABLE + ROW_B;
    let k = c.rf(s1 + 0xb0);
    h::call_keep(c, VEC_SCALE, &[sp + 0x20, row_b], &[k]);
    h::call_keep(c, VEC_ADD, &[s2 + 0x80, sp + 0x20, s1], &[]);
    h::call_keep(c, VEC_ADD, &[s2 + 0xd0, sp + 0x20, s1], &[]);
    h::call_keep(c, ZERO_VEC4, &[s2 + 0xe0], &[]);

    // $at = 1.0 bits, f0 = 1.0; flags and the scale word of the slot at s1.
    c.r[1] = ONE_BITS as u128;
    c.f[0] = ONE_BITS;
    c.r[3] = (s2 + 0xa0) as u128;
    c.w8(s1 + 0xc4, 0);
    c.w32(s1 + 0xc8, ONE_BITS);
    c.w8(s1 + 0xc5, 0);
    c.w16(s1 + 0xc6, 0);
    // Copy the first quadword of s1 to s2+0xa0 (v0 keeps the value).
    let q = c.mem.rd128(s1);
    c.r[2] = q;
    c.mem.wr128(s2 + 0xa0, q);

    h::call_keep(c, ZERO_VEC4, &[s2 + 0xb0], &[]);
    h::call_keep(c, ZERO_VEC4, &[s2 + 0xc0], &[]);
    let k = c.rf(s5 + 0x30);
    h::call_keep(c, VEC_SCALE, &[sp + 0x20, row_b], &[k]);
    h::call_keep(c, VEC_ADD, &[s2 + 0x90, sp + 0x20, s1], &[]);
}
