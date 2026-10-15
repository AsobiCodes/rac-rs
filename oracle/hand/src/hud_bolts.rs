//! HUD bolt counter widget updates (Eudora overlay 0x228fe0..0x22ce40) and a small HUD sound-name setup.
use crate::calls::*;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const SCALE_TICKS: u32 = 0x1f_14b8;
const INT_TO_FLOAT: u32 = 0x1f_27d8;
const FLOAT_TO_INT: u32 = 0x1f_27e8;
const SQRT_Q: u32 = 0x1f_1778;
const WIDGET_DRAW: u32 = 0x22_dbf0;
const RANDOM_SEED_FN: u32 = 0x11_66cc;
const STRING_SET_FN: u32 = 0x11_65b8;
const NAME_SRC: u32 = 0x1d_a1c8;
const NAME_DST: u32 = 0x17_e630;
const NAME_LEN: u32 = 0x13;
const FRAME_COUNTER: u32 = 0x15_ed98;
const GP_LIMIT_A: u32 = -0x73fci32 as u32;
const GP_LIMIT_B: u32 = -0x73f8i32 as u32;

/// Widget fields: +8 maximum, +0xc pointer to the counted value, +0x6c state, +0x70/+0x71 digit bytes, +0x73 pending
/// bytes, +0x74 displayed value, +0x78 target value, +0x7c timer, +0x80 pointer to the spin halfword.
const W_MAX: u32 = 8;
const W_SRC: u32 = 0xc;
const W_STATE: u32 = 0x6c;
const W_DIGIT0: u32 = 0x70;
const W_DIGIT1: u32 = 0x71;
const W_PENDING: u32 = 0x73;
const W_SHOWN: u32 = 0x74;
const W_TARGET: u32 = 0x78;
const W_TIMER: u32 = 0x7c;
const W_SPIN: u32 = 0x80;

fn scale(c: &mut Cpu, n: u32) -> i32 {
    call(c, SCALE_TICKS, &[n], &[]);
    c.result() as i32
}

/// 0x228fe0 bolts_counter_update(widget): clamps the target to the source value, counts the displayed value toward it
/// at a speed from the distance (carrying through the pending byte), and steps the two digit bytes.
pub fn bolts_counter_update(c: &mut Cpu) -> bool {
    preserved(c, false, false, bolts_counter_update_body)
}

fn bolts_counter_update_body(c: &mut Cpu) -> bool {
    let w = c.arg(0);
    let src = c.r32(w + W_SRC);
    if src != 0 {
        let v = c.r32(src) as i32;
        let target = if v > -1 { v } else { 0 };
        let max = c.r32(w + W_MAX) as i32;
        c.w32(w + W_TARGET, target as u32);
        if max < target {
            c.w32(w + W_TARGET, max as u32);
        }
    }
    let (shown, target) = (c.r32(w + W_SHOWN) as i32, c.r32(w + W_TARGET) as i32);
    if shown != target {
        let t = scale(c, 0xb4);
        c.w32(w + W_TIMER, t as u32);
        if (c.r32(w + W_STATE) as i32) >= 0x18 {
            let (shown, target) = (c.r32(w + W_SHOWN) as i32, c.r32(w + W_TARGET) as i32);
            let dist = (shown - target).wrapping_abs();
            if dist != 0 {
                count_toward(c, w, dist);
            }
        }
    }
    let timer = c.r32(w + W_TIMER) as i32;
    if timer >= scale(c, 5) {
        // Timer not yet run out: digits grow toward scaled 8.
        let d0 = c.r8(w + W_DIGIT0) as i32;
        if d0 < scale(c, 8) {
            c.w8(w + W_DIGIT0, d0 as u8 + 1);
        } else {
            let d1 = c.r8(w + W_DIGIT1) as i32;
            if d1 < scale(c, 8) {
                c.w8(w + W_DIGIT1, d1 as u8 + 1);
            }
        }
    } else {
        if c.r8(w + W_DIGIT1) != 0 || c.r8(w + W_DIGIT0) != 0 {
            c.w32(w + W_STATE, 1);
        }
        let d1 = c.r8(w + W_DIGIT1);
        if d1 != 0 {
            c.w8(w + W_DIGIT1, d1 - 1);
        } else {
            let d0 = c.r8(w + W_DIGIT0);
            if d0 != 0 {
                c.w8(w + W_DIGIT0, d0 - 1);
            }
        }
    }
    call(c, WIDGET_DRAW, &[w + 0x40], &[]);
    true
}

