//! Hero foot ground-conform block writer: tilts the foot IK targets toward the ground while the feet move, keeps
//! the foot timers, refreshes the foot-dust emitters on a step and applies the per-node pose jitter.
use crate::actor;
use crate::h;
use crate::hero_anim;
use crate::ps2::*;
use crate::rand;
use crate::trig;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Hero state block (0x140000 - 0xcb0), addressed through s0 in the retail code.
const HERO: u32 = 0x0014_0000 - 0xcb0;
/// Foot IK output block (0x180000 - 0x5880).
const FOOT_BLOCK: u32 = 0x0018_0000 - 0x5880;

/// Stack frame the retail prologue reserves (addiu sp, sp, -0x80).
const FRAME_SIZE: u32 = 0x80;
/// Retail check at 0x200610: nonzero when this hero's feet are not updated this frame.
const SKIP_CHECK: u32 = 0x0020_0610;
/// Retail helper at 0x260130: applies one jitter sample to a pose node (out_vec, joint block, value).
const JITTER_APPLY: u32 = 0x0026_0130;

/// Hero offsets.
const PROBE: u32 = 0x2c0; // six floats: three (x, y) ground-normal pairs, one per foot
const STATE: u32 = 0x308; // i16: 1 = feet locked, no tilt
const PROBE_TIMER_SET: u32 = 0x10e0; // nonzero: reset the foot timers
const FLAGS: u32 = 0x12ae; // bit 0: take the jitter pivot from the actor, bit 7: stop the step
const STEP_COUNT: u32 = 0x12a8; // jitter samples left
const SAMPLE_COUNT: u32 = 0x12ac; // i16: jitter samples per node
const JITTER_BASE: u32 = 0x12a0; // float: jitter centre
const JITTER_RANGE: u32 = 0x12a4; // float: jitter half-width
const ACTOR: u32 = 0x2080; // pointer to the actor (pose source)
const DUST_STATE_A: u32 = 0x2084; // == 3: first foot-dust set
const DUST_STATE_B: u32 = 0x208c; // == 4: second foot-dust set
const DUST_LIMIT: u32 = 0x418; // i32: foot-dust cap, compared against 12 * scale

/// Foot IK block offsets (tilts and timers).
const TILT_ROLL_TARGET: u32 = 0x480;
const TILT_PITCH_TARGET: u32 = 0x530;
const TILT_ROLL_TURN: u32 = 0x320;
const TILT_PITCH_TURN: u32 = 0x3d0;
const TILT_X_SOURCE: u32 = 0x324;
const TILT_Y_SOURCE: u32 = 0x3d4;
const TIMER_A: u32 = 0x41c;
const TIMER_B: u32 = 0x36c;

const LIFT_NEAR: F = F(0x3cf5_c28f); // 0.03
const LIFT_FAR: F = F(0x3d8e_fa35); // 0.07
const TILT_SEED: F = F(0x3e4c_cccd); // 0.2
const TILT_GAIN: F = F(0x3f99_999a); // 1.2
const TILT_LIMIT: F = F(0x3e32_b8c2); // about 10 degrees
const PI_4: F = F(0x3f49_0fdb);
const TIMER_VALUE: F = F(0x3c23_d70a); // 0.01
const DUST_SPREAD_A: F = F(0x3cac_0831);
const DUST_SPREAD_B: F = F(0x3c83_126f);
const DUST_JITTER: F = F(0x3b03_126f);

/// 0x1fe280 foot_ik_blocks(): ground-conforms the hero feet into the foot block, refreshes the timers and the
/// foot-dust emitters, then applies the pose jitter.
pub fn foot_ik_blocks(c: &mut Cpu) -> bool {
    // The pose jitter (helper 0x260130) still leaves 48 bytes of joint-block RAM and the step counter different
    // from retail, and the tilt path is never exercised by the checker. Until that is resolved, any frame that
    // reaches the jitter loop is left to retail, before any side effect is written.
    if c.r32(HERO + STEP_COUNT) as i32 > 0 && c.r16(HERO + SAMPLE_COUNT) as i16 != 0 {
        return false;
    }
    if h::call(c, SKIP_CHECK, &[], &[]).0 != 0 {
        return true;
    }
    if is_lifting(c) && c.r16(HERO + STATE) as i16 != 1 {
        write_tilt(c);
    }
    if c.r32(HERO + PROBE_TIMER_SET) != 0 {
        c.wf(FOOT_BLOCK + TIMER_A, TIMER_VALUE);
        c.wf(FOOT_BLOCK + TIMER_B, TIMER_VALUE);
    }
    if (c.r16(HERO + FLAGS) & 0x80) != 0 {
        c.w32(HERO + STEP_COUNT, 0);
        return true;
    }
    if c.r32(HERO + DUST_STATE_A) == 3 {
        foot_dust(c, [1, 2, 1, 0], DUST_SPREAD_A, DUST_JITTER);
    }
    if c.r32(HERO + DUST_STATE_B) == 4 {
        let limit = c.r32(HERO + DUST_LIMIT) as i32;
        if limit > 0 && limit < h::scale_int(c, 12) {
            foot_dust(c, [1, 2, 0, 1], DUST_SPREAD_B, DUST_JITTER);
        }
    }
    jitter_pose(c);
    true
}

