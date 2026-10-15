//! Copies a quadword from the saved s0 pointer into the active camera's slot block.

use crate::camswitch;
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const SP: usize = 29;
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const S0: usize = 16;
const RA: usize = 31;

/// Global pointer to the active camera object.
const CAM_PTR: u32 = 0x0016_6f00;
/// Halfword key at +0x86 of the camera object; 5 means the object is its own slot.
const CAM_KEY_OFF: u32 = 0x86;
const CAM_KEY_SELF: u32 = 5;
/// Pointer to the slot block at +0x70 of the resolved camera.
const CAM_SLOT_OFF: u32 = 0x70;

/// Offsets of the destination from the slot block, per entry.
const DST_OFF_008: u32 = 0x80;
const DST_OFF_060: u32 = 0x90;
const DST_OFF_0C0: u32 = 0xe0;

fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// The retail prologue's camera lookup: a0 = the camera pointer, v0 = the camera itself when its key is 5, else the
/// result of find_cam_slot(5). Returns v0 (the pointer whose +0x70 field is the slot block).
fn resolve_slot(c: &mut Cpu) -> u32 {
    let cam = c.r32(CAM_PTR);
    set_gpr(c, A0, cam);
    set_gpr(c, V1, CAM_KEY_SELF);
    let key = c.r16(cam.wrapping_add(CAM_KEY_OFF)) as i16 as i64;
    if key == CAM_KEY_SELF as i64 {
        set_gpr(c, V0, cam);
        cam
    } else {
        set_gpr(c, A0, CAM_KEY_SELF);
        h::run_keep(c, camswitch::find_cam_slot, &[CAM_KEY_SELF], &[]);
        c.g(V0) as u32
    }
}

/// Stores the quadword at [s0] to slot block + `off` (lq v0, sq v0), leaving v1 as the destination.
fn copy_quad_to_slot(c: &mut Cpu, off: u32) {
    let slot = resolve_slot(c);
    let block = c.r32(slot.wrapping_add(CAM_SLOT_OFF));
    let dst = block.wrapping_add(off);
    set_gpr(c, V1, dst);
    let src = c.g(S0) as u32;
    c.r[V0] = c.mem.rd128(src & !15);
    c.mem.wr128(dst & !15, c.r[V0]);
}

/// Shared body of the two quadword copies: frame of 0x20 with s0 at 0 and ra at 0x10.
fn copy_quad_frame(c: &mut Cpu, off: u32) {
    let sp = (c.g(SP) as u32).wrapping_sub(0x20);
    set_gpr(c, SP, sp);
    c.mem.wr128(sp & !15, c.r[S0]);
    c.mem.wr128((sp + 0x10) & !15, c.r[RA]);
    c.r[S0] = c.r[A0];
    copy_quad_to_slot(c, off);
    c.r[RA] = c.mem.rd128((sp + 0x10) & !15);
    c.r[S0] = c.mem.rd128(sp & !15);
    set_gpr(c, SP, sp.wrapping_add(0x20));
}

/// 0x2ee008 cam_store_slot_80() -> (): copies the quadword at s0 to the active camera's slot block + 0x80.
pub fn cam_store_slot_80(c: &mut Cpu) -> bool {
    copy_quad_frame(c, DST_OFF_008);
    true
}

/// 0x2ee060 cam_store_slot_90() -> (): copies the quadword at s0 to the active camera's slot block + 0x90.
pub fn cam_store_slot_90(c: &mut Cpu) -> bool {
    copy_quad_frame(c, DST_OFF_060);
    true
}

/// 0x2ee0c0 cam_store_slot_float_e0(f12, f13) -> (): stores f12 at slot block + 0xe0 + 0x44 and f13 at + 0xe0 + 0x48,
/// saving and restoring the caller's f20/f21 around it.
pub fn cam_store_slot_float_e0(c: &mut Cpu) -> bool {
    let sp = (c.g(SP) as u32).wrapping_sub(0x20);
    set_gpr(c, SP, sp);
    let (old20, old21) = (F(c.f[20]), F(c.f[21]));
    let (x, y) = (c.farg(0), c.farg(1));
    c.wf(sp + 0x18, old21);
    c.wf(sp + 0x10, old20);
    c.mem.wr128(sp & !15, c.r[RA]);
    let slot = resolve_slot(c);
    let block = c.r32(slot.wrapping_add(CAM_SLOT_OFF));
    set_gpr(c, V0, block);
    c.r[RA] = c.mem.rd128(sp & !15);
    let dst = block.wrapping_add(DST_OFF_0C0);
    set_gpr(c, V0, dst);
    c.wf(dst + 0x48, y);
    c.wf(dst + 0x44, x);
    set_gpr(c, SP, sp.wrapping_add(0x20));
    true
}
