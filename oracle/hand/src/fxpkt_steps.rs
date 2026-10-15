//! FX packet steps: `fx_pkt_submit` and `fx_pkt_release` look up a flag in a table selected by the packet state, then
//! run the packet callbacks on the object's two slots. `angle_nudge` advances an angle field by a fixed step.
use crate::fxtex;
use crate::h;
use crate::ps2::*;
use crate::slot_dispatch;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Global word (at retail 0x1c0000 - 0x620c) that points at the packet context.
const CTX_PTR: u32 = 0x001B_9DF4;
/// Base of the per-state flag table (retail `addiu` of 0x140000 - 0x2b40).
const FLAG_TABLE: u32 = 0x0013_D4C0;
/// Retail callback that runs one packet slot: `(slot, 1)`.
const PKT_CALLBACK: u32 = 0x0024_27D0;
/// Retail setup of the packet header from the object.
const PKT_SETUP: u32 = 0x0027_7208;
/// Retail tag builder; its result is the upload's third argument.
const PKT_TAG_BUILD: u32 = 0x001F_6B80;
/// Tag builder argument (texture id).
const TEX_ID: u32 = 0x4f4d;
/// Upload mode word passed as the second argument (a 64-bit constant in retail).
const UPLOAD_MODE: u32 = 0x80ff_a888;
/// Header fields written on the stack before the tag builder: (0x10, 3) as two halfwords.
const HDR_A: u16 = 0x10;
const HDR_B: u16 = 3;
/// Angle step added per call: 0.01 as a single.
const ANGLE_STEP: F = F(0x3c23_d70a);

/// Sets a 32-bit GPR the way a MIPS 32-bit op does (sign-extended to 64 bits).
fn set_gpr(c: &mut Cpu, n: usize, v: u32) {
    c.s(n, v as i32 as i64 as u64);
}

/// The flag byte the packet state's table selects: index = state.+0x3c * 10 + state.+0x48, then the signed halfword at
/// index + 6 offsets the table base. Leaves the lookup's scratch registers as retail does.
fn table_flag(c: &mut Cpu) -> u32 {
    let ctx = c.r32(CTX_PTR);
    let state = c.r32(ctx + 0x40);
    let base = c.r32(state + 0x48);
    let index = c.r32(state + 0x3c).wrapping_mul(10).wrapping_add(base);
    let off = c.r16(index + 6) as i16 as i32 as u32;
    let addr = off.wrapping_add(FLAG_TABLE);
    let flag = c.r8(addr) as u32;
    set_gpr(c, 2, addr);
    set_gpr(c, 3, flag);
    set_gpr(c, 4, base);
    set_gpr(c, 5, FLAG_TABLE);
    set_gpr(c, 6, ctx);
    set_gpr(c, 7, 10);
    flag
}

/// Runs the packet callbacks on the object's two slots: the first if set, then the second if set. Retail leaves the
/// second slot's value in a0 either way.
fn run_slots(c: &mut Cpu, obj: u32) {
    let first = c.r32(obj + 0x44);
    if first != 0 {
        h::call_keep(c, PKT_CALLBACK, &[first, 1], &[]);
    }
    let second = c.r32(obj + 0x48);
    set_gpr(c, 4, second);
    if second != 0 {
        h::call_keep(c, PKT_CALLBACK, &[second, 1], &[]);
    }
}

/// 0x26e998 fx_pkt_submit(obj) -> v0: when the table flag is set, runs obj's slot callbacks and returns 8; otherwise
/// resets the FX frame, builds and uploads the packet tags, and returns 2 (0 when the flag is clear).
pub fn fx_pkt_submit(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let obj = c.arg(0);
    let v0 = h::frame(c, 0x40, &[(16, 0x20, h::SQ), (31, 0x30, h::SQ)], |c, sp| submit_body(c, sp, obj));
    c.ret(v0);
    true
}

fn submit_body(c: &mut Cpu, sp: u32, obj: u32) -> u32 {
    if table_flag(c) == 0 {
        return 0;
    }
    if c.r32(obj + 0x44) != 0 {
        run_slots(c, obj);
        return 8;
    }
    h::run(c, fxtex::fx_frame_reset, &[0], &[]);
    h::call(c, PKT_SETUP, &[sp, obj], &[]);
    c.w16(sp + 0x10, HDR_A);
    c.w16(sp + 0x12, HDR_B);
    let tag = h::call(c, PKT_TAG_BUILD, &[TEX_ID], &[]).0;
    h::run_keep(c, slot_dispatch::upload_then_fill_1, &[sp, UPLOAD_MODE, tag, u32::MAX], &[]);
    let a0 = c.arg(0);
    h::run_keep(c, fxtex::fx_pkt_tags, &[a0], &[]);
    2
}

/// 0x26ee90 fx_pkt_release(obj) -> v0: when the table flag is set, runs obj's slot callbacks (the second only if
/// set) and returns 8; returns 0 when the flag is clear.
pub fn fx_pkt_release(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let obj = c.arg(0);
    let v0 = h::frame(c, 0x20, &[(16, 0x0, h::SQ), (31, 0x10, h::SQ)], |c, _sp| {
        if table_flag(c) == 0 {
            return 0;
        }
        run_slots(c, obj);
        8
    });
    c.ret(v0);
    true
}

/// 0x26ea80 angle_nudge(obj): obj's angle at +0x48 becomes angle_add(angle, 0.01); returns it in f0.
pub fn angle_nudge(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    h::frame(c, 0x20, &[(16, 0x0, h::SQ), (31, 0x10, h::SQ)], |c, _sp| {
        let angle = c.rf(obj + 0x48);
        let (_, next) = h::run_keep(c, vecmath::angle_add, &[], &[angle, ANGLE_STEP]);
        c.wf(obj + 0x48, next);
        c.f[12] = angle.0;
        c.f[13] = ANGLE_STEP.0;
        c.fret(next);
    });
    true
}
