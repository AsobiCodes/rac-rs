//! Pose-slot reset (retail 0x2925e0) and the state-driven animation start (retail 0x2997a0).
use crate::h;
use crate::ps2::*;
use crate::{actor, mathx, objects, trig};
use ee_oracle::cpu::Cpu;

/// Size of one pose slot in both slot arrays.
const SLOT_STRIDE: u32 = 0xb0;
/// The slot loop runs while the cursor stays below base + SLOT_SPAN (two slots).
const SLOT_SPAN: u32 = 0x160;
/// First quaternion array (offset from the object base) and the flag byte that selects it.
const QUAT_FIRST: u32 = 0x130;
const FLAG_FIRST: u32 = 0x121;
/// Second quaternion array and its flag byte.
const QUAT_SECOND: u32 = 0x170;
const FLAG_SECOND: u32 = 0x161;
/// Words cleared on every slot.
const CLEAR_104: u32 = 0x104;
const CLEAR_FC: u32 = 0xfc;
/// Axis-quaternion helper arguments: a1 = 2, f12 = 0.0.
const AXIS_KIND: u32 = 2;
const ZERO: F = F(0);
/// Return addresses of the two axis_quat jal sites (0x292618, 0x292630); the retail callee leaves the last one in t7
/// (r15), which the hand's nested call does not.
const RA_FIRST_CALL: u32 = 0x29_2620;
const RA_SECOND_CALL: u32 = 0x29_2638;

/// Writes the low 64 bits of t7 (r15) as the retail axis_quat leaves them, keeping the upper half.
fn set_axis_ra(c: &mut Cpu, ra: u32) {
    c.r[15] = (c.r[15] & !(u64::MAX as u128)) | ra as u128;
}

/// 0x2925e0 pose_slots_reset(base): for each of the two slots, re-derives the axis quaternion of any flagged array
/// (trig::axis_quat with 0.0 and kind 2), then clears the slot words at +0x104 and +0xfc. v0 = 0 on return.
pub fn pose_slots_reset(c: &mut Cpu) -> bool {
    let base = c.arg(0);
    let mut cur = base;
    loop {
        let off = cur - base;
        if c.r8(cur + FLAG_SECOND) != 0 {
            h::run_keep(c, trig::axis_quat, &[base + QUAT_SECOND + off, AXIS_KIND], &[ZERO]);
            set_axis_ra(c, RA_FIRST_CALL);
        }
        if c.r8(cur + FLAG_FIRST) != 0 {
            h::run_keep(c, trig::axis_quat, &[base + QUAT_FIRST + off, AXIS_KIND], &[ZERO]);
            set_axis_ra(c, RA_SECOND_CALL);
        }
        c.w32(cur + CLEAR_104, 0);
        c.w32(cur + CLEAR_FC, 0);
        cur += SLOT_STRIDE;
        if cur >= base + SLOT_SPAN {
            break;
        }
    }
    c.ret(0);
    true
}

/// Global block of the animation state (lui 0x14 + addiu -0xcb0).
const STATE_BASE: u32 = 0x13_f350;
/// Offsets in the global block: pointer to the source record, and two mode words.
const STATE_SRC: u32 = 0x2080;
const STATE_MODE_A: u32 = 0x2084;
const STATE_MODE_B: u32 = 0x2090;
/// Flag word that gates the second animation start.
const STATE_FLAG: u32 = 0x198;
/// Mode value that enables the animation paths.
const MODE_ACTIVE: u32 = 0x3b;
/// Animation ids: the default, and the two chosen from the source kind byte (0x40 / 0x41).
const ANIM_DEFAULT: u32 = 0xb;
const ANIM_KIND_40: u32 = 0x13;
const ANIM_KIND_41: u32 = 0x12;
/// Scale inputs of float_to_int_scaled on the two paths.
const SCALE_DEFAULT: u32 = 7;
const SCALE_FLAGGED: u32 = 0x11;
/// Animation id of the flagged start.
const ANIM_FLAGGED: u32 = 1;
/// Record offsets in the source record and the object.
const REC_KIND: u32 = 0x53;
const REC_KIND_COPY: u32 = 0x52;
const REC_TAG: u32 = 0x50;
const REC_TAG_NEXT: u32 = 0x51;
const REC_SCALE: u32 = 0x54;

/// 0x2997a0 anim_state_start(obj): when the mode word is 0x3b, copies the source record into obj (for animation kinds
/// 0x40/0x41) and loads its tracks, otherwise starts the animation with a scaled id. A second mode word and flag
/// select a flagged start. v0 is the last callee's result, or the mode/flag value on the paths without a call.
pub fn anim_state_start(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    if c.r32(STATE_BASE + STATE_MODE_A) == MODE_ACTIVE {
        let src0 = c.r32(STATE_BASE + STATE_SRC);
        let kind = c.r8(src0 + REC_KIND);
        let anim = match kind {
            0x40 => ANIM_KIND_40,
            0x41 => ANIM_KIND_41,
            _ => ANIM_DEFAULT,
        };
        if anim == ANIM_KIND_40 || anim == ANIM_KIND_41 {
            c.w8(obj + REC_KIND, anim as u8);
            c.w8(obj + REC_KIND_COPY, anim as u8);
            let src = c.r32(STATE_BASE + STATE_SRC);
            let scale = c.r32(src + REC_SCALE);
            c.w32(obj + REC_SCALE, scale);
            let tag = c.r8(src + REC_TAG);
            c.w8(obj + REC_TAG, tag);
            let next = c.r8(src + REC_TAG_NEXT);
            c.w8(obj + REC_TAG_NEXT, next);
            let src = c.r32(STATE_BASE + STATE_SRC);
            if c.r8(src + REC_KIND_COPY) != c.r8(src + REC_KIND) {
                c.w8(obj + REC_TAG, next);
            }
            h::run_keep(c, actor::anim_load_tracks, &[obj], &[]);
        } else {
            let scaled = h::run_keep(c, mathx::float_to_int_scaled, &[SCALE_DEFAULT], &[]).0;
            h::run_keep(c, objects::anim_start, &[obj, anim, 0, scaled], &[]);
        }
    } else {
        let mode_b = c.r32(STATE_BASE + STATE_MODE_B);
        if mode_b == MODE_ACTIVE {
            if c.r32(STATE_BASE + STATE_FLAG) == ANIM_FLAGGED {
                let scaled = h::run_keep(c, mathx::float_to_int_scaled, &[SCALE_FLAGGED], &[]).0;
                h::run_keep(c, objects::anim_start, &[obj, ANIM_FLAGGED, 0, scaled], &[]);
            } else {
                c.ret(ANIM_FLAGGED);
            }
        } else {
            c.ret(mode_b);
        }
    }
    true
}