/// Moves the displayed value one step toward the target: step = max(5 * sqrt(dist / 25), dist * scale(2) / scale(10)),
/// clamped to 1..=0x79, accumulated in the pending byte and paid out in whole units.
fn count_toward(c: &mut Cpu, w: u32, dist: i32) {
    call(c, INT_TO_FLOAT, &[dist as u32], &[]);
    let f = c.fresult();
    call(c, SQRT_Q, &[], &[f / F(0x41c8_0000)]);
    let root = c.fresult();
    call(c, FLOAT_TO_INT, &[], &[root * F(0x40a0_0000)]);
    let mut step = c.result() as i32;
    let limit = |c: &mut Cpu| {
        let a = scale(c, 2);
        let b = scale(c, 0xa);
        dist.wrapping_mul(a).checked_div(b).unwrap_or(-1)
    };
    let l = limit(c);
    if step < l {
        step = limit(c);
    }
    step = if step >= 0x7a {
        0x79
    } else if step > 0 {
        step
    } else {
        1
    };
    let pending = (c.r8(w + W_PENDING) as i32 + step) as u8;
    c.w8(w + W_PENDING, pending);
    if scale(c, 2) < pending as i32 {
        let two = scale(c, 2);
        let whole = (c.r8(w + W_PENDING) as i32).checked_div(two).unwrap_or(-1);
        let shown = c.r32(w + W_SHOWN) as i32;
        let new = if (c.r32(w + W_TARGET) as i32) < shown { shown - whole } else { shown + whole };
        c.w32(w + W_SHOWN, new as u32);
        let two = scale(c, 2);
        let left = (c.r8(w + W_PENDING) as i32).wrapping_sub(whole.wrapping_mul(two));
        c.w8(w + W_PENDING, left as u8);
    }
}

/// 0x22a320 bolts_counter_update_tail(widget): forwards to bolts_counter_update.
pub fn bolts_counter_update_tail(c: &mut Cpu) -> bool {
    preserved(c, false, false, bolts_counter_update_body)
}

/// 0x22ce40 bolts_spin_update(widget): advances the spin counter (mod 60), restarts the timer when the frame
/// marker changes, and steps the digit bytes up to their limits or down once the timer has run out.
pub fn bolts_spin_update(c: &mut Cpu) -> bool {
    preserved(c, false, false, bolts_spin_update_body)
}

fn bolts_spin_update_body(c: &mut Cpu) -> bool {
    let w = c.arg(0);
    let spin = c.r32(w + W_SPIN);
    let v = (c.r16(spin) as i16 as i32 + 1) % 0x3c;
    c.w16(spin, v as u16);
    let marker = c.r32(FRAME_COUNTER);
    if c.r32(w + W_SHOWN) != marker {
        c.w32(w + W_SHOWN, marker);
        let t = scale(c, 0x5a);
        c.w32(w + W_TIMER, t as u32);
    }
    let timer = c.r32(w + W_TIMER) as i32;
    let gp = c.g(28) as u32;
    if timer < scale(c, 5) {
        c.w32(w + W_STATE, 1);
        let d1 = c.r8(w + W_DIGIT1);
        if d1 != 0 {
            c.w8(w + W_DIGIT1, d1 - 1);
        } else {
            let d0 = c.r8(w + W_DIGIT0);
            if d0 != 0 {
                c.w8(w + W_DIGIT0, d0 - 1);
            } else {
                c.w32(w + W_STATE, -6i32 as u32);
            }
        }
    } else {
        let d0 = c.r8(w + W_DIGIT0) as i32;
        if d0 < c.r32(gp.wrapping_add(GP_LIMIT_A)) as i32 {
            c.w8(w + W_DIGIT0, d0 as u8 + 1);
        } else {
            let d1 = c.r8(w + W_DIGIT1) as i32;
            if d1 < c.r32(gp.wrapping_add(GP_LIMIT_B)) as i32 {
                c.w8(w + W_DIGIT1, d1 as u8 + 1);
            }
        }
    }
    true
}

/// 0x22a858 hud_name_set(arg): when the retail value check passes (< 0x50) the 19-byte name at 0x1da1c8 is copied
/// to 0x17e630; then the string setter is called with that buffer and `arg`.
pub fn hud_name_set(c: &mut Cpu) -> bool {
    preserved(c, false, false, hud_name_set_body)
}

fn hud_name_set_body(c: &mut Cpu) -> bool {
    let arg = c.arg(0);
    call(c, RANDOM_SEED_FN, &[arg], &[]);
    if c.result() < 0x50 {
        for i in 0..NAME_LEN {
            let b = c.r8(NAME_SRC + i);
            c.w8(NAME_DST + i, b);
        }
    }
    call(c, STRING_SET_FN, &[NAME_DST, arg], &[]);
    true
}
