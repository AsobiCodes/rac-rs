use crate::h;

use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Scale factor passed to the retail float_to_int_scaled (0x1f14b8) for the 180-unit step.
const SCALE_STEP: u32 = 180;
/// Scale factor for the 5-unit comparison.
const SCALE_FAST: u32 = 5;
/// Scale factor for the per-digit comparisons.
const SCALE_DIGIT: u32 = 8;
/// Slide threshold (signed): the snapped value is refreshed once the counter reaches this.
const SLIDE_LIMIT: i32 = 0x18;

/// Object field offsets.
const OFF_SOURCE_PTR: u32 = 0x0c;
const OFF_LIMIT: u32 = 0x08;
const OFF_SHOWN: u32 = 0x74;
const OFF_TARGET: u32 = 0x78;
const OFF_COUNTER: u32 = 0x6c;
const OFF_SCALED: u32 = 0x7c;
const OFF_DIGITS: u32 = 0x70;
const OFF_ICON: u32 = 0x40;
/// Retail icon_anim (0x22dbf0), run as retail so its scratch residue matches.
const ICON_ANIM: u32 = 0x22dbf0;

/// Scales an integer with the retail float_to_int_scaled, keeping the scratch registers it leaves.
fn scaled(c: &mut Cpu, n: u32) -> i32 {
    let (v0, _) = h::run_keep(c, mathx::float_to_int_scaled, &[n], &[]);
    v0 as i32
}

/// 0x229268 hud_counter_update_snap(obj) -> frame: moves the displayed counter toward its source value (clamped to
/// the limit at +8), snapping once the counter reaches the slide threshold, then steps the two fade bytes at +0x70
/// toward their targets and advances the icon animation at +0x40.
pub fn hud_counter_update_snap(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let source_ptr = c.r32(obj + OFF_SOURCE_PTR);
    if source_ptr != 0 {
        let value = (c.r32(source_ptr) as i32).max(0);
        let limit = c.r32(obj + OFF_LIMIT) as i32;
        let target = if limit < value { limit } else { value };
        c.w32(obj + OFF_TARGET, target as u32);
    }

    if c.r32(obj + OFF_SHOWN) != c.r32(obj + OFF_TARGET) {
        let scaled_step = scaled(c, SCALE_STEP);
        c.w32(obj + OFF_SCALED, scaled_step as u32);
        if (c.r32(obj + OFF_COUNTER) as i32) >= SLIDE_LIMIT {
            let target = c.r32(obj + OFF_TARGET);
            c.w32(obj + OFF_SHOWN, target);
        }
    }

    let fast = scaled(c, SCALE_FAST);
    let digits = obj + OFF_DIGITS;
    if (c.r32(obj + OFF_SCALED) as i32) < fast {
        // Counter is behind: bump the counter flag and step the digits down.
        let high = c.r8(digits + 1);
        let low = c.r8(digits);
        if high != 0 || low != 0 {
            c.w32(obj + OFF_COUNTER, 1);
        }
        if high != 0 {
            c.w8(digits + 1, high.wrapping_sub(1));
        } else if low != 0 {
            c.w8(digits, low.wrapping_sub(1));
        }
    } else {
        // Counter is ahead: step the digits up, low digit first.
        let low = c.r8(digits);
        let limit = scaled(c, SCALE_DIGIT);
        if (low as i32) < limit {
            c.w8(digits, low.wrapping_add(1));
        } else {
            let high = c.r8(digits + 1);
            let limit = scaled(c, SCALE_DIGIT);
            if (high as i32) < limit {
                c.w8(digits + 1, high.wrapping_add(1));
            }
        }
    }

    let (frame, _) = h::call_keep(c, ICON_ANIM, &[obj + OFF_ICON], &[]);
    c.ret(frame as u32);
    true
}
