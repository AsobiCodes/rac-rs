//! Marker and cursor drawing (rec/l4_11): a framed marker from two GS rectangles and an optional HUD quad pair,
//! a conditional sprite blit for objects with a count of at least two, and a probe-based state initialiser.
use crate::h;
use crate::ps2::*;
use crate::sprite_text;
use ee_oracle::cpu::Cpu;

/// Retail hud_sprite_handle(sheet, piece) (0x22e350): returns the handle index of a HUD sprite.
const HUD_SPRITE_HANDLE: u32 = 0x0022_e350;
/// Retail GIF rectangle writer (0x22fbf0) and quad pair writer (0x22e620), called as retail code so the scratch
/// registers they leave match the retail jal.
const GIF_RECT_PKT: u32 = 0x0022_fbf0;
const QUAD_PAIR: u32 = 0x0022_e620;
/// Retail state probe (0x2766a8): returns a small status word for the selector passed in a0.
const STATE_PROBE: u32 = 0x0027_66a8;
/// HUD sheet and piece looked up for the marker's quad pair.
const MARKER_SHEET: u32 = 0xe99e;
const MARKER_PIECE: u32 = 1;
/// Colour word of the outer rectangle; the inner one is read from a gp-relative slot.
const OUTER_RGBA: u32 = 0x80ff_a888;
const GP_INNER_RGBA: i32 = -0x6a10;
/// Doubleword holding the sprite's second colour word (lui 0x18 + 0x4548).
const SPRITE_WORD: u32 = 0x0018_4548;
/// Sprite size table (lui 0x15 + 0x1780): width at +0x160, height at +0x162.
const SIZE_TABLE: u32 = 0x0015_1780;
/// First colour word of the sprite (0x80808080).
const SPRITE_TINT: u32 = 0x8080_8080;
/// Object fields used by `marker_init` and `sprite_gate`.
const OBJ_COUNT: i32 = 0x44;
const OBJ_DU: i32 = 0x38;
const OBJ_DV: i32 = 0x3c;
const OBJ_FLAGS: i32 = 0x34;
const OBJ_PROBE_A: i32 = 0x48;
const OBJ_PROBE_B: i32 = 0x4c;
const OBJ_PROBE_C: i32 = 0x50;
const OBJ_PROBE_D: i32 = 0x54;
const OBJ_PROBE_E: i32 = 0x5c;
/// Probe mode bit in the flags word.
const PROBE_MODE_BIT: u32 = 0x200;

/// Splits a 128-bit register into four 32-bit lanes (lane 0 = low bits).
fn quad(x: u128) -> V {
    V([x as u32, (x >> 32) as u32, (x >> 64) as u32, (x >> 96) as u32])
}

fn offset(v: u32, d: i32) -> u32 {
    v.wrapping_add(d as u32)
}

/// 0x270178 marker_frame(x, y, with_pair): draws two GS rectangles around (x, y) and, when with_pair is non-zero,
/// a HUD quad pair beside them.
pub fn marker_frame(c: &mut Cpu) -> bool {
    let (x, y, with_pair) = (c.arg(0), c.arg(1), c.arg(2));
    h::with_stack(c, 0x40, |c, buf| {
        // Prologue spills s0, s1, s2 and ra into the new frame.
        c.wv(buf, quad(c.r[16]));
        c.wv(buf + 0x10, quad(c.r[17]));
        c.wv(buf + 0x20, quad(c.r[18]));
        c.wv(buf + 0x30, quad(c.r[31]));
        let gp = c.g(28) as u32;
        let outer = [offset(x, -5), offset(y, -5), offset(x, 5), offset(y, 5), OUTER_RGBA, 0];
        h::call_keep(c, GIF_RECT_PKT, &outer, &[]);
        let inner = c.r32(offset(gp, GP_INNER_RGBA));
        let inner_rect = [offset(x, -4), offset(y, -4), offset(x, 4), offset(y, 4), inner, 0];
        h::call_keep(c, GIF_RECT_PKT, &inner_rect, &[]);
        if with_pair != 0 {
            let (handle, _) = h::call_keep(c, HUD_SPRITE_HANDLE, &[MARKER_SHEET, MARKER_PIECE], &[]);
            let pair = [handle, offset(x, -0xd), offset(y, -0x12), 0x1e, 0x1e, 0x80];
            h::call_keep(c, QUAD_PAIR, &pair, &[]);
        }
    });
    true
}

/// 0x2704f8 sprite_gate(obj) -> 16 or 0: when obj's count (+0x44) is at least two, blits one tinted sprite sized
/// from the size table, with the object's du/dv (+0x38/+0x3c); returns 0 otherwise.
pub fn sprite_gate(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    h::with_stack(c, 0x20, |c, buf| {
        c.wv(buf + 0x10, quad(c.r[31]));
        if (c.r32(offset(obj, OBJ_COUNT)) as i32) < 2 {
            c.ret(0);
            return;
        }
        let word_lo = c.r32(SPRITE_WORD);
        let word_hi = c.r32(SPRITE_WORD + 4);
        let width = c.r16(SIZE_TABLE + 0x160) as i16 as i32 as u32;
        let height = c.r16(SIZE_TABLE + 0x162) as i16 as i32 as u32;
        let du = c.r32(offset(obj, OBJ_DU));
        let dv = c.r32(offset(obj, OBJ_DV));
        // The 9th and 10th arguments of sprite_blit_int sit in this frame.
        c.w32(buf, SPRITE_TINT);
        c.w32(buf + 4, 0);
        c.w32(buf + 8, word_lo);
        c.w32(buf + 12, word_hi);
        c.s(3, SPRITE_TINT as u64);
        h::run_keep(c, sprite_text::sprite_blit_int, &[0, 0, width, height, 0, 0, du, dv], &[]);
        c.ret(0x10);
    });
    true
}

/// 0x270570 marker_init(obj): clears the count, stores two probe results, and sets the two state slots to -1.
pub fn marker_init(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    c.w32(offset(obj, OBJ_COUNT), 0);
    let mode = |c: &mut Cpu| c.r32(offset(obj, OBJ_FLAGS)) & PROBE_MODE_BIT;
    let m = mode(c);
    let a = h::call_keep(c, STATE_PROBE, &[m], &[]).0;
    c.w32(offset(obj, OBJ_PROBE_A), a);
    let m = mode(c);
    let b = h::call_keep(c, STATE_PROBE, &[m], &[]).0;
    c.w32(offset(obj, OBJ_PROBE_B), b);
    if mode(c) == 0 {
        if a == 0 {
            let r = h::call_keep(c, STATE_PROBE, &[1], &[]).0;
            c.w32(offset(obj, OBJ_PROBE_A), r);
        }
        if b == 0 {
            let r = h::call_keep(c, STATE_PROBE, &[1], &[]).0;
            c.w32(offset(obj, OBJ_PROBE_B), r);
        }
    }
    c.w32(offset(obj, OBJ_PROBE_E), 0);
    c.w32(offset(obj, OBJ_PROBE_D), u32::MAX);
    c.w32(offset(obj, OBJ_PROBE_C), u32::MAX);
    c.s(3, u64::MAX);
    c.ret(0);
    true
}
