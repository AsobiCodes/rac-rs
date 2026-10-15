//! HUD frame tick, anchored sprite draw and sprite handle lookup (Eudora overlay). They work on the HUD state block
//! at 0x17e5d8 and the 13 widget slots at 0x17dd50.
use crate::calls::preserved;
use crate::h;
use crate::hud_slots;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// HUD state block.
const STATE: u32 = 0x17_e5d8;
/// Pointer to the icon table (8-byte entries: sheet id, piece count, first handle index, ...).
const ICON_TABLE_PTR: u32 = STATE + 0x1c;
/// Pointer to the handle table (4-byte entries: shape index, sub-shape index).
const HANDLE_TABLE_PTR: u32 = STATE + 0x20;
/// Pointer to the sub-shape table (8-byte entries; bytes 6 and 7 are the width and height shift amounts).
const SUBSHAPE_TABLE_PTR: u32 = STATE + 0x24;
/// Pointer to the shape table (8-byte entries; the first word's sign bit hides the shape).
const SHAPE_TABLE_PTR: u32 = STATE + 0x28;
/// Set by the producer once a frame is done; the tick clears it and returns early.
const FRAME_DONE: u32 = STATE + 0x30;

/// Widget slot table: 13 slots of 0x90 bytes, callback pointer at +0x18.
const SLOT_TABLE: u32 = 0x17_dd50;
const SLOT_SIZE: u32 = 0x90;
const SLOT_COUNT: u32 = 13;
const SLOT_CALLBACK: u32 = 0x18;
/// Word that makes the tick bail out (after clearing the done flag) when nonzero.
const TICK_GATE: u32 = 0x15_f404;
/// Fade counter: decremented each tick while nonzero.
const FADE_COUNTER: u32 = 0x15_f620;
/// Level value, clamped and stored each tick.
const LEVEL: u32 = 0x15_f624;
/// Value handed to the draw helper; forced to 100 on the early exits.
const DRAW_VALUE: u32 = 0x15_f628;
/// Retail draw helper (mode, value, base, buffer).
const DRAW_HELPER: u32 = 0x23_00c8;
/// Retail anchored draw worker (index, x, y, width, height).
const DRAW_AT: u32 = 0x22_e620;
const DRAW_MODE: u32 = 0x100;
/// Buffer for the first draw call, and the second draw call's buffer (also the flag byte that gates that call).
const FIRST_BUF: u32 = 0x17_9218;
const SECOND_BUF: u32 = 0x17_9268;
/// gp-relative words, as offsets below gp.
const GP_READ: u32 = 0x76_5c;
const GP_LEVEL_OUT: u32 = 0x75_dc;
const GP_COUNT_CLEAR: u32 = 0x75_e0;

/// Signed division with the R5900 result for a zero divisor (LO = 1 when the dividend is negative, else -1).
fn ee_div(n: i32, d: i32) -> i32 {
    if d == 0 {
        if n < 0 { 1 } else { -1 }
    } else {
        n.wrapping_div(d)
    }
}

fn gp_word(c: &mut Cpu, back: u32) -> u32 {
    let a = (c.g(28) as u32).wrapping_sub(back);
    c.r32(a)
}

fn gp_put(c: &mut Cpu, back: u32, v: u32) {
    let a = (c.g(28) as u32).wrapping_sub(back);
    c.w32(a, v);
}

/// mathx::float_to_int_scaled(8), the scale used by the level clamp.
fn scale_factor(c: &mut Cpu) -> i32 {
    h::run(c, mathx::float_to_int_scaled, &[8], &[]).0 as i32
}

/// Early exit that stores 100 as the draw value and returns it.
fn draw_value_100(c: &mut Cpu) -> bool {
    c.w32(DRAW_VALUE, 100);
    c.ret(100);
    true
}

fn frame_tick_body(c: &mut Cpu) -> bool {
    if c.r32(FRAME_DONE) != 0 {
        c.w32(FRAME_DONE, 0);
        c.ret(0x18_0000);
        return true;
    }
    if c.r32(TICK_GATE) != 0 {
        c.w32(FRAME_DONE, 0);
        c.ret(0xff_0000);
        return true;
    }
    c.w32(STATE + 0x0c, 0xff_fff0);
    for i in 0..SLOT_COUNT {
        let slot = SLOT_TABLE + i * SLOT_SIZE;
        let callback = c.r32(slot + SLOT_CALLBACK);
        if callback != 0 {
            h::call(c, callback, &[slot], &[]);
        }
    }
    if c.r32(FADE_COUNTER) != 0 {
        if gp_word(c, GP_READ) != 0 {
            return draw_value_100(c);
        }
        let q = ee_div(0x80, scale_factor(c));
        let sum = (c.r32(LEVEL) as i32).wrapping_add(q);
        gp_put(c, GP_LEVEL_OUT, (if sum < 0x81 { sum } else { 0x80 }) as u32);
    } else {
        if c.r32(LEVEL) == 0 || gp_word(c, GP_READ) != 0 {
            return draw_value_100(c);
        }
        let q = ee_div(0x80, scale_factor(c));
        let d = (c.r32(LEVEL) as i32).wrapping_sub(q);
        gp_put(c, GP_LEVEL_OUT, d as u32);
        if d < 0 {
            c.w32(LEVEL, 0);
        }
    }
    let base = (c.r32(LEVEL) << 24).wrapping_add(0xf0_f0f0);
    let value = c.r32(DRAW_VALUE);
    h::call(c, DRAW_HELPER, &[DRAW_MODE, value, base, FIRST_BUF], &[]);
    if c.r8(SECOND_BUF) != 0 && (c.r32(FADE_COUNTER) as i32) >= 0x3e9 {
        let value = c.r32(DRAW_VALUE);
        h::call(c, DRAW_HELPER, &[DRAW_MODE, value, base, SECOND_BUF], &[]);
    }
    let fade = c.r32(FADE_COUNTER);
    if fade != 0 {
        c.w32(FADE_COUNTER, fade.wrapping_sub(1));
    }
    if c.r32(FADE_COUNTER) == 1000 {
        gp_put(c, GP_COUNT_CLEAR, 0);
    }
    c.ret(1000);
    true
}

