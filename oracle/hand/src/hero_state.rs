//! Hero per-frame state tick and pose sync (retail 0x1fade8 and 0x1fb8c8).

use crate::ps2::*;
use crate::{h, motion};
use ee_oracle::cpu::Cpu;

/// Base of the hero state block (retail `lui 0x14` minus 0xcb0).
const HERO: u32 = 0x0013_F350;
/// Word holding the current game mode.
const MODE_ADDR: u32 = 0x0015_F5A4;
/// Byte flag read by the state-1 path.
const FLAG_STATE1: u32 = 0x0015_EDB3;
/// Byte flag read by the pose-sync link-offset path.
const FLAG_POSE: u32 = 0x0015_EDB5;
/// Float scale used by the state-1 path.
const SCALE_ADDR: u32 = 0x0015_ED60;
/// Hero object pointer, stored in the state block.
const OBJ_PTR: u32 = 0x2080;
/// Spring/ease step helper (args: block ptr, int, f12, f13).
const SMOOTH_STEP: u32 = 0x0025_5A98;
/// Query helper that returns a small status code in v0.
const QUERY: u32 = 0x0020_0540;
/// Query helper returning nonzero when the link check passes.
const LINK_CHECK: u32 = 0x0020_0610;
/// Retail helpers without Rust yet.
const INIT_A: u32 = 0x001F_A950;
const INIT_B: u32 = 0x001F_AA90;
const PLAY_CTRL: u32 = 0x0022_6128;
/// Slew target block inside the state block.
const SLEW_BLOCK: u32 = HERO + 0x162C;

/// Retail routines the pose sync calls (kept as retail so their VU/scratch side effects match).
const HERO_ITEMS_RESHOW: u32 = 0x0022_6210;
const MEM_COPY: u32 = 0x001F_1628;
const ACTOR_BSPHERE: u32 = 0x0024_36C0;
const UP_OFFSET_NEG: u32 = 0x0022_6828;
const LINK_NODE_SET: u32 = 0x0024_17A0;

/// 0x1fade8 hero_state_tick() -> (): per-frame hero state update, keyed on the game mode and hero state byte.
pub fn hero_state_tick(c: &mut Cpu) -> bool {
    let mode = c.r32(MODE_ADDR);
    if mode == 2 || mode == 6 {
        slot_loop(c);
    } else {
        state_dispatch(c);
    }
    true
}

/// Loop over the active slots (count at +0x1eb0): each runs the smooth-step helper.
fn slot_loop(c: &mut Cpu) {
    let mut i: u32 = 0;
    while (c.r16(HERO + 0x1EB0) as i16 as i32) > i as i32 {
        let s0 = HERO + 0x1E90 + 4 * i;
        let a1 = c.r32(s0 - 0x40);
        let f12 = c.rf(s0 - 0x20);
        let f13 = c.rf(s0);
        h::call(c, SMOOTH_STEP, &[HERO + 0x1DD0 + 16 * i, a1], &[f12, f13]);
        i += 1;
    }
}

/// Picks the branch from the hero state byte (+0x20a4).
fn state_dispatch(c: &mut Cpu) {
    match c.r8(HERO + 0x20A4) {
        3 => {
            let s2 = c.r32(HERO + 0x22E4);
            for i in 0..3u32 {
                let f12 = F(0x3E4C_CCCD);
                let f13 = F(0x3DA3_D70A);
                h::call(c, SMOOTH_STEP, &[HERO + 0x1D50 + 16 * i, s2], &[f12, f13]);
            }
        }
        0 => state0_tick(c),
        1 => state1_tick(c),
        2 => state2_tick(c),
        _ => {}
    }
}

/// State 0: two status probes decide whether the bump block runs before the slot loop.
fn state0_tick(c: &mut Cpu) {
    let mut hit = h::call(c, QUERY, &[3], &[]).0 == 3;
    if !hit {
        hit = h::call(c, QUERY, &[3], &[]).0 == 2;
    }
    if hit && c.r16(HERO + 0x22D8) as i16 == 0 {
        bump_block(c);
    } else {
        slot_loop(c);
    }
}

/// Adds a small offset to the lift value, calls the smooth step, then removes the offset again.
fn bump_block(c: &mut Cpu) {
    let bump = F(0x3C75_C28F);
    let v = c.rf(HERO + 0x1D88) + bump;
    c.wf(HERO + 0x1D88, v);
    let f12 = F(0x3D69_78D5);
    let f13 = F(0x3DA3_D70A);
    h::call(c, SMOOTH_STEP, &[HERO + 0x1D80, 0x2800_00C0], &[f12, f13]);
    let v = c.rf(HERO + 0x1D88) - bump;
    c.wf(HERO + 0x1D88, v);
}

