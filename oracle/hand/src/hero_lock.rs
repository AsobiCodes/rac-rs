#![allow(dead_code, unreachable_code)]
//! Hero lock-on targeting: candidate scoring, the registry pick over lock candidates, and the per-frame update of the
//! lock-on effect slot.
use crate::actor;
use crate::glove_slot;
use crate::h;
use crate::mathx;
use crate::motion;
use crate::objects;
use crate::path;
use crate::ps2::Ee;
use crate::ps2::*;
use crate::trig;
use ee_oracle::cpu::Cpu;

/// Base of the hero globals block (GP-independent scratch area in the EE data segment).
const GLOBALS: u32 = 0x0013_f350;
/// Quadword of the probe origin copied at the start of the candidate score.
const PROBE_QUAD: u32 = GLOBALS + 0x80;
/// Word selecting whether the probe carries a height sample.
const PROBE_KIND: u32 = GLOBALS + 0x2084;
/// Height sample used when `PROBE_KIND` is the height kind.
const PROBE_HEIGHT: u32 = GLOBALS + 0x2d8;
/// Site table for the secondary lock geometry (used for kinds 1 and 0x1e).
const SITE_TABLE: u32 = 0x0016_6d80;
/// Registry pointer slot: a null-terminated array of moby pointers.
const LOCK_REGISTRY: u32 = 0x001a_bb00;
/// Lock-on effect slot pointer.
const FX_SLOT: u32 = GLOBALS + 0xd10;
/// Gp-relative offset of the slew scale constant.
const GP_SLEW_SCALE: i32 = -0x7ea0;

const ZERO: F = F(0);
const ONE: F = F(0x3f80_0000);
const POINT_SEVEN: F = F(0x3f33_3333);
const POINT_EIGHT: F = F(0x3f4c_cccd);
const POINT_ZERO_FIVE: F = F(0x3d4c_cccd);
const POINT_TWO_FIVE: F = F(0x3e80_0000);
const SEVEN: F = F(0x40e0_0000);
/// Initial "best" score of the registry pick (1e8).
const BIG_SCORE: F = F(0x4cbe_bc20);

/// Gp-relative address (gp = $28 of the EE).
fn gp_addr(c: &Cpu, off: i32) -> u32 {
    (c.g(28) as u32).wrapping_add(off as u32)
}

/// Scores one lock candidate. Returns the score and whether any reject test fired (the out flag).
fn score_candidate(c: &mut Cpu, moby: u32, buf: u32, _heading: F, range: F, half: F, pitch: F) -> (F, bool) {
    let mut rejected = false;
    for i in 0..4 {
        let word = c.r32(PROBE_QUAD + 4 * i);
        c.w32(buf + 4 * i, word);
    }
    if c.r32(PROBE_KIND) == 0x14 {
        let height = c.r32(PROBE_HEIGHT);
        c.w32(buf + 8, height);
    }
    let pos = moby + 0x10;
    let dist = h::dist3(c, buf, pos);
    if range < dist {
        rejected = true;
    }

    // Planar distance and pitch angle of the candidate from the probe.
    let plan_dist = h::dist_xy(c, buf, pos);
    let dz = c.rf(moby + 0x18) - c.rf(buf + 8);
    let pitch_probe = h::run(c, trig::fast_atan2, &[], &[plan_dist, dz]).1;
    let mut last = h::run(c, mathx::angle_dist, &[], &[pitch_probe, ZERO]).1;

    // `gap` is the yaw test value, `span` the yaw weight.
    let mut span = plan_dist;
    let mut gap = plan_dist;
    if matches!(c.r32(PROBE_KIND), 1 | 0x1e) {
        // Secondary geometry from the site table.
        let t = SITE_TABLE;
        let dxs = c.rf(moby + 0x10) - c.rf(t + 0x140);
        let dys = c.rf(moby + 0x14) - c.rf(t + 0x144);
        let site_yaw = h::run(c, trig::fast_atan2, &[], &[dxs, dys]).1;
        let site_yaw_ref = c.rf(t + 0x158);
        let site_gap = h::run(c, mathx::angle_dist, &[], &[site_yaw, site_yaw_ref]).1;
        let site_dist = h::dist_xy(c, t + 0x140, pos);
        let site_dz = c.rf(moby + 0x18) - c.rf(t + 0x148);
        let site_pitch = h::run(c, trig::fast_atan2, &[], &[site_dist, site_dz]).1;
        let site_pitch_ref = -c.rf(t + 0x154);
        let site_pitch_gap = h::run(c, mathx::angle_dist, &[], &[site_pitch, site_pitch_ref]).1;
        last = site_pitch_gap;
        gap = if site_pitch_gap < site_gap { site_gap } else { site_pitch_gap };
        span = site_gap + site_pitch_gap;
    }

    if ZERO < half && half < gap {
        rejected = true;
    }
    if ZERO < pitch && pitch < last {
        rejected = true;
    }

    let base = dist + span * dist;
    let in_class = h::run(c, path::is_class_500_540, &[moby], &[]).0 != 0;
    let score = if in_class { base + SEVEN } else { base };
    (score, rejected)
}

/// 0x200758 hero_lock_score(moby, out*, heading, range, half, pitch) -> score: lock candidate score. Writes 1 to
/// *out when a range, half-width or pitch test rejects the candidate. Score = 3D dist + yaw gap * dist, +7 for
/// crate classes.
pub fn hero_lock_score(c: &mut Cpu) -> bool {
    // Not verified: the checker never reaches it and the registry pick disagrees with retail on f0. Retail runs it.
    return false;
    let moby = c.arg(0);
    let out = c.arg(1);
    let (heading, range, half, pitch) = (c.farg(0), c.farg(1), c.farg(2), c.farg(3));
    c.w32(out, 0);
    let (score, rejected) =
        h::with_stack(c, 0x10, |c, buf| score_candidate(c, moby, buf, heading, range, half, pitch));
    if rejected {
        c.w32(out, 1);
    }
    c.fret(score);
    true
}

