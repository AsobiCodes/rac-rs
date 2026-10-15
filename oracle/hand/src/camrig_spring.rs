//! Camera rig helpers: spring the three components of a camera vector toward a target, and the mode-switched
//! hero/offset vector update that follows it.
use crate::camera;
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Rig field holding the pointer to the spring velocity block (rig + 0x70).
const RIG_VEL_BLOCK: u32 = 0x70;
/// Offset of the x velocity inside the velocity block; y and z follow at +4 and +8.
const VEL_X: u32 = 0xe0;
/// Spring helper 0x1ddaa8 (spring_follow, retail: its scratch floats are left as retail leaves them).
const SPRING_FOLLOW: u32 = 0x1ddaa8;
/// Vector helper 0x1f1810 (vec_add(out, a, b)), retail.
const VEC_ADD: u32 = 0x1f1810;
/// Stack frame of 0x2e8828 (`addiu sp, sp, -0x60`).
const SPRING_FRAME: u32 = 0x60;
/// Stack frame of 0x2e88e8 (`addiu sp, sp, -0x50`).
const DISPATCH_FRAME: u32 = 0x50;
/// Mode value (after `-1`) that takes the hero-position branch.
const MODE_HERO: u32 = 0;
/// First and last mode values (after `-1`) whose path returns 0.
const MODE_ZERO_LO: u32 = 2;
const MODE_ZERO_HI: u32 = 6;

/// 0x2e8828 rig_spring_pos(rig, pos*, target*, k, damp, limit): each component of *pos springs toward *target
/// with the velocity at rig->+0xe0 (+4 per axis) via retail spring_follow; the result is stored back into *pos.
pub fn rig_spring_pos(c: &mut Cpu) -> bool {
    let rig = c.arg(0);
    let pos = c.arg(1);
    let target = c.arg(2);
    let (k, damp, limit) = (c.farg(0), c.farg(1), c.farg(2));
    let vel_block = c.r32(rig + RIG_VEL_BLOCK);
    let saves = [(16, 0x00, h::SQ), (17, 0x10, h::SQ), (18, 0x20, h::SQ), (31, 0x30, h::SQ)];
    h::frame(c, SPRING_FRAME, &saves, |c, sp| {
        // The retail prologue also spills the float arguments: k at +0x40, limit at +0x48, damp at +0x50.
        c.wf(sp + 0x40, k);
        c.wf(sp + 0x48, limit);
        c.wf(sp + 0x50, damp);
        for axis in 0..3 {
            let a = c.rf(pos + 4 * axis);
            let b = c.rf(target + 4 * axis);
            let vel = vel_block + VEL_X + 4 * axis;
            let r = h::call_keep(c, SPRING_FOLLOW, &[vel], &[a, b, k, damp, limit]).1;
            c.wf(pos + 4 * axis, r);
        }
    });
    true
}

/// 0x2e88e8 cam_offset_update(cam, src*, dst*, mode) -> 1 or 0: mode 0 adds the hero position to src into dst;
/// modes 2..6 add src to the block at cam->+0x70 +0x40 and return 0; every other mode returns 1 after that add.
pub fn cam_offset_update(c: &mut Cpu) -> bool {
    let cam = c.arg(0);
    let src = c.arg(1);
    let dst = c.arg(2);
    let mode = c.arg(3).wrapping_sub(1);
    let block = c.r32(cam + RIG_VEL_BLOCK) + 0x40;
    let mut ret = 1;
    let saves = [(16, 0x10, h::SQ), (17, 0x20, h::SQ), (18, 0x30, h::SQ), (31, 0x40, h::SQ)];
    h::frame(c, DISPATCH_FRAME, &saves, |c, sp| {
        if mode == MODE_HERO {
            h::run_keep(c, camera::follow_hero_pos, &[cam, sp], &[]);
            h::call_keep(c, VEC_ADD, &[dst, src, sp], &[]);
        } else {
            if (MODE_ZERO_LO..=MODE_ZERO_HI).contains(&mode) {
                ret = 0;
            }
            h::call_keep(c, VEC_ADD, &[dst, src, block], &[]);
        }
    });
    c.ret(ret);
    true
}
