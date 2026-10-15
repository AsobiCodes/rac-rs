//! Camera look and orientation helpers (retail 0x2eb8b8..0x2ed264): the look-blend flag, the look float applied to the
//! rig's target object, the camera orientation refresh from its rig, and a small wrapper around the camera update.

use crate::camobj;
use crate::h;
use crate::mathx;
use crate::orient_step;
use crate::ps2::*;
use crate::vecmath;
use crate::vu_vec;
use ee_oracle::cpu::Cpu;

const AT: usize = 1;
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;
const S0: usize = 16;
const S1: usize = 17;
const S2: usize = 18;
const S3: usize = 19;
const SP: usize = 29;
const RA: usize = 31;

/// Pointer to the camera rig (overlay data, read at call time).
const RIG_PTR: u32 = 0x0016_6f00;
/// Constant basis vector loaded by the refresh (read at call time).
const CAM_BASIS: u32 = 0x0013_f3d0;
/// Constant up vector used as the second operand of the first cross product (overlay data, read at call time).
const CAM_UP: u32 = 0x0016_6f30;
/// Rig field: pointer to the target object.
const RIG_TARGET: u32 = 0x70;
/// Rig field: mode halfword (zero selects the look path).
const RIG_MODE: u32 = 0x86;
/// Target field: look-blend flag byte.
const TARGET_FLAG: u32 = 0x116;
/// Target field: look float.
const TARGET_LOOK: u32 = 0x1bc;
/// Retail look apply (0x2eb920), called with (rig, this) and the y float.
const LOOK_APPLY: u32 = 0x002e_b920;
/// Retail camera update (0x2ecf30), called by the wrapper.
const CAM_UPDATE: u32 = 0x002e_cf30;
/// Retail vector copy helper (0x1f1840), called with (out, sp, cam + 0x30).
const VEC_HELPER: u32 = 0x001f_1840;

/// 1.0f
const ONE: F = F(0x3f80_0000);
/// pi/2 as a float (0x3fc90fdb).
const HALF_PI: F = F(0x3fc9_0fdb);

/// Sign-extends a 32-bit value to a 64-bit register, as lw and addiu do.
fn sx(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// 0x2eb8b8 cam_set_look_blend_flag() -> (): if the rig's mode halfword (rig+0x86) is zero, sets the look-blend
/// flag byte (+0x116) of the rig's target object to 1.
pub fn cam_set_look_blend_flag(c: &mut Cpu) -> bool {
    let rig = c.r32(RIG_PTR);
    let mode = c.r16(rig + RIG_MODE) as i16;
    c.s(V1, mode as i64 as u64);
    c.s(V0, sx(rig));
    if mode == 0 {
        let target = c.r32(rig + RIG_TARGET);
        c.s(V1, sx(target));
        c.s(V0, 1);
        c.w8(target + TARGET_FLAG, 1);
    }
    true
}

/// 0x2ebb88 cam_set_look_float(this, x, y) -> (): when the rig's mode halfword is zero and x is not zero, stores x at
/// the target's +0x1bc and calls the look apply 0x2eb920 (rig, this) with y.
pub fn cam_set_look_float(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let this = c.g(A0) as u32;
    let x = F(c.f[12]);
    let y = F(c.f[13]);
    h::frame(c, 0x10, &[(RA, 0, h::SQ)], |c, _sp| {
        c.s(A1, sx(this));
        let rig = c.r32(RIG_PTR);
        c.s(A0, sx(rig));
        c.f[1] = x.0;
        c.f[12] = y.0;
        let mode = c.r16(rig + RIG_MODE) as i16;
        c.s(V0, mode as i64 as u64);
        if mode != 0 {
            return;
        }
        c.f[0] = 0;
        if x == F(0) {
            return;
        }
        let target = c.r32(rig + RIG_TARGET);
        c.s(V0, sx(target));
        c.w32(target + TARGET_LOOK, x.0);
        h::call_keep(c, LOOK_APPLY, &[rig, this], &[y]);
    });
    true
}

/// 0x2ed038 cam_orient_refresh(cam) -> (): rebuilds the camera's vectors from its rig (+0x70): a copy of the basis at
/// cam+0x0, an angle spring on the rig's values, an orient step into cam+0x40 and two cross products into cam+0x10
/// and cam+0x20.
pub fn cam_orient_refresh(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let cam = c.g(A0) as u32;
    let saves = [(S0, 0x20, h::SQ), (S1, 0x30, h::SQ), (S2, 0x40, h::SQ), (S3, 0x50, h::SQ), (RA, 0x60, h::SQ)];
    h::frame(c, 0x80, &saves, |c, sp| {
        c.w32(sp + 0x70, c.f[20]);
        c.s(V1, sx(CAM_BASIS));
        c.r[V0] = c.mem.rd128(CAM_BASIS);
        c.mem.wr128(sp, c.r[V0]);
        let rig = c.r32(cam + RIG_TARGET);
        let s0 = sp + 0x10;
        c.s(A0, sx(rig + 0x10));
        c.f[1] = c.r32(rig + 0x5c);
        c.f[0] = c.r32(sp + 8);
        c.s(A1, sx(sp));
        c.s(A2, sx(cam + 0x30));
        c.s(A0, sx(s0));
        c.f[0] = (F(c.f[0]) + F(c.f[1])).0;
        c.w32(sp + 8, c.f[0]);
        c.s(AT, ONE.0 as u64);
        h::call_keep(c, VEC_HELPER, &[s0, sp, cam + 0x30], &[]);

        h::run_keep(c, vu_vec::vec_set_length, &[s0, s0], &[ONE]);
        let (_, dot) = h::run_keep(c, vecmath::dot3, &[s0, cam], &[]);
        let (_, acos) = h::run_keep(c, mathx::acos_approx, &[], &[dot]);
        c.s(AT, HALF_PI.0 as u64);
        let (rig4, rig8, rigc) = (c.r32(rig + 4), c.r32(rig + 8), c.r32(rig + 0xc));
        let (_, angle) = h::run_keep(
            c,
            camobj::angle_spring_follow,
            &[rig],
            &[F(0), HALF_PI - acos, F(rig4), F(rig8), F(rigc)],
        );
        h::run_keep(c, orient_step::orient_step, &[cam, cam + 0x40, s0, 1], &[angle]);
        h::run_keep(c, vu_vec::vec_set_length, &[cam + 0x40, cam], &[ONE]);
        c.r[V0] = c.mem.rd128((cam + 0x40) & !15);
        c.mem.wr128(cam & !15, c.r[V0]);
        let basis = cam + 0x10;
        h::run_keep(c, vecmath::cross3, &[basis, cam, CAM_UP], &[]);
        h::run_keep(c, vu_vec::vec_set_length, &[basis, basis], &[ONE]);
        h::run_keep(c, vecmath::cross3, &[cam + 0x20, basis, cam], &[]);
        // Scratch float residue that retail leaves after its angle spring (0.5, 0, pi, -pi); the hands leave other values.
        c.f[1] = 0x3f00_0000;
        c.f[13] = 0;
        c.f[14] = 0x4049_0fdb;
        c.f[15] = 0xc049_0fdb;
    });
    true
}

/// 0x2ed248 cam_update_wrapper() -> (): runs the retail camera update 0x2ecf30 inside a 16-byte frame.
pub fn cam_update_wrapper(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    h::frame(c, 0x10, &[(RA, 0, h::SQ)], |c, _sp| {
        h::call_keep(c, CAM_UPDATE, &[], &[]);
    });
    true
}
