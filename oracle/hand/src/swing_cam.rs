//! Class-7 swing camera: per-frame update (retail 0x2f0020) and the reset/init helpers (0x2efd08, 0x2efd38).

use crate::h;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// First retail helper of the swing reset (called with the camera pointer).
const SWING_PRE_HELPER: u32 = 0x2e_f500;
/// Second retail helper of the swing reset (called with the camera pointer).
const SWING_POST_HELPER: u32 = 0x2e_fb38;
/// Retail helper that the update calls first with the camera pointer.
const SWING_SETUP_HELPER: u32 = 0x2e_f268;
/// Retail helper that the update calls last with the anchor pointer and 0.5.
const SWING_STEP_HELPER: u32 = 0x1d_ff10;

/// Global data table (0x140000 - 0xcb0); its words at +0x2084 hold the hero state.
const TABLE: u32 = 0x13_F350;
const TABLE_HERO_STATE: u32 = 0x2084;
/// Anchor word compared when the hero is in the class-7 state (0x2c).
const TABLE_ANCHOR_CLASS7: u32 = 0x994;
/// Anchor word compared in the other swing states.
const TABLE_ANCHOR_OTHER: u32 = 0x964;

/// Hero state that selects the class-7 anchor path.
const HERO_STATE_CLASS7: u32 = 0x2c;
/// First of the three swing states (0x24..=0x26) that take the other anchor path.
const HERO_STATE_SWING_LO: u32 = 0x24;
const HERO_STATE_SWING_SPAN: u32 = 3;

/// Camera object offsets: the pose block at +0x0/+0x10/+0x20 and the anchor pose at +0x30.
const CAM_POSE: u32 = 0x0;
const CAM_POSE_B: u32 = 0x10;
const CAM_POSE_C: u32 = 0x20;
const CAM_ANCHOR: u32 = 0x30;
/// Pointer to the swing record in the camera object.
const CAM_RECORD: u32 = 0x70;
/// Anchor id word compared against the table, and the record the update reads it from.
const RECORD_ANCHOR_CLASS7: u32 = 0xc0;
const RECORD_ANCHOR: u32 = 0x8;
/// Word the update clears at the end (camera record +0x80).
const RECORD_CLEAR: u32 = 0x80;

/// Global scratch block (0x160000 + 0x6d80) with the saved-pose pointer at +0x184 and the result word at +0x2f4.
const SCRATCH_BLOCK: u32 = 0x16_6D80;
const SCRATCH_POSE_PTR: u32 = 0x184;
const SCRATCH_RESULT: u32 = 0x2f4;
/// Argument passed to the float-to-int helper.
const SCALE_ARG: u32 = 0x2d;
/// Float passed to the step helper (0.5).
const HALF: F = F(0x3f00_0000);

/// Swing state block written by `swing_cam_init` (0x160000 + 0x7100).
const SWING_BLOCK: u32 = 0x16_7100;
/// Limit float written into the swing block (10000.0).
const SWING_LIMIT: F = F(0x461c_4000);
/// Value stored at +0x10 of the swing block: 0x2f0000 - 0x290.
const SWING_LINK: u32 = 0x2f_0000 - 0x290;

/// 0x2efd08 swing_cam_reset(cam) -> void: runs the two swing helpers on the camera object.
pub fn swing_cam_reset(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let cam = c.arg(0);
    h::call_keep(c, SWING_PRE_HELPER, &[cam], &[]);
    h::call_keep(c, SWING_POST_HELPER, &[cam], &[]);
    true
}

/// 0x2efd38 swing_cam_init() -> void: zeroes the swing state words and stores the 10000.0 limit (ignores its args).
pub fn swing_cam_init(c: &mut Cpu) -> bool {
    c.w32(SWING_BLOCK, 0);
    c.w32(SWING_BLOCK + 4, 0);
    c.wf(SWING_BLOCK + 8, SWING_LIMIT);
    c.w32(SWING_BLOCK + 0xc, 0);
    c.w32(SWING_BLOCK + 0x10, SWING_LINK);
    c.fret(SWING_LIMIT);
    c.ret(SWING_LINK);
    true
}

/// True when the hero state selects the class-7 anchor path (0x2c) or one of the other swing states (0x24..=0x26).
fn swing_state_ok(state: u32) -> Option<bool> {
    if state == HERO_STATE_CLASS7 {
        Some(true)
    } else if state.wrapping_sub(HERO_STATE_SWING_LO) < HERO_STATE_SWING_SPAN {
        Some(false)
    } else {
        None
    }
}

/// 0x2f0020 swing_cam_update(cam) -> cam record: re-setup on an anchor change, step the swing, sweep the sphere.
pub fn swing_cam_update(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let cam = c.arg(0);
    let record = c.r32(cam + CAM_RECORD);
    let state = c.r32(TABLE + TABLE_HERO_STATE);
    let class7 = match swing_state_ok(state) {
        Some(class7) => class7,
        None => {
            swing_cam_finish(c, cam);
            return true;
        }
    };
    let (anchor_now, anchor_saved) = if class7 {
        (c.r32(record + RECORD_ANCHOR_CLASS7), c.r32(TABLE + TABLE_ANCHOR_CLASS7))
    } else {
        (c.r32(record + RECORD_ANCHOR), c.r32(TABLE + TABLE_ANCHOR_OTHER))
    };
    if anchor_now != anchor_saved {
        resetup_anchor(c, cam);
    }
    swing_cam_finish(c, cam);
    true
}

/// Anchor changed: copy the camera poses into the saved-pose block, then run the setup and the float-to-int helper.
fn resetup_anchor(c: &mut Cpu, cam: u32) {
    let block = c.r32(SCRATCH_BLOCK + SCRATCH_POSE_PTR);
    copy_quad(c, block + CAM_ANCHOR, cam + CAM_ANCHOR);
    copy_quad(c, block + CAM_POSE, cam + CAM_POSE);
    copy_quad(c, block + CAM_POSE_B, cam + CAM_POSE_B);
    copy_quad(c, block + CAM_POSE_C, cam + CAM_POSE_C);
    h::call(c, SWING_SETUP_HELPER, &[cam], &[]);
    let (scaled, _) = h::run(c, mathx::float_to_int_scaled, &[SCALE_ARG], &[]);
    c.w32(SCRATCH_BLOCK + SCRATCH_RESULT, scaled);
}

/// Tail of the update: the reset helper, then the step helper on the anchor pose, then clears the record word.
fn swing_cam_finish(c: &mut Cpu, cam: u32) {
    h::run(c, swing_cam_reset, &[cam], &[]);
    h::call_keep(c, SWING_STEP_HELPER, &[cam + CAM_ANCHOR], &[HALF]);
    let record = c.r32(cam + CAM_RECORD);
    c.w32(record + RECORD_CLEAR, 0);
    c.ret(record);
}

/// Copies one 16-byte quadword from `src` to `dst`.
fn copy_quad(c: &mut Cpu, dst: u32, src: u32) {
    let q = c.rv(src);
    c.wv(dst, q);
}