/// 0x2009a0 hero_lock_registry_pick(a, b, c, d) -> moby: the registry entry with the lowest hero_lock_score (only
/// live entries with health != 0 and flag 0x1000 are scored). Returns 0 when none qualifies.
pub fn hero_lock_registry_pick(c: &mut Cpu) -> bool {
    // Not verified: retail f0 on return differs from the Rust (first bad: got 0x47e04cb4 want 0x42b5f469). Retail runs it.
    return false;
    let (a, b, cc, d) = (c.farg(0), c.farg(1), c.farg(2), c.farg(3));
    let mut best = 0u32;
    let mut best_score = BIG_SCORE;
    // The float return register keeps the last value the retail loop wrote to it.
    let mut last_f0 = c.fresult();
    let mut index = 0;
    loop {
        let entry = c.r32(LOCK_REGISTRY + 4 * index);
        if entry == 0 {
            break;
        }
        index += 1;
        let owned = h::run(c, objects::attached_value, &[entry], &[]).0;
        if owned == 0 {
            continue;
        }
        let health = c.rf(owned);
        last_f0 = health;
        if health == ZERO || c.r16(entry + 0x34) as u32 & 0x1000 == 0 {
            continue;
        }
        let (flagged, score) = h::with_stack(c, 4, |c, flag| {
            let r = h::run(c, hero_lock_score, &[entry, flag], &[b, a, cc, d]);
            (c.r32(flag), r.1)
        });
        last_f0 = score;
        if flagged != 0 {
            continue;
        }
        if score < best_score {
            best_score = score;
            best = entry;
        }
    }
    c.ret(best);
    c.fret(last_f0);
    true
}

/// 0x200b10 hero_lock_fx_update(): per-frame update of the hero's lock-on effect slot. Picks the target value from the
/// lock state and the glove/slot values, then slews the slot toward it and starts, stops, allocates or frees it.
pub fn hero_lock_fx_update(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let mut target = ONE;
    let mut active = false;
    let mut state = 0u32;

    let locked = c.r32(GLOBALS + 0x2274);
    if locked != 0 {
        active = true;
        let value = h::run(c, objects::attached_value, &[locked], &[]).0;
        state = 0x79;
        if value != 0 {
            let kind = c.r8(value + 0x3c) as u32;
            if kind != 0 {
                state = h::call(c, 0x200ab8, &[kind], &[]).0;
                if c.r32(GLOBALS + 0x208c) == 1 {
                    target = POINT_SEVEN;
                }
            }
        }
    }

    if c.r16(GLOBALS + 0x1dc) != 0 {
        active = true;
        let idx = c.r16(GLOBALS + 0x22ce) as i16 as i32 as u32;
        state = h::call(c, 0x200ab8, &[idx], &[]).0;
    }

    if c.r32(GLOBALS + 0x22a8) == 1 {
        let hero = c.r32(GLOBALS + 0x2080);
        let class = c.r8(hero + 0x53) as u32;
        active = true;
        if matches!(class, 0x54 | 0x82 | 0x5a) && c.r32(GLOBALS + 0xa9c) == 0 {
            active = false;
        }
        target = POINT_EIGHT;
        state = 0x7b;
    }

    if c.r32(GLOBALS + 0x208c) == 0x11 && c.r8(0x0013_d4c6) == 0 {
        active = true;
        state = 0x78;
        let count = c.r32(GLOBALS + 0x22a0) as i32;
        let ratio = F::from_int(count) / F::from_int(10000);
        let mut value = (ONE - ratio) + POINT_TWO_FIVE;
        if ONE < value {
            value = ONE;
        }
        target = value;
    }

    let glove = h::run(c, glove_slot::slot_value_b_fn, &[0], &[]).0 as i32;
    if matches!(glove, 15 | 16 | 19) && c.r8(GLOBALS + 0x20a8) != 0 {
        target = POINT_EIGHT;
        active = true;
        state = 0x76;
    }

    let hero = c.r32(GLOBALS + 0x2080);
    let gp_scale = c.rf(gp_addr(c, GP_SLEW_SCALE));
    let step = gp_scale * POINT_ZERO_FIVE;
    if active {
        if c.r32(FX_SLOT) == 0 {
            let slot = h::run(c, actor::fx_slot_alloc, &[hero, 0x18], &[]).0;
            c.w32(FX_SLOT, slot);
            c.w32(slot + 8, 0);
        }
        let slot = c.r32(FX_SLOT);
        if slot != 0 {
            h::run(c, motion::slew_toward, &[slot + 8], &[target, step]);
            if c.r8(slot + 0x23) as u32 == state {
                h::call(c, 0x241a58, &[hero, slot], &[]);
            } else {
                let steps = h::scale_int(c, 8) as u32;
                h::run(c, actor::fx_slot_start, &[hero, slot, state, 0, steps, 1], &[]);
            }
        }
    } else {
        let slot = c.r32(FX_SLOT);
        if slot != 0 {
            let settled = h::run(c, motion::slew_toward, &[slot + 8], &[ZERO, step]).1;
            if settled == ZERO {
                h::run(c, actor::fx_slot_free, &[hero, FX_SLOT], &[]);
            }
        }
    }
    true
}
