//! Camera helpers (the camera struct pointer at 0x166f00; its rig pointer at +0x70).
use crate::ps2::*;
use crate::camobj::{angle_spring, restore, save, spring};
use crate::mathx::acos;
use crate::trig::fast_atan2;
use crate::mathx::scale_int;
use crate::vecmath::{dot, length, lerp, set_length, wrap_once};
use ee_oracle::cpu::Cpu;

/// Pointer to the active camera.
const CAMERA: u32 = 0x166f00;
/// Hero position (16 bytes).
const HERO_POS: u32 = 0x13f3d0;
/// Camera globals block (direction at +0x20, up axis at +0x30).
const CAM_GLOBALS: u32 = 0x166f10;
/// A world point the camera-facing test measures from.
const FACING_ORIGIN: u32 = 0x173f60;
/// 0x15ede4: index into the per-mode float table at gp - 0x4d60.
const MODE_INDEX: u32 = 0x15ede4;
/// The vector the hero's movement adds to the camera each tick.
const NUDGE: u32 = 0x13f490;
/// Camera state block (lock holder at +0x2fc, lock flag at +0x308, mode word at +0x2084).
const CAM_STATE: u32 = 0x13f350;
/// Tick count the lock holder test compares against.
const HOLD_LIMIT: u32 = 0x167118;
/// Stick step magnitudes (x, y) and the flags that keep their sign.
const STEP_X: u32 = 0x13ca40;
const STEP_Y: u32 = 0x13ca44;
const STEP_X_SIGN: u32 = 0x15ede0;
const STEP_Y_SIGN: u32 = 0x15eddc;
/// First-use hint block (counters at +0x40/+0x42, flag bits at +0x44).
const HINTS: u32 = 0x141848;
const HINT_TICKS: u32 = 0x15eea4;
const HINT_SHIFT_B: u32 = 0x15ed84;
/// 0.6981 (40 degrees in radians).
const Y_DIV: F = F(0x3f32_b8c2);
/// Pointer to the default rig (the reset targets come from it), and the two reset constants.
const DEFAULT_RIG: u32 = 0x1690c0;
const RESET_F14: u32 = 0x15ef44;
const RESET_W: u32 = 0x15ef60;
/// Camera boom follow rate.
const BOOM_FOLLOW: u32 = 0x15ef40;
/// 0x1e0a18 point visibility test (pos*, radius) -> nonzero when visible.
const POINT_VISIBLE: u32 = 0x1e0a18;

/// 0x2e73d0 / 0x2e7698 / 0x2e90a0 noop(): empty function.
pub fn noop(_c: &mut Cpu) -> bool {
    true
}

/// 0x2eb5e8 noop_frame(): reserves and frees a stack frame, nothing else.
pub fn noop_frame(_c: &mut Cpu) -> bool {
    true
}

/// 0x2ea790 return_zero() -> 0.
pub fn return_zero(c: &mut Cpu) -> bool {
    c.ret(0);
    true
}

/// 0x2e73d8 follow_hero_pos(_, out): out = hero position.
pub fn follow_hero_pos(c: &mut Cpu) -> bool {
    let v = c.rv(HERO_POS);
    c.wv(c.arg(1), v);
    true
}

/// 0x2e7500 cam_point_visible(pos*) -> whether the point (copied to the stack) is visible with radius 0.
pub fn cam_point_visible(c: &mut Cpu) -> bool {
    let sp = c.g(29) as u32;
    let buf = sp - 0x20;
    c.s(29, buf as i32 as i64 as u64);
    let v = c.rv(c.arg(0));
    c.wv(buf, v);
    c.set_arg(0, buf);
    c.set_farg(0, F::ZERO);
    c.call(POINT_VISIBLE);
    let r = c.result();
    c.s(29, sp as i32 as i64 as u64);
    c.ret((r != 0) as u32);
    true
}

/// 0x2eb5f8 cam_release_owner(owner) -> -1 when the camera is locked (+0x86), 1 when `owner` held the rig
/// (+0x220, now cleared), else 0.
pub fn cam_release_owner(c: &mut Cpu) -> bool {
    let cam = c.r32(CAMERA);
    if c.r16(cam + 0x86) != 0 {
        c.ret(-1i32 as u32);
        return true;
    }
    let rig = c.r32(cam + 0x70);
    let r = if c.r32(rig + 0x220) == c.arg(0) {
        c.w32(rig + 0x220, 0);
        1
    } else {
        0
    };
    c.ret(r);
    true
}

