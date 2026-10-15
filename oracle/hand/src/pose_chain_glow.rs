//! Chain pose sampling and cheat-gated glow attach for the moby pose code (batch b649).
use crate::actor;
use crate::h::{self, SQ};
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Retail `jal` target: out-of-line vector difference `out = a - b` (args: out, b, a).
const VEC_DIFF: u32 = 0x1f1840;
/// 16-byte pose-chain seed table in RAM (copied to the stack before pose_chain_pos).
const POSE_SEED: u32 = 0x161cc8;
/// Word holding the game mode; the glow pass runs only in mode 2.
const MODE_WORD: u32 = 0x15f5a4;
/// Byte flag that enables the cheat glow.
const CHEAT_FLAG: u32 = 0x15edb0;
/// Object list header: signed count at +0x44, pointer table at +0x178.
const GLOW_LIST: u32 = 0x16c960;
const GLOW_LIST_COUNT: u32 = 0x16c9a4;
const GLOW_LIST_PTRS: u32 = 0x16cad8;
/// Glow layer record, and its enable byte at +1.
const GLOW_LAYER: u32 = 0x17c540;
/// Glow intensity written to the layer's +0x20, +0x24, +0x28 (2.75).
const GLOW_LEVEL: F = F(0x4030_0000);
/// Retail function bodies called from the glow pass.
const SET_OBJ_LAYER_TAIL: u32 = 0x2ee2a8;
const CAM_STEP: u32 = 0x215538;
/// Cheat-driven mode word and its flag.
const SCENE_STATE_WORD: u32 = 0x15f404;
const SCENE_STATE_GP_OFF: i32 = -0x7804;
const SCENE_STATE_HELD: u32 = 0x1413dc;
const SCENE_STATE_VALUE: u32 = 0x13;

/// 0x2e2740 pose_chain_lengths(m): samples a 4-point pose chain into o = m->+0x78, then stores the lengths of
/// the (o+0x10) and (o+0x20) difference vectors at o+0x40 and o+0x44.
pub fn pose_chain_lengths(c: &mut Cpu) -> bool {
    let m = c.arg(0);
    let obj = c.r32(m + 0x78);
    let saves = [(16, 0x30, h::SQ), (17, 0x40, h::SQ), (31, 0x50, h::SQ)];
    h::frame(c, 0x60, &saves, |c, sp| {
        let buf = sp + 0x20;
        let lo = c.mem.rd64(POSE_SEED);
        let hi = c.mem.rd64(POSE_SEED + 8);
        c.mem.wr64(buf, lo);
        c.mem.wr64(buf + 8, hi);
        h::run_keep(c, actor::pose_chain_pos, &[m, 4, buf, obj], &[]);
        let vec_a = sp;
        let vec_b = sp + 0x10;
        h::call_keep(c, VEC_DIFF, &[vec_a, obj + 0x10, obj], &[]);
        h::call_keep(c, VEC_DIFF, &[vec_b, obj + 0x20, obj], &[]);
        // Retail stores the first length in the delay slot of the second call, so it lands before that call runs.
        let first = h::run_keep(c, vecmath::length3, &[vec_a], &[]).1;
        c.wf(obj + 0x40, first);
        let len = h::run_keep(c, vecmath::length3, &[vec_b], &[]).1;
        c.wf(obj + 0x44, len);
        c.fret(len);
        true
    })
}

/// 0x2e28b8 glow_cheat_layers(m): in mode 2 with the cheat flag set, attaches the glow layer to each listed
/// object whose +0xa6 key matches m's, and sets the layer's glow level.
pub fn glow_cheat_layers(c: &mut Cpu) -> bool {
    let m = c.arg(0);
    let saves = [
        (16, 0x00, h::SQ),
        (17, 0x10, h::SQ),
        (18, 0x20, h::SQ),
        (19, 0x30, h::SQ),
        (21, 0x50, h::SQ),
        (20, 0x40, h::SQ),
        (31, 0x60, h::SQ),
    ];
    h::frame(c, 0x80, &saves, |c, _sp| {
        c.s(20, m as u64);
        if c.r32(MODE_WORD) != 2 || c.r8(CHEAT_FLAG) == 0 {
            return true;
        }
        if (c.r16(GLOW_LIST_COUNT) as i16) <= 0 {
            return true;
        }
        c.s(19, GLOW_LIST as u64);
        c.s(1, 0x4030_0000);
        let key = c.r16(m + 0xa6) as i16;
        let mut i: i32 = 0;
        loop {
            let obj = c.r32(GLOW_LIST_PTRS + 4 * i as u32);
            if c.r16(obj + 0xa6) as i16 == key && c.r8(GLOW_LAYER + 1) == 0 {
                h::run_keep(c, actor::layer_attach, &[obj, 0, GLOW_LAYER], &[]);
                c.wf(GLOW_LAYER + 0x28, GLOW_LEVEL);
                c.wf(GLOW_LAYER + 0x20, GLOW_LEVEL);
                c.wf(GLOW_LAYER + 0x24, GLOW_LEVEL);
            }
            i += 1;
            if i >= c.r16(GLOW_LIST_COUNT) as i16 as i32 {
                break;
            }
        }
        true
    })
}

/// 0x2e36f8 object_state_tick(obj): when obj's state byte (+0x20) is 4, clears the scene state words and runs the
/// retail state step; then runs the held-state step while the held word is 0x13.
pub fn object_state_tick(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let obj = c.arg(0);
    h::frame(c, 0x10, &[(31, 0x0, SQ)], |c, _sp| {
        if c.r8(obj + 0x20) == 4 {
            c.w32(SCENE_STATE_WORD, 0);
            let gp = c.g(28) as u32;
            c.w32(gp.wrapping_add(SCENE_STATE_GP_OFF as u32), 0);
            h::call_keep(c, SET_OBJ_LAYER_TAIL, &[3], &[]);
        }
        if c.r32(SCENE_STATE_HELD) == SCENE_STATE_VALUE {
            h::call_keep(c, CAM_STEP, &[0, 1], &[]);
        }
        true
    })
}
