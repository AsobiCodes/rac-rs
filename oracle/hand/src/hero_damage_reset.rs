//! Hero hit points, joint-block reset and the limb-pulse record: retail state block at 0x13f350 and joint blocks at
//! 0x17a780.
use crate::euler_basis;
use crate::h;
use crate::joint_block_pose;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Gameplay state block addressed by $s0 in the functions below (lui 0x14, addiu -0xcb0).
const STATE: u32 = 0x0013_f350;
/// Hero hit-point word.
const HERO_HP: u32 = STATE + 0x22a8;
/// Retail HUD popup, called after a hit.
const HUD_POPUP: u32 = 0x0022_7f40;
/// Retail layer detach, called as retail does (its scratch registers are kept).
const LAYER_DETACH: u32 = 0x0024_1ed0;
/// Pulse init's first-call work: the draw-queue add (a0 = 0x1fade8, a1 = word at STATE + 0x2080).
const PULSE_INIT_FLAG: u32 = STATE + 0x1eb2;

/// Joint-block array (lui 0x18, addiu -0x5880): 31 blocks of 0xb0 bytes.
const JOINT_BLOCKS: u32 = 0x0017_a780;
const BLOCK_SIZE: u32 = 0xb0;
const BLOCK_END: u32 = JOINT_BLOCKS + 0x1590;
/// Byte flag: when nonzero the block has a slot to detach.
const BLOCK_FLAG: u32 = 0x1;
/// Halfword: the block's joint-state slot.
const BLOCK_SLOT: u32 = 0xa2;
/// Word: weight, reset to 1.0.
const BLOCK_WEIGHT: u32 = 0xac;
/// Offsets of the three vectors zeroed in each block.
const BLOCK_VEC_A: u32 = 0x40;
const BLOCK_VEC_B: u32 = 0x60;
const BLOCK_VEC_C: u32 = 0x90;

/// 1.0 as f32 bits.
const ONE_BITS: u32 = 0x3f80_0000;

/// Halfword count of pulse records (STATE + 0x1eb0).
const PULSE_COUNT: u32 = STATE + 0x1eb0;
/// Halfword init flag of the pulse init (STATE + 0x1eb2), left in v1 by the retail init.
/// Pulse records: 16 bytes each, from STATE + 0x1dd0.
const PULSE_RECORDS: u32 = STATE + 0x1dd0;
/// Per-record word and the two float arrays, indexed by record number * 4.
const PULSE_WORD_OFF: u32 = 0x1e50;
const PULSE_F12_OFF: u32 = 0x1e70;
const PULSE_F13_OFF: u32 = 0x1e90;

/// 0x1f8168 hero_take_damage(strength): when strength is nonzero, hp -= min(strength, 1) (signed, kept at 0 or
/// above), then the HUD popup 0x227f40 runs.
pub fn hero_take_damage(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let strength = c.arg(0);
    h::frame(c, 0x10, &[], |c, sp| {
        if strength == 0 {
            // Retail skips the save, so its restore reads the frame as it is.
            c.r[31] = c.mem.rd128(sp);
            c.s(5, 0);
            return;
        }
        c.mem.wr128(sp, c.r[31]);
        let s = strength as i32;
        let hit = if s < 2 { s } else { 1 };
        let raw = c.r32(HERO_HP).wrapping_sub(hit as u32) as i32;
        let stored = if raw >= 0 { raw } else { 0 };
        c.s(5, u64::from(s < 2));
        c.s(2, hit as i64 as u64);
        c.s(6, STATE as u64);
        c.s(3, raw as i64 as u64);
        c.w32(HERO_HP, stored as u32);
        h::call_keep(c, HUD_POPUP, &[], &[]);
        c.r[31] = c.mem.rd128(sp);
    });
    true
}

/// 0x1f85e0 hero_joint_blocks_reset(): for each of the 31 joint blocks, detaches its layer when the flag byte is
/// set, zeroes its three vectors and sets its weight to 1.0.
pub fn hero_joint_blocks_reset(c: &mut Cpu) -> bool {
    c.s(1, ONE_BITS as u64);
    let saves = [(16, 0x00, h::SQ), (17, 0x10, h::SQ), (18, 0x20, h::SQ), (19, 0x30, h::SQ), (20, 0x40, h::SQ), (31, 0x50, h::SQ)];
    h::frame(c, 0x70, &saves, |c, sp| {
        c.w32(sp + 0x60, c.f[20]);
        let mut blk = JOINT_BLOCKS;
        let mut va = JOINT_BLOCKS + BLOCK_VEC_A;
        let mut vb = JOINT_BLOCKS + BLOCK_VEC_B;
        let mut vc = JOINT_BLOCKS + BLOCK_VEC_C;
        let mut last_c = vc;
        loop {
            if c.r8(blk + BLOCK_FLAG) != 0 {
                let state = c.r16(blk + BLOCK_SLOT) as i16 as i32 as u32;
                let slot = h::run_keep(c, joint_block_pose::joint_state_slot_hand, &[state], &[]).0;
                if slot != 0 {
                    h::call_keep(c, LAYER_DETACH, &[slot, blk], &[]);
                }
            }
            h::run_keep(c, euler_basis::zero_vec4, &[va], &[]);
            va += BLOCK_SIZE;
            h::run_keep(c, euler_basis::zero_vec4, &[vb], &[]);
            vb += BLOCK_SIZE;
            last_c = vc;
            h::run_keep(c, euler_basis::zero_vec4, &[vc], &[]);
            vc += BLOCK_SIZE;
            c.wf(blk + BLOCK_WEIGHT, F(ONE_BITS));
            let more = (va as i32) < (BLOCK_END as i32);
            blk += BLOCK_SIZE;
            if !more {
                break;
            }
        }
        c.s(2, 0);
        c.s(4, last_c as u64);
    });
    true
}

/// 0x1fb0e0 limb_pulse_record(record*, word, f12, f13): copies the 16-byte record at record* into the next pulse
/// slot, stores the word and the two floats at the slot's index, initialises the pulse and bumps the slot count.
pub fn limb_pulse_record(c: &mut Cpu) -> bool {
    // Declined: the hand's rewrite left at (r1) differing from retail on the first-init path (draw_queue_add), so
    // retail runs the whole function. Nothing is written before the decline.
    let _ = c;
    false
}
