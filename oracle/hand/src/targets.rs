//! Target picking and attached-transform helpers (0x2529a8..0x253038).
use crate::h::*;
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Player/lock-on block (0x13f350): +0x2080 current target, +0x2084 / +0x208c mode words, +0xc0 / +0xd0 default
/// aim vectors.
const PLAYER_BLOCK: u32 = 0x13f350;
/// Table of nearby-candidate mobys: halfword count at 0x1b0930, halfword moby indices from 0x1b0932.
const CANDIDATE_COUNT: u32 = 0x1b0930;
/// Moby table base pointer and moby stride shift (256 bytes).
const MOBY_TABLE_PTR: u32 = 0x15ff58;
/// Pointer list of 20 scripted target mobys.
const TARGET_LIST: u32 = 0x1d1600;
const TARGET_LIST_LEN: u32 = 20;

/// 0x2529a8 pick_lock_target(player, out, range) -> mode (0 none, 1 found, 2 forced): picks the nearest valid
/// candidate (classes 0xcb / 0x76c in state 3, or class 0x10e with +0xbc set) within `range` (xy distance) of the
/// player's position (+0x10) and fills the aim block `out`: +0x00 target position, +0x10 target aim vector, +0x20 /
/// +0x30 aim points, +0x40 target pointer, +0x44 mode. With the lock modes 0x18 / 0x72 the current target is
/// dropped and mode 2 is reported.
pub fn pick_lock_target(c: &mut Cpu) -> bool {
    let (player, out, mut range) = (c.arg(0), c.arg(1), c.farg(0));
    let (mut best, mut mode) = (c.r32(PLAYER_BLOCK + 0x2080), 0);
    if c.r32(PLAYER_BLOCK + 0x208c) == 0x18 || c.r32(PLAYER_BLOCK + 0x2084) == 0x72 {
        best = 0;
        mode = 2;
    }
    let mut consider = |c: &mut Cpu, m: u32, best: &mut u32, mode: &mut u32| {
        let d = dist_xy(c, player + 0x10, m + 0x10);
        if d < range {
            *best = m;
            range = d;
            *mode = 1;
        }
    };
    let count = c.r16(CANDIDATE_COUNT) as i16 as i32;
    for i in 1..=count {
        let idx = c.r16(CANDIDATE_COUNT + 2 * i as u32) as i16 as i32;
        let m = c.r32(MOBY_TABLE_PTR).wrapping_add((idx << 8) as u32);
        if m == 0 {
            continue;
        }
        let class = c.r16(m + 0xa6) as i16;
        let state = c.r8(m + 0x20);
        if (class == 0xcb || class == 0x76c) && state == 3 {
            consider(c, m, &mut best, &mut mode);
        }
    }
    for i in 0..TARGET_LIST_LEN {
        let m = c.r32(TARGET_LIST + 4 * i);
        if m == 0 || c.r16(m + 0xa6) != 0x10e {
            continue;
        }
        let state = c.r8(m + 0x20);
        if state != 0xfe && state != 0xfd && c.r8(m + 0xbc) != 0 {
            consider(c, m, &mut best, &mut mode);
        }
    }
    let (aim_a, aim_b) = (out + 0x20, out + 0x30);
    if best != 0 {
        c.w32(out + 0x40, best);
        copy_q(c, out, best + 0x10);
        copy_q(c, out + 0x10, best + 0x40);
        if best == c.r32(PLAYER_BLOCK + 0x2080) {
            copy_q(c, aim_a, PLAYER_BLOCK + 0xc0);
            copy_q(c, aim_b, PLAYER_BLOCK + 0xd0);
        } else {
            copy_q(c, aim_b, out);
            let z = c.rf(out + 0x38) + F(0x3f19_999a);
            c.wf(out + 0x38, z);
            copy_q(c, aim_a, aim_b);
        }
    } else {
        c.w32(out + 0x40, 0);
        for off in [0, 0x10, 0x30, 0x20] {
            c.wv(out + off, V([0; 4]));
        }
    }
    c.w32(out + 0x44, mode);
    c.ret(mode);
    true
}

/// 0x252c38 actor_ground_height(moby) -> the float word at [+0x78]+8 when flag 0x20 of the halfword at +0x34 is
/// set, else 0 (also 0 for a null moby).
pub fn actor_ground_height(c: &mut Cpu) -> bool {
    let m = c.arg(0);
    let v = if m != 0 && c.r16(m + 0x34) & 0x20 != 0 {
        let p = c.r32(m + 0x78);
        c.r32(p + 8)
    } else {
        0
    };
    c.ret(v);
    true
}

