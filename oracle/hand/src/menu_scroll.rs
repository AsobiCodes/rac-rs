//! Pause-menu cursor and scroll update (retail 0x26a698).

use ee_oracle::cpu::{div32, Cpu};

use crate::h;
use crate::ps2::*;

const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;

/// Menu state flags word; the pad bits below steer the cursor.
const MENU_FLAGS: u32 = 0x0013_CB04;
/// a0 and v1 values that retail leaves on the early-exit paths.
const ARG_HI: u32 = 0x0014_0000;
const MENU_STATE: u32 = 0x0013_C940;
/// Link record: +4 holds a node pointer, the node's +0x38 word is cached at +8.
const LINK: u32 = 0x001B_9DF0;
const LINK_NODE: u32 = LINK + 4;
const LINK_VALUE: u32 = LINK + 8;
/// Gate word: with no node value and this word zero, the update exits early.
const LINK_GATE: u32 = LINK + 0x124;
const NODE_VALUE_OFF: u32 = 0x38;
/// Cursor-move cue helper, called with (1, 0x11, obj.cue_arg) when the cursor changed.
const CURSOR_CUE: u32 = 0x0027_E470;

const FLAG_GATE_MASK: u32 = 0x0d00;
const FLAG_LINK: u32 = 0x0010;
const FLAG_STEP_BACK: u32 = 0x1000;
const FLAG_STEP_FWD: u32 = 0x4000;

/// Menu object fields.
const OBJ_CUE_ARG: u32 = 0x14;
const OBJ_ROWS: u32 = 0x24;
const OBJ_CURSOR: u32 = 0x3c;
const OBJ_ITEM_COUNT: u32 = 0x40;
const OBJ_SCROLL_Y: u32 = 0x5c;
const OBJ_SCROLL: u32 = 0x60;

/// Scroll unit (row height) and the Y origin of the scroll window.
const ROW_HEIGHT: u32 = 0x260;
const SCROLL_Y_ORIGIN: i32 = 0x190;
/// Y stored when the list fits on screen.
const UNSCROLLED_Y: u32 = 0x60;

fn sx(v: u32) -> u64 {
    v as i32 as i64 as u64
}

fn set_hilo(c: &mut Cpu, lo: u64, hi: u64) {
    c.lo = (c.lo & !(u64::MAX as u128)) | lo as u128;
    c.hi = (c.hi & !(u64::MAX as u128)) | hi as u128;
}

fn early_exit(c: &mut Cpu, result: i64) -> bool {
    c.s(V0, result as u64);
    c.s(V1, MENU_STATE as u64);
    c.s(A0, ARG_HI as u64);
    true
}

/// 0x26a698 menu_scroll_cursor(obj) -> 1/-1/0: steps the menu cursor from the pad flags, plays the cursor cue
/// when it moved, and recomputes the scroll offset (obj+0x60) and the Y of the list (obj+0x5c).
pub fn menu_scroll_cursor(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let flags = c.r32(MENU_FLAGS);
    if flags & FLAG_GATE_MASK != 0 && c.r32(LINK_GATE) == 0 {
        return early_exit(c, 1);
    }
    if flags & FLAG_LINK != 0 {
        let node = c.r32(LINK_NODE);
        let value = c.r32(node.wrapping_add(NODE_VALUE_OFF));
        if value != 0 {
            c.w32(LINK_VALUE, value);
        } else if c.r32(LINK_GATE) == 0 {
            return early_exit(c, -1);
        }
    }

    // Cursor step; `before` is the cursor the change test compares against.
    let before = c.r32(obj + OBJ_CURSOR);
    let mut cursor = before;
    if flags & FLAG_STEP_BACK != 0 && before != 0 {
        cursor = before.wrapping_sub(1);
        c.w32(obj + OBJ_CURSOR, cursor);
    }
    let mut v1 = flags;
    if flags & FLAG_STEP_FWD != 0 {
        let next = cursor.wrapping_add(1);
        v1 = next;
        let item_count = c.r32(obj + OBJ_ITEM_COUNT) as i32;
        if (next as i32) < item_count {
            c.w32(obj + OBJ_CURSOR, next);
            cursor = next;
        }
    }
    c.s(V0, sx(cursor));
    c.s(V1, sx(v1));
    c.s(A0, 1);
    if cursor != before {
        let cue_arg = c.r32(obj + OBJ_CUE_ARG);
        h::call_keep(c, CURSOR_CUE, &[1, 0x11, cue_arg], &[]);
    }

    // Scroll: the list fits when (rows << 4) / ROW_HEIGHT is below the item count.
    let rows = c.r32(obj + OBJ_ROWS);
    c.s(A1, sx(ROW_HEIGHT));
    let units = rows << 4;
    let item_count = c.r32(obj + OBJ_ITEM_COUNT);
    let (quot, rem) = div32(units, ROW_HEIGHT);
    set_hilo(c, quot, rem);
    if !((quot as i64) < (item_count as i32 as i64)) {
        c.w32(obj + OBJ_SCROLL_Y, UNSCROLLED_Y);
        c.s(V0, 0);
        c.s(V1, sx(item_count));
        c.s(A0, sx(units));
        return true;
    }

    // Scrolling: keep the offset between the cursor and the visible window.
    let (quot2, rem2) = div32(units.wrapping_sub(0x28), ROW_HEIGHT);
    set_hilo(c, quot2, rem2);
    let cursor_now = c.r32(obj + OBJ_CURSOR) as i32;
    let scroll_then = c.r32(obj + OBJ_SCROLL) as i32;
    let mut steps = quot2 as u32 as i32;
    if scroll_then >= cursor_now {
        c.w32(obj + OBJ_SCROLL, (cursor_now.wrapping_sub(1)) as u32);
        if cursor_now.wrapping_sub(1) < 0 {
            c.w32(obj + OBJ_SCROLL, 0);
        }
    }
    steps = steps.wrapping_sub(2);
    let diff = cursor_now.wrapping_sub(steps);
    if (c.r32(obj + OBJ_SCROLL) as i32) < diff {
        c.w32(obj + OBJ_SCROLL, diff as u32);
    }
    let scroll = c.r32(obj + OBJ_SCROLL) as i32;
    let product = scroll as i64 * ROW_HEIGHT as i32 as i64;
    let product32 = product as i32;
    set_hilo(c, sx(product32 as u32), sx((product >> 32) as u32));
    let y = SCROLL_Y_ORIGIN.wrapping_sub(product32);
    c.w32(obj + OBJ_SCROLL_Y, y as u32);
    c.s(V0, 0);
    c.s(V1, sx(product32 as u32));
    c.s(A0, sx(ROW_HEIGHT));
    true
}