/// 0x22e090 hud_frame_tick() -> v0 status: per-frame HUD tick. Honors the done flag and the gate word, runs the 13
/// widget callbacks, steps the level and fade counters and calls the draw helper; returns 1000 on the normal path.
pub fn hud_frame_tick(c: &mut Cpu) -> bool {
    preserved(c, true, false, frame_tick_body)
}

fn draw_anchored_body(c: &mut Cpu) -> bool {
    let index = c.arg(1) as i32;
    if index < 0 {
        return true;
    }
    let obj = c.arg(0);
    let mut x = (c.arg(2) as i32).wrapping_add(c.r16(obj + 0x48) as i16 as i32);
    let mut y = (c.arg(3) as i32).wrapping_add(c.r16(obj + 0x4a) as i16 as i32);
    let flags = c.arg(4);
    let entry1 = c.r32(HANDLE_TABLE_PTR).wrapping_add((index as u32) << 2);
    let sub_index = c.r16(entry1 + 2) as i16 as i32 as u32;
    let entry2 = c.r32(SUBSHAPE_TABLE_PTR).wrapping_add(sub_index << 3);
    let w0 = 1i32 << (c.r8(entry2 + 6) & 31);
    let h0 = 1i32 << (c.r8(entry2 + 7) & 31);
    let (mut w, mut h) = (w0, h0);
    if flags & 1 != 0 {
        x = x.wrapping_sub(w0 >> 1);
        y = y.wrapping_sub(h0 >> 1);
    }
    if flags & 2 != 0 {
        x = x.wrapping_add(w >> 2);
        y = y.wrapping_add(h >> 2);
        w >>= 1;
        h >>= 1;
    }
    if flags & 4 != 0 {
        x = x.wrapping_sub(w0 >> 1);
        y = y.wrapping_sub(h0 >> 1);
        w <<= 1;
        h <<= 1;
    }
    let (v0, _) = h::call(c, DRAW_AT, &[index as u32, x as u32, y as u32, w as u32, h as u32], &[]);
    c.ret(v0);
    true
}

/// 0x22e268 hud_draw_sprite_anchored(obj, index, x, y, flags) -> v0 of the draw: offsets x/y by the object's
/// offsets and by the sprite size under anchor flags 1, 2 and 4, then draws via 0x22e620. Negative index: no-op.
pub fn hud_draw_sprite_anchored(c: &mut Cpu) -> bool {
    preserved(c, true, false, draw_anchored_body)
}

fn sprite_handle_body(c: &mut Cpu) -> bool {
    let sheet = c.arg(0);
    let piece = c.arg(1) as i32;
    let (icon_index, _) = h::run(c, hud_slots::hud_icon_index, &[sheet], &[]);
    let icon = c.r32(ICON_TABLE_PTR).wrapping_add(icon_index << 3);
    if c.r16(icon) == 0xffff || piece >= c.r16(icon + 2) as i32 {
        c.ret(0);
        return true;
    }
    let index = (c.r16(icon + 4) as u32).wrapping_add(piece as u32);
    let entry1 = c.r32(HANDLE_TABLE_PTR).wrapping_add(index << 2);
    let shape_index = c.r16(entry1) as i16 as i32 as u32;
    let shape = c.r32(SHAPE_TABLE_PTR).wrapping_add(shape_index << 3);
    if (c.r32(shape) as i32) < 0 {
        c.ret(0);
        return true;
    }
    let sub_index = c.r16(entry1 + 2) as i16 as i32 as u32;
    let sub_ptr = c.r32(SUBSHAPE_TABLE_PTR).wrapping_add(sub_index << 3);
    let sub_word = c.r32(sub_ptr);
    c.ret(if sub_word & 0x8000_0000 == 0 { index } else { 0 });
    true
}

/// 0x22e350 hud_sprite_handle(sheet, piece) -> handle index or 0: finds the sheet's icon entry (hud_icon_index),
/// rejects pieces past its count and hidden shapes (sign bit set), else returns the piece's table index.
pub fn hud_sprite_handle(c: &mut Cpu) -> bool {
    preserved(c, true, false, sprite_handle_body)
}