/// 0x2eb6c0 cam_set_override(also_58, a, b): unless the camera is locked, raises the rig's one-shot flag (+0x130+0x3c)
/// and sets +0x130+0x40 = a, +0x130+0x48 = b (and +0x130+0x58 = a when `also_58`).
pub fn cam_set_override(c: &mut Cpu) -> bool {
    let cam = c.r32(CAMERA);
    if c.r16(cam + 0x86) != 0 {
        return true;
    }
    let (a, b) = (c.farg(0), c.farg(1));
    let o = c.r32(cam + 0x70) + 0x130;
    c.w16(o + 0x3c, 1);
    c.wf(o + 0x48, b);
    c.wf(o + 0x40, a);
    if c.arg(0) != 0 {
        c.wf(o + 0x58, a);
    }
    true
}

/// 0x2d9be0 actor_state_is_1(m) -> m+0x20 == 1.
pub fn actor_state_is_1(c: &mut Cpu) -> bool {
    let s = c.r8(c.arg(0) + 0x20);
    c.ret((s == 1) as u32);
    true
}

/// 0x2eac38 cam_facing_away(cam, mode, p) -> mode == 1 and (FACING_ORIGIN - p) points against the rig direction
/// (rig + 0x130).
pub fn cam_facing_away(c: &mut Cpu) -> bool {
    let (cam, mode, p) = (c.arg(0), c.arg(1), c.arg(2));
    let r = mode == 1 && {
        let dir = c.r32(cam + 0x70) + 0x130;
        let (o, p, dir) = (c.rv(FACING_ORIGIN), c.rv(p), c.rv(dir));
        dot(o.with(o - p, XYZ), dir) < F::ZERO
    };
    c.ret(r as u32);
    true
}

/// Retail VU helpers and the hero copy that the camera functions below run as retail does, so the VF and ACC
/// registers they leave match retail's.
const VU_SUB: u32 = 0x1f1840;
const VU_ADD: u32 = 0x1f1810;
const VU_LERP: u32 = 0x1f1870;
const VU_SCALE: u32 = 0x1f1898;
const VU_DOT3: u32 = 0x1f1930;
const VU_LENGTH3: u32 = 0x1f1970;
const VU_SET_LENGTH: u32 = 0x1f1a98;
const HERO_POS_COPY: u32 = 0x2e73d8;
/// Retail rotate (out, in, axis; angle in f12), cross product (out, a, b) and atan2 (f12 = y, f13 = x) helpers.
const VU_ROTATE: u32 = 0x2527c8;
const VU_CROSS3: u32 = 0x1f1958;
const FAST_ATAN2: u32 = 0x1f1e48;

/// Scratch base of retail's frame of `size` bytes (retail's sp after its prologue).
fn frame_base(c: &Cpu, size: u32) -> u32 {
    (c.g(29) as u32).wrapping_sub(size)
}

/// Calls retail `addr` and keeps its scratch registers and VF/ACC as a retail jal leaves them.
fn call_keep(c: &mut Cpu, addr: u32, ints: &[u32], floats: &[F]) -> crate::h::Ret {
    crate::h::call_keep(c, addr, ints, floats)
}

/// 0x2e7b10 follow_held_height(out, h): out = up * (up . (hero + h * dir)), the hero raised by h along the camera
/// direction and projected on the up axis.
pub fn follow_held_height(c: &mut Cpu) -> bool {
    let out = c.arg(0);
    let scratch = frame_base(c, 0x70);
    let hero = scratch + 0x10;
    let (dir, up) = (CAM_GLOBALS + 0x20, CAM_GLOBALS + 0x30);
    call_keep(c, VU_SCALE, &[scratch, dir], &[c.farg(0)]);
    let cam = c.r32(CAMERA);
    call_keep(c, HERO_POS_COPY, &[cam, hero], &[]);
    call_keep(c, VU_ADD, &[scratch, scratch, hero], &[]);
    let d = call_keep(c, VU_DOT3, &[up, scratch], &[]).1;
    call_keep(c, VU_SCALE, &[out, up], &[d]);
    true
}

