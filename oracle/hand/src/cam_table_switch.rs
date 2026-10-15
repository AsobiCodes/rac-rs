//! Camera table switch: the request flag word, the cam-arg spill and the block install that the camera tick runs.

use crate::camswitch;
use crate::h::{self, SQ};
use crate::misc_leaf;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const RA: usize = 31;
const S0: usize = 16;
const S1: usize = 17;
const S2: usize = 18;
const S3: usize = 19;
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const GP: usize = 28;

/// Request flag word at gp-0x5ae0 (bits set by the caller, cleared when the switch is done).
const GP_FLAGS: i32 = -0x5ae0;
/// Spill threshold word at gp-0x5ab4 (compared with the table delta).
const GP_THRESHOLD: i32 = -0x5ab4;
/// Spill length word at gp-0x5aac (written with the table delta when the spill runs).
const GP_SPILL_LEN: i32 = -0x5aac;
/// Pointer word at gp-0x5ac0 (the current block's publish slot).
const GP_PUBLISH: i32 = -0x5ac0;

/// Index word of the active table entry (overlay data, read at call time).
const TABLE_INDEX: u32 = 0x16_1150;
/// Table of 4-byte entries (overlay data, read at call time).
const TABLE: u32 = 0x16_1138;
/// Pointer to the current block (overlay data, read at call time).
const BLOCK: u32 = 0x16_1140;
/// Room limit compared with the table delta (overlay data, read at call time).
const ROOM_LIMIT: u32 = 0x16_1154;

/// Block tag written into the first word of the new block.
const BLOCK_TAG: u32 = 0x7000_0000;
/// Bits OR-ed into the publish word.
const PUBLISH_BITS: u32 = 0x3000_0000;
/// Bits OR-ed into the block's flag word before the flush.
const BLOCK_FLAG_BITS: u32 = 0xc0;
/// Publish pointer offset from the block.
const PUBLISH_OFFSET: u32 = 0x10;
/// Spill argument pointer used by the switch (0x1e0000 - 0x4e30).
const SPILL_ARG_SWITCH: u32 = 0x1E_0000 - 0x4E30;
/// Spill argument pointer used by the wait (0x1e0000 - 0x4e18).
const SPILL_ARG_WAIT: u32 = 0x1E_0000 - 0x4E18;
/// Round limit of the wait loop (100000).
const WAIT_ROUNDS: u32 = 0x1_86A0;
/// Argument of each countdown round.
const COUNTDOWN_ARG: u32 = 0x400;

/// Retail: allocates the block (returns it in v0; a0 = 1).
const BLOCK_ALLOC: u32 = 0x12_2fe0;
/// Retail: flushes the cache (syscall stub).
const FLUSH_CACHE: u32 = 0x11_8a80;
/// Retail: installs a block entry (a0 = block, a1 = table entry).
const BLOCK_INSTALL: u32 = 0x12_32c8;
/// Retail: follow-up after the spill in the wait loop (not named yet).
const WAIT_FINISH: u32 = 0x1e_9610;

fn gp(c: &Cpu, off: i32) -> u32 {
    (c.g(GP) as u32).wrapping_add(off as u32)
}

fn gp_word(c: &mut Cpu, off: i32) -> u32 {
    c.r32(gp(c, off))
}

