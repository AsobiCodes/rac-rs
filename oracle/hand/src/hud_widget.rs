//! HUD widget drawing (Eudora overlay 0x22d8f0..0x22ddc0): the ammo bar widget and the icon frame animation.
use crate::calls::*;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const INT_TO_FLOAT: u32 = 0x1f_27d8;
const FLOAT_TO_INT: u32 = 0x1f_27e8;
const SCALE_TICKS: u32 = 0x1f_14b8;
const COLOR_LERP: u32 = 0x1f_2830;
const SPRINTF: u32 = 0x11_6248;
const DRAW_TEXT: u32 = 0x1e_e168;
const SPRITE_LOOKUP: u32 = 0x22_e350;
const BAR_DRAW: u32 = 0x22_e620;
const BAR_END_DRAW: u32 = 0x22_e9a8;
const FRAME_DRAW: u32 = 0x22_e268;
const RANDOM_TICKS: u32 = 0x24_a418;
const ICON_TABLE_PTR: u32 = 0x17_e5f4;
/// Frame counter (0x15f5ac), the compact-layout flag (0x15ed80) and the "%d/%d" format string (0x15f7a8).
const FRAME_COUNTER: u32 = 0x15_f5ac;
const COMPACT_FLAG: u32 = 0x15_ed80;
const COUNT_FORMAT: u32 = 0x15_f7a8;
const BAR_SPRITE: u32 = 0x7580;

fn gp_word(c: &mut Cpu, off: i32) -> u32 {
    let gp = c.g(28) as u32;
    c.r32(gp.wrapping_add(off as u32))
}

fn to_float(c: &mut Cpu, v: u32) -> F {
    call(c, INT_TO_FLOAT, &[v], &[]);
    c.fresult()
}

fn to_int(c: &mut Cpu, f: F) -> u32 {
    call(c, FLOAT_TO_INT, &[], &[f]);
    c.result()
}

fn unit_clamp(x: F) -> F {
    if F::ONE < x {
        F::ONE
    } else if x < F::ZERO {
        F::ZERO
    } else {
        x
    }
}

fn scale(c: &mut Cpu, n: u32) -> i32 {
    call(c, SCALE_TICKS, &[n], &[]);
    c.result() as i32
}

/// 0x22d8f0 ammo_bar_draw(widget) -> widget+0x58: draws the two-part bar (fill fractions from the digit bytes at
/// +0x70/+0x71), the frame, and the "count/max" text with a shadow.
#[allow(dead_code)]
pub fn ammo_bar_draw(c: &mut Cpu) -> bool {
    preserved(c, true, false, ammo_bar_draw_body)
}

fn ammo_bar_draw_body(c: &mut Cpu) -> bool {
    // Retail's frame is 0xb0 bytes: callees see the same stack depth (and stale stack words) as in retail.
    with_frame(c, 0xb0, |c, buf| ammo_bar_draw_in(c, buf))
}

