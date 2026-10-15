//! Planet unlock order and two per-frame motion helpers: `unlock_planet` records a planet once, `bob_z` bobs a height
//! by a sine offset, and `wobble_euler` writes two sine/cosine rotation terms and advances two phases.
use crate::h;
use crate::trig;
use crate::vecmath;
use ee_oracle::cpu::Cpu;
use crate::ps2::*;

/// Unlock flag bytes, one per planet id; the first `PLANET_COUNT` are counted.
const UNLOCK_FLAGS: u32 = 0x13dd40;
const PLANET_COUNT: u32 = 20;
/// Order array of unlocked planet ids, one word per unlock.
const UNLOCK_ORDER: u32 = 0x13d510;
/// Current level word (read at call time, it differs per level).
const LEVEL_WORD: u32 = 0x15ed84;
/// Retail order update, called for a new unlock whose planet is not the current level.
const UPDATE_ORDER: u32 = 0x2551b8;
/// The `lui 0x14` value the prologue leaves in v0 and t1.
const HI_14: u32 = 0x0014_0000;
/// Bits of pi as a single, left in `at` by the retail angle wrap.
const PI_BITS: u32 = 0x4049_0fdb;
/// Byte offset of the z height in the object at a0.
const OBJ_Z: u32 = 0x18;
/// Frame offsets of the saved float registers (f20..f23).
const SAVE_F20: u32 = 0x40;
const SAVE_F21: u32 = 0x48;
const SAVE_F22: u32 = 0x50;
const SAVE_F23: u32 = 0x58;

const AT: usize = 1;
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const T0: usize = 8;
const T1: usize = 9;

/// Sets a general register to a 32-bit value, sign-extended as the MIPS 32-bit ops leave it.
fn set_gpr(c: &mut Cpu, r: usize, v: u32) {
    c.s(r, v as i32 as i64 as u64);
}

/// 0x253078 unlock_planet(planet) -> (): if the planet is not unlocked yet, marks it, appends it to the order array at
/// the current unlocked count, and calls the order update 0x2551b8 unless it is the current level.
pub fn unlock_planet(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let planet = c.arg(0);
    h::frame(c, 0x10, &[(31, 0, h::SQ)], |c, _sp| {
        let flag_addr = UNLOCK_FLAGS.wrapping_add(planet);
        let flag = c.r8(flag_addr);
        if flag != 0 {
            set_gpr(c, V0, flag as u32);
            set_gpr(c, V1, flag_addr);
            set_gpr(c, A0, UNLOCK_FLAGS);
            set_gpr(c, A2, 0);
            set_gpr(c, T1, HI_14);
            return;
        }
        let mut count = 0u32;
        for i in 0..PLANET_COUNT {
            if c.r8(UNLOCK_FLAGS + i) != 0 {
                count += 1;
            }
        }
        let level = c.r32(LEVEL_WORD);
        let slot = UNLOCK_ORDER.wrapping_add(4 * count);
        c.w8(flag_addr, 1);
        c.w32(slot, planet);
        set_gpr(c, V0, level);
        set_gpr(c, V1, slot);
        set_gpr(c, A0, flag_addr);
        set_gpr(c, A1, 1);
        set_gpr(c, A2, count);
        set_gpr(c, T0, UNLOCK_FLAGS);
        set_gpr(c, T1, HI_14);
        if planet != level {
            h::call_keep(c, UPDATE_ORDER, &[planet], &[]);
        }
    });
    true
}

/// 0x254f80 bob_z(obj, phase, off, amp) -> (): advances the phase by the step in f13 (angle_add), moves z back by the
/// old offset, sets the offset to sin(phase) * amp, and adds it to z.
pub fn bob_z(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let obj = c.arg(0);
    let phase_p = c.arg(1);
    let off_p = c.arg(2);
    let amp = c.farg(0);
    let step = c.farg(1);
    h::frame(
        c,
        0x50,
        &[(31, 0x30, h::SQ), (18, 0x20, h::SQ), (17, 0x10, h::SQ), (16, 0, h::SQ)],
        |c, sp| {
            c.wf(sp.wrapping_add(SAVE_F20), F(c.f[20]));
            let phase = c.rf(phase_p);
            let (_, next_phase) = h::run_keep(c, vecmath::angle_add, &[], &[phase, step]);
            c.wf(phase_p, next_phase);
            let z = c.rf(obj.wrapping_add(OBJ_Z)) - c.rf(off_p);
            c.wf(obj.wrapping_add(OBJ_Z), z);
            let (_, s) = h::run_keep(c, trig::fast_sin, &[], &[next_phase]);
            let off = s * amp;
            c.wf(off_p, off);
            let z_new = c.rf(obj.wrapping_add(OBJ_Z)) + off;
            c.wf(obj.wrapping_add(OBJ_Z), z_new);
            c.f[0] = off.0;
            c.f[1] = z_new.0;
        },
    );
    true
}

/// 0x255000 wobble_euler(out, pa, pb, amp, step_a, step_b) -> (): writes out[0x10] = amp sin(a) sin(b) and
/// out[0x11] = amp sin(a) cos(b), then advances the phases a and b by their steps (angle_add).
pub fn wobble_euler(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let out = c.arg(0);
    let pa = c.arg(1);
    let pb = c.arg(2);
    let amp = c.farg(0);
    let step_a = c.farg(1);
    let step_b = c.farg(2);
    h::frame(
        c,
        0x60,
        &[(31, 0x30, h::SQ), (18, 0x20, h::SQ), (17, 0x10, h::SQ), (16, 0, h::SQ)],
        |c, sp| {
            c.wf(sp.wrapping_add(SAVE_F20), F(c.f[20]));
            c.wf(sp.wrapping_add(SAVE_F21), F(c.f[21]));
            c.wf(sp.wrapping_add(SAVE_F22), F(c.f[22]));
            c.wf(sp.wrapping_add(SAVE_F23), F(c.f[23]));
            let a = c.rf(pa);
            let b = c.rf(pb);
            let (_, sin_a) = h::run_keep(c, trig::fast_sin, &[], &[a]);
            let (_, sin_b) = h::run_keep(c, trig::fast_sin, &[], &[b]);
            c.wf(out.wrapping_add(0x40), amp * sin_a * sin_b);
            let (_, sin_a2) = h::run_keep(c, trig::fast_sin, &[], &[a]);
            let (_, cos_b) = h::run_keep(c, trig::fast_cos, &[], &[b]);
            c.wf(out.wrapping_add(0x44), amp * sin_a2 * cos_b);
            let (_, na) = h::run_keep(c, vecmath::angle_add, &[], &[a, step_a]);
            c.wf(pa, na);
            let (_, nb) = h::run_keep(c, vecmath::angle_add, &[], &[b, step_b]);
            c.wf(pb, nb);
            // The retail angle_add wrap leaves PI in at; the hand's angle_add does not write at.
            set_gpr(c, AT, PI_BITS);
        },
    );
    true
}