/// State 1: adjusts the lift value, slews the target and runs the smooth step on the camera block.
fn state1_tick(c: &mut Cpu) {
    let mut f12 = F(0x3D8F_5C29);
    if c.r16(HERO + 0x22FC) as i16 != 0 {
        f12 = F(0x3DCF_DF3C);
    }
    let mut f0 = c.rf(HERO + 0x1D88);
    if c.r8(FLAG_STATE1) != 0 {
        f0 = f0 - F(0x3C23_D70A);
        f12 = f12 * F(0x3FD9_999A);
    } else {
        f0 = f0 - F(0x3CA3_D70A);
    }
    c.wf(HERO + 0x1D88, f0);
    let f13 = c.rf(SCALE_ADDR) * F(0x3B44_9BA6);
    h::run(c, motion::slew_toward, &[SLEW_BLOCK], &[f12, f13]);
    let f12 = c.rf(SLEW_BLOCK);
    let a1 = c.r32(HERO + 0x2300);
    h::call(c, SMOOTH_STEP, &[SLEW_BLOCK + 0x754, a1], &[f12, F(0x3DA3_D70A)]);
}

/// State 2: smooth step on the lift block, with a fixed offset applied around the call.
fn state2_tick(c: &mut Cpu) {
    let offset = F(0x3D8F_5C29);
    let v = c.rf(HERO + 0x1D88) - offset;
    c.wf(HERO + 0x1D88, v);
    let f12 = F(0x3E80_0000);
    let f13 = F(0x3DA3_D70A);
    h::call(c, SMOOTH_STEP, &[HERO + 0x1D80, 0x3000_00C0], &[f12, f13]);
    let v = c.rf(HERO + 0x1D88) + offset;
    c.wf(HERO + 0x1D88, v);
}

/// 0x1fb8c8 hero_pose_sync() -> (): copies the hero pose into the hero object, refreshes its bounds and link nodes.
pub fn hero_pose_sync(c: &mut Cpu) -> bool {
    let obj = c.r32(HERO + OBJ_PTR);
    c.w8(obj + 0xA4, 0xFF);
    let flag = c.r8(HERO + 0x20A5);
    if flag != 0 {
        h::call_keep(c, PLAY_CTRL, &[flag as u32, 0xFF], &[]);
    } else {
        h::call_keep(c, HERO_ITEMS_RESHOW, &[0, 0xFF], &[]);
    }

    let pose = c.rv(HERO + 0x80);
    let obj = c.r32(HERO + OBJ_PTR);
    c.wv(obj + 0x10, pose);
    h::call_keep(c, INIT_A, &[], &[]);

    let anim = c.rv(HERO + 0x90);
    let obj = c.r32(HERO + OBJ_PTR);
    c.wv(obj + 0x40, anim);
    h::call_keep(c, INIT_B, &[], &[]);

    let obj = c.r32(HERO + OBJ_PTR);
    h::call_keep(c, MEM_COPY, &[obj + 0xC0, HERO, 0x30], &[]);
    let obj = c.r32(HERO + OBJ_PTR);
    h::call_keep(c, ACTOR_BSPHERE, &[obj], &[]);

    let linked = c.r32(HERO + 0x1CC) == 0 && {
        let o = c.r32(HERO + OBJ_PTR);
        c.r32(o + 0x94) != 0
    };
    if linked {
        refresh_links(c);
    }

    // Retail leaves a0 = the object and v1 = its +0xA6 halfword (lh) at exit, whatever the branch.
    let obj = c.r32(HERO + OBJ_PTR);
    c.s(4, obj as i32 as i64 as u64);
    let state = c.r16(obj + 0xA6) as i16;
    c.s(3, state as i64 as u64);
    if state == 0 {
        let v = c.r16(obj + 0x34);
        let v = if c.r8(FLAG_POSE) != 0 { v | 0x8000 } else { v & 0x7FFF };
        c.w16(obj + 0x34, v);
        c.ret(v as u32);
    } else {
        // Retail's v0 still holds the hero object pointer (set before the lh test).
        c.ret(HERO);
    }
    true
}

/// Writes the hero pose into a stack buffer and sets the two link nodes from the lift and span values.
fn refresh_links(c: &mut Cpu) {
    h::with_stack(c, 0x60, |c, sp| {
        let pose = c.rv(HERO + 0x80);
        c.wv(sp, pose);
        let f12 = c.rf(HERO + 0x224);
        let min_span = F(0x3D4C_CCCD);
        h::call_keep(c, UP_OFFSET_NEG, &[sp, sp], &[f12, min_span]);

        let span = clamp_span(c.rf(HERO + 0x220) - c.rf(HERO + 0x224), min_span);
        let obj = c.r32(HERO + OBJ_PTR);
        let node = c.rf(HERO + 0x234);
        h::call_keep(c, LINK_NODE_SET, &[obj, 0, sp], &[node, span]);

        let mut lift = c.rf(HERO + 0x234) + min_span;
        if c.r32(HERO + 0x208C) == 4 {
            lift = lift + F(0x3E19_999A);
        }
        if h::call_keep(c, LINK_CHECK, &[], &[]).0 != 0 {
            lift = c.rf(HERO + 0x224);
        }

        let pose = c.rv(HERO + 0x80);
        c.wv(sp, pose);
        h::call_keep(c, UP_OFFSET_NEG, &[sp, sp], &[lift]);
        let span = clamp_span(c.rf(HERO + 0x220) - lift, min_span);
        let obj = c.r32(HERO + OBJ_PTR);
        let node = c.rf(HERO + 0x234);
        h::call_keep(c, LINK_NODE_SET, &[obj, 1, sp], &[node, span]);
    });
}

/// Returns `span`, raised to `min` when it is below it.
fn clamp_span(span: F, min: F) -> F {
    if span < min { min } else { span }
}
