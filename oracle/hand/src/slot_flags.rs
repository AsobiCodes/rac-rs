//! Slot flag refresh and the slot-table flag pass (retail 0x26da80..0x26dbbc).
//! The slot table at 0x1ba2b8 holds five (value, flags) word pairs; a value below the limit word at 0x1b9efc gets flag
//! bit 2. The object state at s0 keeps its request word at +0x50 and its status word at +0x10.

use crate::h;
use crate::ps2::*;
use crate::{cam_latch, stream_ctl, table_fill};
use ee_oracle::cpu::Cpu;

/// Slot table: five entries of (value word, flags word).
const SLOT_TABLE: u32 = 0x001b_a2b8;
/// Flags word of the first slot (the table base plus 4).
const SLOT_FLAGS_FIRST: u32 = 0x001b_a2bc;
const SLOT_COUNT: u32 = 5;
const SLOT_STRIDE: u32 = 8;
/// Flag bit set on a slot whose value is non-zero and below the limit.
const SLOT_FLAG_IN_RANGE: u32 = 2;
/// Limit word that slot values are compared against (unsigned).
const SLOT_LIMIT: u32 = 0x001b_9efc;
/// Camera request inputs: the first word is at 0x1b9ef8, the other two at 0x1390a8 and 0x1390ac.
const CAM_REQ_ARG0: u32 = 0x001b_9ef8;
const CAM_REQ_ARG1: u32 = 0x0013_90a8;
const CAM_REQ_ARG2: u32 = 0x0013_90ac;
/// Camera latch active halfword.
const CAM_LATCH_FLAG: u32 = 0x0015_16d8;
/// Stream-kick condition on the object: request word at +0x50 must equal this.
const STREAM_KICK_REQUEST: u32 = 1;
/// Object fields.
const OBJ_REQUEST: u32 = 0x50;
const OBJ_STATUS: u32 = 0x10;
const OBJ_SLOT_COUNT_WORD: u32 = 0x38;
const OBJ_SEED: u32 = 0x54;
/// Status bit set on the object at the end of the update.
const STATUS_DONE: u32 = 4;
/// Request word values written from the camera request result.
const REQUEST_OK: u32 = 1;
const REQUEST_FAILED: u32 = 3;
/// Final destination words for a non-zero seed.
const SEED_OUT: u32 = 0x0015_f640;
const SEED_COUNT_OUT: u32 = 0x0017_953c;
/// Seed value handed to the table fill.
const FILL_SEED: u32 = 1;
/// MIPS register numbers.
const REG_V1: usize = 3;

/// 0x26da80 slot_flags_update(obj) -> 0: clears the object's slot words, refreshes the slot flags and, when the
/// camera latch is idle, issues a camera latch request and records its outcome in the request word.
pub fn slot_flags_update(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let obj = c.arg(0);
    h::run(c, table_fill::fill_table, &[FILL_SEED], &[]);
    c.w32(obj + OBJ_SEED, 0);
    c.w32(obj + OBJ_SLOT_COUNT_WORD, 0);
    for i in 0..SLOT_COUNT {
        let value = c.r32(SLOT_TABLE + i * SLOT_STRIDE);
        let flags_at = SLOT_FLAGS_FIRST + i * SLOT_STRIDE;
        if value != 0 && value < c.r32(SLOT_LIMIT) {
            let flags = c.r32(flags_at);
            c.w32(flags_at, flags | SLOT_FLAG_IN_RANGE);
        }
    }
    c.w32(obj + OBJ_REQUEST, 0);
    if c.r16(CAM_LATCH_FLAG) as i16 == 0 {
        let args = [c.r32(CAM_REQ_ARG0), c.r32(CAM_REQ_ARG1), c.r32(CAM_REQ_ARG2)];
        // The caller tests v0 right after the call, so the callee's v0 stays as retail leaves it.
        let (result, _) = h::run_keep(c, cam_latch::cam_latch_request, &args, &[]);
        let outcome = if result != 0 { REQUEST_OK } else { REQUEST_FAILED };
        c.w32(obj + OBJ_REQUEST, outcome);
    }
    let status = c.r32(obj + OBJ_STATUS) | STATUS_DONE;
    c.w32(obj + OBJ_STATUS, status);
    // Retail leaves the new status word in v1.
    c.s(REG_V1, status as i32 as i64 as u64);
    c.ret(0);
    true
}

/// 0x26db50 slot_flags_finish(obj) -> 0: kicks the stream when the request is OK and the latch is active, refills the
/// table, and copies a non-zero seed (and its count word) to the output words.
pub fn slot_flags_finish(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let obj = c.arg(0);
    if c.r16(CAM_LATCH_FLAG) as i16 != 0 && c.r32(obj + OBJ_REQUEST) == STREAM_KICK_REQUEST {
        h::run(c, stream_ctl::stream_kick, &[], &[]);
    }
    h::run(c, table_fill::fill_table, &[FILL_SEED], &[]);
    let seed = c.r32(obj + OBJ_SEED);
    if seed != 0 {
        c.w32(SEED_OUT, seed);
        let count = c.r32(obj + OBJ_SLOT_COUNT_WORD);
        c.w32(SEED_COUNT_OUT, count);
    }
    c.ret(0);
    true
}
