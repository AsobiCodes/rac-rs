//! Camera rig ownership: `rig_claim_owner` takes the rig for an owner by priority, `rig_set_pair` stores the two
//! rig values and raises the rig's flag, and `rig_offset_vectors` builds the rig's offset vectors.
use crate::h;
use crate::ps2::*;
use crate::vu_vec;
use ee_oracle::cpu::Cpu;

/// Global pointer to the camera object (lui 0x16, lw 0x6f00).
const CAMERA_PTR: u32 = 0x0016_6f00;
/// Global pointer to the claim priority table (lui 0x16, lw -0x10b0); records are 32 bytes apart.
const PRIO_TABLE_PTR: u32 = 0x0015_ef50;
/// Retail out-of-line vector helpers (args: tmp, b, a) and (args: out, a, tmp).
const VEC_DIFF: u32 = 0x1f1840;
const VEC_ADD: u32 = 0x1f1810;

/// Sets GPR `r` to a sign-extended 32-bit value, as the MIPS 32-bit ops leave it.
fn set(c: &mut Cpu, r: usize, v: u32) {
    c.s(r, v as i32 as i64 as u64);
}

/// 0x2eb630 rig_claim_owner(owner) -> -1 when the camera is locked (+0x86); else 1 when the rig (+0x220) was free
/// or already held by `owner`, or when the holder's priority record has a lower byte than the owner's (the rig is
/// then taken: +0x220 = owner); else 0.
pub fn rig_claim_owner(c: &mut Cpu) -> bool {
    let owner = c.arg(0);
    let cam = c.r32(CAMERA_PTR);
    set(c, 6, owner);
    set(c, 3, cam);
    if c.r16(cam + 0x86) as i16 != 0 {
        c.ret(-1i32 as u32);
        return true;
    }
    let rig = c.r32(cam + 0x70);
    set(c, 7, rig);
    let holder = c.r32(rig + 0x220);
    if holder == 0 {
        c.w32(rig + 0x220, owner);
        c.ret(1);
        return true;
    }
    if holder == owner {
        c.ret(1);
        return true;
    }
    let table = c.r32(PRIO_TABLE_PTR);
    let holder_idx = (c.r16(holder + 0x84) as i16 as i32 as u32) << 5;
    let owner_idx = (c.r16(owner + 0x84) as i16 as i32 as u32) << 5;
    let holder_rec = c.r32(table.wrapping_add(holder_idx) + 0x1c);
    let owner_rec = c.r32(table.wrapping_add(owner_idx) + 0x1c);
    let holder_byte = c.r8(holder_rec + 0x1c) as u32;
    let owner_byte = c.r8(owner_rec + 0x1c) as u32;
    let takes = holder_byte < owner_byte;
    if takes {
        c.w32(rig + 0x220, owner);
    }
    set(c, 4, holder_rec);
    set(c, 5, owner_rec);
    set(c, 3, owner_byte);
    c.ret(takes as u32);
    true
}

/// 0x2eb728 rig_set_pair(a, b) (f12 = a, f13 = b) -> the camera's lock value when locked (+0x86); else sets the rig's
/// pair at rig+0x130: +0x4c = a, +0x54 = b, +0x3e = 1, and returns that address.
pub fn rig_set_pair(c: &mut Cpu) -> bool {
    let cam = c.r32(CAMERA_PTR);
    set(c, 3, cam);
    let lock = c.r16(cam + 0x86) as i16 as i32;
    if lock != 0 {
        c.ret(lock as u32);
        return true;
    }
    let o = c.r32(cam + 0x70).wrapping_add(0x130);
    c.w32(o + 0x54, c.farg(1).0);
    c.w16(o + 0x3e, 1);
    c.w32(o + 0x4c, c.farg(0).0);
    set(c, 3, 1);
    c.ret(o);
    true
}

/// 0x2ea7a0 rig_offset_vectors(p, a1, a2, a3, t0, f12, f13): with the rig at p+0x70, builds the offset vectors from the
/// inputs and the two lengths (vec_set_length), chaining the retail VU helpers as retail does.
pub fn rig_offset_vectors(c: &mut Cpu) -> bool {
    let (p, a1, a2, a3, t0) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3), c.arg(4));
    let (f20, f21) = (c.farg(0), c.farg(1));
    let s2 = c.r32(p + 0x70).wrapping_add(0x90);
    let saves = [
        (31, 0x90, h::SQ),
        (20, 0x80, h::SQ),
        (19, 0x70, h::SQ),
        (18, 0x60, h::SQ),
        (17, 0x50, h::SQ),
        (16, 0x40, h::SQ),
    ];
    h::frame(c, 0xb0, &saves, |c, sp| {
        c.w32(sp + 0xa0, f20.0);
        c.w32(sp + 0xa8, f21.0);
        h::call_keep(c, VEC_DIFF, &[sp, a2, a3], &[]);
        h::run_keep(c, vu_vec::vec_set_length, &[sp + 0x10, sp], &[f20]);
        h::call_keep(c, VEC_ADD, &[a2, a2, sp + 0x10], &[]);
        h::call_keep(c, VEC_DIFF, &[sp + 0x20, a2, s2], &[]);
        h::run_keep(c, vu_vec::vec_set_length, &[sp + 0x20, sp + 0x20], &[f21]);
        h::call_keep(c, VEC_ADD, &[sp + 0x30, s2, sp + 0x20], &[]);
        h::call_keep(c, VEC_DIFF, &[a1, sp + 0x30, t0], &[]);
    });
    true
}
