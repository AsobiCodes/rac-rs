//! Hero hand pose helpers (retail 0x1fbc08, 0x1fbfc8, 0x1fc2e8): steer the hand yaw toward its blend target, rebuild
//! the pivot frame, and slew the hand rate and weight limits.
use crate::actor;
use crate::h;
use crate::mathx;
use crate::motion;
use crate::ps2::*;
use crate::trig;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Base of the hero global block (retail `a2` / `s6`, 0x140000 - 0xcb0).
const BLOCK: u32 = 0x0013_F350;
/// Hero actor pointer inside the block.
const ACTOR_PTR: u32 = 0x2080;
/// Primary and secondary hand modes.
const STATE_A: u32 = 0x208c;
const STATE_B: u32 = 0x2084;
/// Halfword gate for the hand blend target.
const GATE_30E: u32 = 0x30e;
/// Hand yaw angle (float).
const HAND_ANGLE: u32 = 0xd20;
/// Hand weight and its two fallback limits.
const WEIGHT_2D8: u32 = 0x2d8;
const WEIGHT_2F0: u32 = 0x2f0;
const WEIGHT_2F4: u32 = 0x2f4;
/// 16-byte pose slot written every tick.
const POSE_OUT: u32 = 0x001A_EC90;
/// Four direction entries of three floats each.
const DIR_TABLE: u32 = 0x0017_C400;
/// Limit table; the clamp value sits at +0x28.
const LIMIT_TABLE: u32 = 0x0017_3F40;
const LIMIT_OFF: u32 = 0x28;
/// Probe flags and sample count used by the line test.
const PROBE_FLAGS: u32 = 0x22;
const LINE_PROBE: u32 = 0x001E_24D8;

/// Frame size of the hand pose routine.
const POSE_FRAME: u32 = 0xf0;

/// Rate and slew fields used by hand_rate_slew.
const SRC_AA8: u32 = 0xaa8;
const NONZERO_A9C: u32 = 0xa9c;
const RATE_A94: u32 = 0xa94;
const SLEW_A90: u32 = 0xa90;

/// Actor fields.
const ACT_FLAGS: u32 = 0x34;
const ACT_BIT_BD: u32 = 0xbd;
const ACT_SPRING_A: u32 = 0x84;
const ACT_SPRING_B: u32 = 0x88;
const ACT_COUNT: u32 = 0x53;
const ACT_LIST: u32 = 0x24;
const ACT_PTR_6C: u32 = 0x6c;

/// Pose-chain block copied to the stack before pose_chain_world.
const CHAIN_BLOCK: u32 = 0x001D_9900;
const CHAIN_LEN: u32 = 0x24;
const CHAIN_OWNER_FALLBACK: u32 = 0x0014_13D0;
const CHAIN_ARG_A3: u32 = 0x0013_FE10;
const U64_MASK: u128 = u64::MAX as u128;

const BLEND_LO: F = F(0x3f78_51ec);
const BLEND_HI: F = F(0x3fc0_0000);
const HAND_CLAMP: F = F(0x3d54_fdf4);
const HAND_CLAMP_NEG: F = F(0xbd54_fdf4);
const STEP_1024: F = F(0x3a80_0000);
const HALF: F = F(0x3f00_0000);
const SPRING_BIAS: F = F(0x3df5_c28f);
const SPRING_ADD: F = F(0x3e75_c28f);
const THREE: F = F(0x4040_0000);
const NEG_ONE: F = F(0xbf80_0000);

/// Leaves the retail exit values of v0 and v1 behind (the caller can read them).
fn leave(c: &mut Cpu, v0: u32, v1: u32) {
    c.r[2] = v0 as i32 as i64 as u128;
    c.r[3] = v1 as i32 as i64 as u128;
}

