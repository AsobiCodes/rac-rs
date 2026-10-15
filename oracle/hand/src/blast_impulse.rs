//! Blast impulse over a list of mobys (retail 0x24d5b8): for each moby except `skip`, the push direction is the
//! angle from the blast source to the moby, and the impulse (cos, sin) * scale, lift, W_LANE is handed to the
//! per-moby damage routine at 0x24c648.
use crate::h;
use crate::ps2::*;
use crate::{trig, vu_geom};
use ee_oracle::cpu::Cpu;

/// Moby position offsets (x, y).
const MOBY_X: u32 = 0x10;
const MOBY_Y: u32 = 0x14;
/// Per-moby damage routine (retail, called with a0 = moby, a1 = impulse vector).
const DAMAGE_FN: u32 = 0x24c648;
/// Stack frame size.
const FRAME: u32 = 0x100;
/// Fourth lane of the impulse vector (retail's constant 0x45afdf66).
const W_LANE: F = F(0x45af_df66);
/// Callee-saved GPR slots: (register, offset, width).
const SAVES: [(usize, u32, u8); 9] = [
    (16, 0x40, h::SQ),
    (17, 0x50, h::SQ),
    (18, 0x60, h::SQ),
    (19, 0x70, h::SQ),
    (20, 0x80, h::SQ),
    (21, 0x90, h::SQ),
    (22, 0xa0, h::SQ),
    (23, 0xb0, h::SQ),
    (31, 0xc0, h::SQ),
];

/// 0x24d5b8 blast_impulse_list(obj a0, src pos a1, moby list a2, count a3, skip t0, owner t1, flag_a t2, flag_b t3,
/// scale f13, lift f14, f12 -> stack slot): pushes every listed moby except `skip` away from the source point.
pub fn blast_impulse_list(c: &mut Cpu) -> bool {
    let (obj, src, list, count) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3) as i32);
    let (skip, owner, flag_a, flag_b) = (c.arg(4), c.arg(5), c.arg(6), c.arg(7));
    let (tag, scale, lift) = (c.farg(0), c.farg(1), c.farg(2));
    let quad = c.mem.rd128(src);
    c.r[2] = quad;
    h::frame(c, FRAME, &SAVES, |c, sp| {
        c.mem.wr128(sp, quad);
        let (sx, sy) = (c.rf(sp), c.rf(sp + 4));
        let mut p = list;
        let mut n = count;
        while n > 0 {
            let moby = c.r32(p);
            if moby != skip {
                let hscale = c.r16(obj + 0xa6);
                c.ret(hscale as u32);
                c.w32(sp + 0x20, obj);
                c.w32(sp + 0x24, owner);
                c.w8(sp + 0x28, flag_a as u8);
                c.w8(sp + 0x29, flag_b as u8);
                c.w16(sp + 0x2a, hscale);
                c.wf(sp + 0x2c, tag);
                c.w32(sp + 0x30, owner);
                let dx = c.rf(moby + MOBY_X) - sx;
                let dy = c.rf(moby + MOBY_Y) - sy;
                let (_, angle) = h::run_keep(c, vu_geom::fast_atan2, &[], &[dx, dy]);
                let (_, cos) = h::run_keep(c, trig::fast_cos, &[], &[angle]);
                c.wf(sp + 0x10, cos * scale);
                let (_, sin) = h::run_keep(c, trig::fast_sin, &[], &[angle]);
                c.wf(sp + 0x14, sin * scale);
                c.wf(sp + 0x18, lift);
                c.wf(sp + 0x1c, W_LANE);
                h::call_keep(c, DAMAGE_FN, &[moby, sp + 0x10], &[]);
            }
            p += 4;
            n -= 1;
        }
    });
    true
}
