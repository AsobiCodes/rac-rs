//! Hero per-frame wrappers: the joint-block pose pass (0x1f8590), the held-layer frame (0x201af8), the hero gate
//! and item frame (0x2039d0) and a float scale lookup from a per-actor table (0x241440).
use crate::glove_gate;
use crate::h;
use crate::glow;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Joint-block pose pass: first block address is 0x180000 - 0x5880 and the span is 0x1550, in 0xb0 steps.
const JOINT_BLOCK_FIRST: u32 = 0x18_0000 - 0x5880;
const JOINT_BLOCK_SPAN: u32 = 0x1550;
const JOINT_BLOCK_STRIDE: u32 = 0xb0;
const JOINT_BLOCK_POSE: u32 = 0x1f_8210;

/// Hero gate/item frame: the global block sits at 0x140000 - 0xcb0.
const GATE_BLOCK: u32 = 0x14_0000 - 0xcb0;
const GATE_GROUP_OFF: u32 = 0x308;
const GATE_STATE_OFF: u32 = 0x208c;
const GATE_KIND_OFF: u32 = 0x2084;
const GATE_FLAG_A: u32 = 0x20ac;
const GATE_FLAG_B: u32 = 0x20a7;
const GATE_FLAG_C: u32 = 0x20ae;
const STATE_A: u32 = 0x15;
const STATE_B: u32 = 0x16;
const STATE_C: u32 = 0x03;
const KIND_LINK: u32 = 0x12;
const HERO_LOCK_FX: u32 = 0x20_0b10;
const HERO_ITEM_TICK: u32 = 0x20_37f0;
const ITEM_HUD_LAYER: u32 = 0x20_2628;
const ITEM_PREP: u32 = 0x20_1b28;
const ITEM_SETUP: u32 = 0x1f_b158;

/// Scale lookup: table of per-index pointers at 0x197400, each entry points to a record whose word at +0x48 holds
/// a second pointer; that record's word at +0x1c points to a halfword (at +4) that is converted to float and scaled.
const SCALE_TABLE: u32 = 0x19_0000 + 0x7400;
const SCALE_INDEX_LIMIT: u32 = 0xff;
const SCALE_FACTOR_BITS: u32 = 0x3d80_0000;

/// 0x1f8590 hero_joint_blocks_pose(): calls the joint-block pose for each block in turn, returns v0 = 0.
pub fn hero_joint_blocks_pose(c: &mut Cpu) -> bool {
    let saves = [(16, 0x00, h::SQ), (17, 0x10, h::SQ), (31, 0x20, h::SQ)];
    h::frame(c, 0x30, &saves, |c, _sp| {
        let end = (JOINT_BLOCK_FIRST + JOINT_BLOCK_SPAN) as i32;
        let mut block = JOINT_BLOCK_FIRST;
        loop {
            h::call_keep(c, JOINT_BLOCK_POSE, &[block], &[]);
            block = block.wrapping_add(JOINT_BLOCK_STRIDE);
            if (block as i32) >= end {
                break;
            }
        }
        c.s(2, 0);
    });
    true
}

/// 0x201af8 held_layers_frame(): per-frame wrapper: 0x200dc8, then 0x2017d0, then the hero lock effect update.
pub fn held_layers_frame(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    h::frame(c, 0x10, &[(31, 0x00, h::SQ)], |c, _sp| {
        h::call_keep(c, 0x20_0dc8, &[], &[]);
        h::call_keep(c, 0x20_17d0, &[], &[]);
        // Plain call: retail returns 0x200b10's v0/v1/a0 to the caller, which h::run_keep does not reproduce.
        c.call(HERO_LOCK_FX);
    });
    true
}

/// 0x2039d0 hero_gate_item_frame(): updates the gate flags of the hero block, then runs the item and glow ticks.
pub fn hero_gate_item_frame(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    h::frame(c, 0x20, &[(16, 0x00, h::SQ), (31, 0x10, h::SQ)], |c, _sp| {
        c.s(4, 1);
        let group_is_one = c.r16(GATE_BLOCK + GATE_GROUP_OFF) as i16 == 1;
        let mut set_flags = true;
        if !group_is_one {
            let state_minus = c.r32(GATE_BLOCK + GATE_STATE_OFF).wrapping_sub(STATE_A);
            set_flags = if state_minus < 2 {
                true
            } else {
                h::run_keep(c, glove_gate::hero_skip_gate, &[1], &[]).0 != 0
            };
        }
        if set_flags {
            c.w8(GATE_BLOCK + GATE_FLAG_A, 1);
            c.w8(GATE_BLOCK + GATE_FLAG_B, 1);
        }
        let skip_again = h::run_keep(c, glove_gate::hero_skip_gate, &[], &[]).0;
        if skip_again != 0 {
            c.w8(GATE_BLOCK + GATE_FLAG_C, 1);
        } else {
            c.s(4, GATE_BLOCK as u64);
            let state = c.r32(GATE_BLOCK + GATE_STATE_OFF);
            let listed = state == STATE_A || state == STATE_B || state == STATE_C;
            if listed || c.r32(GATE_BLOCK + GATE_KIND_OFF) == KIND_LINK {
                c.w8(GATE_BLOCK + GATE_FLAG_C, 1);
            } else {
                c.w8(GATE_BLOCK + GATE_FLAG_C, 0);
            }
        }
        h::call_keep(c, ITEM_PREP, &[], &[]);
        h::call_keep(c, ITEM_HUD_LAYER, &[], &[]);
        h::run_keep(c, glow::glow_pulse_update, &[], &[]);
        h::call_keep(c, ITEM_SETUP, &[], &[]);
        h::call_keep(c, HERO_ITEM_TICK, &[], &[]);
    });
    true
}

/// 0x241440 scale_table_value(index, n) -> f0: (float)(table record's halfword) * 0.0625, or 0 when n >= 0xff.
pub fn scale_table_value(c: &mut Cpu) -> bool {
    let index = c.arg(0);
    let n = c.arg(1);
    h::frame(c, 0x10, &[(31, 0x00, h::SQ)], |c, _sp| {
        if n >= SCALE_INDEX_LIMIT {
            c.s(2, 0);
            c.fret(F(0));
            return;
        }
        let slot = c.r32(SCALE_TABLE.wrapping_add(index.wrapping_mul(4)));
        let rec = c.r32(slot.wrapping_add(n.wrapping_mul(4)).wrapping_add(0x48));
        let inner = c.r32(rec.wrapping_add(0x1c));
        let value = c.r16(inner.wrapping_add(4)) as i16 as i32 as u32;
        c.s(2, slot.wrapping_add(n.wrapping_mul(4)) as u64);
        c.s(3, rec as u64);
        c.s(5, inner as u64);
        c.s(4, value as u64);
        let (_, f) = h::run_keep(c, mathx::int_to_float, &[value], &[]);
        c.f[1] = SCALE_FACTOR_BITS;
        c.s(1, SCALE_FACTOR_BITS as u64);
        c.fret(f * F(SCALE_FACTOR_BITS));
    });
    true
}
