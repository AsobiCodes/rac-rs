//! Four small camera-rig setters from the level 4 overlay. Each one takes the rig pointer at 0x166f00, and does nothing
//! while the rig's busy halfword (+0x86) is set. Otherwise it writes into the camera sub-block at rig+0x70.
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// MIPS register numbers used below.
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;

/// Global pointer to the camera rig (lui 0x16 / lw 0x6f00).
const RIG_PTR: u32 = 0x0016_6f00;
/// Rig offset of the camera sub-block pointer.
const RIG_SUB: u32 = 0x70;
/// Rig halfword that blocks the setters while non-zero.
const RIG_BUSY: u32 = 0x86;
/// Sub-block offset of the look-target block (a1 = sub + 0x40).
const LOOK_BLOCK: u32 = 0x40;
/// Global pointer to the look-target source block (lui 0x17 / lw -0x6f40), read at call time.
const LOOK_SOURCE_PTR: u32 = 0x0016_90c0;

/// Sets general register `reg` to a sign-extended 32-bit value, as the 32-bit MIPS ops do.
fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// The rig's busy halfword, sign-extended as `lh` loads it (only non-zero matters for the branch).
fn busy_word(c: &mut Cpu, rig: u32) -> u32 {
    c.r16(rig + RIG_BUSY) as i16 as i32 as u32
}

/// 0x2eb760 cam_set_look_target(a0 flag, f12 x, f13 k) -> (): stores the look-target spring k (f13) and, when the
/// flag is set, the look-target value (f12 + the source block's +0xf0 float) into the camera block.
pub fn cam_set_look_target(c: &mut Cpu) -> bool {
    let rig = c.r32(RIG_PTR);
    let busy = busy_word(c, rig);
    if busy != 0 {
        set_gpr(c, V0, busy);
        set_gpr(c, V1, rig);
        return true;
    }
    let sub = c.r32(rig + RIG_SUB);
    let block = sub + LOOK_BLOCK;
    set_gpr(c, V1, 1);
    c.w16(block + 0xda, 1);
    c.wf(block + 0xb8, c.farg(1));
    set_gpr(c, A1, block);
    if c.g(A0) == 0 {
        c.wf(block + 0xb4, c.farg(0));
        set_gpr(c, V0, sub);
        return true;
    }
    let source = c.r32(LOOK_SOURCE_PTR);
    set_gpr(c, V0, 0x0017_0000);
    set_gpr(c, V1, source);
    let value = c.rf(source + 0xf0) + c.farg(0);
    c.fret(value);
    c.wf(block + 0xb4, value);
    true
}

/// 0x2eb7d8 cam_set_auto_follow(a0 h) -> (): stores the halfword a0 at the camera sub-block's +0x10 (the one-shot
/// auto-follow request).
pub fn cam_set_auto_follow(c: &mut Cpu) -> bool {
    let rig = c.r32(RIG_PTR);
    let busy = busy_word(c, rig);
    if busy != 0 {
        set_gpr(c, V0, busy);
        set_gpr(c, V1, rig);
        return true;
    }
    let sub = c.r32(rig + RIG_SUB);
    c.w16(sub + 0x10, c.arg(0) as u16);
    set_gpr(c, V0, sub);
    set_gpr(c, V1, rig);
    true
}

/// 0x2eb800 cam_set_target_xy_spring(f12 x, f13 y) -> (): stores the non-zero target spring values f12 and f13 into
/// the camera block at +0xdc and +0xe0; a zero value is skipped.
pub fn cam_set_target_xy_spring(c: &mut Cpu) -> bool {
    let rig = c.r32(RIG_PTR);
    let busy = busy_word(c, rig);
    if busy != 0 {
        set_gpr(c, V0, busy);
        set_gpr(c, V1, rig);
        return true;
    }
    c.f[0] = 0;
    let sub = c.r32(rig + RIG_SUB);
    let block = sub + LOOK_BLOCK;
    let zero = F::new(0.0);
    if c.farg(0) != zero {
        c.wf(block + 0xdc, c.farg(0));
    }
    if c.farg(1) != zero {
        c.wf(block + 0xe0, c.farg(1));
    }
    set_gpr(c, V0, block);
    set_gpr(c, V1, rig);
    true
}

/// 0x2eb890 cam_no_retreat() -> (): sets the camera block's +0x226 halfword to 1, which skips the retreat latch.
pub fn cam_no_retreat(c: &mut Cpu) -> bool {
    let rig = c.r32(RIG_PTR);
    let busy = busy_word(c, rig);
    if busy != 0 {
        set_gpr(c, V1, busy);
        set_gpr(c, V0, rig);
        return true;
    }
    let sub = c.r32(rig + RIG_SUB);
    c.w16(sub + 0x226, 1);
    set_gpr(c, V0, 1);
    set_gpr(c, V1, sub);
    true
}