/// Calls retail `f(cam)`.
fn call1(c: &mut Cpu, f: u32, cam: u32) {
    c.set_arg(0, cam);
    c.call(f);
}

/// 0x2ece20 cam_rig_update(cam): the rig steps of one camera tick.
pub fn cam_rig_update(c: &mut Cpu) -> bool {
    let cam = c.arg(0);
    let sp = c.g(29) as u32;
    let buf = sp - 0x30;
    c.s(29, buf as i32 as i64 as u64);
    call1(c, 0x2ecd10, cam);
    call1(c, 0x2ec1b0, cam);
    input_axes(c, cam, buf, buf | 4);
    let (a, b) = (c.rf(buf), c.rf(buf + 4));
    orbit_tick(c, cam, a, b);
    call1(c, 0x2eaf90, cam);
    follow_tick(c, cam);
    // 0x2eb5e8 is empty.
    call1(c, 0x2ec280, cam);
    c.s(29, sp as i32 as i64 as u64);
    true
}

/// 0x2ece98 cam_update(cam): one camera tick (mode float from the table at gp - 0x4d60 into gp - 0x7cbc, then the
/// follow, rig and output steps).
pub fn cam_update(c: &mut Cpu) -> bool {
    let cam = c.arg(0);
    let gp = c.g(28) as u32;
    let i = c.r32(MODE_INDEX);
    let f = c.rf((gp.wrapping_sub(0x4d60)).wrapping_add(i << 2));
    c.wf(gp.wrapping_sub(0x7cbc), f);
    hero_nudge(c, cam);
    call1(c, 0x2e9270, cam);
    // 0x2e73d0 and 0x2e7698 (cam, rig + 0x1d0) are empty.
    call1(c, 0x2e8aa0, cam);
    c.set_arg(0, cam);
    cam_rig_update(c);
    // 0x2e90a0 is empty.
    rig_reset(c, cam);
    true
}

/// 0x2ecd10 cam_volume_scan(cam): walks the 64 x 64 camera-volume grid and passes each type-5 volume's moby
/// length to 0x1e9498, which is empty on this build: nothing to do (declines if 0x1e9498 is not the empty stub).
pub fn cam_volume_scan(c: &mut Cpu) -> bool {
    c.r32(0x1e9498) == 0x03e0_0008 && c.r32(0x1e949c) == 0
}

/// 0x2ec1b0 cam_boom_follow(cam): while the rig is active (+0x10 > 0), moves the boom vector (rig + 0x130) toward
/// rig[0] - rig[0x90] by the follow rate, limits its length to max (+0x2c) and to max - the previous slack, and
/// stores the new slack max - length at rig + 0x200.
pub fn cam_boom_follow(c: &mut Cpu) -> bool {
    let rig = c.r32(c.arg(0) + 0x70);
    let rig_active = c.r16(rig + 0x10) as i16;
    if rig_active <= 0 {
        c.set_arg(1, rig);
        c.ret(rig_active as i32 as u32);
        return true;
    }
    let (boom, slack_at) = (rig + 0x130, rig + 0x200);
    let buf = frame_base(c, 0x50);
    call_keep(c, VU_SUB, &[buf, rig, rig + 0x90], &[]);
    let rate = c.rf(BOOM_FOLLOW);
    call_keep(c, VU_LERP, &[boom, boom, buf], &[rate]);
    let mut len = call_keep(c, VU_LENGTH3, &[boom], &[]).1;
    let max = c.rf(boom + 0x2c);
    if max < len {
        call_keep(c, VU_SET_LENGTH, &[boom, boom], &[max]);
        len = max;
    } else {
        let t = max - c.rf(slack_at);
        if len < t {
            call_keep(c, VU_SET_LENGTH, &[boom, boom], &[t]);
            len = t;
        }
    }
    let slack = c.rf(boom + 0x2c) - len;
    c.wf(slack_at, slack);
    c.fret(slack);
    true
}

/// gp-relative address (gp - back).
fn gp_addr(c: &Cpu, back: u32) -> u32 {
    (c.g(28) as u32).wrapping_sub(back)
}