/// 0x252d58 attach_point_place(owner, attach, local, euler, out_pos, out_ang): places the point `local` of an attach
/// moby: out_pos = local * rot(attach+0xc0) + attach.pos; builds the combined orientation of the attach rotation
/// and the euler angles (into out_ang via 0x24fef0). Uses the part's flag 4 of [part+0x3c] (part = 0x252c38 of
/// attach) to add the part offset and chain to 0x252ed0; otherwise adds the part offset scaled to unit length when
/// the owner's +0x22 byte is set and below the attach's. Returns 0 when the attach has no part.
pub fn attach_point_place(c: &mut Cpu) -> bool {
    const ACTOR_GROUND_HEIGHT: u32 = 0x252c38;
    const VEC3_MUL_MAT3: u32 = 0x1f1c68;
    const VEC_ADD: u32 = 0x1f1810;
    const EULER_MATRIX: u32 = 0x1f2008;
    const MAT3_COPY_W1: u32 = 0x1f2250;
    const MAT4_MUL: u32 = 0x1f2370;
    const MAT4_TO_EULER: u32 = 0x24fef0;
    const PART_PLACE: u32 = 0x252ed0;
    const VEC_CLAMP_LEN: u32 = 0x2522f0;
    // Retail's frame is 0x130 bytes; the buffers sit at its sp, so the pointers left in scratch registers match.
    const FRAME: u32 = 0x130;

    let (owner, attach, local, euler) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3));
    let (out_pos, out_ang) = (c.arg(4), c.arg(5));
    // Every helper is retail code run with call_keep: the VU0 registers and the scratch registers it leaves stay.
    let part = call_keep(c, ACTOR_GROUND_HEIGHT, &[attach], &[]).0;
    if part == 0 {
        c.ret(0);
        return true;
    }
    let rot = attach + 0xc0;
    call_keep(c, VEC3_MUL_MAT3, &[out_pos, local, rot], &[]);
    call_keep(c, VEC_ADD, &[out_pos, out_pos, attach + 0x10], &[]);
    with_stack(c, FRAME, |c, buf| {
        let (rot_copy, orient, offset) = (buf, buf + 0x40, buf + 0x80);
        call_keep(c, EULER_MATRIX, &[orient, euler], &[]);
        call_keep(c, MAT3_COPY_W1, &[rot_copy, rot], &[]);
        call_keep(c, MAT4_MUL, &[orient, rot_copy, orient], &[]);
        call_keep(c, MAT4_TO_EULER, &[orient, out_ang], &[]);
        // retail's delay slot sets a2 = part + 0x10 before the flag test, so it is left on both paths
        c.s(6, (part + 0x10) as i32 as i64 as u64);
        if c.r32(part + 0x3c) & 4 != 0 {
            call_keep(c, VEC_ADD, &[out_pos, out_pos, part + 0x10], &[]);
            call_keep(c, PART_PLACE, &[owner, attach, out_pos, out_ang, local, euler], &[]);
            return;
        }
        // retail: a0 = owner's byte, then v1 = part + 0x10 in its delay slot
        let a = c.r8(owner + 0x22) as u32;
        c.s(4, a as u64);
        if a != 0 {
            c.s(3, (part + 0x10) as i32 as i64 as u64);
            if a < c.r8(attach + 0x22) as u32 {
                copy_q(c, offset, part + 0x10);
                call_keep(c, VEC_CLAMP_LEN, &[offset], &[F::ONE]);
                call_keep(c, VEC_ADD, &[out_pos, out_pos, offset], &[]);
            }
        }
    });
    c.ret(1);
    true
}

/// 0x252fa0 compose_orientation(out, pos_src, euler_a, euler_b): out = orientation of euler_a^T * euler_b (via
/// 0x24fef0) and the position vector copied from pos_src to out+0x10.
pub fn compose_orientation(c: &mut Cpu) -> bool {
    let (out, pos_src, euler_a, euler_b) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3));
    with_stack(c, 0xc0, |c, buf| {
        let (a, b, prod) = (buf, buf + 0x40, buf + 0x80);
        call(c, 0x1f2008, &[a, euler_a], &[]);
        run(c, vecmath::mat3_transpose, &[a, a], &[]);
        call(c, 0x1f2008, &[b, euler_b], &[]);
        run(c, vecmath::mat4_mul, &[prod, a, b], &[]);
        call(c, 0x24fef0, &[prod, out], &[]);
    });
    copy_q(c, out + 0x10, pos_src);
    true
}

/// 0x253038 sphere_visible_scaled(sphere, radius) -> the frustum test 0x1f2878 (-1 outside, 0 straddle, 1 inside)
/// of the sphere scaled by 1/1024 (all lanes).
pub fn sphere_visible_scaled(c: &mut Cpu) -> bool {
    let (v, radius) = (c.arg(0), c.farg(0));
    let r = with_stack(c, 0x30, |c, buf| {
        run(c, vecmath::vec4_scale, &[buf, v], &[F(0x3a80_0000)]);
        call(c, 0x1f2878, &[buf], &[radius]).0
    });
    c.ret(r);
    true
}
