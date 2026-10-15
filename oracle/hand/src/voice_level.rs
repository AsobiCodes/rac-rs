//! Voice level curve: `voice_level` maps a distance to a level between the record's near (+8) and far (+0xc) values,
//! linearly or squared (flag bit 0 at +0x19). `voice_distance_level` measures the camera distance to a voice position
//! first, and `voice_bearing` gives the bearing of a position in the view matrix.
use crate::h;
use crate::mathx;
use crate::vecmath;
use crate::vu_geom;
use crate::vu_vec;
use ee_oracle::cpu::Cpu;
use crate::ps2::*;

/// The camera position the voice distance is measured from (a VU vector in RAM).
const CAM_POS: u32 = 0x166ec0;
const NEAR_OFF: u32 = 0x8;
const FAR_OFF: u32 = 0xc;
const FLAG_OFF: u32 = 0x19;
const FLAG_SQUARED: u8 = 1;
/// Frame size and register saves of `voice_bearing` (s0 at +0x10, ra at +0x20).
const BEARING_FRAME: u32 = 0x40;
const BEARING_S0_OFF: u32 = 0x10;
const BEARING_RA_OFF: u32 = 0x20;
/// 180.0 and 1/pi as floats.
const DEG_PER_TURN: F = F(0x4334_0000);
const INV_PI: F = F(0x3ea2_f983);

/// The level for distance `x` in the record at `s`, with the thresholds `y` (near edge) and `z` (far edge).
fn level_of(c: &mut Cpu, s: u32, x: F, y: F, z: F) -> u32 {
    let near = c.r32(s + NEAR_OFF);
    let far = c.r32(s + FAR_OFF);
    if x <= y {
        return far;
    }
    if z <= x {
        return near;
    }
    let squared = (c.r8(s + FLAG_OFF) & FLAG_SQUARED) != 0;
    let span = far.wrapping_sub(near) as i32;
    let t = h::run_keep(c, mathx::int_to_float, &[span as u32], &[]).1;
    let (dx, dy) = (z - x, z - y);
    let (num, den) = if squared { (dx * dx, dy * dy) } else { (dx, dy) };
    let quotient = num * t / den;
    let q = h::run_keep(c, mathx::float_to_int, &[], &[quotient]).0;
    // Retail leaves the converted integer in f12 and the divisor in f1 (f0 keeps the int_to_float result).
    c.set_farg(0, F(q));
    c.f[1] = den.0;
    near.wrapping_add(q)
}

/// 0x27d138 voice_level(rec, x, y, z) -> level: `rec.near` at or past x<=y, `rec.far` when z<=x, else a value between.
pub fn voice_level(c: &mut Cpu) -> bool {
    let s = c.arg(0);
    let (x, y, z) = (c.farg(0), c.farg(1), c.farg(2));
    let v = level_of(c, s, x, y, z);
    c.ret(v);
    true
}

/// 0x27d228 voice_distance_level(rec, pos) -> level of the camera distance to `pos` under record `rec`'s thresholds.
pub fn voice_distance_level(c: &mut Cpu) -> bool {
    let rec = c.arg(0);
    let pos = c.arg(1);
    let dist = h::run(c, vu_vec::dist3, &[pos, CAM_POS], &[]).1;
    let p = c.r32(rec + NEAR_OFF);
    let (y, z) = (c.rf(p), c.rf(p + 4));
    let v = level_of(c, p, dist, y, z);
    c.ret(v);
    true
}

/// 0x27d270 voice_bearing(pos, mat) -> int: the bearing of `pos` from the camera, -atan2 of its view-space x/y in
/// degrees, scaled by clamp(|v| - 1, 0, 1) (v = (pos - camera) * mat).
pub fn voice_bearing(c: &mut Cpu) -> bool {
    let pos = c.arg(1);
    let mat = c.arg(2);
    let v = h::frame(c, BEARING_FRAME, &[(16, BEARING_S0_OFF, h::SQ), (31, BEARING_RA_OFF, h::SQ)], |c, sp| {
        h::vec_sub(c, sp, pos, CAM_POS);
        h::run(c, vu_vec::vec4_mul_mat4, &[sp, sp, mat], &[]);
        let len = h::run(c, vecmath::length_xy, &[sp], &[]).1;
        let weight = h::run(c, mathx::fclamp, &[], &[len - F::ONE, F::ZERO, F::ONE]).1;
        let (x, y) = (c.rf(sp), c.rf(sp + 4));
        let angle = h::run(c, vu_geom::fast_atan2, &[], &[x, y]).1;
        let deg = -angle * DEG_PER_TURN * weight;
        h::run(c, mathx::float_to_int, &[], &[deg * INV_PI]).0
    });
    c.ret(v);
    true
}