/// 0x2e7928 cam_hero_nudge(cam): adds the vector at 0x13f490 to the camera position (cam + 0x30) and to rig
/// points 0xa0, 0x40, 0xd0, 0x00; copies a height (0x13f49c) into rig + 0x1c8 unless the lock holder is a 0x22d moby
/// that is not yet past 30 ticks; while the mode word at 0x1413d4 is 0xb or 0xc, also clears rig + 0x10 and adds
/// (globals[0x80] * globals[0xa4] - the nudge's part perpendicular to the up axis) to the same five points.
pub fn cam_hero_nudge(c: &mut Cpu) -> bool {
    hero_nudge(c, c.arg(0));
    true
}

fn hero_nudge(c: &mut Cpu, cam: u32) {
    // Retail's frame: the nudge copy at +0, temporaries at +0x10 and +0x20. The VU helpers run as retail calls so
    // the VF and ACC registers they leave match retail's.
    let frame = frame_base(c, 0xb0);
    let (nudge_copy, tmp_a, tmp_b) = (frame, frame + 0x10, frame + 0x20);
    let rig = c.r32(cam + 0x70);
    let nudge = c.rv(NUDGE);
    c.wv(nudge_copy, nudge);
    for p in [cam + 0x30, rig + 0xa0, rig + 0x40, rig + 0xd0, rig] {
        call_keep(c, VU_ADD, &[p, nudge_copy, p], &[]);
    }
    let run = if c.r16(CAM_STATE + 0x308) as i16 != 0 {
        false
    } else {
        let holder = c.r32(CAM_STATE + 0x2fc);
        holder == 0 || c.r16(holder + 0xa6) as i16 != 0x22d || {
            let t = scale_int(c, 30);
            t < c.r32(HOLD_LIMIT) as i32
        }
    };
    if run {
        let h = c.rf(CAM_STATE + 0x14c);
        let rig = c.r32(cam + 0x70);
        c.wf(rig + 0x1c8, h);
    }
    if matches!(c.r32(CAM_STATE + 0x2084), 0xb | 0xc) {
        let rig = c.r32(cam + 0x70);
        c.w16(rig + 0x10, 0);
        let scale = c.rf(CAM_GLOBALS + 0xa4);
        call_keep(c, VU_SCALE, &[tmp_a, CAM_GLOBALS + 0x80], &[scale]);
        let k = call_keep(c, VU_DOT3, &[CAM_GLOBALS + 0x30, nudge_copy], &[]).1;
        call_keep(c, VU_SCALE, &[tmp_b, CAM_GLOBALS + 0x30], &[k]);
        call_keep(c, VU_SUB, &[tmp_b, nudge_copy, tmp_b], &[]);
        call_keep(c, VU_SUB, &[tmp_a, tmp_a, tmp_b], &[]);
        for p in [cam + 0x30, rig + 0xa0, rig + 0x40, rig + 0xd0, rig] {
            call_keep(c, VU_ADD, &[p, tmp_a, p], &[]);
        }
    }
}

/// 0x2ebc20 cam_input_axes(cam, out_x, out_y): the camera stick inputs. out_x is 0 while rig flag 1 is set, else
/// +-the step at 0x13ca40 (negated unless 0x15eee0), replaced by rig + 0x1c8 + 0x1c when that is nonzero and the stick
/// is idle (which also restarts the idle counters at gp - 0x4d84/0x4d80); otherwise the rig value at +0x14 follows
/// the mode float gp - 0x7cbc, and after the idle counter passes 60 scaled ticks the first-use bit for the current
/// mode is set (and counters at 0x141848 + 0x40/0x42 updated). out_y is 0 while rig flag 2 is set, else +-the step
/// at 0x13ca44; zero results become rig[0x1c8 + 0x24] / 0.6981.
pub fn cam_input_axes(c: &mut Cpu) -> bool {
    input_axes(c, c.arg(0), c.arg(1), c.arg(2));
    true
}

