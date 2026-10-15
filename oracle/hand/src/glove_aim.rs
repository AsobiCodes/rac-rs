//! Hero glove aim: the per-frame aim lag block, the aim-slot clear and the four jitter timers (retail 0x1fd3c0,
//! 0x1fd5a8 and 0x1fda90).
use crate::h;
use crate::mathx;
use crate::path;
use crate::ps2::*;
use crate::rand;
use ee_oracle::cpu::Cpu;

/// Hero state block (`lui 0x14` then `addiu -0xcb0`).
const CTX: u32 = 0x0013_f350;
/// Scratch table the aim code writes (`lui 0x18` then `addiu -0x5880`).
const SCRATCH: u32 = 0x0017_a780;
/// Hero state halfword: 1 once the aim lag has been updated this frame.
const AIM_FLAG: u32 = 0x22c8;
/// Hero byte that gates the glove aim block.
const AIM_GATE: u32 = 0x20ab;
/// Byte selecting the alternate aim tuning.
const AIM_ALT: u32 = 0x20a8;
/// Retail state probe: returns 0x1a while the hero is busy.
const PROBE_STATE: u32 = 0x0020_0540;
/// Retail helper: applies the aim offset, returns non-zero on success.
const APPLY_AIM: u32 = 0x001f_d2e8;
/// Default aim scale float (`lwc1 -0x129c(0x160000)`).
const DEFAULT_SCALE: u32 = 0x0015_ed64;
/// Pointer to the moby checked against class 500..=540 (`lw 0x3f58(0x170000)`).
const AIM_MOBY: u32 = 0x0017_3f58;
/// World vector the aim distance is measured to (`0x170000 + 0x3f60`).
const AIM_TARGET: u32 = 0x0017_3f60;
/// Hero position used as the first distance operand (`0x140000 - 0xc30`).
const AIM_ORIGIN: u32 = 0x0013_f3d0;
/// Offset of the lag float inside the hero block.
const LAG_OFF: u32 = 0x234;
/// Jitter timer words (4 x i32) live at +0x1060; the float offsets at +0x1024/+0x1028 of each 16-byte slot.
const JITTER_BASE: u32 = CTX + 0x1060;
/// Lead-in result word written by slot 0 (`0x140000 + 0x3b0`).
const LEAD_RESULT: u32 = 0x0014_03b0;
/// Scratch row stride (`mult ..., 0xb0`).
const ROW_STRIDE: u32 = 0xb0;
/// Scratch row base index added to the slot number (`addiu ..., 0xd`).
const ROW_BASE: u32 = 13;

const ZERO: F = F(0);
const K_DIST_SCALE: F = F(0x3d4c_cccd);
const K_LAG_SCALE: F = F(0x3e99_999a);
const K_NEG_LAG: F = F(0xbeb2_b8c2);
const K_BASE_DIST: F = F(0x3fa6_6666);
const K_ALT_DIST: F = F(0x3fcc_cccc);
const K_LOW_A: F = F(0x3f11_eb85);
const K_LOW_ALT: F = F(0x3f33_3333);
const K_SPAN: F = F(0x3fa0_0000);
const K_CLAMP_LO: F = F(0xbf9c_61aa);
const K_CLAMP_HI: F = F(0xbf06_0a92);