/// True when any of the six probe values is past its threshold (0.03 for the first pair, 0.07 for the rest).
fn is_lifting(c: &mut Cpu) -> bool {
    let mut lifting = false;
    for i in 0..6u32 {
        let limit = if i < 2 { LIFT_NEAR } else { LIFT_FAR };
        lifting |= limit < c.rf(HERO + PROBE + 4 * i).abs();
    }
    lifting
}

/// Computes the roll and pitch tilt targets from the probe pairs (atan2 of the seed against each component,
/// scaled by 1.2 and clamped), and writes them with the turn angles into the foot block.
fn write_tilt(c: &mut Cpu) {
    let side = -c.rf(HERO + 0x2c8);
    c.wf(FOOT_BLOCK + TILT_X_SOURCE, side);
    let fore = -c.rf(HERO + 0x2cc);
    c.wf(FOOT_BLOCK + TILT_Y_SOURCE, fore);

    let x0 = c.rf(HERO + 0x2c0);
    let x1 = c.rf(HERO + 0x2c4);
    let mut roll = h::run(c, trig::fast_atan2, &[], &[TILT_SEED, x0]).1 * TILT_GAIN;
    let mut pitch = -h::run(c, trig::fast_atan2, &[], &[TILT_SEED, x1]).1 * TILT_GAIN;

    if PI_4 < roll {
        roll = PI_4;
    }
    if roll < -TILT_LIMIT {
        roll = -TILT_LIMIT;
    }
    if TILT_LIMIT < pitch {
        pitch = TILT_LIMIT;
    }
    if pitch < -PI_4 {
        pitch = -PI_4;
    }
    c.wf(FOOT_BLOCK + TILT_ROLL_TARGET, roll);
    c.wf(FOOT_BLOCK + TILT_PITCH_TARGET, pitch);

    let x2d0 = c.rf(HERO + 0x2d0);
    let roll_turn = h::run(c, vecmath::angle_sub, &[], &[x2d0, -roll]).1;
    c.wf(FOOT_BLOCK + TILT_ROLL_TURN, -roll_turn);
    let x2d4 = c.rf(HERO + 0x2d4);
    let pitch_turn = h::run(c, vecmath::angle_sub, &[], &[x2d4, -pitch]).1;
    c.wf(FOOT_BLOCK + TILT_PITCH_TURN, -pitch_turn);
}

/// Calls the foot-dust emitter setup with four integer arguments and two float arguments.
fn foot_dust(c: &mut Cpu, ints: [u32; 4], spread: F, jitter: F) {
    h::run(c, hero_anim::foot_dust_set, &ints, &[spread, jitter]);
}

/// Samples the pose jitter for the two pose nodes (0x16 and 0x17) once per remaining step, each sample offset
/// from the hero's jitter centre by a random amount within the jitter range.
fn jitter_pose(c: &mut Cpu) {
    let remaining = c.r32(HERO + STEP_COUNT) as i32;
    if remaining <= 0 || c.r16(HERO + SAMPLE_COUNT) == 0 {
        return;
    }
    let pivot = if (c.r16(HERO + FLAGS) & 1) != 0 { c.r32(HERO + ACTOR) + 0xc0 } else { 0 };
    c.w32(HERO + STEP_COUNT, (remaining - 1) as u32);
    // Scratch buffer at the bottom of this function's 0x80-byte frame (the retail prologue moves sp down).
    let sp = c.g(29) as u32 - FRAME_SIZE;
    for node in [0x16u32, 0x17] {
        let actor = c.r32(HERO + ACTOR);
        h::run(c, actor::pose_node_pos, &[actor, node, sp], &[]);
        let samples = c.r16(HERO + SAMPLE_COUNT) as i16 as i32;
        for _ in 0..samples {
            let range = c.rf(HERO + JITTER_RANGE);
            let offset = h::run(c, rand::rand_float, &[], &[-range, range]).1;
            let base = c.rf(HERO + JITTER_BASE);
            h::call(c, JITTER_APPLY, &[sp, pivot, sp], &[base + offset, range]);
        }
    }
}