fn input_axes(c: &mut Cpu, cam: u32, out_x: u32, out_y: u32) {
    let rig = c.r32(cam + 0x70);
    let r = rig + 0x1a8;
    let (idle, armed) = (gp_addr(c, 0x4d84), gp_addr(c, 0x4d80));
    let step = if c.r32(r + 0x10) & 1 != 0 {
        F::ZERO
    } else {
        let s = c.rf(STEP_X);
        if c.r32(STEP_X_SIGN) == 0 {
            -s
        } else {
            s
        }
    };
    c.wf(out_x, step);
    if step == F::ZERO {
        let v = c.rf(r + 0x1c);
        c.w32(idle, 0);
        c.w32(armed, 0);
        if !(v == F::ZERO) {
            c.wf(out_x, v);
        }
    } else {
        let mode = c.rf(gp_addr(c, 0x7cbc));
        c.wf(r + 0x14, mode);
        let hints = HINTS;
        if (c.r16(hints + 0x40) as u32) < 3 {
            let n = c.r32(idle).wrapping_add(1);
            c.w32(idle, n);
            if c.r32(armed) == 0 && (scale_int(c, 60) as i32) < n as i32 {
                let k = c.r16(hints + 0x40) as u32;
                let a0 = if k > 0xfffe {
                    c.r32(gp_addr(c, 0x7d5c))
                } else {
                    c.w16(hints + 0x40, (k + 1) as u16);
                    c.r32(HINT_TICKS)
                };
                let q = scale_int(c, a0 as i32).wrapping_div(600);
                let shift = if (c.r16(hints + 0x42) as i32) < q {
                    let n = c.r32(gp_addr(c, 0x7d5c)) as i32;
                    let t = scale_int(c, n).wrapping_div(600);
                    c.w16(hints + 0x42, t as u16);
                    c.r32(HINT_SHIFT_B)
                } else {
                    c.r32(gp_addr(c, 0x7e7c))
                };
                c.w32(armed, 1);
                let bits = c.r32(hints + 0x44) | (1u32 << (shift & 31)) | 0x8000_0000;
                c.w32(hints + 0x44, bits);
            }
        }
    }
    let y = if c.r32(r + 0x10) & 2 != 0 {
        c.wf(out_y, F::ZERO);
        c.rf(r + 0x24) / Y_DIV
    } else {
        let s = c.rf(STEP_Y);
        c.wf(out_y, -s);
        if c.r32(STEP_Y_SIGN) == 0 {
            c.wf(out_y, s);
        }
        if !(c.rf(out_y) == F::ZERO) {
            return;
        }
        c.rf(r + 0x24) / Y_DIV
    };
    c.wf(out_y, y);
}

/// 0x2e90a8 cam_rig_reset(cam): settles the rig's pending pans onto the defaults: springs the three follow values
/// (rig + 0x130 + 0x2c, +0x30 and rig + 0x40 + 0xb0) toward the explicit targets (set flags at +0x3c, +0x3e, +0xda)
/// or the default rig's values, clears the flags and targets, and restores the rig constants (rig + 0x1a8..,
/// 0x1d0.., 0x220..).
pub fn cam_rig_reset(c: &mut Cpu) -> bool {
    rig_reset(c, c.arg(0));
    true
}