/// Body of hero_hand_pose, with `buf` the 0xf0-byte frame below sp.
fn pose_body(c: &mut Cpu, buf: u32) {
    let state_a = c.r32(BLOCK + STATE_A);
    let state_b = c.r32(BLOCK + STATE_B);
    if state_a == 0x12 || state_a == 0x15 || state_b == 0x7d || state_a == 0x16 {
        let a = c.r32(BLOCK + ACTOR_PTR);
        let flags = c.r16(a + ACT_FLAGS) & 0xfbff;
        c.w16(a + ACT_FLAGS, flags);
        leave(c, BLOCK, flags as u32);
        return;
    }

    let a = c.r32(BLOCK + ACTOR_PTR);
    let flags = c.r16(a + ACT_FLAGS) | 0x400;
    c.w16(a + ACT_FLAGS, flags);
    let a = c.r32(BLOCK + ACTOR_PTR);
    c.w8(a + ACT_BIT_BD, 1);

    let gate = c.r16(BLOCK + GATE_30E) as i16;
    let target = if gate != 0 || state_a == 4 { BLEND_HI } else { BLEND_LO };

    // Shortest yaw step toward the target, clamped to +-HAND_CLAMP.
    let cur = c.rf(BLOCK + HAND_ANGLE);
    let d = h::run(c, vecmath::angle_sub, &[], &[target, cur]).1;
    let delta = if HAND_CLAMP < d {
        HAND_CLAMP
    } else if d < HAND_CLAMP_NEG {
        HAND_CLAMP_NEG
    } else {
        d
    };
    let cur = c.rf(BLOCK + HAND_ANGLE);
    let angle = h::run(c, vecmath::angle_add, &[], &[cur, delta]).1;
    c.wf(BLOCK + HAND_ANGLE, angle);

    let actor = c.r32(BLOCK + ACTOR_PTR);
    h::run(c, actor::pivot_blend_hand, &[actor, buf], &[]);

    // Pivot frame: heading from the blended vector, then the hand's cos/sin rows.
    let (pivot_x, pivot_y) = (c.rf(buf), c.rf(buf + 4));
    let heading = h::run(c, trig::fast_atan2, &[], &[pivot_x, pivot_y]).1;
    let cos_h = h::run(c, trig::fast_cos, &[], &[heading]).1;
    let sin_h = h::run(c, trig::fast_sin, &[], &[heading]).1;
    let cur = c.rf(BLOCK + HAND_ANGLE);
    let cos_cur = h::run(c, trig::fast_cos, &[], &[cur]).1;
    c.wf(buf + 4, sin_h * cos_cur);
    c.wf(buf, cos_h * cos_cur);
    let cur = c.rf(BLOCK + HAND_ANGLE);
    let sin_cur = h::run(c, trig::fast_sin, &[], &[cur]).1;
    c.wf(buf + 8, -sin_cur);
    for i in 0..16 {
        let b = c.r8(buf + i);
        c.w8(POSE_OUT + i, b);
    }

    // Pick the hand weight from the first valid limit.
    let mut weight = c.rf(BLOCK + WEIGHT_2D8);
    if weight < F::ONE {
        let w0 = c.rf(BLOCK + WEIGHT_2F0);
        if F::ONE <= w0 {
            weight = w0;
        } else {
            let w1 = c.rf(BLOCK + WEIGHT_2F4);
            if F::ONE < w1 {
                weight = w1;
            } else {
                let a = c.r32(BLOCK + ACTOR_PTR);
                c.w32(a + ACT_SPRING_A, 0);
                c.w32(a + ACT_SPRING_B, 0);
                leave(c, a, a);
                c.fret(w1);
                c.f[1] = 0x3f80_0000;
                return;
            }
        }
    }
    let mut hi = weight;
    let mut lo = weight;

    // Four passes over the direction table: place the sample point and probe the line.
    let mut entry = DIR_TABLE;
    for _ in 0..4 {
        let actor = c.r32(BLOCK + ACTOR_PTR);
        let scale = c.rf(actor + 0xc) * STEP_1024;
        h::vec_scale(c, buf + 0x10, actor, STEP_1024);

        let z = c.rf(buf + 0x18) + c.rf(entry + 8) * scale;
        let px = cos_h * c.rf(entry);
        let py = sin_h * c.rf(entry + 4);
        let nz = -c.rf(buf + 8);
        let out_x = c.rf(buf + 0x10) + px * scale;
        let out_y = c.rf(buf + 0x14) + py * scale;
        let len = ((z - weight) + HALF) / nz;
        c.wf(buf + 0x18, z);
        c.wf(buf + 0x10, out_x);
        c.wf(buf + 0x14, out_y);
        h::vec_set_length(c, buf + 0x20, buf, len);
        h::vec_add(c, buf + 0x20, buf + 0x10, buf + 0x20);

        let hit = h::call(c, LINE_PROBE, &[buf + 0x10, buf + 0x20, PROBE_FLAGS, 0, 0], &[]).0;
        if hit != 0 {
            let lim = c.rf(LIMIT_TABLE + LIMIT_OFF);
            lo = h::run(c, mathx::fmin, &[], &[lo, lim]).1;
            hi = h::run(c, mathx::fmax, &[], &[hi, lim]).1;
        }
        entry += 0x10;
    }

    let actor = c.r32(BLOCK + ACTOR_PTR);
    c.wf(actor + ACT_SPRING_A, lo - SPRING_BIAS);
    let actor = c.r32(BLOCK + ACTOR_PTR);
    c.wf(actor + ACT_SPRING_B, hi + SPRING_ADD);
    let actor = c.r32(BLOCK + ACTOR_PTR);
    let lhs = c.rf(actor + ACT_SPRING_A) + THREE;
    let rhs = c.rf(actor + ACT_SPRING_B);
    if lhs < rhs {
        c.wf(actor + ACT_SPRING_B, lhs);
    }
    leave(c, actor, BLOCK);
    c.fret(lhs);
    c.f[1] = rhs.0;
}

