//! Camera latch request, pump and drain (retail 0x258120..0x2582e8).
//! The latch state sits at 0x1516d0: the active flag is a halfword at +8, the request fields at +0xc..+0x14,
//! and the setup block at +0x30.

use crate::camswitch;
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const LATCH: u32 = 0x0015_16d0;
const LATCH_FLAG: u32 = LATCH + 8;
const LATCH_SETUP: u32 = LATCH + 0x30;
const REG_V1: usize = 3;
const REG_A0: usize = 4;

/// Callee that fills the latch setup block; returns non-zero on success.
const SETUP_FN: u32 = 0x0012_ed58;
/// Cam argument spill, called with `SPILL_ARG` when setup fails.
const SPILL_ARG: u32 = 0x001d_ad98;
const RESET_FN: u32 = 0x001f_15a0;
/// Drain steps: a poll with argument 0, a sync, and three flush calls.
const POLL_FN: u32 = 0x0012_2298;
const SYNC_FN: u32 = 0x0025_7c28;
const FLUSH_A: u32 = 0x0012_eb00;
const FLUSH_B: u32 = 0x0012_dc80;
const FLUSH_C: u32 = 0x0012_eaf0;

/// Sets a general register to a sign-extended 32-bit value.
fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// Reads the latch flag as a sign-extended halfword.
fn latch_flag(c: &mut Cpu) -> u32 {
    c.r16(LATCH_FLAG) as i16 as i32 as u32
}

/// The three flush calls that end every drain step.
fn flush(c: &mut Cpu) {
    h::call_keep(c, FLUSH_A, &[], &[]);
    h::call_keep(c, FLUSH_B, &[], &[]);
    h::call_keep(c, FLUSH_C, &[], &[]);
}

/// One drain step: poll, sync, then flush.
fn drain_step(c: &mut Cpu) {
    h::call_keep(c, POLL_FN, &[0], &[]);
    h::call_keep(c, SYNC_FN, &[], &[]);
    flush(c);
}

/// Arms the latch when it is idle and `value` is non-zero. Returns `value << 11` on success, else 0.
fn request(c: &mut Cpu, owner: u32, mode: u32, value: u32) -> u32 {
    if latch_flag(c) != 0 || value == 0 {
        return 0;
    }
    let (ok, _) = h::call_keep(c, SETUP_FN, &[mode, value, owner, LATCH_SETUP], &[]);
    if ok == 0 {
        h::run_keep(c, camswitch::spill_cam_args, &[SPILL_ARG], &[]);
        h::call_keep(c, RESET_FN, &[], &[]);
        return 0;
    }
    c.w32(LATCH + 0x14, owner);
    c.w16(LATCH_FLAG, 1);
    c.w32(LATCH + 0x0c, mode);
    c.w32(LATCH + 0x10, value);
    value << 11
}

/// 0x258120 cam_latch_request(owner, mode, value) -> v0: arms the camera latch if it is idle and value != 0;
/// returns value << 11 when armed, else 0
pub fn cam_latch_request(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let (owner, mode, value) = (c.arg(0), c.arg(1), c.arg(2));
    let armed = request(c, owner, mode, value);
    c.ret(armed);
    true
}

/// 0x2581c0 cam_latch_pump(owner, mode, value) -> v0: cam_latch_request, then drains steps while the latch flag
/// stays set; returns the request result
pub fn cam_latch_pump(c: &mut Cpu) -> bool {
    let (owner, mode, value) = (c.arg(0), c.arg(1), c.arg(2));
    let armed = request(c, owner, mode, value);
    if armed != 0 {
        let flag = latch_flag(c);
        set_gpr(c, REG_V1, flag);
        if flag != 0 {
            loop {
                drain_step(c);
                if latch_flag(c) == 0 {
                    break;
                }
            }
        }
    }
    c.ret(armed);
    true
}

/// 0x258240 cam_latch_drain(owner) -> v0: with a non-zero owner, drains steps while the latch flag is set;
/// otherwise runs the sync and flush calls once; returns the latch flag
pub fn cam_latch_drain(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    if c.arg(0) == 0 {
        h::call_keep(c, SYNC_FN, &[], &[]);
        flush(c);
    } else {
        set_gpr(c, REG_A0, LATCH);
        if latch_flag(c) != 0 {
            loop {
                drain_step(c);
                if latch_flag(c) == 0 {
                    break;
                }
            }
        }
    }
    set_gpr(c, REG_V1, LATCH);
    let flag = latch_flag(c);
    c.ret(flag);
    true
}