fn rig_reset(c: &mut Cpu, cam: u32) {
    let gp = |c: &mut Cpu, back: u32| {
        let a = gp_addr(c, back);
        c.rf(a)
    };
    let rig = c.r32(cam + 0x70);
    let r = rig + 0x1a8;
    let f = c.rf(RESET_F14);
    c.wf(r + 0x14, f);
    for o in [0x1c, 0x20, 0x10, 0x24] {
        c.w32(r + o, 0);
    }
    let def = c.r32(DEFAULT_RIG);
    let (s0, s1) = (rig + 0x130, rig + 0x40);
    let (d0, d1) = (def + 0x130, def + 0x40);
    // Distance spring.
    let limit = c.rf(r + 0x1c);
    let (a, b) = (c.rf(s0 + 0x2c), if c.r16(s0 + 0x3c) != 0 { c.rf(s0 + 0x40) } else { c.rf(d0 + 0x2c) });
    let (k, damp) = (c.rf(s0 + 0x48), gp(c, 0x4e0c));
    let v = spring(c, rig + 0x174, a, b, k, damp, limit);
    c.wf(s0 + 0x2c, v);
    // Second spring.
    let (a, b) = (c.rf(s0 + 0x30), if c.r16(s0 + 0x3e) != 0 { c.rf(s0 + 0x4c) } else { c.rf(d0 + 0x30) });
    let (k, damp) = (c.rf(s0 + 0x54), gp(c, 0x4e08));
    let v = spring(c, s0 + 0x50, a, b, k, damp, F::ZERO);
    c.wf(s0 + 0x30, v);
    // Third spring.
    let (a, b) = if c.r16(s1 + 0xda) != 0 {
        (c.rf(s1 + 0xb0), c.rf(s1 + 0xb4))
    } else {
        (c.rf(s1 + 0xb0), c.rf(d1 + 0xb0))
    };
    let k = c.rf(s1 + 0xb8);
    let v = spring(c, s1 + 0xbc, a, b, k, F(0x3e4c_cccd), F::ZERO);
    c.wf(s1 + 0xb0, v);
    c.w16(s0 + 0x3c, 0);
    c.w16(s0 + 0x3e, 0);
    c.w32(s0 + 0x40, 0);
    c.w32(s0 + 0x4c, 0);
    let (f2, d) = (gp(c, 0x4e70), c.rf(d0 + 0x2c));
    c.wf(s0 + 0x58, d);
    c.w16(s1 + 0xda, 0);
    c.w32(s1 + 0xb4, 0);
    let rig = c.r32(cam + 0x70);
    c.w16(rig + 0x10, 1);
    let rig2 = c.r32(cam + 0x70);
    c.w16(rig2 + 0x22, 0);
    c.w8(s1 + 0xd6, 0);
    c.wf(s1 + 0xe4, f2);
    c.wf(s1 + 0xe8, F(0x3e4c_cccd));
    c.wf(s1 + 0xdc, f2);
    c.wf(s1 + 0xe0, F(0x3e4c_cccd));
    let (f0, f1, f2) = (gp(c, 0x4e34), gp(c, 0x4e2c), gp(c, 0x4e30));
    let tail = c.r32(RESET_W);
    let o = c.r32(cam + 0x70) + 0x1d0;
    c.wf(o + 0x44, f0);
    c.w32(o + 0x48, tail);
    c.wf(o + 0x3c, f1);
    c.wf(o + 0x40, f2);
    let o = c.r32(cam + 0x70) + 0x220;
    let f = gp(c, 0x4df4);
    c.wf(o + 0xc, f);
    c.w16(o + 6, 0);
}

/// Half pi.
const HALF_PI: F = F(0x3fc9_0fdb);
/// Largest elevation error (radians) the boom is rotated back by directly.
const ELEV_SNAP: F = F(0x3e86_0a92);
/// The camera up axis (CAM_GLOBALS + 0x30).
const UP_AXIS: u32 = 0x166f40;

/// (b x a) in retail operand order, as 0x1f1958 (w = 0; nothing downstream reads it).
fn cross_ba(a: V, b: V) -> V {
    let x = b.y().vu_mul(a.z()).vu_sub(b.z().vu_mul(a.y()));
    let y = b.z().vu_mul(a.x()).vu_sub(b.x().vu_mul(a.z()));
    let z = b.x().vu_mul(a.y()).vu_sub(b.y().vu_mul(a.x()));
    V::ZERO_W1.with_xyz(x, y, z).with(V::splat(F::ZERO), W)
}

/// 0x2527c8 rotate(v, axis, angle) -> v turned about `axis` (retail routine run on scratch stack memory).
fn rotate(c: &mut Cpu, buf: u32, v: V, axis: V, angle: F) -> V {
    c.wv(buf, v);
    c.wv(buf + 0x10, axis);
    c.set_arg(0, buf + 0x20);
    c.set_arg(1, buf);
    c.set_arg(2, buf + 0x10);
    c.set_farg(0, angle);
    c.call(0x2527c8);
    c.rv(buf + 0x20)
}

/// 0x2ec990 follow_tick(cam): the follow camera's boom step. Moves rig[0] to rig[0x90] + boom, springs the boom
/// length (rig + 0x158), pitch (rig + 0x154) and yaw (rig + 0x150) toward their targets, rebuilds the look vector
/// (rig + 0x140) by rotating it, snaps small pitch errors, and sets cam + 0x30 = rig[0xa0] + dir * boom[0x30] +
/// boom[0x10].
pub fn follow_tick_hand(c: &mut Cpu) -> bool {
    follow_tick(c, c.arg(0));
    true
}

