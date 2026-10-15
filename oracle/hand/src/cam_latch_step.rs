//! Camera-latch sequencer: advances the four-stage latch state at `+0x44` of the caller's record once per call.

use crate::cam_latch;
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Offset of the sequencer state word in the caller's record.
const STATE_OFF: u32 = 0x44;
/// Offsets of the owner pointers in the record (stage 0 and stage 2).
const OWNER_STAGE0_OFF: u32 = 0x48;
const OWNER_STAGE2_OFF: u32 = 0x4c;
/// Hub flag halfword: the latch may only be armed while it is zero.
const HUB_FLAG_ADDR: u32 = 0x0015_16d8;
/// Index word that selects an 8-byte entry of the latch table.
const TABLE_INDEX_ADDR: u32 = 0x0015_ed88;
/// Base of the latch table (entries are indexed by `index * 8`).
const TABLE_BASE: u32 = 0x0013_7b80;
/// Stage 0 entry fields: request mode, request value (armed only when the value is non-zero).
const STAGE0_MODE_OFF: u32 = 0x2c8;
const STAGE0_VALUE_OFF: u32 = 0x2cc;
/// Stage 2 entry fields: request mode, request value.
const STAGE2_MODE_OFF: u32 = 0x2f8;
const STAGE2_VALUE_OFF: u32 = 0x2fc;

/// Sequencer states.
const STATE_IDLE: u32 = 0;
const STATE_HUB_WAIT_A: u32 = 1;
const STATE_ARM_B: u32 = 2;
const STATE_HUB_WAIT_B: u32 = 3;
const STATE_DONE: u32 = 4;
/// Written when a latch request is refused.
const STATE_REFUSED: u32 = u32::MAX;

/// Try to arm the latch for one stage: owner pointer set, hub flag clear and the table entry's value
/// non-zero. The state becomes `advanced` if the request armed, `REFUSED` otherwise.
fn request_stage(c: &mut Cpu, s: u32, owner_off: u32, mode_off: u32, value_off: u32, advanced: u32) {
    let owner = c.r32(s + owner_off);
    if owner == 0 || c.r16(HUB_FLAG_ADDR) != 0 {
        return;
    }
    let entry = TABLE_BASE + (c.r32(TABLE_INDEX_ADDR) << 3);
    let value = c.r32(entry + value_off);
    if value == 0 {
        return;
    }
    let mode = c.r32(entry + mode_off);
    let (armed, _) = h::run_keep(c, cam_latch::cam_latch_request, &[owner, mode, value], &[]);
    let next = if armed == 0 { STATE_REFUSED } else { advanced };
    c.w32(s + STATE_OFF, next);
}

/// 0x270ed8 cam_latch_step(record) -> v0 = 0: advances the latch sequencer state at record+0x44 by one step.
pub fn cam_latch_step(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let s = c.arg(0);
    match c.r32(s + STATE_OFF) {
        STATE_IDLE => request_stage(c, s, OWNER_STAGE0_OFF, STAGE0_MODE_OFF, STAGE0_VALUE_OFF, STATE_HUB_WAIT_A),
        STATE_HUB_WAIT_A => {
            if c.r16(HUB_FLAG_ADDR) == 0 {
                c.w32(s + STATE_OFF, STATE_ARM_B);
            }
        }
        STATE_ARM_B => request_stage(c, s, OWNER_STAGE2_OFF, STAGE2_MODE_OFF, STAGE2_VALUE_OFF, STATE_HUB_WAIT_B),
        STATE_HUB_WAIT_B => {
            if c.r16(HUB_FLAG_ADDR) == 0 {
                c.w32(s + STATE_OFF, STATE_DONE);
            }
        }
        _ => {}
    }
    c.ret(0);
    true
}
