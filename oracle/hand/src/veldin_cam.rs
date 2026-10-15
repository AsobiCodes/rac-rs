//! Veldin camera basis: `veldin_basis` builds the camera axes in an object from the base vector scaled by a
//! per-mode factor; `veldin_reset` clears the object's 7-word block and rebuilds the axes; `cam_mode_tick` toggles
//! the camera mode word, arms the timers and runs the reset, then starts the moby cue when one is set.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Base of the camera globals (`lui 0x14` + `addiu -0xcb0`, below the overlay).
const CTX: u32 = 0x14_0000 - 0xcb0;
/// Word holding the source-vector pointer (the vector is read at +0xe0).
const CTX_SRC_PTR: u32 = 0x2080;
/// Mode byte: 2 and 1 pick a scale, anything else the default one.
const CTX_MODE: u32 = 0x20a4;
/// Constant vector (`lui 0x14` + `addiu -0xc30`).
const UP_VEC: u32 = 0x13_f3d0;
/// Word at `lui 0x16` + `0x6f04`: a vector pointer read at call time.
const LINK_PTR: u32 = 0x16_6f04;

/// Camera object of the mode tick (`lui 0x16` + `addiu 0x6d80`, in the overlay).
const CAM: u32 = 0x16_0000 + 0x6d80;
/// Mode halfword of the camera object.
const CAM_MODE: u32 = 0x270;
/// Byte cleared by the tick.
const CAM_FLAG: u32 = 0x273;
/// First float timer (0.05); the second one at +0x294.
const CAM_TIMER_A: u32 = 0x288;
const CAM_TIMER_B: u32 = 0x294;
/// Word tested for the moby cue (its moby pointer).
const CAM_CUE_OWNER: u32 = 0x254;

const ONE: F = F(0x3f80_0000);
const TIMER: F = F(0x3d4c_cccd);
const SCALE_MODE_2: F = F(0x4118_0000);
const SCALE_MODE_1: F = F(0x3f66_6666);
const SCALE_DEFAULT: F = F(0x3fcc_cccd);

/// Stack frame of `veldin_basis` (retail `addiu sp,-0x50`).
const BASIS_FRAME: u32 = 0x50;
/// Stack frame of `veldin_reset` and `cam_mode_tick` (retail `addiu sp,-0x20`).
const SMALL_FRAME: u32 = 0x20;

/// 0x2ed400 veldin_basis(obj): scales the source vector into the scratch at sp, builds the camera axes in obj
/// (obj+0x30 = scaled base + UP_VEC, obj+0x20 = scratch, obj+0x10 = normalised cross product, obj+0x40 = copy of obj).
pub fn veldin_basis(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    h::frame(c, BASIS_FRAME, &[(16, 0x20, h::SQ), (17, 0x30, h::SQ), (31, 0x40, h::SQ)], |c, sp| {
        let src = c.r32(CTX + CTX_SRC_PTR).wrapping_add(0xe0);
        h::run_keep(c, crate::vu_vec::vec_set_length, &[sp, src], &[ONE]);
        // Scale factor per mode; the scaled copy goes to sp+0x10.
        let half = sp + 0x10;
        let scale = match c.r8(CTX + CTX_MODE) {
            2 => SCALE_MODE_2,
            1 => SCALE_MODE_1,
            _ => SCALE_DEFAULT,
        };
        h::run_keep(c, crate::vecmath::vec_scale, &[half, sp], &[scale]);
        c.set_arg(2, half);
        h::call_keep(c, 0x1f1810, &[obj + 0x30, UP_VEC, half], &[]);
        // Copy the scratch quadword to obj+0x20, then build the cross product into obj+0x10.
        let q = c.mem.rd128(sp);
        c.r[2] = q;
        c.mem.wr128(obj + 0x20, q);
        let link = c.r32(LINK_PTR);
        h::run_keep(c, crate::vecmath::cross3, &[obj + 0x10, link, sp], &[]);
        h::run_keep(c, crate::vu_vec::vec_set_length, &[obj + 0x10, obj + 0x10], &[ONE]);
        h::run_keep(c, crate::vecmath::cross3, &[obj, sp, obj + 0x10], &[]);
        // Copy obj's first quadword to obj+0x40.
        let q = c.mem.rd128(obj);
        c.r[2] = q;
        c.mem.wr128(obj + 0x40, q);
        c.s(3, (obj + 0x40) as u64);
        c.set_arg(0, obj);
        c.set_arg(1, sp);
        c.set_arg(2, obj + 0x10);
    });
    true
}

/// 0x2ed520 veldin_reset(obj): zeroes the seven words of the block at obj+0x70, rebuilds the basis of obj and
/// clears the halfword at obj+0x7e.
pub fn veldin_reset(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    h::frame(c, SMALL_FRAME, &[(16, 0x00, h::SQ), (31, 0x10, h::SQ)], |c, _sp| {
        let block = c.r32(obj + 0x70);
        for off in [0x18u32, 0x00, 0x08, 0x04, 0x0c, 0x10, 0x14] {
            c.w32(block.wrapping_add(off), 0);
        }
        h::run_keep(c, veldin_basis, &[obj], &[]);
        c.w16(obj + 0x7e, 0);
    });
    true
}

/// 0x2eddd0 cam_mode_tick(a0): flips the camera mode halfword (1 or 2), clears the flag byte, stores the 0.05 timers,
/// runs `veldin_reset` on a0 and starts the moby cue (0x27e470) when the owner word at CAM+0x254 is set.
pub fn cam_mode_tick(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    h::frame(c, SMALL_FRAME, &[(16, 0x00, h::SQ), (31, 0x10, h::SQ)], |c, _sp| {
        let mode = c.r16(CAM + CAM_MODE) as i16;
        c.w16(CAM + CAM_MODE, if mode != 0 { 2 } else { 1 });
        c.w8(CAM + CAM_FLAG, 0);
        c.f[0] = TIMER.0;
        c.wf(CAM + CAM_TIMER_A, TIMER);
        c.wf(CAM + CAM_TIMER_B, TIMER);
        h::run_keep(c, veldin_reset, &[obj], &[]);
        let owner = c.r32(CAM + CAM_CUE_OWNER);
        if owner != 0 {
            h::run_keep(c, crate::voice_slot::moby_cue, &[0, 0, owner], &[]);
        } else {
            c.set_arg(0, 0);
            c.set_arg(2, 0);
        }
    });
    true
}
