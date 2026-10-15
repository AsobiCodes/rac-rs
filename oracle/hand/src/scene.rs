//! Per-frame scene helpers of the 0x255000..0x25f000 overlay area: spherical offsets, layer scale, the moby update
//! pass, the timed effect request and simple particle integration.
use crate::actor;
use crate::h::*;
use crate::ps2::*;
use crate::trig;
use ee_oracle::cpu::Cpu;

/// Byte flag at 0x15edb7: layer scale effects enabled.
const LAYER_SCALE_ON: u32 = 0x15edb7;
/// Effect request block: counter (0x15f570), owner (0x15f574), argument (0x161f08).
const FX_COUNTER: u32 = 0x15f570;
const FX_OWNER: u32 = 0x15f574;
const FX_ARG: u32 = 0x161f08;

fn gp(c: &Cpu, back: u32) -> u32 {
    (c.g(28) as u32).wrapping_sub(back)
}

fn cos(c: &mut Cpu, x: F) -> F {
    run(c, trig::fast_cos, &[], &[x]).1
}

fn sin(c: &mut Cpu, x: F) -> F {
    run(c, trig::fast_sin, &[], &[x]).1
}

/// 0x2550d0 spherical_to_xyz(out, r, yaw, pitch): out = r * (cos(yaw) cos(pitch), sin(yaw) cos(pitch), sin(pitch)).
pub fn spherical_to_xyz(c: &mut Cpu) -> bool {
    let (out, r, yaw, pitch) = (c.arg(0), c.farg(0), c.farg(1), c.farg(2));
    let x = cos(c, yaw) * r * cos(c, pitch);
    c.wf(out, x);
    let y = sin(c, yaw) * r * cos(c, pitch);
    c.wf(out + 4, y);
    let z = sin(c, pitch) * r;
    c.wf(out + 8, z);
    true
}

/// 0x255f48 layer_set_scale(owner, id, layer, scale): with the effect flag on, attaches the layer when idle and sets
/// its scale (+0x20/+0x24/+0x28); with it off, detaches an attached layer.
pub fn layer_set_scale(c: &mut Cpu) -> bool {
    let (owner, id, layer, scale) = (c.arg(0), c.arg(1), c.arg(2), c.farg(0));
    if c.r8(LAYER_SCALE_ON) != 0 {
        if c.r8(layer + 1) == 0 {
            run(c, actor::layer_attach, &[owner, id, layer], &[]);
        }
        c.wf(layer + 0x28, scale);
        c.wf(layer + 0x20, scale);
        c.wf(layer + 0x24, scale);
    } else if c.r8(layer + 1) != 0 {
        run(c, actor::layer_detach, &[owner, layer], &[]);
    }
    true
}

/// 0x255fb8 layer_scale_factor(layer, s): stores s at +0x70 when the effect flag is on, else 1.0.
pub fn layer_scale_factor(c: &mut Cpu) -> bool {
    let (layer, s) = (c.arg(0), c.farg(0));
    let v = if c.r8(LAYER_SCALE_ON) != 0 { s } else { F::ONE };
    c.wf(layer + 0x70, v);
    true
}

/// 0x2566d8 fx_request_tick(): counts the request timer down; when it expires the owner is cleared. While an owner
/// is set and gp[-0x4cf8] is non-zero, an existing handle (gp[-0x4cfc]) is serviced with 0x228f58(handle, 10),
/// otherwise one is created by 0x228840; with no owner the handle is reset to -1.
pub fn fx_request_tick(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let counter = c.r32(FX_COUNTER);
    if counter != 0 {
        let left = counter.wrapping_sub(1);
        c.w32(gp(c, 0x7690), left);
        if left == 0 {
            c.w32(FX_OWNER, 0);
        }
    }
    let handle = gp(c, 0x4cfc);
    if c.r32(FX_OWNER) == 0 || c.r32(gp(c, 0x4cf8)) == 0 {
        c.w32(handle, u32::MAX);
        return true;
    }
    if c.r32(handle) == u32::MAX {
        let r = call(c, 0x228840, &[0xc, 0, 0x22a2d0, 0x22a320, 0x22a340, 0, 0], &[]).0;
        c.w32(handle, r);
    } else {
        let h = c.r32(handle);
        call(c, 0x228f58, &[h, 0xa], &[]);
    }
    true
}

/// 0x256778 fx_request(owner, arg): `owner` takes the request when none is set (starting the 2-tick timer) or when
/// it already holds it; the argument is stored at 0x161f08 and, when non-zero, announced through 0x1f6b80 /
/// 0x22a858.
pub fn fx_request(c: &mut Cpu) -> bool {
    let (owner, arg) = (c.arg(0), c.arg(1));
    let cur = c.r32(FX_OWNER);
    if cur != owner && cur != 0 {
        return true;
    }
    if arg != 0 {
        let r = call(c, 0x1f6b80, &[arg], &[]).0;
        call(c, 0x22a858, &[r], &[]);
    }
    c.w32(FX_ARG, arg);
    if cur == owner {
        c.w32(gp(c, 0x7690), 2);
    } else {
        c.w32(FX_OWNER, owner);
        c.w32(FX_COUNTER, 2);
    }
    true
}

/// 0x256bf8 moby_update_all(): walks the live moby list from 0x243030 (next link +0x28): skips released ones
/// (state < 0), refreshes the matrix (0x242d48) unless flag 0x40, runs the per-moby update function at +0x74, and
/// refreshes the bounding sphere unless flag 4.
pub fn moby_update_all(c: &mut Cpu) -> bool {
    let mut m = call(c, 0x243030, &[], &[]).0;
    c.w32(gp(c, 0x6c9c), m);
    while m != 0 {
        if c.r8(m + 0x20) as i8 >= 0 {
            if c.r16(m + 0x34) & 0x40 == 0 {
                call(c, 0x242d48, &[m], &[]);
            }
            let update = c.r32(m + 0x74);
            if update != 0 {
                call(c, update, &[m], &[]);
            }
            if c.r16(m + 0x34) & 4 == 0 {
                run(c, crate::moby::actor_transform_bsphere, &[m], &[]);
            }
        }
        m = c.r32(m + 0x28);
    }
    true
}

/// 0x25ea90 particle_integrate(p): position += velocity (+0x10 += +0x30, +0x20 += +0x30); the age word at +4 steps
/// by 0xfa000000 (copied to +0xc) and the particle is retired by 0x259b88 when it goes negative.
pub fn particle_integrate(c: &mut Cpu) -> bool {
    let p = c.arg(0);
    // Retail 0x1f1810 uses VU0 temporaries (VF51/VF300 lanes), so it runs as retail code.
    call(c, 0x1f1810, &[p + 0x10, p + 0x10, p + 0x30], &[]);
    call(c, 0x1f1810, &[p + 0x20, p + 0x20, p + 0x30], &[]);
    let age = c.r32(p + 4).wrapping_add(0xfa00_0000);
    c.w32(p + 4, age);
    c.w32(p + 0xc, age);
    if (age as i32) < 0 {
        call(c, 0x259b88, &[p], &[]);
    }
    true
}
