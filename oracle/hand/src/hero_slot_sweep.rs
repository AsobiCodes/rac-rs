//! Per-frame sweep of the hero/moby slot block at 0x13f350: releases pending mobys and HUD slots, clears the hand
//! state and runs the loop-clear and follow-up calls.

use crate::h;
use ee_oracle::cpu::Cpu;
use crate::ps2::*;

/// Base of the slot block (retail `s0` = 0x140000 - 0xcb0).
const BLOCK: u32 = 0x0013_f350;
const MODE: u32 = BLOCK + 0x20a4;
const SLOT_A: u32 = BLOCK + 0x22a8;
const SLOT_B: u32 = BLOCK + 0x22ac;
const SLOT_C: u32 = BLOCK + 0x22b0;
const MOBY_1620: u32 = BLOCK + 0x1620;
const MOBY_1624: u32 = BLOCK + 0x1624;
const HUD_HANDLE: u32 = BLOCK + 0x1634;
const MOBY_A84: u32 = BLOCK + 0xa84;
const MOBY_A88: u32 = BLOCK + 0xa88;
const MOBY_A8C: u32 = BLOCK + 0xa8c;
const SRC_QUAD: u32 = BLOCK + 0x80;
const FLOAT_A94: u32 = BLOCK + 0xa94;
const FRAME_2080: u32 = BLOCK + 0x2080;
const COUNTER_2084: u32 = BLOCK + 0x2084;
/// Mode value that selects the slot-release path.
const MODE_RELEASE: u8 = 1;
/// Mode value that selects the HUD-slot path.
const MODE_HUD: u8 = 2;
/// Value of `COUNTER_2084` that ends the sweep early.
const COUNTER_DONE: u32 = 0x64;
/// Global state word (0x160000 - 0xa5c) that picks the early exit.
const STATE_WORD: u32 = 0x0015_f5a4;
const STATE_SKIP_A: u32 = 2;
const STATE_SKIP_B: u32 = 6;
/// Value returned in v0 on the early-exit paths.
const RET_SKIP: u32 = 6;
/// Flag that the sweep call takes (retail `jal 0x215538`).
const SWEEP_FN: u32 = 0x0021_5538;
const ONE_F32: u32 = 0x3f80_0000;
const NO_HANDLE: u16 = 0xffff;
const PAD_MASK: u16 = 0xfff9;

/// 0x203bd8 slot_sweep_tick(): releases the pending moby and HUD slots, clears the hand loop, then runs the sweep.
pub fn slot_sweep_tick(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    if c.r8(MODE) == MODE_RELEASE {
        let v0 = c.r32(SLOT_A);
        let v1 = c.r16(SLOT_C) as i16 as i32 as u32;
        let m = c.r32(MOBY_1620);
        c.w32(SLOT_B, v0);
        c.w32(SLOT_A, v1);
        if m != 0 {
            let _ = h::run(c, crate::actor::free_moby, &[m], &[]);
            c.w32(MOBY_1620, 0);
        }
        let m = c.r32(MOBY_1624);
        if m != 0 {
            let _ = h::run(c, crate::actor::free_moby, &[m], &[]);
            c.w32(MOBY_1624, 0);
        }
    }
    if c.r8(MODE) == MODE_HUD {
        let handle = c.r16(HUD_HANDLE) as i16 as i32;
        if handle != -1 {
            let _ = h::run(c, crate::hud_slots::hud_slot_release, &[handle as u32, 0], &[]);
            c.w16(HUD_HANDLE, NO_HANDLE);
        }
    }
    let _ = h::run(c, crate::hero_damage_reset::hero_joint_blocks_reset, &[], &[]);
    let v1 = c.r32(MOBY_A84);
    c.w8(MODE, 0);
    if v1 != 0 {
        let flags = c.r16(v1 + 0x34);
        c.w16(v1 + 0x34, flags & PAD_MASK);
    }
    let m = c.r32(MOBY_A8C);
    c.w32(MOBY_A84, 0);
    if m != 0 {
        let _ = h::run(c, crate::actor::free_moby, &[m], &[]);
        c.w32(MOBY_A8C, 0);
    }
    let _ = h::run(c, crate::foot_probe::slot_release, &[], &[]);
    let m = c.r32(MOBY_A88);
    c.w32(FRAME_2080, m);
    let quad = c.rv(SRC_QUAD);
    c.wv(m + 0x10, quad);
    c.w32(m + 0x98, 0);
    c.w32(FLOAT_A94, ONE_F32);
    // retail sets a1 = s0+0x80 (SRC_QUAD) before this jal; pass it too.
    let _ = h::run_keep(c, crate::hero_anim::hero_loop_clear_hand, &[m, SRC_QUAD], &[]);
    if c.r32(COUNTER_2084) == COUNTER_DONE {
        let state = c.r32(STATE_WORD);
        if state == STATE_SKIP_A || state == STATE_SKIP_B {
            c.ret(RET_SKIP);
            return true;
        }
    }
    let _ = h::call_keep(c, SWEEP_FN, &[0, 1], &[]);
    true
}
