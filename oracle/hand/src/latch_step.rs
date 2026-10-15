//! Camera latch step for one request object (retail 0x26dbc0..0x26dd2c).
//! The object's state word at +0x50 drives it. State 0 waits for the latch gate, issues a camera latch request
//! (0x258120) and goes to state 1 on success or state 3 on failure. State 1 waits for the gate again, copies a
//! table block with mem_copy, rebases the offsets in a record of words and goes to state 2. Other states do nothing.
//! The function always returns 0.

use crate::cam_latch;
use crate::h;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Object fields.
const OBJ_FLAGS: u32 = 0x10;
const OBJ_STATE: u32 = 0x50;
const OBJ_SAVED_A: u32 = 0x38;
const OBJ_SAVED_B: u32 = 0x54;
/// Flag bit cleared when the copy finishes (retail `and` with -5).
const FLAG_BIT4: u32 = 0x4;

/// States of the object.
const STATE_REQUEST: u32 = 0;
const STATE_COPY: u32 = 1;
const STATE_DONE: u32 = 2;
const STATE_FAILED: u32 = 3;

/// Latch gate halfword: nonzero means the latch is busy and nothing happens.
const LATCH_GATE: u32 = 0x0015_0000 + 0x16d8;
/// Request parameters: the latch call takes a1 from +0x1528 and a2 from +0x152c of this block.
const REQUEST_PARAMS: u32 = 0x0013_0000 + 0x7b80;
const REQUEST_ARG_A1: u32 = 0x1528;
const REQUEST_ARG_A2: u32 = 0x152c;
/// Word passed to the latch request as a0.
const REQUEST_ARG_A0: u32 = 0x001c_0000 - 0x6108;

/// Table base register block (retail s0). Its word at +0x108 points at the offset table.
const TABLE_BLOCK: u32 = 0x001c_0000 - 0x6210;
const TABLE_BASE_OFF: u32 = 0x108;
/// Index word selecting one entry of the offset table.
const TABLE_INDEX: u32 = 0x0016_0000 - 0x1278;
/// Word copied into the object at +0x54.
const SAVED_WORD: u32 = 0x0016_0000 - 0x09c0;
/// Record whose +0x2c word is the count of rebased entries.
const RECORD: u32 = 0x0018_0000 - 0x6af0;
const RECORD_COUNT: u32 = 0x2c;
/// Gp-relative slot that receives the table base.
const GP_BASE_SLOT: i32 = -0x75c0;
/// Stride of the rebased entries.
const ENTRY_STRIDE: u32 = 0x10;

/// Stack frame of the retail prologue: s0, s1, s2 and ra saved at 0x0, 0x10, 0x20, 0x30.
const FRAME_SIZE: u32 = 0x40;

/// 0x26dbc0 latch_step(obj) -> 0: advances the camera latch request state machine of obj (state at +0x50).
pub fn latch_step(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let saves = [(16, 0x0, h::SQ), (17, 0x10, h::SQ), (18, 0x20, h::SQ), (31, 0x30, h::SQ)];
    h::frame(c, FRAME_SIZE, &saves, |c, _sp| {
        let obj = c.arg(0);
        match c.r32(obj.wrapping_add(OBJ_STATE)) {
            STATE_REQUEST => issue_request(c, obj),
            STATE_COPY => copy_table(c, obj),
            _ => {}
        }
    });
    c.ret(0);
    true
}

/// State 0: when the gate is open, issue the latch request; success moves to state 1, failure to state 3.
fn issue_request(c: &mut Cpu, obj: u32) {
    if c.r16(LATCH_GATE) as i16 != 0 {
        return;
    }
    let a0 = c.r32(REQUEST_ARG_A0);
    let a1 = c.r32(REQUEST_PARAMS + REQUEST_ARG_A1);
    let a2 = c.r32(REQUEST_PARAMS + REQUEST_ARG_A2);
    let (latched, _) = h::run(c, cam_latch::cam_latch_request, &[a0, a1, a2], &[]);
    let next = if latched == 0 { STATE_FAILED } else { STATE_COPY };
    c.w32(obj.wrapping_add(OBJ_STATE), next);
}

/// State 1: when the gate is open, copy the table block, then rebase the record's entries by the table base.
fn copy_table(c: &mut Cpu, obj: u32) {
    if c.r16(LATCH_GATE) as i16 != 0 {
        return;
    }
    let index = c.r32(TABLE_INDEX);
    let base = c.r32(TABLE_BLOCK.wrapping_add(TABLE_BASE_OFF));
    let offset = c.r32(base.wrapping_add(index << 2));
    let block = base.wrapping_add(offset);
    let count = c.r32(block);
    let len = c.r32(block.wrapping_add(4));
    let copy_len = ((len.wrapping_add(3)) & !3).wrapping_sub(8);
    h::run(c, mathx::mem_copy, &[base, block.wrapping_add(8), copy_len], &[]);

    let saved = c.r32(SAVED_WORD);
    c.w32(obj.wrapping_add(OBJ_SAVED_B), saved);
    let old_count = c.r32(RECORD + RECORD_COUNT);
    c.w32(obj.wrapping_add(OBJ_SAVED_A), old_count);
    c.w32(RECORD + RECORD_COUNT, count);

    let table = c.r32(TABLE_BLOCK.wrapping_add(TABLE_BASE_OFF));
    let gp = c.r[28] as u32;
    c.w32(gp.wrapping_add(GP_BASE_SLOT as u32), table);
    if (count as i32) > 0 {
        rebase_entries(c, table);
    }
    let flags = c.r32(obj.wrapping_add(OBJ_FLAGS)) & !FLAG_BIT4;
    c.w32(obj.wrapping_add(OBJ_FLAGS), flags);
    c.w32(obj.wrapping_add(OBJ_STATE), STATE_DONE);
}

/// Adds `table - 8` to the first word of each stride-sized entry, for as many entries as the record count says
/// (the count is read again each time, as retail does).
fn rebase_entries(c: &mut Cpu, table: u32) {
    let delta = table.wrapping_sub(8);
    let mut ptr = table;
    let mut done: i32 = 0;
    loop {
        let word = c.r32(ptr);
        c.w32(ptr, word.wrapping_add(delta));
        done += 1;
        ptr = ptr.wrapping_add(ENTRY_STRIDE);
        if done >= c.r32(RECORD + RECORD_COUNT) as i32 {
            break;
        }
    }
}
