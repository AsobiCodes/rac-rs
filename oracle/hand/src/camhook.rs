//! Camera-switch per-object hook dispatch (retail 0x1ddd30..0x1ddd74).

use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const A0: usize = 4;
const A1: usize = 5;
const V0: usize = 2;
const V1: usize = 3;
const RA: usize = 31;

/// Hook table of 0x14-byte records; each record's function pointer is at +8.
const HOOK_TABLE: u32 = 0x001d_d080;
const HOOK_STRIDE: u32 = 0x14;
const HOOK_FN_OFF: u32 = 8;
/// Halfword index inside the camera object.
const OBJ_HOOK_INDEX: u32 = 0x8c;

fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// 0x1ddd30 dispatch_cam_hook(obj) -> v0: calls the hook of the record indexed by obj's halfword at +0x8c (records
/// of 0x14 bytes at HOOK_TABLE, fn pointer at +8) with obj as the argument; returns the hook's result, or 0 when the
/// record has no hook. Exit: v1 = the record offset, a0 and a1 = obj, HI/LO = the offset product when there is no
/// hook; with a hook the callee's scratch registers stay (h::call_keep).
pub fn dispatch_cam_hook(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    h::frame(c, 0x10, &[(RA, 0x00, h::SQ)], |c, _sp| {
        let obj = c.arg(0);
        set_gpr(c, A1, obj);
        set_gpr(c, A0, HOOK_STRIDE);
        let index = c.r16(obj + OBJ_HOOK_INDEX) as i16 as i64;
        let product = index * HOOK_STRIDE as i64;
        let offset = product as i32 as u32;
        c.lo = c.lo & !(u64::MAX as u128) | (product as i32 as i64 as u64) as u128;
        c.hi = c.hi & !(u64::MAX as u128) | ((product >> 32) as i32 as i64 as u64) as u128;
        set_gpr(c, V1, offset);
        let hook = c.r32(HOOK_TABLE.wrapping_add(offset) + HOOK_FN_OFF);
        set_gpr(c, V0, hook);
        if hook != 0 {
            h::call_keep(c, hook, &[obj], &[]);
        }
    });
    true
}
