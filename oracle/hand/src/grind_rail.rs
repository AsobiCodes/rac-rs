//! Rail-grind zone tick and rail probe (kerwan grind gadgets). The tick checks whether the hero is near a zone
//! point for the current mode and, if so, looks up the rail point and latches it in the grind state block.
//! The probe samples a point at the hero, looks up the rail for it and latches it when the drop is in range.
use crate::h;
use crate::hero_space;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Byte that gates the whole tick (0x13d4dd).
const TICK_ENABLE: u32 = 0x13d4dd;
/// Grind state block (0x13f350). The camera module calls the same block CAM_STATE.
const BLOCK: u32 = 0x13f350;
/// Hero position (0x13f3d0, 16 bytes).
const HERO_POS: u32 = 0x13f3d0;
/// Mode word the zone test switches on (0x15ed84).
const ZONE_MODE: u32 = 0x15ed84;
/// Rail lookup routine (0x1ffdb0), retail.
const FIND_RAIL: u32 = 0x1ffdb0;
/// Word at 0x13cae4 whose bit 7 picks the lower or higher drop limit.
const LIMIT_FLAGS: u32 = 0x13cae4;

/// Halfword that must be zero for the tick to run.
const OFF_GATE: u32 = 0x1c8;
/// Word the tick clears and sets when it latches a rail point.
const OFF_FOUND: u32 = 0x56c;
/// Latched rail point (16 bytes).
const OFF_RAIL_POINT: u32 = 0x500;
/// Latched rail index words and float, from the lookup's out parameters.
const OFF_RAIL_INDEX: u32 = 0x560;
const OFF_RAIL_WORD: u32 = 0x564;
const OFF_RAIL_FLOAT: u32 = 0x568;
const OFF_RAIL_LAST: u32 = 0x584;
/// Mode word that gates the search (0 to 2 always search).
const OFF_STATE: u32 = 0x208c;
/// Kind word: 0x29 or 0x2a counts as a locked rail.
const OFF_KIND: u32 = 0x2084;
/// Lock words used with OFF_KIND.
const OFF_LOCK_A: u32 = 0x570;
const OFF_LOCK_B: u32 = 0x5c4;
/// Halfwords used with state 4.
const OFF_FLAG_41E: u32 = 0x41e;
const OFF_FLAG_1CA: u32 = 0x1ca;
/// Halfword cooldown that stops the probe.
const OFF_COOLDOWN: u32 = 0x1e4;
/// Probe found flag (0x13f94c).
const OFF_PROBE_FLAG: u32 = 0x5fc;
/// Probe latch block (0x13f930, 16 bytes) and its words.
const OFF_PROBE_POINT: u32 = 0x5e0;
const OFF_PROBE_A: u32 = 0x5f0;
const OFF_PROBE_B: u32 = 0x5f4;
const OFF_PROBE_FLOAT: u32 = 0x5f8;
const OFF_PROBE_LAST: u32 = 0x60c;
/// Offset of the zone origin (the hero-side point, 0x13f3d0 + 0x10) inside the block.
const OFF_ZONE_ORIGIN: u32 = 0x80;

/// Distance threshold (12.0) for the zone tests.
const ZONE_NEAR_12: u32 = 0x4140_0000;
/// Distance threshold (14.0) for mode 6.
const ZONE_NEAR_14: u32 = 0x4160_0000;

/// Zone points for mode 0x10 (bits of 3 floats each).
const ZONE_10_A: [u32; 3] = [0x42e6_0000, 0x4384_0000, 0x4305_0000];
const ZONE_10_B: [u32; 3] = [0x4380_8000, 0x4396_8000, 0x4300_0000];
/// Zone points for mode 0xe.
const ZONE_E_A: [u32; 3] = [0x431d_0000, 0x4384_8000, 0x424c_0000];
const ZONE_E_B: [u32; 3] = [0x4322_0000, 0x43aa_8000, 0x4240_0000];
const ZONE_E_C: [u32; 3] = [0x439a_0000, 0x42d0_0000, 0x428a_0000];
/// Zone point for mode 6.
const ZONE_6: [u32; 3] = [0x4331_0000, 0x4398_0000, 0x430f_0000];

