//! Camera-switch helpers (retail 0x1dd878..0x1ddd74): argument spill, cam table copy, slot search, flag clear and
//! the per-object camera hook dispatch.

use crate::h;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const SP: usize = 29;
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const S0: usize = 16;
const S1: usize = 17;
const RA: usize = 31;

/// First cam slot and the end of the slot table (stride 0xa0, 48 slots).
const CAM_SLOT_FIRST: u32 = 0x0016_7250;
const CAM_SLOT_END: u32 = 0x0016_9050;
const CAM_SLOT_STRIDE: u32 = 0xa0;
/// Halfword key inside a cam slot.
const CAM_SLOT_KEY: u32 = 0x86;

/// Cam table A (also the value of s1 and a1 at exit) and table B.
const CAM_TABLE_A: u32 = 0x0016_9050;
const CAM_TABLE_B: u32 = 0x0016_9710;
/// Pointer to the copy source, stored at 0x166f00.
const CAM_SOURCE_PTR: u32 = 0x0016_6f00;
/// Camera state block; its pointer at +0x180 names the current cam object.
const CAM_STATE: u32 = 0x0016_6d80;
const CAM_STATE_CUR: u32 = 0x180;

/// Hook table of 0x14-byte records; each record's function pointer is at +8.
const HOOK_TABLE: u32 = 0x001d_d080;
const HOOK_STRIDE: u32 = 0x14;
const HOOK_FN_OFF: u32 = 8;
/// Halfword index inside the camera object.
const OBJ_HOOK_INDEX: u32 = 0x8c;

fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// 0x1dd878 spill_cam_args(a1..t3, f12, f14, f16, f18) -> (): stores the argument registers into the caller's
/// 0x80-byte frame (spill slots 0x38..0x78) for the callee to read back.
pub fn spill_cam_args(c: &mut Cpu) -> bool {
    let sp = (c.g(SP) as u32).wrapping_sub(0x80);
    for (i, reg) in (A1..=11).enumerate() {
        let v = c.g(reg);
        c.mem.wr64(sp + 0x48 + 8 * i as u32, v);
    }
    c.w32(sp + 0x38, c.f[12]);
    c.w32(sp + 0x3c, c.f[14]);
    c.w32(sp + 0x40, c.f[16]);
    c.w32(sp + 0x44, c.f[18]);
    true
}

/// 0x1dd928 copy_cam_tables() -> (): copies 0xa0 qwords from the source pointer at 0x166f00 into cam table A, then
/// 0x280 qwords into table B (from B - 0x500), and stores B at A + 0x70.
pub fn copy_cam_tables(c: &mut Cpu) -> bool {
    let saves = [(S0, 0x00, h::SQ), (S1, 0x10, h::SQ), (RA, 0x20, h::SQ)];
    h::frame(c, 0x30, &saves, |c, _sp| {
        set_gpr(c, S1, CAM_TABLE_A);
        set_gpr(c, S0, CAM_TABLE_B);
        let src = c.r32(CAM_SOURCE_PTR);
        h::run(c, mathx::copy_qwords, &[CAM_TABLE_A, src, 0xa0], &[]);
        h::run(c, mathx::copy_qwords, &[CAM_TABLE_B, CAM_TABLE_B - 0x500, 0x280], &[]);
        c.w32(CAM_TABLE_A + 0x70, CAM_TABLE_B);
    });
    true
}

/// 0x1dda60 find_cam_slot(key) -> slot or 0: scans the 48 cam slots for one whose halfword at +0x86 equals the key.
/// Exit: v1 is the slot reached (the slot found, or the table end), a1 = the table end.
pub fn find_cam_slot(c: &mut Cpu) -> bool {
    let key = c.g(A0);
    let mut p = CAM_SLOT_FIRST;
    let found = loop {
        if c.r16(p + CAM_SLOT_KEY) as i16 as i64 as u64 == key {
            break Some(p);
        }
        p += CAM_SLOT_STRIDE;
        if p >= CAM_SLOT_END {
            break None;
        }
    };
    set_gpr(c, A1, CAM_SLOT_END);
    set_gpr(c, V1, p);
    c.ret(found.unwrap_or(0));
    true
}

/// 0x1ddcb0 clear_cur_cam_flags() -> (): sets the halfword at +0x7e of the current cam object to 1 and clears the
/// byte at +0x7d.
pub fn clear_cur_cam_flags(c: &mut Cpu) -> bool {
    let cur = c.r32(CAM_STATE + CAM_STATE_CUR);
    set_gpr(c, V0, CAM_STATE);
    set_gpr(c, V1, cur);
    set_gpr(c, A0, cur);
    set_gpr(c, A1, 1);
    c.w16(cur + 0x7e, 1);
    c.w8(cur + 0x7d, 0);
    true
}

/// 0x1ddd30 dispatch_cam_hook(obj) -> v0: calls the hook of the record indexed by obj's halfword at +0x8c (records
/// of 0x14 bytes at HOOK_TABLE, fn pointer at +8) with obj as the argument; returns the hook's result, or 0 when the
/// record has no hook. A record with a hook declines (retail runs): the callee leaves a0, a1 and v1 with values this
/// Rust cannot reproduce, and h::call would restore them instead.
pub fn dispatch_cam_hook(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let index = c.r16(obj + OBJ_HOOK_INDEX) as i16 as i32;
    let offset = index.wrapping_mul(HOOK_STRIDE as i32) as u32;
    let entry = HOOK_TABLE.wrapping_add(offset);
    let hook = c.r32(entry + HOOK_FN_OFF);
    if hook != 0 {
        return false;
    }
    set_gpr(c, V1, offset);
    set_gpr(c, A0, HOOK_STRIDE);
    set_gpr(c, A1, obj);
    c.ret(0);
    true
}
