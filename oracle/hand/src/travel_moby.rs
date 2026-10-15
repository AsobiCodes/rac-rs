//! Ship-travel moby object: spawn and seed (0x26f2d0), release (0x26f378), and its per-frame angle step (0x26f9a8).
use crate::h;
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Retail moby allocator: takes a kind in a0, returns the new moby in v0 (0 when none is free).
const MOBY_ALLOC: u32 = 0x0027_5d20;
/// Moby kind passed to the allocator by `ship_travel_spawn`.
const TRAVEL_KIND: u32 = 0x0000_046e;
/// Overlay float table base (`lui $v0, 0x16` + `addiu 0x6d80`); read from RAM at call time.
const TABLE: u32 = 0x0016_6d80;
/// Offsets into the table of the three seed floats (x, y, z of the spawn position).
const SEED_X: u32 = TABLE + 0x140;
const SEED_Y: u32 = TABLE + 0x144;
const SEED_Z: u32 = TABLE + 0x148;
/// Float constants of the seed code: 8.0 (x offset), 0.5 (y offset), 0.1 (z offset), -1.9 (+0x44).
const EIGHT: u32 = 0x4100_0000;
const HALF: u32 = 0x3f00_0000;
const TENTH: u32 = 0x3dcc_cccd;
const MINUS_NINETEEN_TENTHS: u32 = 0xbff3_3333;
/// Function pointer stored at moby+0x74: the address of `ship_angle_step` (0x26f9a8).
const ANGLE_STEP_FN: u32 = 0x0026_f9a8;
/// Per-frame angle increment loaded by `ship_angle_step` (0.02).
const ANGLE_INC: u32 = 0x3ca3_d70a;
/// Object offsets: moby pointer (obj+0x44), angle (obj+0x40).
const OBJ_MOBY: u32 = 0x44;
const OBJ_ANGLE: u32 = 0x40;

/// 0x26f2d0 ship_travel_spawn(obj = a0) -> v0 0: spawns a moby of kind 0x46e into obj+0x44 and seeds its
/// position from the overlay float table, the angle step pointer and the back pointer to obj.
pub fn ship_travel_spawn(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let (moby, _) = h::call_keep(c, MOBY_ALLOC, &[TRAVEL_KIND], &[]);
    c.r[5] = moby as i32 as i128 as u128; // a1 holds the new moby
    if moby != 0 {
        c.w32(obj.wrapping_add(OBJ_MOBY), moby);
        c.w16(moby.wrapping_add(0x34), 0);
        let x = c.rf(SEED_X) + F(EIGHT);
        let y = c.rf(SEED_Y) + F(HALF);
        let z = c.rf(SEED_Z) - F(TENTH);
        c.wf(moby.wrapping_add(0x10), x);
        c.wf(moby.wrapping_add(0x14), y);
        c.wf(moby.wrapping_add(0x18), z);
        c.wf(moby.wrapping_add(0x44), F(MINUS_NINETEEN_TENTHS));
        c.w32(moby.wrapping_add(0x74), ANGLE_STEP_FN);
        let back = c.r32(moby.wrapping_add(0x78));
        c.w32(back, obj);
        // Registers retail leaves behind on this path.
        c.r[4] = back as i32 as i128 as u128;
        c.r[3] = ANGLE_STEP_FN as i32 as i128 as u128;
        c.r[1] = MINUS_NINETEEN_TENTHS as i32 as i128 as u128;
        c.f[0] = y.0;
        c.f[1] = z.0;
        c.f[2] = HALF;
        c.f[3] = MINUS_NINETEEN_TENTHS;
        c.f[4] = TENTH;
    }
    c.ret(0);
    true
}

/// 0x26f378 ship_travel_release(obj = a0) -> v0 0: releases the moby at obj+0x44 and stores the result back.
pub fn ship_travel_release(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let moby = c.r32(obj.wrapping_add(OBJ_MOBY));
    let (freed, _) = h::run_keep(c, crate::moby_release::release_moby, &[moby], &[]);
    c.w32(obj.wrapping_add(OBJ_MOBY), freed);
    c.ret(0);
    true
}

/// 0x26f9a8 ship_angle_step(obj = a0): obj+0x40 = angle_add(obj+0x40, 0.02); no return value.
pub fn ship_angle_step(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let angle = c.rf(obj.wrapping_add(OBJ_ANGLE));
    c.f[13] = ANGLE_INC;
    let (_, next) = h::run_keep(c, vecmath::angle_add, &[], &[angle, F(ANGLE_INC)]);
    c.wf(obj.wrapping_add(OBJ_ANGLE), next);
    true
}
