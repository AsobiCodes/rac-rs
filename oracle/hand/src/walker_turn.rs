//! Walker turn-toward-point step: turns the walker's heading toward a target point and reports the state bits.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Offsets into the walker object (`obj`, a0).
const OBJ_POS_X: u32 = 0x10;
const OBJ_POS_Y: u32 = 0x14;
const OBJ_HEADING: u32 = 0x48;

/// Offsets into the turn block (`blk`, a1).
const BLK_ANGLE: u32 = 0x14;
const BLK_TURN_LIMIT: u32 = 0x28;
const BLK_ARRIVE_DIST: u32 = 0x2c;
const BLK_TURN_K: u32 = 0x3c;
const BLK_TURN_D: u32 = 0x40;
const BLK_TURN_MAX: u32 = 0x44;
const BLK_TICKS: u32 = 0x48;

/// State bits returned in v0.
const BIT_ARRIVED: u32 = 0x4;
const BIT_TURNING: u32 = 0x10;
const BIT_STUCK: u32 = 0x2;

/// Scale argument to `float_to_int_scaled` (retail passes 4).
const TICK_SCALE: u32 = 4;

/// Speed threshold above which the stuck bit is cleared (0.1f).
const STUCK_SPEED: F = F(0x3dcc_cccd);

/// Retail function that steps the walker along its path field.
const STEP_FIELD: u32 = 0x24b490;

/// 0x24b968 walker_turn(obj, blk, target, out) -> bits: turns the heading toward the target point, springs the turn
/// block's angle, and returns arrived (4) / turning (0x10) / stuck (2) bits.
pub fn walker_turn(c: &mut Cpu) -> bool {
    let (obj, blk, target, out) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3));
    let mut bits = 0u32;

    h::run(c, crate::euler_basis::zero_vec4, &[out], &[]);

    // Arrived when the target is closer than the block's arrival distance.
    let dist = h::dist3(c, obj + OBJ_POS_X, target);
    if dist < c.rf(blk + BLK_ARRIVE_DIST) {
        bits = BIT_ARRIVED;
    }

    // Heading from the object toward the target (xy plane).
    let dx = c.rf(target) - c.rf(obj + OBJ_POS_X);
    let dy = c.rf(target + 4) - c.rf(obj + OBJ_POS_Y);
    let angle = h::run(c, crate::vu_geom::fast_atan2, &[], &[dx, dy]).1;

    // Spring the block's angle toward the heading.
    let k = c.rf(blk + BLK_TURN_K);
    let d = c.rf(blk + BLK_TURN_D);
    let max = c.rf(blk + BLK_TURN_MAX);
    h::run(c, crate::motion::angle_spring_field, &[obj, blk + BLK_ANGLE], &[angle, k, d, max]);

    // Remaining turn to the heading the object now has.
    let heading = c.rf(obj + OBJ_HEADING);
    let remaining = h::run(c, crate::mathx::angle_dist, &[], &[angle, heading]).1;

    if c.rf(blk + BLK_TURN_LIMIT) <= remaining {
        bits |= BIT_TURNING;
    }

    // Count ticks, capped by the scaled float limit.
    let ticks_ptr = blk + BLK_TICKS;
    h::run(c, crate::mathx::tick_i32, &[ticks_ptr], &[]);
    if remaining < c.rf(blk + BLK_TURN_LIMIT) {
        let mut n = h::run(c, crate::mathx::float_to_int_scaled, &[TICK_SCALE], &[]).0;
        if (c.r32(ticks_ptr) as i32) < n as i32 {
            n = h::run(c, crate::mathx::float_to_int_scaled, &[TICK_SCALE], &[]).0;
            c.w32(ticks_ptr, n);
        }
    }

    if c.r32(ticks_ptr) != 0 {
        // Retail step along the path field, then a stuck check on the remaining turn.
        let ratio = (c.rf(blk + BLK_TURN_LIMIT) - remaining) / c.rf(blk + BLK_TURN_LIMIT);
        let (step, _) = h::call(c, STEP_FIELD, &[obj, blk, target, out], &[ratio]);
        bits |= step;
        if STUCK_SPEED < remaining {
            bits &= !BIT_STUCK;
        }
    }

    c.ret(bits);
    true
}
