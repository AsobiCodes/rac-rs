//! Quaternion vector rotation (0x252738) and the slot bitmap release (0x259b88).
use crate::h;
use crate::ps2::*;

use ee_oracle::cpu::Cpu;

// ---- rotate_vec_quat ----

/// Frame size of the retail prologue (`addiu sp,sp,-0x80`).
const ROT_FRAME: u32 = 0x80;
/// Scratch quaternion offsets from the new sp.
const NEG_Q: u32 = 0x00;
const SRC_Q: u32 = 0x10;
const TMP_Q: u32 = 0x20;
/// Retail vec_scale(out, v, s) and quat_mul(out, a, b), run as retail code (their VU0 side effects are exact).
const VEC_SCALE: u32 = 0x001f_1898;
const QUAT_MUL: u32 = 0x001f_2400;
/// Bits of -1.0f.
const NEG_ONE_BITS: u32 = 0xbf80_0000;
/// Offset of the w lane (word 3) of a quaternion.
const LANE_W: u32 = 0x0c;

/// 0x252738 rotate_vec_quat(out, src, q) -> void: rotates the xyz of `src` by quaternion `q`, out = q * src' * conj(q),
/// where src' is src with w = 0 and conj(q) is q with xyz negated.
pub fn rotate_vec_quat(c: &mut Cpu) -> bool {
    let out = c.arg(0);
    let src = c.arg(1);
    let q = c.arg(2);
    let saves = [(19, 0x60, h::SQ), (18, 0x50, h::SQ), (16, 0x30, h::SQ), (17, 0x40, h::SQ), (31, 0x70, h::SQ)];
    h::frame(c, ROT_FRAME, &saves, |c, sp| {
        // conj(q) = vec_scale(q, -1), then its w lane is q.w again.
        c.s(1, NEG_ONE_BITS as i32 as i64 as u64);
        c.f[12] = NEG_ONE_BITS;
        h::call_keep(c, VEC_SCALE, &[sp + NEG_Q, q], &[F(NEG_ONE_BITS)]);
        let w = c.r32(q + LANE_W);
        c.f[0] = w;
        c.w32(sp + NEG_Q + LANE_W, w);
        // src' = src with w = 0.
        c.r[2] = c.mem.rd128(src & !0xf);
        c.mem.wr128((sp + SRC_Q) & !0xf, c.r[2]);
        c.w32(sp + SRC_Q + LANE_W, 0);
        // tmp = q * src'
        h::call_keep(c, QUAT_MUL, &[sp + TMP_Q, q, sp + SRC_Q], &[]);
        // out = tmp * conj(q)
        h::call_keep(c, QUAT_MUL, &[out, sp + TMP_Q, sp], &[]);
    });
    true
}

// ---- slot_release ----

/// gp-relative words: slot base, running minimum, free cursor, countdown.
const GP_BASE: i32 = -0x6b14;
const GP_MIN: i32 = -0x6b10;
const GP_CURSOR: i32 = -0x6b0c;
const GP_COUNT: i32 = -0x6b08;
/// Bitmap of slot indices, and its second copy 0x100 bytes on.
const BITMAP: u32 = 0x001b_1980;
const BITMAP_COPY: u32 = 0x100;
/// Slot flag byte (offset 1) bit that selects the second bitmap copy, and the value it is left with.
const FLAG_COPY: u8 = 0x40;
const FLAG_DONE: u8 = 0x80;

/// Bit-clear mask for slot index `idx` within its bitmap byte.
fn clear_mask(idx: u32) -> u8 {
    !(1u8 << (idx & 7))
}

/// Lane-wise signed minimum of four 32-bit words (PMINW).
fn pminw(a: u128, b: u128) -> u128 {
    let mut out = 0u128;
    for lane in 0..4 {
        let sh = lane * 32;
        let x = (a >> sh) as u32 as i32;
        let y = (b >> sh) as u32 as i32;
        out |= (x.min(y) as u32 as u128) << sh;
    }
    out
}

/// 0x259b88 slot_release(p) -> void: frees the slot at p. Its index is (p - base) >> 6. Clears the index's bit in
/// the bitmap (and in the second copy when p[1] has 0x40), moves the free cursor down to the next set bit when
/// the cursor was this index, folds the index into the running minimum and sets p[1] = 0x80.
pub fn slot_release(c: &mut Cpu) -> bool {
    let p = c.arg(0);
    let gp = c.g(28) as u32;
    let gp_word = |off: i32| gp.wrapping_add(off as u32);
    let base = c.r32(gp_word(GP_BASE));
    let idx = p.wrapping_sub(base) >> 6;
    let flags = c.r8(p.wrapping_add(1));
    c.s(14, base as i32 as i64 as u64);
    c.s(15, BITMAP as u64);
    let count = c.r32(gp_word(GP_COUNT)).wrapping_sub(1);
    c.w32(gp_word(GP_COUNT), count);
    let t9_old = c.r32(gp_word(GP_CURSOR)) as i32;
    let min_word = c.r32(gp_word(GP_MIN));
    c.s(24, min_word as i32 as i64 as u64);
    c.s(5, idx as u64);

    if flags & FLAG_COPY != 0 {
        let a = BITMAP + (idx >> 3) + BITMAP_COPY;
        let b = c.r8(a) & clear_mask(idx);
        c.w8(a, b);
    }
    let a = BITMAP + (idx >> 3);
    let mask = clear_mask(idx);
    let a3 = c.r8(a) & mask;
    c.w8(a, a3);

    let mut v0 = mask as u32;
    let mut v1 = idx & 7;
    let mut t2 = c.g(10) as u32;
    let mut t9 = t9_old.wrapping_sub(1);
    if idx == t9_old as u32 {
        if t9 >= 0 {
            let mut at = (t9 as u32) >> 3;
            v1 = c.r8(BITMAP + at) as u32;
            t2 = (t9 as u32) & 7;
            v0 = 1 << t2;
            loop {
                t2 = v1 & v0;
                v0 >>= 1;
                if t2 != 0 {
                    break;
                }
                t9 -= 1;
                if v0 != 0 {
                    continue;
                }
                at = at.wrapping_sub(1);
                if t9 < 0 {
                    // Retail's bltz delay slot (add v1, at, t7) runs before the exit: v1 is the byte address.
                    v1 = BITMAP.wrapping_add(at);
                    break;
                }
                v1 = c.r8(BITMAP + at) as u32;
                v0 = 0x80;
            }
        }
        c.w32(gp_word(GP_CURSOR), t9 as u32);
    }
    // Retail's delay slot and the not-taken path both leave t9 decremented.
    c.s(25, t9 as i64 as u64);

    c.w8(p.wrapping_add(1), FLAG_DONE);
    c.r[24] = pminw(c.r[24], c.r[5]);
    let new_min = c.r[24] as u32;
    c.w32(gp_word(GP_MIN), new_min);

    c.s(1, FLAG_DONE as u64);
    c.s(2, v0 as u64);
    c.s(3, v1 as u64);
    c.s(6, 0xff);
    c.s(7, a3 as u64);
    c.s(10, t2 as u64);
    true
}
