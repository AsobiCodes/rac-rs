//! Camera view tracking for the L4 overlay: the look-at slews and spring, two 32-entry history rings, and the
//! view matrix rebuild.
use crate::h;
use crate::hero_anim;
use crate::motion;
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// View state block (retail $a2 / $s1).
const VIEW: u32 = 0x0013_F350;
/// Hero block read by view_frame (retail $s0 in 0x204718).
const HERO: u32 = 0x0013_C940;
/// Xy vector of the view speed; view_frame stores it here and passes it to length_xy.
const SPEED_VEC: u32 = VIEW + 0x1d20;
/// Vector whose xy length clamped_hero_len caps at 1.0.
const HERO_XY: u32 = 0x0014_1070;
/// Float constants in the overlay data: slew rate, spring stiffness, spring max.
const SLEW_RATE: u32 = 0x0015_ED60;
const SPRING_RATE: u32 = 0x0015_ED64;
const SPRING_MAX: u32 = 0x0015_ED6C;

/// Byte selecting the look-at targets.
const MODE: u32 = 0x20a4;
/// State ids compared against 4, 0x11, 0x12, 0x7f and 6.
const STATE_A: u32 = 0x208c;
const STATE_B: u32 = 0x2084;
const STATE_C: u32 = 0x2094;
/// Counter and its limit used by the state-4 branch.
const COUNTER: u32 = 0x420;
const LIMIT: u32 = 0x198;
/// Halfword gate for the state-4 branch.
const GATE_HALF: u32 = 0x41e;
/// Byte gate that enables the distance test.
const FLAG: u32 = 0x257;

/// Look-at slew targets: 0x228 and 0x22c are slewed toward, 0x230 is the spring target.
const TGT_A: u32 = 0x228;
const TGT_B: u32 = 0x22c;
const TGT_C: u32 = 0x230;
/// Slewed values (0x220 chases TGT_A, 0x224 chases TGT_B) and the spring value and velocity.
const SLEW_A: u32 = 0x220;
const SPRING_POS: u32 = 0x234;
const SPRING_VEL: u32 = 0x238;

/// History rings: 32 floats at 0x2128 (index 0x21a8, count 0x21ac) and 32 quadwords at 0x1b00 (index 0x21b0,
/// count 0x21b4).
const RING_F_BASE: u32 = 0x2128;
const RING_F_IDX: u32 = 0x21a8;
const RING_F_COUNT: u32 = 0x21ac;
const RING_Q_IDX: u32 = 0x21b0;
const RING_Q_COUNT: u32 = 0x21b4;
const RING_Q_BASE: u32 = 0x1b00;

/// Look-at targets (0x228, 0x22c, 0x230) per mode byte; other modes leave them unchanged.
fn mode_targets(mode: u8) -> Option<[F; 3]> {
    match mode {
        0 => Some([F(0x3f4c_cccd), F(0x3f33_3333), F(0x3ee6_6666)]),
        1 => Some([F(0x3f19_9999), F(0x3ee6_6667), F(0x3e99_999a)]),
        2 => Some([F(0x40a8_0000), F(0x408e_6666), F(0x4070_0000)]),
        3 => Some([F(0x3f4c_cccd), F(0x3f19_999a), F(0x3ee6_6666)]),
        _ => None,
    }
}

/// Sets at most one look-at target from the state ids (retail 0x204530..0x2045f8).
fn state_targets(c: &mut Cpu) {
    let state = c.r32(VIEW + STATE_A);
    if state == 4 {
        let counter = c.r32(VIEW + COUNTER) as i32;
        let limit = c.r32(VIEW + LIMIT) as i32;
        if counter < limit && c.r16(VIEW + GATE_HALF) == 0 {
            let v = c.rf(VIEW + 0x434);
            c.wf(VIEW + TGT_B, v);
        }
        return;
    }
    let kind = c.r32(VIEW + STATE_B);
    if kind == 6 {
        c.wf(VIEW + TGT_B, F(0x3f00_0000));
        return;
    }
    if kind == 4 {
        c.wf(VIEW + TGT_A, F(0x3eb3_3334));
        return;
    }
    if state.wrapping_sub(0x11) < 2 {
        c.w32(VIEW + TGT_B, 0);
        c.w32(VIEW + TGT_A, 0);
        return;
    }
    let flag = c.r8(VIEW + 0x12e2);
    if flag != 0 && c.r32(VIEW + 0x300) != 0 {
        c.wf(VIEW + TGT_A, F(0x3f4c_cccd));
        c.wf(VIEW + TGT_B, F(0x3f66_6666));
        return;
    }
    if c.r32(VIEW + STATE_B) == 0x7f {
        c.wf(VIEW + TGT_C, F(0x3f4c_cccd));
    }
}

