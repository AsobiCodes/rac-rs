//! Camera pre-update: aims the camera from the state block, springs the look-at and follow vectors, shifts the
//! four-sample history ring and tracks the clip-distance state.
use crate::camobj;
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Camera object (retail s5).
const CAM: u32 = 0x0016_6f10;
/// Camera state block (retail s6 = 0x140000 - 0xa20).
const STATE: u32 = 0x0014_0000 - 0x0a20;
/// Track block at 0x13f350 (retail s4 after the first pass; a3 - 0xcb0).
const TRACK: u32 = 0x0014_0000 - 0x0cb0;

/// Spring follow of one component: `camobj::spring_follow(a0, cur, target, k1, k2, 0)`.
fn spring(c: &mut Cpu, a0: u32, cur: F, target: F, k1: F, k2: F) -> F {
    h::run(c, camobj::spring_follow, &[a0], &[cur, target, k1, k2, F(0)]).1
}

/// 0x1df2d8 camera_pre_update(): one frame of camera pre-update using the EE stack scratch at sp.
fn update(c: &mut Cpu, sp: u32) {
    let cam = CAM;
    let s6 = STATE;
    let src_a = s6 - 0x210;
    let one = F(0x3f80_0000);
    let k_a = F(0x3c75_c28f);
    let k_b = F(0x3e4c_cccd);

    // Aim direction: normalise the state vector to length -1, then nudge it when nearly antiparallel to the view.
    h::vec_set_length(c, sp, s6, F(0xbf80_0000));
    // Save the old cam+0x30 vector to cam+0x40, then store the normalised vector at cam+0x30.
    h::copy_q(c, cam + 0x40, cam + 0x30);
    h::copy_q(c, cam + 0x30, sp);
    let d = h::dot3(c, cam + 0x20, sp);
    if d < F(0xbf7a_e148) {
        for i in 0..3 {
            let v = c.rf(sp + 4 * i) + k_b;
            c.wf(sp + 4 * i, v);
        }
    }

    // Three springs on the look-at vector components (cam+0x20..0x28).
    let cur = c.rf(cam + 0x20);
    let tgt = c.rf(sp);
    let r = spring(c, cam + 0x50, cur, tgt, k_a, k_b);
    c.wf(cam + 0x20, r);
    let cur = c.rf(cam + 0x24);
    let tgt = c.rf(sp + 4);
    let r = spring(c, cam + 0x54, cur, tgt, k_a, k_b);
    c.wf(cam + 0x24, r);
    let cur = c.rf(cam + 0x28);
    let tgt = c.rf(sp + 8);
    let r = spring(c, cam + 0x58, cur, tgt, k_a, k_b);
    c.wf(cam + 0x28, r);
    h::vec_set_length(c, cam + 0x20, cam + 0x20, one);

    // Offset vectors: cam+0x70 = src - cam+0x60, its length and dot with sp.
    h::vec_sub(c, cam + 0x70, src_a, cam + 0x60);
    let len = h::length3(c, cam + 0x70);
    c.wf(cam + 0xa0, len);
    let dot = h::dot3(c, cam + 0x70, sp);
    c.wf(cam + 0xa8, dot);
    h::vec_set_length(c, sp + 0x10, sp, dot);
    h::copy_q(c, cam + 0x90, sp + 0x10);
    h::vec_sub(c, cam + 0x80, cam + 0x70, sp + 0x10);
    let len2 = h::length3(c, cam + 0x80);
    c.wf(cam + 0xa4, len2);
    h::vec_scale(c, cam + 0x80, cam + 0x80, one / len2);
    h::copy_q(c, cam + 0x60, src_a);

    // Track selection: mode words at +0x2284 / +0x2084 decide which look-at source is used.
    let track = TRACK;
    let mode = c.r32(track + 0x2284);
    let common = if mode != 0x50 { true } else { c.r32(track + 0x2084) == 0x11 };
    if common {
        let f80 = c.rf(track + 0x80);
        let f84 = c.rf(track + 0x84);
        let f88 = c.rf(track + 0x88);
        c.wf(cam, f80);
        c.wf(cam + 4, f84);
        let cur = c.rf(cam + 8);
        let r = spring(c, cam + 0x10, cur, f88, F(0x3bf5_c28f), F(0x3e33_3333));
        c.wf(cam + 8, r);
        c.wf(cam + 0xc, f88);
    } else {
        let f80 = c.rf(track + 0x80);
        let f84 = c.rf(track + 0x84);
        c.wf(cam + 4, f84);
        c.wf(cam, f80);
    }

    // Four-sample history ring at cam+0xac..0xbc: shift down one slot, append the new sample.
    for k in 0..4 {
        let v = c.rf(cam + 0xb0 + 4 * k);
        c.wf(cam + 0xac + 4 * k, v);
    }
    let f98 = c.rf(track + 0x98);
    c.wf(cam + 0xbc, f98);

    // Clip-distance state: target pointer at +0x2fc, its type at +0xa6 (0x4ba / 0x336 reset the state).
    let ptr = c.r32(track + 0x2fc);
    let ty = if ptr != 0 { c.r16(ptr + 0xa6) as i16 as i32 } else { -1 };
    if ptr == 0 || ty == 0x4ba || ty == 0x336 {
        c.w32(cam + 0xdc, 0);
        c.w32(cam + 0xd4, 0);
        let f88 = c.rf(track + 0x88);
        c.wf(cam + 0xd8, f88);
    } else {
        let old = c.r32(cam + 0xd4);
        if ptr != old {
            c.w32(cam + 0xd4, ptr);
            c.w32(cam + 0xdc, 0);
            let f = c.rf(ptr + 0x18);
            c.wf(cam + 0xd8, f);
        } else {
            let dist = c.rf(ptr + 0x18) - c.rf(cam + 0xd8);
            c.wf(cam + 0xdc, dist);
            if h::fabs(dist) < F(0x3a83_126f) {
                c.w32(cam + 0xdc, 0);
            }
            let f = c.rf(ptr + 0x18);
            c.wf(cam + 0xd8, f);
        }
    }
}

/// 0x1df2d8 camera_pre_update(): runs the camera pre-update in the scratch stack frame.
pub fn camera_pre_update(c: &mut Cpu) -> bool {
    h::with_stack(c, 0xd0, |c, sp| update(c, sp));
    true
}