/// Sign-extends a 32-bit register value the way the MIPS ops do.
fn sx(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// Random float in [lo, hi] (`rand_float_signed`).
fn rand_signed(c: &mut Cpu, lo: F, hi: F) -> F {
    h::run(c, rand::rand_float_signed, &[], &[lo, hi]).1
}

/// Random float in [lo, hi) (`rand_float`).
fn rand_float(c: &mut Cpu, lo: F, hi: F) -> F {
    h::run(c, rand::rand_float, &[], &[lo, hi]).1
}

/// Random integer in [lo, hi] (`rand_int`).
fn rand_int(c: &mut Cpu, lo: i32, hi: i32) -> u32 {
    h::run(c, rand::rand_int, &[lo as u32, hi as u32], &[]).0
}

/// `float_to_int_scaled(n)`.
fn scaled(c: &mut Cpu, n: i32) -> i32 {
    h::scale_int(c, n)
}

/// 0x1fd3c0 hero_glove_aim_block(): clears the aim flag, then (when gated on) eases the aim lag toward the
/// front-hit distance and returns 1 once applied.
pub fn glove_aim_block(c: &mut Cpu) -> bool {
    c.w16(CTX + AIM_FLAG, 0);
    if c.r8(CTX + AIM_GATE) as u32 == 0 {
        c.ret(0x0014_0000);
        c.s(3, 0);
        return true;
    }
    let (state, probe_f0) = h::call(c, PROBE_STATE, &[0], &[]);
    if state == 0x1a {
        c.ret(0x1a);
        c.s(3, 0x1a);
        c.fret(probe_f0);
        return true;
    }
    if c.r32(CTX + 0x2084) == 0x81 {
        let base = c.rf(DEFAULT_SCALE);
        c.wf(SCRATCH + 0x694, K_NEG_LAG);
        c.wf(SCRATCH + 0x6d4, base * K_DIST_SCALE);
        c.wf(SCRATCH + 0x6d8, base * K_LAG_SCALE);
    }
    let flag = c.r16(CTX + AIM_FLAG) as i16 as i32;
    let f13 = if flag == 0 { K_BASE_DIST } else { K_ALT_DIST };
    let f12 = if c.r8(CTX + AIM_ALT) as u32 != 0 { K_LOW_ALT } else { K_LOW_A };
    let (applied, _) = h::call(c, APPLY_AIM, &[0], &[f12, f13]);
    if applied == 0 {
        c.ret(0);
        return true;
    }
    let mut scale = c.rf(DEFAULT_SCALE);
    if c.r8(CTX + AIM_ALT) as u32 != 0 {
        let gp = c.g(28) as u32;
        let moby = c.r32(AIM_MOBY);
        scale = c.rf(gp.wrapping_sub(0x7e9c));
        if moby != 0 {
            let (hit, _) = h::run(c, path::is_class_500_540, &[moby], &[]);
            if hit != 0 {
                c.ret(hit);
                return true;
            }
            scale = c.rf(DEFAULT_SCALE);
        }
    }
    c.wf(SCRATCH + 0x6d4, scale * K_DIST_SCALE);
    let lag_in = scale * K_LAG_SCALE;
    c.wf(SCRATCH + 0x6d8, lag_in);
    let dist = h::dist3(c, AIM_ORIGIN, AIM_TARGET);
    let lag = c.rf(CTX + LAG_OFF);
    let mut v = (K_BASE_DIST - (dist - lag)) / K_BASE_DIST * K_SPAN * K_CLAMP_LO;
    if K_CLAMP_HI < v {
        v = K_CLAMP_HI;
    }
    if v < K_CLAMP_LO {
        v = K_CLAMP_LO;
    }
    c.wf(SCRATCH + 0x694, v);
    c.w16(CTX + AIM_FLAG, 1);
    c.ret(1);
    c.fret(v);
    true
}

/// 0x1fd5a8 clear_aim_slots(): zeroes the two words at +0x1004/+0x1008 and the (+0x1024, +0x1028) pair of each of
/// the four 16-byte slots.
pub fn clear_aim_slots(c: &mut Cpu) -> bool {
    c.w32(CTX + 0x1004, 0);
    c.w32(CTX + 0x1008, 0);
    let mut slot = CTX;
    for _ in 0..4 {
        c.w32(slot + 0x1024, 0);
        c.w32(slot + 0x1028, 0);
        slot += 0x10;
    }
    c.ret(CTX);
    c.s(3, slot as u64);
    c.s(4, sx(0xffff_ffff));
    true
}

/// One expired jitter timer: redraws that slot's float offset and its sample word (per-slot branches of 0x1fda90).
fn jitter_event(c: &mut Cpu, k: u32, timer: u32) {
    let slot = CTX + 0x10 * k;
    match k {
        0 => {
            let off = rand_signed(c, F(0x3e32_b8c2), F(0x3f06_0a92));
            c.wf(slot + 0x1028, off);
            let lo = scaled(c, 0x46);
            let hi = scaled(c, 0x96);
            let n = rand_int(c, lo, hi);
            c.w32(LEAD_RESULT, n);
        }
        1 | 2 => {
            let (lo_f, hi_f) = if k == 1 {
                (F(0x3e32_b8c2), F(0x3f06_0a92))
            } else {
                (F(0x3e86_0a92), F(0x3f5f_66f3))
            };
            let off = rand_signed(c, lo_f, hi_f);
            c.wf(slot + 0x1028, off);
            let lo = scaled(c, 0x28);
            let hi = scaled(c, 0x5a);
            let n = rand_int(c, lo, hi);
            c.w32(timer, n);
        }
        _ => {
            let r1 = rand_float(c, F(0xbdb2_b8c2), F(0x3f06_0a92));
            c.wf(slot + 0x1024, r1);
            let f20 = c.rf(slot + 0x1028);
            let r2 = rand_signed(c, F(0x3eb2_b8c2), F(0x3f75_be0b));
            let neg = (f20 <= ZERO && r2 <= ZERO) || (ZERO <= f20 && ZERO <= r2);
            c.wf(slot + 0x1028, if neg { -r2 } else { r2 });
            let lo = scaled(c, 0x23);
            let hi = scaled(c, 0x46);
            let n = rand_int(c, lo, hi);
            c.w32(timer, n);
        }
    }
}

/// 0x1fda90 aim_jitter_tick(): when the aim is idle, ticks the four jitter timers, redraws the offsets of each slot
/// whose timer fired, then copies every slot's offsets into the scratch rows.
pub fn aim_jitter_tick(c: &mut Cpu) -> bool {
    if c.r32(CTX + 0x2084) != 0 {
        let busy = c.r32(CTX + 0x2084);
        c.ret(0x0014_0000);
        c.s(3, sx(busy));
        c.s(4, CTX as u64);
        return true;
    }
    let owner = c.r32(CTX + 0x2080);
    let busy = c.r8(owner + 0x53) as u32;
    if busy != 0 {
        c.ret(owner);
        c.s(3, busy as u64);
        c.s(4, CTX as u64);
        return true;
    }
    for k in 0..4u32 {
        let timer = JITTER_BASE + 4 * k;
        let (fired, _) = h::run(c, mathx::tick_i32, &[timer], &[]);
        if fired != 0 {
            jitter_event(c, k, timer);
        }
        let row = (k + ROW_BASE) * ROW_STRIDE;
        let slot = CTX + 0x10 * k;
        let f0 = c.rf(slot + 0x1024);
        let f1 = c.rf(slot + 0x1028);
        c.wf(SCRATCH.wrapping_add(row).wrapping_add(0x64), f0);
        c.wf(SCRATCH.wrapping_add(row).wrapping_add(0x68), f1);
    }
    let last = (3 + ROW_BASE) * ROW_STRIDE;
    c.ret(0);
    c.s(4, last as u64);
    c.s(3, (SCRATCH + last) as u64);
    true
}