/// 0x204428 smooth_view_targets(): picks the look-at targets for the mode, slews two values toward them and steps
/// the spring. Returns the spring's remaining distance (f0), or the xy distance when the early gate exits.
pub fn smooth_view_targets(c: &mut Cpu) -> bool {
    let mode = c.r8(VIEW + MODE);
    if let Some([a, b, d]) = mode_targets(mode) {
        c.wf(VIEW + TGT_A, a);
        c.wf(VIEW + TGT_B, b);
        c.wf(VIEW + TGT_C, d);
    }
    state_targets(c);
    let gate_flag = c.r8(VIEW + FLAG) != 0;
    if gate_flag
        && c.r32(VIEW + STATE_C) != 0x12
        && c.r32(VIEW + STATE_A) != 0x11
        && c.r8(VIEW + 0x12e4) == 0
    {
        let dist = h::run(c, vecmath::dist_xy, &[VIEW + 0x210, VIEW + 0x80], &[]).1;
        let half = c.rf(VIEW + SPRING_POS) * F(0x3f00_0000);
        if !(half < dist) {
            c.fret(dist);
            return true;
        }
    }
    let step = c.rf(SLEW_RATE) * F(0x3ca3_d70a);
    let target_a = c.rf(VIEW + TGT_A);
    motion::slew(c, VIEW + SLEW_A, target_a, step);
    let target_b = c.rf(VIEW + TGT_B);
    motion::slew(c, VIEW + SLEW_A + 4, target_b, step);
    let spring_k = c.rf(SPRING_RATE);
    let rate = spring_k * F(0x3ca3_d70a);
    let damp = spring_k * F(0x3e99_999a);
    let max = c.rf(SPRING_MAX) * F(0x4080_0000);
    let target_c = c.rf(VIEW + TGT_C);
    let rest = motion::spring_settle(c, VIEW + SPRING_POS, VIEW + SPRING_VEL, target_c, rate, damp, max);
    c.fret(rest);
    true
}

/// Advances a 32-entry ring index (wrapping) and its count (saturating at 32); returns the index it had before.
fn ring_advance(c: &mut Cpu, idx_off: u32, count_off: u32) -> u32 {
    let idx = c.r32(VIEW + idx_off) as i32;
    let next = idx + 1;
    let base = if -1 < next { next } else { idx + 0x20 };
    let new_idx = next - ((base >> 5) << 5);
    let count = c.r32(VIEW + count_off) as i32 + 1;
    c.w32(VIEW + count_off, count as u32);
    c.w32(VIEW + idx_off, new_idx as u32);
    if count >= 0x21 {
        c.w32(VIEW + count_off, 0x20);
    }
    idx as u32
}

/// 0x2048d8 clamped_hero_len(): min(xy length of the vector at 0x141070, 1.0).
pub fn clamped_hero_len(c: &mut Cpu) -> bool {
    let len = h::run(c, vecmath::length_xy, &[HERO_XY], &[]).1;
    c.fret(if F::ONE < len { F::ONE } else { len });
    true
}

/// 0x204718 view_frame(): stores the wrench swing count and the clamped speed, pushes the speed and the view
/// vector into their history rings, and rebuilds the view matrix.
pub fn view_frame(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let swings = h::run(c, hero_anim::wrench_swing_count, &[], &[]).0;
    c.w8(VIEW + 0x20b3, swings as u8);
    let clamped = h::run(c, clamped_hero_len, &[], &[]).1;
    c.wf(VIEW + 0x229c, clamped);
    c.w32(VIEW + 0x240, 0);

    let vel_x = c.rf(HERO + 0x108);
    let vel_y = c.rf(HERO + 0x10c);
    c.wf(SPEED_VEC, vel_x);
    c.wf(SPEED_VEC + 4, vel_y);
    let speed = h::run(c, vecmath::length_xy, &[SPEED_VEC], &[]).1;
    if speed < F(0x3e80_0000) {
        let flags = c.r32(HERO + 0x1b0);
        let bit = |n: u32| ((flags >> n) & 1) as i32;
        c.wf(SPEED_VEC, F::from_int(bit(13) - bit(15)));
        c.wf(SPEED_VEC + 4, F::from_int(bit(14) - bit(12)));
    }
    h::call(c, 0x0020_aab0, &[], &[]);

    let sample = c.rf(VIEW + 0x98);
    let f_slot = c.r32(VIEW + RING_F_IDX);
    c.wf(VIEW + RING_F_BASE + (f_slot << 2), sample);
    ring_advance(c, RING_F_IDX, RING_F_COUNT);

    let quad_src = c.rv(VIEW + 0x80);
    let q_slot = c.r32(VIEW + RING_Q_IDX);
    c.wv(VIEW + RING_Q_BASE + (q_slot << 4), quad_src);
    ring_advance(c, RING_Q_IDX, RING_Q_COUNT);

    h::call(c, 0x001f_2008, &[VIEW, VIEW + 0x90], &[]);
    h::run(c, vecmath::mat4_identity, &[VIEW + 0x40], &[]);
    h::run(c, vecmath::mat3_transpose, &[VIEW + 0x40, VIEW], &[]);
    h::run(c, smooth_view_targets, &[], &[]);
    c.call(0x0021_4450); // raw: its leftover registers are the input state of the next call
    // Last call is raw: its v0/v1/f0 results are what view_frame returns to its caller (h::call would restore them).
    c.call(0x0021_4708);
    true
}
