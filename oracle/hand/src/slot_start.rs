//! Hand-written 0x257620: starts a timed request from the slot state block at 0x1516d0.
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Slot state block (retail `lui 0x15` + `addiu 0x16d0`).
const SLOT: u32 = 0x0015_16d0;
/// Word: request id. 0 = none, -1 = not started.
const F_ID: u32 = 0x34;
/// Half: copy of the index argument a0.
const F_INDEX: u32 = 0x38;
/// Half: low 16 bits of the a2 argument.
const F_PARAM_LO: u32 = 0x3a;
/// Half: copy of the mode argument a1.
const F_MODE: u32 = 0x3c;
/// Half: state, 9 = idle.
const F_STATE: u32 = 0x3e;
/// Half: cleared just before the request call.
const F_CLEAR: u32 = 0x44;
/// Word: timeout value 10 written before the call.
const F_TIMEOUT: u32 = 0x48;
/// Word: 0xbb80 written before the call.
const F_PERIOD: u32 = 0x4c;
const STATE_IDLE: i32 = 9;
/// Table of request pointers, indexed by a0 + 1 (4-byte entries).
const TABLE: u32 = 0x0013_a628;
/// Pointer to a string/pointer pair passed as the fourth stack word.
const LABEL_PTR: u32 = 0x0025_8468;
/// Retail request function, called with eight arguments.
const REQUEST: u32 = 0x0012_ec08;
/// Stack frame size of the retail function.
const FRAME: u32 = 0x30;
/// Offset of the saved return address in the frame.
const RA_SLOT: u32 = 0x20;

/// Writes a sign-extended 32-bit result into a general register like a MIPS 32-bit op.
fn set_gpr(c: &mut Cpu, n: usize, v: u32) {
    c.r[n] = (v as i32 as i128) as u128;
}

/// Splits a 128-bit register value into the four 32-bit lanes of a quadword (lane 0 = low word).
fn lanes(v: u128) -> V {
    V([v as u32, (v >> 32) as u32, (v >> 64) as u32, (v >> 96) as u32])
}

/// Joins the four 32-bit lanes of a quadword into a 128-bit register value.
fn join(v: V) -> u128 {
    v.0.iter().rev().fold(0u128, |acc, &w| (acc << 32) | w as u128)
}

/// 0x257620 slot_start(a0 index, a1 mode, a2 param) -> v0: starts a request in slot SLOT unless the slot is idle
/// (state 9), its id is 0, or the table entry for the index is 0; a -1 id returns 0x130000 without starting.
pub fn slot_start(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let (a0, a1, a2) = (c.arg(0), c.arg(1), c.arg(2));
    let sp_saved = c.r[29];
    let sp = (c.g(29) as u32).wrapping_sub(FRAME);
    set_gpr(c, 29, sp);
    c.wv(sp + RA_SLOT, lanes(c.r[31]));
    set_gpr(c, 10, a0);
    set_gpr(c, 12, SLOT);
    set_gpr(c, 13, a2);
    set_gpr(c, 14, STATE_IDLE as u32);

    let id = c.r32(SLOT + F_ID);
    let state = c.r16(SLOT + F_STATE) as i16 as i32;
    set_gpr(c, 2, state as u32);
    set_gpr(c, 11, id);

    let mut entry = 0u32;
    let mut start = false;
    if state != STATE_IDLE && id != 0 {
        if id == u32::MAX {
            set_gpr(c, 2, 0x0013_0000);
        } else {
            let slot = (a0.wrapping_add(1) << 2).wrapping_add(TABLE);
            set_gpr(c, 3, slot);
            set_gpr(c, 2, TABLE);
            entry = c.r32(slot);
            set_gpr(c, 4, entry);
            set_gpr(c, 9, SLOT + F_ID);
            start = entry != 0;
        }
    }

    if start {
        let flag = if a1 & 1 != 0 { 0x24 } else { 0x20 };
        c.w32(sp, flag);
        c.w32(sp + 8, LABEL_PTR);
        c.w32(sp + 0x10, SLOT + F_ID);
        c.w32(sp + 0x14, 0);
        c.w16(SLOT + F_INDEX, a0 as u16);
        c.w16(SLOT + F_MODE, a1 as u16);
        c.w16(SLOT + F_PARAM_LO, a2 as u16);
        c.w16(SLOT + F_STATE, STATE_IDLE as u16);
        c.w32(SLOT + F_TIMEOUT, 10);
        c.w32(SLOT + F_PERIOD, 0xbb80);
        c.w16(SLOT + F_CLEAR, 0);
        set_gpr(c, 2, 10);
        set_gpr(c, 3, 0xbb80);
        let t0 = a2 as u16 as i16 as i32 as u32;
        crate::h::call_keep(c, REQUEST, &[entry, 0, 0, 0, t0, 0, 1, id], &[]);
    }

    // retail reloads ra from the frame, then returns with sp restored
    c.r[31] = join(c.rv(sp + RA_SLOT));
    c.r[29] = sp_saved;
    true
}
