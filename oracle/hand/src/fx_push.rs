//! Screen-sprite queue push (retail 0x1e07c8) and the once-per-boot tag append (retail 0x1eb6f8).
use crate::gs_pkt_misc;
use crate::h;
use crate::ps2::{Ee, F};
use ee_oracle::cpu::Cpu;

/// Gate word: the push is skipped while it reads GATE_CLOSED.
const GATE: u32 = 0x15f5a4;
const GATE_CLOSED: u32 = 2;
/// Sprite queue: 0x30-byte items from QUEUE, item count at +0xc0, room for QUEUE_MAX items.
const QUEUE: u32 = 0x169140;
const QUEUE_COUNT: u32 = QUEUE + 0xc0;
const QUEUE_MAX: i32 = 3;
const ITEM_SIZE: u32 = 0x30;
/// t0 holding -1 means "no per-item override" (the default angle is used).
const NO_OVERRIDE: u32 = u32::MAX;
/// Default angle (pi/2) stored for the modes below.
const F_PI_2: F = F(0x3fc9_0fdb);
/// a3 == 13, or 0x1f <= a3 < 0x28, selects the pi/2 default.
const MODE_DEFAULT_ONE: i32 = 0x0d;
const MODE_DEFAULT_LO: i32 = 0x1f;
const MODE_DEFAULT_HI: i32 = 0x28;

/// 0x1e07c8 sprite_queue_push(a0, a1, a2 quad ptr, a3, t0, f12, f13, f14) -> void: append one queue item unless the gate
/// is closed or the queue is full.
pub fn push(c: &mut Cpu) -> bool {
    if c.r32(GATE) == GATE_CLOSED {
        return true;
    }
    let count = c.r32(QUEUE_COUNT);
    if (count as i32) >= QUEUE_MAX {
        return true;
    }
    let item = QUEUE.wrapping_add(count.wrapping_mul(ITEM_SIZE));
    let (a0, a1, a2, a3, t0) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3), c.arg(4));
    c.w32(item + 0x20, a0);
    c.w32(item + 0x14, a1);
    let f12 = c.farg(0);
    c.wf(item + 0x10, f12);
    let f13 = c.farg(1);
    c.wf(item + 0x1c, f13);
    c.w32(item + 0x18, a3);
    if a2 != 0 {
        let quad = c.rv(a2);
        c.wv(item, quad);
        c.w16(item + 0x24, 1);
    } else {
        c.w16(item + 0x24, 0);
    }
    if t0 != NO_OVERRIDE {
        c.w32(item + 0x2c, t0);
        c.w16(item + 0x26, 4);
        let f14 = c.farg(2);
        c.wf(item + 0x28, f14);
    } else {
        let mode = a3 as i32;
        if mode == MODE_DEFAULT_ONE || (MODE_DEFAULT_LO..MODE_DEFAULT_HI).contains(&mode) {
            c.w32(item + 0x2c, 2);
            c.wf(item + 0x28, F_PI_2);
            c.w16(item + 0x26, 0);
        }
    }
    c.w32(QUEUE_COUNT, count + 1);
    true
}

/// Tag-slot word at gp-0x780c, set to TAG_VALUE after the first append.
const GP_TAG_SLOT: u32 = (-0x780c_i32) as u32;
const TAG_VALUE: u32 = 0x7f;
/// Flag word; the tag pair is appended only while it reads 0.
const TAG_FLAG: u32 = 0x15f5b8;
/// Retail routine run after the tag slot is set.
const TAG_INIT: u32 = 0x1eae20;

/// 0x1eb6f8 tag_once() -> void: save ra in the frame; on the first call append the tag pair, set the tag slot to 0x7f
/// and run the retail init routine.
pub fn tag_once(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let frame = (c.r[29] as u32).wrapping_sub(0x10);
    let ra = c.r[31];
    c.mem.wr128(frame, ra);
    if c.r32(TAG_FLAG) == 0 {
        let gp = c.r[28] as u32;
        h::run_keep(c, gs_pkt_misc::append_tag_pair, &[], &[]);
        c.w32(gp.wrapping_add(GP_TAG_SLOT), TAG_VALUE);
        h::call_keep(c, TAG_INIT, &[], &[]);
    }
    true
}