fn follow_tick(c: &mut Cpu, cam: u32) {
    let sv = save(c);
    let sp = c.g(29) as u32;
    let buf = sp - 0x80;
    c.s(29, buf as i32 as i64 as u64);
    let rig = c.r32(cam + 0x70);
    let (boom_p, look_p, dir_p) = (rig + 0x130, rig + 0x140, CAM_GLOBALS + 0x20);
    let (a, b) = (c.rv(rig + 0x90), c.rv(boom_p));
    c.wv(rig, a.with(a + b, XYZ));
    let slack = c.rf(rig + 0x1d0 + 0x30);
    let max_len = c.rf(boom_p + 0x2c) - slack;
    let len = length(c.rv(look_p));
    c.set_arg(0, cam);
    c.set_arg(1, buf + 0x30);
    c.set_arg(2, buf + 0x34);
    c.call(0x2ec788);
    let (k, damp) = (c.rf(buf + 0x30), c.rf(buf + 0x34));
    let len = spring(c, rig + 0x158, len, max_len, k, damp, F::ZERO);
    let (dir, boom, look) = (c.rv(dir_p), c.rv(boom_p), c.rv(look_p));
    let along_boom = dot(boom, dir);
    let tilt = acos(c, along_boom / max_len);
    let along_look = dot(look, dir);
    let tilt_now = acos(c, along_look / len);
    let (k, damp) = (c.rf(gp_addr(c, 0x4e14)), c.rf(gp_addr(c, 0x4e10)));
    let pitch = angle_spring(c, rig + 0x154, tilt_now, tilt, k, damp, F::ZERO);
    let flat_boom = boom.with(boom - dir.with(dir * along_boom, XYZ), XYZ);
    let flat_look = look.with(look - dir.with(dir * along_look, XYZ), XYZ);
    let cos = dot(flat_boom, flat_look);
    let flat = length(flat_boom) * length(flat_look);
    let yaw_err = HALF_PI - acos(c, cos / flat);
    let (k, damp) = (c.rf(gp_addr(c, 0x4e1c)), c.rf(gp_addr(c, 0x4e18)));
    let yaw = angle_spring(c, rig + 0x150, F::ZERO, yaw_err, k, damp, F::ZERO);
    let side = cross_ba(flat_look, dir);
    let sign = if F::ZERO <= dot(side, flat_boom) { F::ONE } else { -F::ONE };
    let turned = rotate(c, buf, flat_look, dir, yaw * sign);
    let axis = cross_ba(dir, turned);
    let turned = rotate(c, buf, turned, axis, pitch);
    let new_look = set_length(turned, len);
    c.wv(look_p, new_look);
    let lifted = dir.with(dir * c.rf(boom_p + 0x30), XYZ);
    let pos = lifted.with(lifted + c.rv(rig + 0xa0), XYZ);
    let up = c.rv(UP_AXIS);
    let along_up = dot(new_look, up);
    let l = length(new_look);
    if !(l == F::ZERO) {
        let elev = HALF_PI - acos(c, along_up / l);
        if elev.abs() < ELEV_SNAP {
            let axis = cross_ba(new_look, up);
            let ang = wrap_once(ELEV_SNAP - elev.abs());
            let r = rotate(c, buf, new_look, axis, ang);
            c.wv(look_p, r);
            c.wf(boom_p + 0x24, F::ZERO);
            c.wf(boom_p + 0x28, F::ZERO);
            c.wf(boom_p + 0x20, F::ZERO);
        }
    }
    let off = c.rv(boom_p + 0x10);
    c.wv(cam + 0x30, pos.with(pos + off, XYZ));
    c.s(29, sp as i32 as i64 as u64);
    restore(c, &sv);
}

/// 0.3: dead zone of the stick axis, and 1 / (1 - 0.3) the rescale beyond it.
const DEAD_ZONE: F = F(0x3e99_999a);
const DEAD_RESCALE: F = F(0x3fb6_db6e);

