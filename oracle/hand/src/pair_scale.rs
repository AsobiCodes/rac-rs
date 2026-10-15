//! Pair scaling step and class flag: helpers on the actor and path records (Eudora addresses).
use crate::h;
use crate::mathx;
use crate::packed_vu;
use crate::path;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Pair record: s16 value at +0 (also the output), s16 countdown at +2, s16 sources at +0xc and +0xe, bytes at +4..+6.
const VALUE: u32 = 0x0;
const COUNT: u32 = 0x2;
const SRC_A: u32 = 0xc;
const SRC_B: u32 = 0xe;
const BYTE_A: u32 = 0x4;
const BYTE_B: u32 = 0x5;
const BYTE_C: u32 = 0x6;
/// Stack frame of `scale_pair_step`: saves s0, s1, s2 and ra.
const PAIR_FRAME: u32 = 0x50;
/// Stack frame of `class_flag`: saves s0 and ra.
const FLAG_FRAME: u32 = 0x20;
/// Class record field tested by `class_flag`, and the class value it must equal.
const CLASS_FIELD: u32 = 0xa6;
const CLASS_WANTED: i32 = 0x1f6;

/// float_to_int_scaled of the s16 field at `addr` (retail 0x1f14b8).
fn scaled_field(c: &mut Cpu, addr: u32) -> u32 {
    let n = c.r16(addr) as i16 as i32 as u32;
    h::run(c, mathx::float_to_int_scaled, &[n], &[]).0 as u32
}

/// 0x250018 scale_pair_step(owner, p) -> int: when the s16 at p is nonzero, rescales it by the ratio of the two source
/// fields (returns the new value, or 1 when it is not positive); when it is zero, unpacks three bytes into p+4..p+6
/// (returns the first byte). The countdown at p+2 is cleared unless it was already zero (then returns 0).
pub fn scale_pair_step(c: &mut Cpu) -> bool {
    let owner = c.arg(0) as u32;
    let p = c.arg(1) as u32;
    let saves = [(16, 0x10, h::SQ), (17, 0x20, h::SQ), (18, 0x30, h::SQ), (31, 0x40, h::SQ)];
    h::frame(c, PAIR_FRAME, &saves, |c, sp| {
        let s2 = c.r16(p + VALUE) as i16 as i32;
        if s2 != 0 && c.r16(p + COUNT) == 0 {
            c.ret(0);
            return;
        }
        let first = scaled_field(c, p + SRC_A);
        c.w16(p + VALUE, first as u16);
        c.w16(p + COUNT, 0);
        if s2 == 0 {
            h::run(c, packed_vu::unpack_bytes, &[owner, sp, sp + 4, sp + 8], &[]);
            let (b0, b1, b2) = (c.r8(sp), c.r8(sp + 4), c.r8(sp + 8));
            c.w8(p + BYTE_A, b0);
            c.w8(p + BYTE_C, b2);
            c.w8(p + BYTE_B, b1);
            c.ret(b0 as u32);
            return;
        }
        // 0x25005c: retail calls float_to_int_scaled(p+0xc) and discards the result.
        scaled_field(c, p + SRC_A);
        // 0x250068: this third result is s0, moved to f1 at 0x250084, and is the value multiplied.
        let third = scaled_field(c, p + SRC_B);
        let ratio = F::from_int(s2) / F::from_int(third as i32);
        let scaled = F::from_int(third as i32) * ratio;
        let y = scaled.to_int() as u32;
        if (y as i16) > 0 {
            c.w16(p + VALUE, y as u16);
            c.ret(y);
        } else {
            c.w16(p + VALUE, 1);
            c.ret(1);
        }
    });
    true
}

/// 0x250f28 class_flag(obj) -> int: 0 for a null object, 1 when path::is_class_500_540 is false, else whether the
/// object's field at 0xa6 is 0x1f6.
pub fn class_flag(c: &mut Cpu) -> bool {
    let obj = c.arg(0) as u32;
    let saves = [(16, 0x0, h::SQ), (31, 0x10, h::SQ)];
    h::frame(c, FLAG_FRAME, &saves, |c, _sp| {
        let v = if obj == 0 {
            0
        } else if h::run(c, path::is_class_500_540, &[obj], &[]).0 as u32 == 0 {
            1
        } else {
            (c.r16(obj + CLASS_FIELD) as i16 as i32 == CLASS_WANTED) as u32
        };
        c.ret(v);
    });
    true
}
