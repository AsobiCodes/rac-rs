//! Camera slot fill and the camera refresh step (retail 0x2ee130, 0x2ee8a8).
//! The fill copies quadword blocks into a camera slot and the global camera block at 0x166ec0, and for modes 2 and 3
//! sets the slot's eye data (mode 3 also runs the eye-to-polar angles). The refresh runs 0x2ee4b0 on the object and
//! then, when the camera block's flag at +0x40 is set, clears obj+0x89 and runs the ease step on obj+0x30.

use crate::cam_polar_hero;
use crate::camswitch;
use crate::h::{self, SQ};
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Stack frame of the fill (retail `addiu sp,sp,-0xf0`).
const FILL_FRAME: u32 = 0xf0;
/// Saved registers of the fill, as retail stores them: s0-s7, fp, ra.
const FILL_SAVES: [(usize, u32, u8); 10] = [
    (16, 0x50, SQ),
    (17, 0x60, SQ),
    (18, 0x70, SQ),
    (19, 0x80, SQ),
    (20, 0x90, SQ),
    (21, 0xa0, SQ),
    (22, 0xb0, SQ),
    (23, 0xc0, SQ),
    (30, 0xd0, SQ),
    (31, 0xe0, SQ),
];
/// Stack frame of the refresh (retail `addiu sp,sp,-0x30`), saving s0, s1 and ra.
const REFRESH_FRAME: u32 = 0x30;
const REFRESH_SAVES: [(usize, u32, u8); 3] = [(16, 0x0, SQ), (17, 0x10, SQ), (31, 0x20, SQ)];

/// Key passed to find_cam_slot (retail `addiu a0, zero, 5`).
const CAM_KEY: u32 = 5;
/// Global camera block (0x166ec0): read and written by the fill as two quadwords.
const CAM_TABLE: u32 = 0x0016_6ec0;
/// Camera config block for the eye-to-polar call (0x140000 - 0xc30).
const CAM_CONFIG: u32 = 0x0013_f3d0;

/// Retail helpers: the camera slot setup (takes the slot), the refresh step (takes the object) and the ease step.
const SLOT_SETUP: u32 = 0x001d_dd78;
const REFRESH_STEP: u32 = 0x002e_e4b0;
const EASE_STEP: u32 = 0x001d_ff10;

/// Float bit patterns: 1.0 and 0.5.
const ONE: u32 = 0x3f80_0000;
const HALF: u32 = 0x3f00_0000;

/// Sets a general register to a sign-extended 32-bit value.
fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// Copies one 16-byte quadword from `src` to `dst`.
fn copy_quad(c: &mut Cpu, dst: u32, src: u32) {
    let v = c.rv(src);
    c.wv(dst, v);
}

/// 0x2ee130 cam_slot_fill(src, cfg, mode, a3, t0): finds the camera slot for key 5, copies the source quadwords and
/// the global camera block into it, stores mode and t0, and for mode 2 or 3 copies a3 into the eye fields (mode 3 also
/// sets the block flags, copies the eye block and runs cam_eye_to_polar). Other modes return 3 in v0.
pub fn cam_slot_fill(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let (src, cfg, mode, a3, t0) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3), c.arg(4));
    h::frame(c, FILL_FRAME, &FILL_SAVES, |c, sp| {
        c.w32(sp + 0x40, a3);
        let slot = h::run_keep(c, camswitch::find_cam_slot, &[CAM_KEY], &[]).0;
        copy_quad(c, sp, src);
        copy_quad(c, sp + 0x10, cfg);
        copy_quad(c, sp + 0x20, CAM_TABLE);
        copy_quad(c, sp + 0x30, CAM_TABLE + 0x10);
        h::call_keep(c, SLOT_SETUP, &[slot], &[]);

        let block = c.r32(slot + 0x70);
        c.w8(slot + 0x7c, 7);
        c.w8(slot + 0x7d, 1);
        copy_quad(c, slot + 0x30, sp);
        copy_quad(c, CAM_TABLE, sp);
        copy_quad(c, slot + 0x40, sp + 0x10);
        copy_quad(c, block + 0x80, sp);
        copy_quad(c, block + 0x90, sp + 0x10);
        c.w32(block + 0x120, t0);
        c.w8(slot + 0x88, mode as u8);
        set_gpr(c, 3, block + 0x90);
        set_gpr(c, 4, block + 0x80);
        set_gpr(c, 5, slot + 0x40);
        set_gpr(c, 6, slot + 0x30);

        if mode == 3 {
            c.f[0] = ONE;
            c.w32(block + 0x128, 0);
            c.w32(block + 0x124, ONE);
            copy_quad(c, slot + 0x30, sp + 0x20);
            copy_quad(c, slot + 0x40, sp + 0x30);
            copy_quad(c, block + 0x110, CAM_CONFIG);
            set_gpr(c, 3, block + 0x110);
            h::run_keep(c, cam_polar_hero::cam_eye_to_polar, &[block + 0xfc, sp + 0x20, CAM_CONFIG], &[]);
        }
        if mode == 2 || mode == 3 {
            c.w32(block + 0xe4, a3);
            c.w32(block + 0xe0, a3);
            set_gpr(c, 2, a3);
        } else {
            set_gpr(c, 2, 3);
        }
    });
    true
}

/// 0x2ee8a8 cam_slot_refresh(obj): runs 0x2ee4b0 on obj; when the camera block at obj+0x70+0xe0 has a non-zero flag at
/// +0x40, clears obj+0x89 and runs 0x1dff10(obj+0x30, 0.5). Returns nothing (v0 is the flag, or the ease step's).
pub fn cam_slot_refresh(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let obj = c.arg(0);
    h::frame(c, REFRESH_FRAME, &REFRESH_SAVES, |c, _sp| {
        let block = c.r32(obj + 0x70).wrapping_add(0xe0);
        h::call_keep(c, REFRESH_STEP, &[obj], &[]);
        if c.r32(block + 0x40) != 0 {
            c.w8(obj + 0x89, 0);
            h::call_keep(c, EASE_STEP, &[obj + 0x30], &[F(HALF)]);
        } else {
            set_gpr(c, 2, 0);
        }
    });
    true
}