/// 0x1fbc08 hero_hand_pose(): steers the hero hand yaw toward its blend target, rebuilds the pivot frame and
/// clamps the hand weight spring values.
pub fn hero_hand_pose(c: &mut Cpu) -> bool {
    h::with_stack(c, POSE_FRAME, |c, buf| pose_body(c, buf));
    true
}

/// 0x1fbfc8 hand_rate_slew(f12 target, f13 gain, f14, f15 rate): slews the hand rate toward a frame-count target.
pub fn hand_rate_slew(c: &mut Cpu) -> bool {
    let target = c.farg(0);
    let gain = c.farg(1);
    let f14 = c.farg(2);
    let f15 = c.farg(3);
    let cur = c.rf(BLOCK + SRC_AA8);
    let delta = if cur < target {
        target - cur
    } else {
        let f1 = if c.r32(BLOCK + NONZERO_A9C) != 0 {
            F(0x4000_0000)
        } else {
            HALF / c.rf(BLOCK + RATE_A94)
        };
        let actor = c.r32(BLOCK + ACTOR_PTR);
        let index = c.r8(actor + ACT_COUNT) as u32;
        let list = c.r32(actor + ACT_LIST) + index * 4;
        let entry = c.r32(list + 0x48);
        let count = c.r8(entry + 0x10) as i32;
        target + (f1 * F::from_int(count) - cur)
    };
    if F::ZERO < gain {
        let mut rate = c.rf(BLOCK + RATE_A94);
        if c.r32(BLOCK + NONZERO_A9C) != 0 {
            let actor = c.r32(BLOCK + ACTOR_PTR);
            let p = c.r32(actor + ACT_PTR_6C);
            rate = c.rf(p);
        }
        let num = f14 * delta;
        let den = gain * rate;
        let q = num / den;
        if f15 == NEG_ONE {
            c.wf(BLOCK + SLEW_A90, q);
        } else {
            h::run(c, motion::slew_toward, &[BLOCK + SLEW_A90], &[q, f15]);
        }
    }
    true
}

/// 0x1fc2e8 hand_pose_chain(owner): copies the pose-chain block to the stack and runs pose_chain_world.
pub fn hand_pose_chain(c: &mut Cpu) -> bool {
    let mut owner = c.arg(0);
    if owner == 0 {
        owner = c.r32(CHAIN_OWNER_FALLBACK);
    }
    let t0 = c.r32(CHAIN_BLOCK + 0x20);
    h::with_stack(c, 0x40, |c, buf| {
        for i in 0..CHAIN_LEN {
            let b = c.r8(CHAIN_BLOCK + i);
            c.w8(buf + i, b);
        }
        // Retail leaves these scratch registers behind the block loads (ldl/ldr keep the upper half of the GPR).
        c.r[2] = (c.r[2] & !U64_MASK) | (0x001E_0000u64 as u128);
        c.r[9] = (c.r[9] & !U64_MASK) | dword(c, CHAIN_BLOCK + 0x10) as u128;
        c.r[3] = (c.r[3] & !U64_MASK) | dword(c, CHAIN_BLOCK + 0x18) as u128;
        c.r[10] = CHAIN_BLOCK as u128;
        c.s(8, t0 as i32 as i64 as u64);
        // Callee leftovers (v0/v1, a0-a3, t0-t9) stay as after a retail jal.
        h::run_keep(c, actor::pose_chain_world, &[owner, 9, buf, CHAIN_ARG_A3, t0], &[]);
    });
    true
}

/// Reads the 8-byte little-endian doubleword at `addr`.
fn dword(c: &mut Cpu, addr: u32) -> u64 {
    (c.r32(addr) as u64) | ((c.r32(addr + 4) as u64) << 32)
}