fn ammo_bar_draw_in(c: &mut Cpu, buf: u32) -> bool {
    let w = c.arg(0);
    let row = if c.r32(COMPACT_FLAG) != 0 { 0xa } else { 0x12 };
    let digit0 = c.r8(w + 0x70) as u32;
    if digit0 != 0 {
        let pos = c.r32(w + 0x50);
        let a = to_float(c, digit0);
        let lim = gp_word(c, -0x730c);
        let b = to_float(c, lim);
        let fill = unit_clamp(a / b);
        let d1 = c.r8(w + 0x71) as u32;
        let a = to_float(c, d1);
        let lim = gp_word(c, -0x7308);
        let b = to_float(c, lim);
        let fade = unit_clamp(a / b);
        let width = to_int(c, fill * F(0x4300_0000));
        let wf = to_float(c, width);
        let height = to_int(c, wf * F(0x3f33_3333));
        let max = c.r32(w + 8) as i32;
        let text_off = if max < 100 { gp_word(c, -0x7304) } else { gp_word(c, -0x7300) };
        let tf = to_float(c, text_off);
        let bar_len = to_int(c, tf * fill);
        let (left, right) = (pos + 4, pos.wrapping_add(bar_len));
        call(c, SPRITE_LOOKUP, &[BAR_SPRITE, 1], &[]);
        let s = c.result();
        call(c, BAR_DRAW, &[s, pos.wrapping_sub(0x1c), row, 0x20, 0x20, height], &[]);
        call(c, SPRITE_LOOKUP, &[BAR_SPRITE, 0], &[]);
        let s = c.result();
        call(c, BAR_DRAW, &[s, left, row, right, 0x20, height], &[]);
        call(c, SPRITE_LOOKUP, &[BAR_SPRITE, 1], &[]);
        let s = c.result();
        call(c, BAR_END_DRAW, &[s, left.wrapping_add(right), row, 0x20, 0x20, height], &[]);
        let sheet = c.r32(w);
        call(c, SPRITE_LOOKUP, &[sheet, 3], &[]);
        let icon = c.result();
        c.w32(w + 0x44, icon);
        call(c, FRAME_DRAW, &[w, icon, pos, row, 0, width], &[]);
        let shown = c.r32(w + 0x74);
        let maxv = c.r32(w + 8);
        call(c, SPRINTF, &[buf, COUNT_FORMAT, shown, maxv], &[]);
        let (from, to) = if shown != 0 {
            (gp_word(c, -0x72fc), gp_word(c, -0x72f8))
        } else {
            (gp_word(c, -0x72f4), gp_word(c, -0x72f0))
        };
        call(c, COLOR_LERP, &[from, to], &[fade]);
        let text_color = c.result();
        call(c, COLOR_LERP, &[0, 0x8000_0000], &[fade]);
        let shadow = c.result();
        let (dx, dy) = (gp_word(c, -0x72ec), gp_word(c, -0x72e8));
        let x = pos.wrapping_add(dx).wrapping_add(text_off);
        let y = row.wrapping_add(dy);
        call(c, DRAW_TEXT, &[x.wrapping_add(1), y.wrapping_add(1), shadow, buf, -1i32 as u32], &[]);
        call(c, DRAW_TEXT, &[x, y, text_color, buf, -1i32 as u32], &[]);
    }
    let r = c.r32(w + 0x58);
    c.ret(r);
    true
}

/// 0x22dbf0 icon_anim(widget) -> frame: picks the icon frame (also stored at widget+4) from the icon table entry
/// of widget halfword 0, by the mode byte at +2: 0 constant, 1 loop, 2 ping-pong, 3 ping-pong then a random rest.
pub fn icon_anim(c: &mut Cpu) -> bool {
    preserved(c, true, false, icon_anim_body)
}

fn icon_anim_body(c: &mut Cpu) -> bool {
    let w = c.arg(0);
    let table = c.r32(ICON_TABLE_PTR);
    let e = table.wrapping_add((c.r16(w) as i16 as i32 as u32) << 3);
    let mut frame: i32 = 0;
    if c.r16(e) != 0xffff {
        let (first, count, step) = (c.r16(e + 4) as i32, c.r16(e + 2) as i32, c.r8(e + 7) as i32);
        let now = c.r32(FRAME_COUNTER) as i32;
        let since = |c: &mut Cpu| now.wrapping_sub(c.r32(w + 0xc) as i32);
        match c.r8(w + 2) {
            0 => frame = first,
            1 => {
                let q = since(c).checked_div(step).unwrap_or(-1);
                frame = first + q.checked_rem(count).unwrap_or(q);
            }
            2 => {
                let q = since(c).checked_div(step).unwrap_or(-1);
                let r = q.checked_rem(2 * count - 2).unwrap_or(q);
                frame = pingpong(r, count) + first;
            }
            3 => {
                let start = c.r32(w + 0xc) as i32;
                if start < now {
                    let q = (now - start).checked_div(step).unwrap_or(-1);
                    if q < 2 * count - 2 {
                        frame = pingpong(q, count) + first;
                    } else {
                        let a = scale(c, 10);
                        let b = scale(c, 0x1e);
                        call(c, RANDOM_TICKS, &[b as u32], &[]);
                        let rnd = c.result() as i32;
                        c.w32(w + 0xc, (2 * count + a + rnd) as u32);
                        frame = first;
                    }
                } else {
                    frame = c.r32(w + 4) as i32;
                }
            }
            _ => {
                c.w32(w + 4, 0);
                c.ret(0);
                return true;
            }
        }
    }
    c.w32(w + 4, frame as u32);
    c.ret(frame as u32);
    true
}

/// Frame `r` of a ping-pong cycle over `count` frames (0..count-1 then back down).
fn pingpong(r: i32, count: i32) -> i32 {
    if r < count {
        r
    } else {
        2 * count - (r + 2)
    }
}
