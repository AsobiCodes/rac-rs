//! Suck-cannon trail puff update (retail 0x25e7f8): advances one puff's rise and drift, fades its alpha and
//! sets its frame index. Twin of novalis update_suck_puff (type-0x12 trail puff pool update).
use crate::ps2::*;
use crate::{h, mathx, slot_bits};
use ee_oracle::cpu::Cpu;

const AT: usize = 1;
const V1: usize = 3;
const A0: usize = 4;

/// Overlay float read from RAM at call time (rate scale of the puff step).
const RATE_ADDR: u32 = 0x0015_ed60;
/// Gp-relative float: scale applied to the drift term when the puff is near the floor.
const GP_DRIFT_OFF: u32 = 0x7ea0;
/// Retail helper (not a hand) that decides whether the puff is still held (non-zero v0 = held).
const PUFF_HOLD_CHECK: u32 = 0x001f_1530;
/// Puff record fields: position block at +0x00 (+0x0c y-scale word, +0x18 drift), packed word at +0x04, alpha
/// byte at +0x08; the motion record is at +0x20 (height +0x04, speed +0x0c, fade +0x08).
const MOTION: u32 = 0x20;
const F_PACKED: u32 = 0x04;
const F_ALPHA: u32 = 0x08;
const F_DRIFT: u32 = 0x18;
const F_SPEED: u32 = 0x0c;

/// Rise threshold: the height must be at least this before the drift term is applied.
const RISE_MIN: F = F(0x3e61_47ae);
const RATE_SCALE: F = F(0x3c54_fdf4);
const RISE_STEP: F = F(0x45ff_f000);
const PACK_SCALE: F = F(0x437f_0000);
const FADE_STEP: F = F(0x3c44_9ba6);
const ONE: F = F(0x3f80_0000);
const DRIFT_STEP: F = F(0x3ca3_d70a);

/// 0x25e7f8 puff_update(puff) -> frame index: rises the puff, drifts it, fades its alpha and packs the frame index
/// into the top byte of the packed word; releases the slot when it has risen out of range.
pub fn puff_update(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let p = c.arg(0);
    let m = p + MOTION;
    let gp = c.g(28) as u32;
    let rate = c.rf(RATE_ADDR);
    let speed = c.rf(m + F_SPEED);
    let height = c.rf(m + 4) + rate * RATE_SCALE * speed;
    c.wf(m + 4, height);
    if height <= F(0) {
        h::run_keep(c, slot_bits::slot_release, &[p], &[]);
        return true;
    }
    let (held, _) = h::call_keep(c, PUFF_HOLD_CHECK, &[p + 0xa], &[]);
    if held != 0 {
        h::run_keep(c, slot_bits::slot_release, &[p], &[]);
        return true;
    }
    let mut height = c.rf(m + 4);
    if RISE_MIN <= height {
        let speed = c.rf(m + F_SPEED);
        let drift_scale = c.rf(gp.wrapping_sub(GP_DRIFT_OFF));
        let neg = -speed;
        c.wf(m + F_SPEED, neg);
        height = height + drift_scale * RATE_SCALE * neg;
        c.wf(m + 4, height);
    }
    let rise = rate * RISE_STEP;
    let lift = c.rf(p + F_SPEED) + rise;
    c.wf(p + F_SPEED, lift);
    let scaled = c.rf(m + 4) * PACK_SCALE;
    let (packed, _) = h::run_keep(c, mathx::float_to_int, &[], &[scaled]);
    c.f[12] = packed;
    let word = c.r32(p + F_PACKED);
    let new_word = (word & 0x00ff_ffff) | (packed << 24);
    c.w32(p + F_PACKED, new_word);
    c.s(V1, new_word as i32 as i64 as u64);
    c.s(A0, 0x00ff_ffff);
    let fade = c.rf(m + F_ALPHA) + rate * FADE_STEP;
    c.wf(m + F_ALPHA, fade);
    if ONE < fade {
        c.wf(m + F_ALPHA, fade - ONE);
    }
    let alpha = c.rf(m + F_ALPHA) * PACK_SCALE;
    let drift = c.rf(p + F_DRIFT) - DRIFT_STEP;
    c.wf(p + F_DRIFT, drift);
    let frame = alpha.to_int();
    c.w8(p + F_ALPHA, frame as u8);
    c.ret(frame as u32);
    // Scratch registers as retail leaves them: f0 the converted frame, f1 the scaled alpha, f2 the drift, f3 one.
    c.f[0] = frame as u32;
    c.f[1] = alpha.0;
    c.f[2] = drift.0;
    c.f[3] = ONE.0;
    c.s(AT, DRIFT_STEP.0 as u64);
    true
}
