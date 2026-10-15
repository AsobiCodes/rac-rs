//! Rocket lookup and the aim-vector rotation step (retail 0x252438 and 0x252870).
use crate::h;
use crate::h::run_keep;
use crate::mathx;
use crate::ps2::*;
use crate::slot_bits;
use crate::trig;
use crate::vecmath;
use crate::vu_vec;
use ee_oracle::cpu::Cpu;

/// Halfword count of the rocket index table (signed).
const ROCKET_COUNT: u32 = 0x001b_0870;
/// Halfword indices follow the count; each names a record in the rocket table.
const ROCKET_INDEX: u32 = ROCKET_COUNT + 2;
/// Word holding the base address of the rocket record table (lw at 0x15ff58).
const ROCKET_TABLE_BASE: u32 = 0x0015_ff58;
/// Rocket record: word at +0x78 points at its target (pvar).
const REC_TARGET: u32 = 0x78;
/// Target: word at +0x18 is the key compared against the caller's actor.
const TARGET_KEY: u32 = 0x18;

/// Each index step in the rocket table is 0x100 bytes.
const INDEX_SHIFT: u32 = 8;

const ONE: u32 = 0x3f80_0000;
const HALF: u32 = 0x3f00_0000;
const HALF_PI: u32 = 0x3fc9_0fdb;
const FRAME: u32 = 0x80;
/// Offsets inside the 0x80-byte frame: the quaternion/vector scratch is at sp+0..0x10 (w at +0xc).
const QUAT_W: u32 = 0x0c;
const SAVE_F20: u32 = 0x60;
const SAVE_F21: u32 = 0x68;
const SAVE_F22: u32 = 0x70;

/// 0x252438 rocket_find(key) -> rec: first rocket record whose target's +0x18 word equals key; 0 when none.
pub fn rocket_find(c: &mut Cpu) -> bool {
    let key = c.arg(0);
    let count = c.r16(ROCKET_COUNT) as i16 as i32;
    if count <= 0 {
        c.ret(0);
        return true;
    }
    let base = c.r32(ROCKET_TABLE_BASE);
    for k in 0..count as u32 {
        let index = c.r16(ROCKET_INDEX + 2 * k) as i16 as i32;
        let rec = ((index << INDEX_SHIFT) as u32).wrapping_add(base);
        let target = c.r32(rec.wrapping_add(REC_TARGET));
        if c.r32(target.wrapping_add(TARGET_KEY)) == key {
            c.ret(rec);
            return true;
        }
    }
    c.ret(0);
    true
}

/// 0x252870 orient_step(out, a, b, flag, angle) -> void: writes to out the vector a turned about the normal of (a, b)
/// by sin/cos of angle times the half-complement of the angle between a and b (flag != 0 skips the angle).
pub fn orient_step(c: &mut Cpu) -> bool {
    let out = c.arg(0);
    let a = c.arg(1);
    let b = c.arg(2);
    let flag = c.arg(3);
    let angle = c.farg(0);
    let (f20, f21, f22) = (F(c.f[20]), F(c.f[21]), F(c.f[22]));
    let saves = [
        (19, 0x40, h::SQ),
        (18, 0x30, h::SQ),
        (17, 0x20, h::SQ),
        (16, 0x10, h::SQ),
        (31, 0x50, h::SQ),
    ];
    h::frame(c, FRAME, &saves, |c, sp| {
        c.wf(sp + SAVE_F20, f20);
        c.wf(sp + SAVE_F21, f21);
        c.wf(sp + SAVE_F22, f22);
        let tmp = sp;
        run_keep(c, vecmath::cross3, &[tmp, b, a], &[]);
        run_keep(c, vu_vec::vec_set_length, &[tmp, tmp], &[F(ONE)]);
        let mut dot = run_keep(c, vecmath::dot3, &[a, b], &[]).1;
        if flag == 0 {
            let len_a = run_keep(c, vecmath::length3, &[a], &[]).1;
            let len_b = run_keep(c, vecmath::length3, &[b], &[]).1;
            let prod = len_a * len_b;
            if prod == F(0) {
                let q = c.rv(a);
                c.wv(out, q);
                return;
            }
            dot = dot / prod;
        }
        let acos = run_keep(c, mathx::acos_approx, &[], &[dot]).1;
        let t = (F(HALF_PI) - acos) * F(HALF) * angle;
        let sin = run_keep(c, trig::fast_sin, &[], &[t]).1;
        run_keep(c, vecmath::vec_scale, &[tmp, tmp], &[sin]);
        let cos = run_keep(c, trig::fast_cos, &[], &[t]).1;
        c.wf(sp + QUAT_W, cos);
        run_keep(c, slot_bits::rotate_vec_quat, &[out, a, tmp], &[]);
    });
    true
}