/// Probe offset (0.3, 0.0, 1.34) from the hero.
const PROBE_X: u32 = 0x3e99_999a;
const PROBE_Y: u32 = 0x0000_0000;
const PROBE_Z: u32 = 0x3fab_851f;
/// Lower drop limit (-0.5) and the rail-drop upper limit (0.57).
const DROP_MIN: u32 = 0xbf00_0000;
const DROP_HIGH: u32 = 0x3f11_eb85;
/// Probe drop limits (-0.7 or -1.2 by flag) and the probe upper limit (0.4).
const PROBE_LOW: u32 = 0xbf33_3333;
const PROBE_LOW_FLAGGED: u32 = 0xbf99_999a;
const PROBE_HIGH: u32 = 0x3ecc_cccd;

/// 0x200000 grind_zone_tick(): when the hero is near a zone point for the current mode, looks up the rail
/// point and latches it (found flag, rail point, index words, drop height) into the grind state block.
pub fn grind_zone_tick(c: &mut Cpu) -> bool {
    if c.r8(TICK_ENABLE) == 0 || c.r16(BLOCK + OFF_GATE) != 0 {
        return true;
    }
    c.w32(BLOCK + OFF_FOUND, 0);
    h::with_stack(c, 0x80, |c, sp| zone_tick_body(c, sp));
    true
}

fn zone_tick_body(c: &mut Cpu, sp: u32) {
    if !zone_near(c, sp) || !search_allowed(c) {
        return;
    }
    copy_quad(c, HERO_POS, sp + 0x10);
    let (found, _) = h::call(c, FIND_RAIL, &[sp + 0x10, sp + 0x30, sp, sp + 0x34, sp + 0x38, sp + 0x3c, 0, 0], &[]);
    if found == 0 {
        return;
    }
    let same_rail = c.r32(sp + 0x30) == c.r32(BLOCK + OFF_RAIL_INDEX);
    if c.r32(BLOCK + OFF_KIND) == 0x29 && c.r32(BLOCK + OFF_LOCK_B) != 0 && same_rail {
        return;
    }
    let diff = c.rf(HERO_POS + 8) - c.rf(sp + 8);
    if !(F(DROP_MIN) < diff) {
        return;
    }
    if !(diff < F(DROP_HIGH)) && c.r16(BLOCK + OFF_FLAG_1CA) == 0 {
        return;
    }
    copy_quad(c, sp, BLOCK + OFF_RAIL_POINT);
    c.w32(BLOCK + OFF_FOUND, 1);
    let index = c.r32(sp + 0x30);
    let word = c.r32(sp + 0x34);
    let float = c.rf(sp + 0x38);
    let last = c.r32(sp + 0x3c);
    c.w32(BLOCK + OFF_RAIL_INDEX, index);
    c.w32(BLOCK + OFF_RAIL_WORD, word);
    c.wf(BLOCK + OFF_RAIL_FLOAT, float);
    c.w32(BLOCK + OFF_RAIL_LAST, last);
}

/// Whether the mode's zone test passes (the hero is within reach of one of its points).
fn zone_near(c: &mut Cpu, sp: u32) -> bool {
    let origin = BLOCK + OFF_ZONE_ORIGIN;
    match c.r32(ZONE_MODE) {
        0x10 => within(c, origin, sp, ZONE_10_A, ZONE_NEAR_12) || within(c, origin, sp + 0x10, ZONE_10_B, ZONE_NEAR_12),
        0xe => {
            within(c, origin, sp, ZONE_E_A, ZONE_NEAR_12)
                || within(c, origin, sp + 0x10, ZONE_E_B, ZONE_NEAR_12)
                || within(c, origin, sp + 0x20, ZONE_E_C, ZONE_NEAR_12)
        }
        6 => within(c, origin, sp, ZONE_6, ZONE_NEAR_14),
        _ => true,
    }
}

/// Writes `point` at `at`, measures its distance from `origin` and tests it against `limit`.
fn within(c: &mut Cpu, origin: u32, at: u32, point: [u32; 3], limit: u32) -> bool {
    point_at(c, at, point);
    let d = h::dist3(c, origin, at);
    !(F(limit) < d)
}

/// Whether the state word lets the tick look up a rail.
fn search_allowed(c: &mut Cpu) -> bool {
    let state = c.r32(BLOCK + OFF_STATE);
    if state <= 2 {
        return true;
    }
    let kind = c.r32(BLOCK + OFF_KIND);
    if (kind == 0x29 || kind == 0x2a) && (c.r32(BLOCK + OFF_LOCK_A) != 0 || c.r32(BLOCK + OFF_LOCK_B) != 0) {
        return true;
    }
    state == 4 && (c.r16(BLOCK + OFF_FLAG_41E) != 0 || c.r16(BLOCK + OFF_FLAG_1CA) != 0)
}