/// Sets a 32-bit result the way addiu/lw leave it: sign-extended to 64 bits.
fn set32(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// 0x2910e8 cam_table_switch() -> (): when the active table entry is behind the current block by more than the room
/// limit, spills the camera args and clears the flag; otherwise allocates a block, tags it, publishes its slot and
/// installs the active entry.
pub fn cam_table_switch(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    h::frame(c, 0x30, &[(S0, 0x0, SQ), (S1, 0x10, SQ), (RA, 0x20, SQ)], |c, _sp| {
        let index = c.r32(TABLE_INDEX);
        let entry = c.r32(TABLE.wrapping_add(index << 2));
        let flags = gp_word(c, GP_FLAGS) | 0x1f;
        c.w32(gp(c, GP_FLAGS), flags);
        let delta = c.r32(BLOCK).wrapping_sub(entry);
        let mut spilled = false;
        if (c.r32(ROOM_LIMIT) as i32) < (delta as i32) && (gp_word(c, GP_THRESHOLD) as i32) < (delta as i32) {
            c.w32(gp(c, GP_SPILL_LEN), delta);
            h::run_keep(c, camswitch::spill_cam_args, &[SPILL_ARG_SWITCH], &[]);
            set32(c, V1, 1);
            spilled = true;
        }
        if spilled {
            c.w32(gp(c, GP_FLAGS), 0);
            return;
        }
        let publish = gp_word(c, GP_PUBLISH);
        c.w32(publish, BLOCK_TAG);
        let block = c.r32(BLOCK);
        c.w32(block.wrapping_add(4), 0);
        let block = c.r32(BLOCK);
        c.w32(block.wrapping_add(8), 0);
        let block = c.r32(BLOCK);
        c.w32(block.wrapping_add(0xc), 0);
        h::call_keep(c, BLOCK_ALLOC, &[1], &[]);
        let new_block = c.g(V0) as u32;
        let word = c.r32(new_block) | BLOCK_FLAG_BITS;
        c.w32(new_block, word);
        h::call_keep(c, FLUSH_CACHE, &[0], &[]);
        let index = c.r32(TABLE_INDEX);
        let entry = c.r32(TABLE.wrapping_add(index << 2));
        h::call_keep(c, BLOCK_INSTALL, &[new_block, entry], &[]);
    });
    true
}

/// 0x2911f8 cam_table_wait(flags) -> (): while the flags in a0 are set in the request word, counts down in rounds;
/// after more than WAIT_ROUNDS rounds it spills the camera args and calls the retail follow-up.
pub fn cam_table_wait(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let mask = c.g(A0) as u32;
    h::frame(c, 0x50, &[(S0, 0x0, SQ), (S1, 0x10, SQ), (S2, 0x20, SQ), (S3, 0x30, SQ), (RA, 0x40, SQ)], |c, _sp| {
        let mut round: u32 = 0;
        while gp_word(c, GP_FLAGS) & mask != 0 {
            h::run_keep(c, misc_leaf::countdown_by_two, &[COUNTDOWN_ARG], &[]);
            let old = round;
            round = round.wrapping_add(1);
            if (WAIT_ROUNDS as i32) < (old as i32) {
                h::run_keep(c, camswitch::spill_cam_args, &[SPILL_ARG_WAIT], &[]);
                let a0 = c.g(A0) as u32;
                h::call_keep(c, WAIT_FINISH, &[a0], &[]);
                return;
            }
        }
    });
    true
}

/// 0x291278 cam_table_publish(a0, a1) -> v0: stores a1 | 0x30000000 at the current block, a0 at +4, clears +8 and
/// +0xc, then publishes block+0x10 at gp-0x5ac0 and returns it in v0.
pub fn cam_table_publish(c: &mut Cpu) -> bool {
    let a1 = c.g(A1) | u64::from(PUBLISH_BITS);
    c.s(A1, a1);
    let block = c.r32(BLOCK);
    c.w32(block, a1 as u32);
    let value = c.g(A0) as u32;
    let block = c.r32(BLOCK);
    c.w32(block.wrapping_add(4), value);
    let block = c.r32(BLOCK);
    c.w32(block.wrapping_add(8), 0);
    let block = c.r32(BLOCK);
    c.w32(block.wrapping_add(0xc), 0);
    let block = c.r32(BLOCK);
    set32(c, A0, block);
    set32(c, V1, block);
    let publish = block.wrapping_add(PUBLISH_OFFSET);
    set32(c, V0, publish);
    c.w32(gp(c, GP_PUBLISH), publish);
    true
}
