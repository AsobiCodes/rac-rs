//! Random-number helpers built on the game's generator (0x1160d8): integer ranges, float ranges, angles and
//! random directions.
use crate::ps2::*;
use crate::h;
use crate::rcall::{restore, save};
use crate::trig;
use ee_oracle::cpu::Cpu;

/// Pointer to the struct holding the generator state (0x1160d8 is the retail routine).
const RAND_STATE_PTR: u32 = 0x12_f76c;
/// Retail helper that turns (out, length, yaw, pitch) into a vector.
const POLAR_TO_VEC_FN: u32 = 0x25_50d0;
/// 0x1.921fb6p1 (pi).
const PI: F = F(0x4049_0fdb);
/// 2^-15, 2^-12 and 2^-11: scales of the 15-, 12- and 12-bit random fractions.
const TWO_M15: F = F(0x3800_0000);
const TWO_M12: F = F(0x3980_0000);
const TWO_M11: F = F(0x3a00_0000);

/// One draw of the generator (0x1160d8: state word at +0x58 of the struct pointed to by *0x12f76c, LCG
/// x * 0x41c64e6d + 0x3039, 31-bit result), shifted down to its high half.
fn draw(c: &mut Cpu) -> i32 {
    let state = c.r32(RAND_STATE_PTR) + 0x58;
    let x = c.r32(state).wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039);
    c.w32(state, x);
    ((x & 0x7fff_ffff) as i32) >> 16
}

/// Uniform float in [lo, hi) with 15 bits of resolution.
fn float_range(c: &mut Cpu, lo: F, hi: F) -> F {
    let r = F::from_int(draw(c) & 0x7fff);
    lo + r * (hi - lo) * TWO_M15
}

/// Angle in [-pi, pi) with 12 bits of resolution.
fn angle(c: &mut Cpu) -> F {
    F::from_int((draw(c) & 0xfff) - 0x800) * PI * TWO_M11
}

/// 0x24a418 rand_below(n) -> random integer in [0, n).
pub fn rand_below(c: &mut Cpu) -> bool {
    let n = c.arg(0) as i32;
    if n == 0 {
        return false;
    }
    let r = draw(c) & 0x7fff;
    c.ret(r.wrapping_rem(n) as u32);
    true
}

/// 0x24a458 rand_int(lo, hi) -> random integer in [lo, hi].
pub fn rand_int(c: &mut Cpu) -> bool {
    let (lo, hi) = (c.arg(0) as i32, c.arg(1) as i32);
    let n = hi.wrapping_sub(lo).wrapping_add(1);
    if n == 0 {
        return false;
    }
    let r = draw(c) & 0x7fff;
    c.ret(r.wrapping_rem(n).wrapping_add(lo) as u32);
    true
}

/// 0x24a4b0 rand_float(lo, hi) -> random float in [lo, hi).
pub fn rand_float(c: &mut Cpu) -> bool {
    let (lo, hi) = (c.farg(0), c.farg(1));
    let r = float_range(c, lo, hi);
    c.fret(r);
    true
}

/// 0x24a510 rand_float_signed(lo, hi) -> lo + 12-bit fraction of (hi - lo), negated when the next bit is set.
pub fn rand_float_signed(c: &mut Cpu) -> bool {
    let (lo, hi) = (c.farg(0), c.farg(1));
    let r = draw(c);
    let v = lo + F::from_int(r & 0xfff) * (hi - lo) * TWO_M12;
    c.fret(if r & 1 != 0 { -v } else { v });
    true
}

/// 0x24a578 rand_angle() -> random angle in [-pi, pi).
pub fn rand_angle(c: &mut Cpu) -> bool {
    let a = angle(c);
    c.fret(a);
    true
}

/// 0x24a5c8 rand_polar_vec(out, lo, hi): random vector of random length in [lo, hi) and random yaw and pitch,
/// built by the retail polar-to-vector routine.
pub fn rand_polar_vec(c: &mut Cpu) -> bool {
    let (out, lo, hi) = (c.arg(0), c.farg(0), c.farg(1));
    let yaw = angle(c);
    let pitch = angle(c);
    let len = float_range(c, lo, hi);
    let saved = save(c);
    c.set_arg(0, out);
    c.set_farg(0, len);
    c.set_farg(1, yaw);
    c.set_farg(2, pitch);
    c.call(POLAR_TO_VEC_FN);
    restore(c, &saved);
    true
}

/// 0x24a640 rand_sphere_vec(out, lo, hi): out = random length in [lo, hi) times a random direction. The unit
/// vector comes from the VU0 sin/cos micro calls, in retail's order (cos yaw, sin pitch, sin yaw, sin pitch, cos pitch).
pub fn rand_sphere_vec(c: &mut Cpu) -> bool {
    let (out, lo, hi) = (c.arg(0), c.farg(0), c.farg(1));
    let len = float_range(c, lo, hi);
    let yaw = angle(c);
    let pitch = angle(c);
    let cos_yaw = h::run_keep(c, trig::fast_cos, &[], &[yaw]).1;
    let sin_pitch = h::run_keep(c, trig::fast_sin, &[], &[pitch]).1;
    let x = cos_yaw * sin_pitch * len;
    let sin_yaw = h::run_keep(c, trig::fast_sin, &[], &[yaw]).1;
    let sin_pitch = h::run_keep(c, trig::fast_sin, &[], &[pitch]).1;
    let y = sin_yaw * sin_pitch * len;
    let cos_pitch = h::run_keep(c, trig::fast_cos, &[], &[pitch]).1;
    let z = cos_pitch * len;
    c.wf(out, x);
    c.wf(out + 4, y);
    c.wf(out + 8, z);
    c.fret(z);
    true
}

/// Sets the assembler temporary `at` (GPR 1) to a sign-extended 32-bit constant.
fn set_at(c: &mut Cpu, v: u32) {
    c.s(1, v as i32 as i64 as u64);
}

/// 0x24a720 ease_cos(a, b, t) -> a at t = 0, b at t = 1, else a + (b - a) * (1 - cos(pi t)) / 2.
pub fn ease_cos(c: &mut Cpu) -> bool {
    let (a, b, t) = (c.farg(0), c.farg(1), c.farg(2));
    // Retail leaves `at` as its constant loads set it: 1.0 once t is compared with 1, 0.5 at the end of the blend.
    let r = if t == F::ZERO {
        a
    } else if t == F::ONE {
        set_at(c, 0x3f80_0000);
        b
    } else {
        set_at(c, 0x4049_0fdb);
        let cos = h::run_keep(c, trig::fast_cos, &[], &[t * PI]).1;
        set_at(c, 0x3f00_0000);
        a + (b - a) * ((F::ONE - cos) * F(0x3f00_0000))
    };
    c.fret(r);
    true
}