/// 0x2ebe28 orbit_tick(cam, x_in, y_in): the orbit camera's yaw/pitch step. Springs rig[0x1a8] toward x_in,
/// rescales y_in past the dead zone, springs rig[0x1b0] (pitch rate, limited depending on whether the stick pulls
/// against the motion), rotates the boom (rig + 0x130) about the camera direction by the pitch or rig[0x1c8]
/// rate, derives the boom's tilt angle and stores the signed turn (rig + 0x168) it needs, then rotates the boom
/// about the cross axis by it.
pub fn orbit_tick_hand(c: &mut Cpu) -> bool {
    orbit_tick(c, c.arg(0), c.farg(0), c.farg(1));
    true
}

fn orbit_tick(c: &mut Cpu, cam: u32, a: F, b: F) {
    let sv = save(c);
    // Retail's frame (sp - 0xa0); the side vector lives at its base.
    let sp = c.g(29) as u32;
    let frame = sp.wrapping_sub(0xa0);
    c.s(29, frame as i32 as i64 as u64);
    let rig = c.r32(cam + 0x70);
    let (r, boom_p, dir_p) = (rig + 0x1a8, rig + 0x130, CAM_GLOBALS + 0x20);
    let (k, damp) = (c.rf(gp_addr(c, 0x4e64)), c.rf(gp_addr(c, 0x4e60)));
    let (limit_slow, limit_fast) = (c.rf(gp_addr(c, 0x4e50)), c.rf(gp_addr(c, 0x4e54)));
    let cur = c.rf(r);
    let v = spring(c, r + 4, cur, a, k, damp, F::ZERO);
    c.wf(r, v);
    let g = c.rf(r + 0x14);
    c.wf(boom_p + 0x34, g * v);
    let b = if b < -DEAD_ZONE {
        (b + DEAD_ZONE) * DEAD_RESCALE
    } else if DEAD_ZONE < b {
        (b - DEAD_ZONE) * DEAD_RESCALE
    } else {
        F::ZERO
    };
    let cur = c.rf(r + 8);
    let mut limit = limit_slow;
    if !(b == F::ZERO) && !(b.abs() < cur.abs() && (F::ZERO < b) == (F::ZERO < cur) && (b < F::ZERO) == (cur < F::ZERO)) {
        limit = limit_fast;
    }
    let v = spring(c, r + 0xc, cur, b, k, damp, limit);
    c.wf(r + 8, v);
    let pitch = v * Y_DIV;
    let ang = if a == F::ZERO { c.rf(r + 0x20) } else { c.rf(boom_p + 0x34) };
    call_keep(c, VU_ROTATE, &[boom_p, boom_p, dir_p], &[ang]);
    let along = call_keep(c, VU_DOT3, &[boom_p, dir_p], &[]).1;
    let len = call_keep(c, VU_LENGTH3, &[boom_p], &[]).1;
    call_keep(c, VU_CROSS3, &[frame, dir_p, boom_p], &[]);
    call_keep(c, VU_SET_LENGTH, &[frame, frame], &[F::ONE]);
    let tilt = call_keep(c, FAST_ATAN2, &[], &[len, along]).1;
    if tilt.abs() < pitch.abs() && limit == limit_slow {
        c.wf(r + 8, tilt / Y_DIV);
    }
    let turn = if tilt < pitch {
        let up = wrap_once(tilt + max_turn(c));
        if pitch < up {
            wrap_once(pitch - tilt)
        } else {
            max_turn(c)
        }
    } else if pitch < tilt {
        let rig2 = c.r32(cam + 0x70);
        if (c.r16(rig2 + 0x208) as i16) < 0 {
            F::ZERO
        } else {
            let down = wrap_once(tilt + -max_turn(c));
            if down < pitch {
                wrap_once(pitch - tilt)
            } else {
                -max_turn(c)
            }
        }
    } else {
        F::ZERO
    };
    c.wf(boom_p + 0x38, turn);
    if pitch < F::ZERO {
        let rig2 = c.r32(cam + 0x70);
        c.w16(rig2 + 0x22, 1);
    }
    call_keep(c, VU_ROTATE, &[boom_p, boom_p, frame], &[turn]);
    c.s(29, sp as i32 as i64 as u64);
    restore(c, &sv);
}

/// The largest boom turn per tick (gp - 0x4e4c).
fn max_turn(c: &mut Cpu) -> F {
    let a = gp_addr(c, 0x4e4c);
    c.rf(a)
}
