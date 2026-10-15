//! Hero water routines: swim heading bucket, water-exit gate, and the breath drain / surface reaction.
use crate::h;
use crate::hud_slots;
use crate::mathx;
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Hero struct base in RAM (retail `lui 0x14` + `addiu -0xcb0`).
const HERO: u32 = 0x0013_F350;
/// Global byte that skips the breath drain when set (retail 0x13d4c6).
const DRAIN_OFF_FLAG: u32 = 0x0013_D4C6;
/// Global byte that selects event 0x35 over 0x33 (retail 0x13d4c4).
const EVENT_FLAG: u32 = 0x0013_D4C4;
/// Global word whose bits 1 and 3 select event 0x35 (retail 0x13cae0).
const EVENT_BITS: u32 = 0x0013_CAE0;
/// Global word cleared when the surface warning fires (retail 0x167114).
const WARN_WORD: u32 = 0x0016_7114;

/// Hero offsets (relative to HERO).
const HERO_STATE_A: u32 = 0x2084;
const HERO_STATE_B: u32 = 0x208C;
const EXIT_FLAG: u32 = 0x20A8;
const EXIT_FLAG_B: u32 = 0x20AB;
const DEPTH_INT: u32 = 0x01BC;
const DEPTH_F: u32 = 0x229C;
const BREATH: u32 = 0x22A0;
const EXIT_TIMER: u32 = 0x0D14;
const HEIGHT_F: u32 = 0x02F0;
const FLOOR_F: u32 = 0x0088;
const SIDE_F: u32 = 0x0094;
const CEIL_F: u32 = 0x0108;
const TURN_TARGET: u32 = 0x0098;
const TURN_HEAD: u32 = 0x0180;
const SAVED_QUAD: u32 = 0x0080;
const AQUA_PARAM: u32 = 0x2080;
/// Offset of a float constant in the gp-relative pool.
const GP_POOL_F: i32 = -0x7E94;

/// Retail helper addresses.
const CALL_HEAD_INIT: u32 = 0x0020_4970;
/// Retail angle subtract (0x1f26c8) and angular distance (0x1f2788): called as retail so scratch registers match.
const ANGLE_SUB_RETAIL: u32 = 0x001f_26c8;
const ANGLE_DIST_RETAIL: u32 = 0x001f_2788;
const CALL_EXIT_START: u32 = 0x0020_1740;
const CALL_EVENT: u32 = 0x0021_5538;
const CALL_AQUA_REQUEST: u32 = 0x001E_24D8;

/// Float bit patterns.
const ONE: u32 = 0x3F80_0000;
const TWO: u32 = 0x4000_0000;
const PI: u32 = 0x4049_0FDB;
const HALF_PI: u32 = 0x3FC9_0FDB;
const NEG_HALF_PI: u32 = 0xBFC9_0FDB;
const QUARTER_PI: u32 = 0x3F49_0FDB;
const DEPTH_LIMIT: u32 = 0x3F33_3333;
const SIDE_LIMIT: u32 = 0xBF5F_66F3;
const SURFACE_MARGIN: u32 = 0x3ECC_CCCD;
const EXIT_NUDGE: u32 = 0x3C23_D70A;
const EXIT_BIAS: u32 = 0x3E99_999A;

/// Breath refill value and the drain divisor scale.
const BREATH_FULL: u32 = 0x2710;
const DRAIN_SCALE_INPUT: u32 = 60;

/// Hud slot request parameters (retail 0x2293a8, 0x229598, 0x2296e8 are data pointers).
const HUD_SLOT: u32 = 4;
const HUD_TAG: u32 = 0x753F;
const HUD_PTR_A: u32 = 0x0022_93A8;
const HUD_PTR_B: u32 = 0x0022_9598;
const HUD_PTR_C: u32 = 0x0022_96E8;

/// Event ids passed to the event poster.
const EVENT_EXIT_START: u32 = 0x23;
const EVENT_DROWN: u32 = 0x6A;
const EVENT_SURFACE: u32 = 0x06;
const EVENT_WATER_A: u32 = 0x33;
const EVENT_WATER_B: u32 = 0x35;

/// Signed MIPS `div` (LO only): divide by zero gives 1 for negative dividends and -1 otherwise.
fn mips_div(n: i32, d: i32) -> i32 {
    if d == 0 {
        if n < 0 { 1 } else { -1 }
    } else {
        n.wrapping_div(d)
    }
}

/// Angular gap between a turn angle and an aim angle (retail mathx::angle_dist).
fn angle_gap(c: &mut Cpu, turn: F, aim: u32) -> F {
    h::call_keep(c, ANGLE_DIST_RETAIL, &[], &[turn, F(aim)]).1
}

/// Posts a water event through the retail event poster.
fn post_event(c: &mut Cpu, event: u32) {
    h::call(c, CALL_EVENT, &[event, 1], &[]);
}

/// 0x214f38 heading_bucket() -> 0..3: quadrant of the hero's turn error (head 0x180 against target 0x98)
pub fn heading_bucket(c: &mut Cpu) -> bool {
    // Retail frame: sp -0x30, s0 at 0x0, ra at 0x10, f20/f21 spilled at 0x20/0x28 (retail restores them).
    h::frame(c, 0x30, &[(16, 0x0, h::SQ), (31, 0x10, h::SQ)], |c, sp| {
        c.w32(sp.wrapping_add(0x28), c.f[21]);
        c.w32(sp.wrapping_add(0x20), c.f[20]);
        let bucket = heading_bucket_body(c);
        c.ret(bucket);
    });
    true
}