/// 0x2003a0 grind_probe_point(out): the hero point offset by (0.3, 0.0, 1.34), written to `out`.
pub fn grind_probe_point(c: &mut Cpu) -> bool {
    let out = c.arg(0);
    h::run(c, hero_space::point_from_offset, &[out], &[F(PROBE_X), F(PROBE_Y), F(PROBE_Z)]);
    true
}

/// 0x2003d8 grind_rail_probe(): per-tick rail probe. Clears the probe flag, skips during the cooldown, probes a
/// point, finds the rail for it when the state allows, and latches the point when the drop is in range.
pub fn grind_rail_probe(c: &mut Cpu) -> bool {
    h::with_stack(c, 0x50, |c, sp| rail_probe_body(c, sp));
    true
}

fn rail_probe_body(c: &mut Cpu, sp: u32) {
    c.w32(BLOCK + OFF_PROBE_FLAG, 0);
    if c.r16(BLOCK + OFF_COOLDOWN) != 0 {
        // Retail leaves v0 = 0x140000 and v1 = the cooldown halfword (sign-extended).
        let cooldown = c.r16(BLOCK + OFF_COOLDOWN) as i16 as i32 as u32;
        c.ret(0x0014_0000);
        set_v1(c, cooldown);
        return;
    }
    h::run(c, grind_probe_point, &[sp], &[]);
    let state = c.r32(BLOCK + OFF_STATE);
    if !(state == 2 || (state == 4 && c.r16(BLOCK + OFF_FLAG_41E) == 0)) {
        // Retail leaves v0 = 2 and v1 = the state word.
        c.ret(2);
        set_v1(c, state);
        return;
    }
    // Raw call (not h::call) so the lookup's leftover registers stay as retail leaves them.
    let args = [sp, sp + 0x20, sp + 0x10, sp + 0x24, sp + 0x28, sp + 0x2c, 0, 0];
    for (i, a) in args.iter().enumerate() {
        c.set_arg(i, *a);
    }
    c.call(FIND_RAIL);
    let found = c.result();
    if found == 0 {
        return;
    }
    let flags = c.r32(LIMIT_FLAGS);
    // Retail leaves v0 = flags & 0x80 and v1 = 0x140000 on the paths that do not latch.
    c.ret(flags & 0x80);
    set_v1(c, 0x0014_0000);
    let diff = c.rf(sp + 8) - c.rf(sp + 0x18);
    let low = if flags & 0x80 != 0 { PROBE_LOW_FLAGGED } else { PROBE_LOW };
    if !(F(low) < diff) || !(diff < F(PROBE_HIGH)) {
        return;
    }
    copy_quad(c, sp + 0x10, BLOCK + OFF_PROBE_POINT);
    c.w32(BLOCK + OFF_PROBE_FLAG, 1);
    let a = c.r32(sp + 0x20);
    let b = c.r32(sp + 0x24);
    let f = c.rf(sp + 0x28);
    let last = c.r32(sp + 0x2c);
    c.w32(BLOCK + OFF_PROBE_A, a);
    c.w32(BLOCK + OFF_PROBE_B, b);
    c.wf(BLOCK + OFF_PROBE_FLOAT, f);
    c.w32(BLOCK + OFF_PROBE_LAST, last);
    // Retail leaves v0 = 1 and v1 = the grind block on the latch path.
    c.ret(1);
    set_v1(c, BLOCK);
}

/// Sets v1 (register 3), which `ret` does not cover.
fn set_v1(c: &mut Cpu, v: u32) {
    c.s(3, v as i32 as i64 as u64);
}

/// Copies a 16-byte quadword from `src` to `dst`.
fn copy_quad(c: &mut Cpu, src: u32, dst: u32) {
    let v = c.rv(src);
    c.wv(dst, v);
}

/// Writes a zero w lane and the three float bits of `p` at `at`.
fn point_at(c: &mut Cpu, at: u32, p: [u32; 3]) {
    c.w32(at + 12, 0);
    for (i, bits) in p.iter().enumerate() {
        c.w32(at + 4 * i as u32, *bits);
    }
}
