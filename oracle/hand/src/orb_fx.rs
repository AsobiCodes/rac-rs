//! Nanotech orb effects: the puff burst an arrived orb throws out (0x2d9ed0) and the position of one of the orb's
//! sparkle trail points (0x2da308).
use crate::ps2::*;
use crate::{h, rand, vecmath, vu_vec};
use ee_oracle::cpu::Cpu;

/// Register numbers of the saved registers (MIPS names).
const S0: usize = 16;
const S1: usize = 17;
const S2: usize = 18;
const S3: usize = 19;
const RA: usize = 31;
const GP: usize = 28;
/// Register numbers of the assembler temporary and the v1 result register.
const AT: usize = 1;
const V0: usize = 2;
const V1: usize = 3;

/// 0x166ec0: the camera position the puff points away from.
const CAMERA: u32 = 0x0016_6ec0;
/// 0x15ed6c: the per-tick time step (PAL_DT, 0.02).
const PAL_DT: u32 = 0x0015_ed6c;
/// 0x13f5e0: the gravity-down vector (0, 0, -1).
const DOWN: u32 = 0x0013_f5e0;
/// 0x1d62b0: the table of sparkle trail offsets, 16 bytes (one vector) each.
const TRAIL_TABLE: u32 = 0x001d_62b0;

/// gp-relative offsets of the puff constants.
const GP_SPEED_LO: i32 = -0x50d0;
const GP_SPEED_HI: i32 = -0x50cc;
const GP_LIFE: i32 = -0x50c8;
const GP_COLOUR_BIG: i32 = -0x50c4;
const GP_COLOUR_SMALL: i32 = -0x50c0;
const GP_SIZE_BIG: i32 = -0x50bc;
const GP_SIZE_SMALL: i32 = -0x50b8;

/// Retail routines the puff and the sparkle call without a hand version.
const ROTATE: u32 = 0x0025_27c8;
const VEC_SUB: u32 = 0x001f_1840;
const VEC_ADD: u32 = 0x001f_1810;
const SPARK: u32 = 0x0026_54b8;

/// Stores a whole register (all 128 bits, as `sq` does) at `addr`.
fn save_reg(c: &mut Cpu, addr: u32, reg: usize) {
    let q = c.r[reg];
    c.wv(addr, V([q as u32, (q >> 32) as u32, (q >> 64) as u32, (q >> 96) as u32]));
}

/// Address of a gp-relative constant.
fn gp_addr(c: &Cpu, off: i32) -> u32 {
    (c.g(GP) as u32).wrapping_add(off as u32)
}

/// One puff spark: a random draw for the spark's seed, then the type-0x3c spark spawn at `pos` along `vel`.
fn puff_spark(c: &mut Cpu, pos: u32, vel: u32, colour_off: i32, size_off: i32) {
    let life = c.r8(gp_addr(c, GP_LIFE)) as u32;
    let seed = h::run_keep(c, rand::rand_below, &[255], &[]).0 & 0xff;
    let colour = c.r32(gp_addr(c, colour_off));
    let size = c.rf(gp_addr(c, size_off));
    h::call_keep(c, SPARK, &[pos, vel, colour, life, seed, 1], &[size]);
}

/// 0x2d9ed0 orb_puff_burst(orb): the puff an arrived nanotech orb throws out. Takes the orb's position (orb + 0x10),
/// points a random direction away from the camera, tilts it by a random angle about that axis and emits two sparks
/// (size 0.4 and 0.2) along it.
pub fn orb_puff_burst(c: &mut Cpu) -> bool {
    let pos = c.arg(0).wrapping_add(0x10);
    h::with_stack(c, 0x80, |c, sp| {
        save_reg(c, sp + 0x70, RA);
        save_reg(c, sp + 0x60, S3);
        save_reg(c, sp + 0x50, S2);
        save_reg(c, sp + 0x40, S1);
        save_reg(c, sp + 0x30, S0);
        // The retail code loads the time step through `at`; the callees leave it as they find it.
        c.s(AT, 0x0016_0000);
        let dt = c.rf(PAL_DT);
        // away = camera - pos (frame + 0x00)
        h::call_keep(c, VEC_SUB, &[sp, CAMERA, pos], &[]);
        // m = rand_float(lo * dt, hi * dt); down = (0, 0, -1) scaled by m (frame + 0x10)
        let lo = c.rf(gp_addr(c, GP_SPEED_LO)) * dt;
        let hi = c.rf(gp_addr(c, GP_SPEED_HI)) * dt;
        let m = h::run_keep(c, rand::rand_float, &[], &[lo, hi]).1;
        h::run_keep(c, vecmath::vec_scale, &[sp + 0x10, DOWN], &[m]);
        // dir = rotate(down * m, away, random angle) (frame + 0x20)
        let angle = h::run_keep(c, rand::rand_angle, &[], &[]).1;
        h::call_keep(c, ROTATE, &[sp + 0x20, sp + 0x10, sp], &[angle]);
        // Two sparks along dir at the orb position: size 0.4 with colour 1, size 0.2 with colour 2.
        puff_spark(c, pos, sp + 0x20, GP_COLOUR_BIG, GP_SIZE_BIG);
        puff_spark(c, pos, sp + 0x20, GP_COLOUR_SMALL, GP_SIZE_SMALL);
    });
    true
}

/// 0x2da308 orb_sparkle_pos(orb, out, idx): writes the world position of sparkle trail point `idx` of the orb to
/// `out`: the rotated table offset (about the orb's axis at orb.trail + 0x20 by its angle) is stretched to the trail
/// length when the orb's state is 2, then added to the orb's centre (trail + 0x10) and its x lifts z.
pub fn orb_sparkle_pos(c: &mut Cpu) -> bool {
    let (obj, out, idx) = (c.arg(0), c.arg(1), c.arg(2));
    h::with_stack(c, 0x60, |c, sp| {
        save_reg(c, sp + 0x50, RA);
        save_reg(c, sp + 0x40, S3);
        save_reg(c, sp + 0x30, S2);
        save_reg(c, sp + 0x20, S1);
        save_reg(c, sp + 0x10, S0);
        let trail = c.r32(obj + 0x78);
        let table = TRAIL_TABLE.wrapping_add(idx << 4);
        let axis = trail + 0x20;
        let angle = c.rf(trail + 0x2c);
        h::call_keep(c, ROTATE, &[sp, table, axis], &[angle]);
        // Retail sets v0 = 2 in the delay slot of the slot test (always run); v1 = state only on the slot != 0 path.
        c.s(V0, 2);
        // A trail point with a length slot (trail + 0x30 + 4 * idx) and an orb state of 2 is stretched to that length.
        let slot = c.r32(trail + 0x30 + idx * 4);
        let state = c.r8(obj + 0x20) as u32;
        if slot != 0 {
            c.s(V1, state as u64);
        }
        if slot != 0 && state == 2 {
            let len = c.rf(slot + 0x28);
            h::run_keep(c, vu_vec::vec_set_length, &[sp, sp], &[len]);
        }
        h::call_keep(c, VEC_ADD, &[out, trail + 0x10, sp], &[]);
        // out.z += trail.x (the x of the trail's first vector)
        let x = c.rf(trail);
        let sum = c.rf(out + 8) + x;
        c.f[1] = x.0;
        c.fret(sum);
        c.wf(out + 8, sum);
    });
    true
}