/// Quadrant (0..3) of the turn error between the hero's head and target angles.
fn heading_bucket_body(c: &mut Cpu) -> u32 {
    h::call_keep(c, CALL_HEAD_INIT, &[0], &[F(ONE)]);
    let head = c.rf(HERO + TURN_HEAD);
    let target = c.rf(HERO + TURN_TARGET);
    let turn = h::call_keep(c, ANGLE_SUB_RETAIL, &[], &[head, target]).1;
    let quarter = F(QUARTER_PI);
    if angle_gap(c, turn, PI) < quarter {
        3
    } else if angle_gap(c, turn, NEG_HALF_PI) < quarter {
        1
    } else if angle_gap(c, turn, HALF_PI) < quarter {
        0
    } else {
        2
    }
}

/// 0x21a800 water_exit_gate() -> 0/1: in a water state with the exit flag set, starts the exit when the depth is
/// small and the shallow limit holds
pub fn water_exit_gate(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let state = c.r32(HERO + HERO_STATE_A);
    let in_range = state.wrapping_sub(2) < 2;
    let flagged = (in_range || state == 0) && c.r8(HERO + EXIT_FLAG) != 0;
    if !flagged {
        c.ret(0);
        return true;
    }
    let scaled = h::run(c, mathx::float_to_int_scaled, &[6], &[]).0 as i32;
    if (c.r32(HERO + DEPTH_INT) as i32) >= scaled {
        c.ret(0);
        return true;
    }
    if !(c.rf(HERO + DEPTH_F) < F(DEPTH_LIMIT)) || c.r8(HERO + EXIT_FLAG_B) != 0 {
        c.ret(0);
        return true;
    }
    h::call(c, CALL_EXIT_START, &[], &[]);
    c.w32(HERO + EXIT_TIMER, 2);
    post_event(c, EVENT_EXIT_START);
    c.ret(1);
    true
}

/// 0x21a8b8 breath_water_reaction() -> 0/1: drains breath while submerged (state 0x11), then runs the surface or
/// drown request
pub fn breath_water_reaction(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let result = if c.r32(HERO + HERO_STATE_B) == 0x11 {
        breath_drain(c)
    } else {
        let state = c.r32(HERO + HERO_STATE_A);
        if state != 0x6A && state != 0x76 {
            c.w32(HERO + BREATH, BREATH_FULL);
        }
        exit_request(c)
    };
    c.ret(result);
    true
}

/// Breath drain while submerged: refills the aquarium slot, drains the breath counter, and checks the surface.
fn breath_drain(c: &mut Cpu) -> u32 {
    let mut drain = 0u32;
    if c.r8(DRAIN_OFF_FLAG) == 0 {
        let scale = h::run(c, mathx::float_to_int_scaled, &[DRAIN_SCALE_INPUT], &[]).0 as i32;
        let divisor = scale.wrapping_mul(16).wrapping_sub(scale);
        drain = mips_div(BREATH_FULL as i32, divisor) as u32;
        let slot_args = [
            HUD_SLOT,
            HUD_TAG,
            HUD_PTR_A,
            HUD_PTR_B,
            HUD_PTR_C,
            HERO + BREATH,
            BREATH_FULL,
            drain,
        ];
        h::run(c, hud_slots::hud_slot_request, &slot_args, &[]);
    }
    let left = (c.r32(HERO + BREATH).wrapping_sub(drain) as i32).max(0) as u32;
    c.w32(HERO + BREATH, left);
    if left == 0 && !surface_ready(c) {
        post_event(c, EVENT_DROWN);
        return 1;
    }
    surface_warning(c)
}

/// The surface test once the breath counter is empty: height, side and ceiling limits all hold.
fn surface_ready(c: &mut Cpu) -> bool {
    let gp = c.g(28) as u32;
    let shallow = c.rf(HERO + HEIGHT_F) - F(TWO) < c.rf(HERO + FLOOR_F);
    shallow
        && c.rf(HERO + SIDE_F) < F(SIDE_LIMIT)
        && c.rf(gp.wrapping_add(GP_POOL_F as u32)) < c.rf(HERO + CEIL_F)
}

/// Warns near the surface when the height plus margin is under the floor level.
fn surface_warning(c: &mut Cpu) -> u32 {
    let raised = c.rf(HERO + HEIGHT_F) + F(SURFACE_MARGIN);
    if raised < c.rf(HERO + FLOOR_F) {
        c.w32(WARN_WORD, 0);
        post_event(c, EVENT_SURFACE);
        return 1;
    }
    exit_request(c)
}

/// Requests the aquarium/drown state (only while the hero is in state 0x12). Posts a water event on success.
fn exit_request(c: &mut Cpu) -> u32 {
    if c.r32(HERO + HERO_STATE_B) != 0x12 {
        return 0;
    }
    let accepted = h::with_stack(c, 0x50, |c, buf| {
        let quad = c.rv(HERO + SAVED_QUAD);
        c.wv(buf, quad);
        let nudged = c.rf(HERO + HEIGHT_F) + F(EXIT_NUDGE);
        c.wf(buf + 8, nudged);
        let copy = c.rv(buf);
        c.wv(buf + 0x10, copy);
        let biased = c.rf(buf + 0x18) + F(EXIT_BIAS);
        c.wf(buf + 0x18, biased);
        let aqua = c.r32(HERO + AQUA_PARAM);
        h::call(c, CALL_AQUA_REQUEST, &[buf, buf + 0x10, 2, aqua, 0], &[]).0
    });
    if accepted == 0 {
        return 0;
    }
    let event = if c.r8(EVENT_FLAG) != 0 && c.r32(EVENT_BITS) & 0xA != 0 {
        EVENT_WATER_B
    } else {
        EVENT_WATER_A
    };
    post_event(c, event);
    1
}
