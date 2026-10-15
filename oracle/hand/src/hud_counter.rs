//! HUD counter helpers of the level 4 slot code (0x228bb0..0x22a31c): alignment of a slot's x/y pair, counter sizing
//! from the digit count of its maximum, and the slot init that calls both.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Slot fields (object pointer in a0).
const OBJ_PTR: u32 = 0x0c;
const OBJ_MAX: u32 = 0x08;
const OBJ_WIDTH: u32 = 0x58;
const OBJ_HEIGHT: u32 = 0x5c;
const OBJ_FLAGS: u32 = 0x60;
const OBJ_CLAMP_LO: u32 = 0x74;
const OBJ_CLAMP_HI: u32 = 0x78;
const OBJ_SCALED: u32 = 0x7c;

/// Flag bits at +0x60.
const FLAG_NO_Y_HALF: u32 = 0x1 | 0x2;
const FLAG_NO_X: u32 = 0x4;
const FLAG_X_FULL: u32 = 0x8;
const FLAG_WIDE_WIDTH: u32 = 0xc;

/// Clamp used when the value pointer is null or unaligned (100000 - 1).
const CLAMP_DEFAULT: u32 = 0x1_869f;
/// Digit count of a value below 10 is handled without the division loop.
const DECIMAL: u32 = 10;
/// Counter width limit and its minimum (min 14).
const WIDTH_STEP: u32 = 14;
/// Height steps per digit.
const HEIGHT_STEP: u32 = 12;
/// Init value of the slot's size fields.
const SLOT_SIZE: u32 = 0x20;
/// Added to the scaled value stored at +0x7c.
const SCALED_BIAS: u32 = 30;

/// Sets GPR `r` to a sign-extended 32-bit value (ALU and lw results).
fn set(c: &mut Cpu, r: usize, v: u32) {
    c.s(r, v as i32 as i64 as u64);
}

/// 0x228bb0 counter_align(obj, x, y) -> 0: nudges the x at a1 and the y at a2 of a slot by part of its size
/// (width +0x58, height +0x5c), by the flags at +0x60 (bit 4: no x change, bit 8: x moves by the full width).
pub fn counter_align(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let px = c.arg(1);
    let py = c.arg(2);
    let flags = c.r32(obj.wrapping_add(OBJ_FLAGS));
    let width = c.r32(obj.wrapping_add(OBJ_WIDTH));
    let mut v1 = c.r32(obj.wrapping_add(OBJ_HEIGHT));
    if flags & FLAG_NO_Y_HALF == 0 {
        v1 = ((v1 as i32) >> 1) as u32;
        let y = c.r32(py).wrapping_sub(v1);
        c.w32(py, y);
    }
    if flags & FLAG_NO_X == 0 {
        if flags & FLAG_X_FULL != 0 {
            let x = c.r32(px).wrapping_sub(width);
            c.w32(px, x);
        } else {
            // Retail: the bit-8 test leaves v0 = 0 and the *px load is skipped, so the store is 0 - (width >> 1).
            v1 = ((width as i32) >> 1) as u32;
            c.w32(px, 0u32.wrapping_sub(v1));
        }
    }
    set(c, 8, width);
    set(c, 3, v1);
    set(c, 4, flags);
    set(c, 7, flags);
    set(c, 2, 0);
    true
}

/// 0x228dc0 counter_size(obj) -> v0: clamps the value at *(obj+0xc) to the maximum at +8 into +0x74/+0x78, then sizes
/// the counter from the digit count of the maximum: width (+0x58) and height (+0x5c) in steps of 14 and 12.
pub fn counter_size(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let a0 = c.g(4);
    c.s(5, a0);
    let ptr = c.r32(obj.wrapping_add(OBJ_PTR));
    let max = c.r32(obj.wrapping_add(OBJ_MAX));
    let clamp = if ptr == 0 || ptr & 3 != 0 {
        CLAMP_DEFAULT
    } else {
        let p = c.r32(ptr);
        if (max as i32) < (p as i32) {
            max
        } else {
            p
        }
    };
    c.w32(obj.wrapping_add(OBJ_CLAMP_HI), clamp);
    c.w32(obj.wrapping_add(OBJ_CLAMP_LO), clamp);

    // Digit count of max: a0 counts the divisions by 10 (v1 ends as the leading quotient).
    let mut v1 = max;
    let mut digits: u32 = 0;
    let t0 = c.r32(obj.wrapping_add(OBJ_FLAGS));
    let t1 = c.r32(obj.wrapping_add(OBJ_HEIGHT));
    if (max as i32) >= DECIMAL as i32 {
        loop {
            v1 = ((v1 as i32) / DECIMAL as i32) as u32;
            digits = digits.wrapping_add(1);
            if (v1 as i32) < DECIMAL as i32 {
                break;
            }
        }
        set(c, 6, DECIMAL);
        set(c, 7, 0);
    }

    let v0;
    let a0_out;
    if t0 & 3 == 0 && t0 & FLAG_WIDE_WIDTH != 0 {
        // Height from the digit count: (digits + 1) * 12 + t1, width grows by 14 up to 14.
        v1 = HEIGHT_STEP;
        a0_out = digits.wrapping_add(1).wrapping_mul(HEIGHT_STEP).wrapping_add(t1);
        c.w32(obj.wrapping_add(OBJ_HEIGHT), a0_out);
        let width = c.r32(obj.wrapping_add(OBJ_WIDTH));
        if (width as i32) < WIDTH_STEP as i32 {
            c.w32(obj.wrapping_add(OBJ_WIDTH), WIDTH_STEP);
            v0 = WIDTH_STEP;
        } else {
            v0 = 0;
        }
    } else {
        if (t1 as i32) < (HEIGHT_STEP as i32) {
            c.w32(obj.wrapping_add(OBJ_HEIGHT), HEIGHT_STEP);
        }
        // Tail: width += (digits + 1) * 14.
        v1 = digits.wrapping_add(1).wrapping_mul(WIDTH_STEP);
        let total = c.r32(obj.wrapping_add(OBJ_WIDTH)).wrapping_add(v1);
        c.w32(obj.wrapping_add(OBJ_WIDTH), total);
        v0 = total;
        a0_out = WIDTH_STEP;
    }
    set(c, 2, v0);
    set(c, 3, v1);
    set(c, 4, a0_out);
    set(c, 8, t0);
    set(c, 9, t1);
    true
}

/// 0x22a2d0 slot_init(obj) -> v0: clears the 16-bit fields at +0x48/+0x4a, stores the scaled value + 30 at +0x7c,
/// sets width and height to 0x20 and sizes the counter (counter_size).
pub fn slot_init(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    h::frame(c, 0x20, &[(16, 0x0, h::SQ), (31, 0x10, h::SQ)], |c, _sp| {
        let a0 = c.g(4);
        c.s(16, a0);
        let f = c.farg(0);
        let (scaled, _) = h::run(c, crate::mathx::float_to_int_scaled, &[DECIMAL], &[f]);
        c.w16(obj.wrapping_add(0x48), 0);
        c.w16(obj.wrapping_add(0x4a), 0);
        c.w32(obj.wrapping_add(OBJ_SCALED), scaled.wrapping_add(SCALED_BIAS));
        set(c, 3, SLOT_SIZE);
        c.w32(obj.wrapping_add(OBJ_HEIGHT), SLOT_SIZE);
        c.w32(obj.wrapping_add(OBJ_WIDTH), SLOT_SIZE);
        let s0 = c.g(16);
        c.s(4, s0);
        let _ = counter_size(c);
    });
    true
}
