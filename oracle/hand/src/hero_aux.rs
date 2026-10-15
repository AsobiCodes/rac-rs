//! Hero weapon release, a 3x3 absolute-difference sum, and the knock-vector scaler.
use crate::h::{self, SQ};
use crate::hero_anim;
use crate::hero_rig;
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Hero state block (retail `lui 0x14` + `addiu -0xcb0`).
const HERO_BLOCK: u32 = 0x0013_f350;
/// Byte: weapon release latch.
const RELEASE_LATCH: u32 = 0x20a8;
/// Byte: cleared together with the latch.
const RELEASE_CLEAR: u32 = 0x20aa;
/// Word: fade request; when set the fade mode is raised.
const FADE_REQUEST: u32 = 0xd08;
/// Word: fade mode written by a fade request.
const FADE_MODE: u32 = 0xd14;
/// Word: weapon slot; a non-zero slot skips the animation.
const WEAPON_SLOT: u32 = 0x2084;
/// Word: owner pointer whose kind byte is compared with the weapon kind.
const WEAPON_OWNER: u32 = 0x2080;
/// Word: weapon kind.
const WEAPON_KIND: u32 = 0x2294;
/// Byte offset of the kind inside the owner.
const OWNER_KIND: u32 = 0x53;
/// Float -2.0: the animation blend passed when the idle sequence is played.
const ANIM_BLEND: F = F(0xc000_0000);

/// Sets integer register `r` to the sign-extended 32-bit value `v`.
fn set_gpr(c: &mut Cpu, r: usize, v: u32) {
    c.s(r, v as i32 as i64 as u64);
}

/// 0x201740 hero_weapon_release(): when the release latch is set, either raises the fade mode, or (no live slot and
/// the owner's kind matches the weapon kind) plays the idle sequence; then clears the latch.
pub fn hero_weapon_release(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    h::frame(c, 0x20, &[(16, 0x0, SQ), (31, 0x10, SQ)], |c, _sp| {
        set_gpr(c, 3, HERO_BLOCK);
        if c.r8(HERO_BLOCK + RELEASE_LATCH) == 0 {
            c.ret(0);
            return;
        }
        if c.r32(HERO_BLOCK + FADE_REQUEST) != 0 {
            c.w32(HERO_BLOCK + FADE_MODE, 1);
        } else if c.r32(HERO_BLOCK + WEAPON_SLOT) == 0 {
            let owner = c.r32(HERO_BLOCK + WEAPON_OWNER);
            let kind = c.r32(HERO_BLOCK + WEAPON_KIND);
            let owner_kind = c.r8(owner.wrapping_add(OWNER_KIND)) as u32;
            set_gpr(c, 3, owner_kind);
            if owner_kind == kind {
                let seq = h::run_keep(c, hero_rig::hero_idle_seq, &[0], &[]).0;
                h::run_keep(c, hero_anim::hero_set_anim, &[seq, 0], &[ANIM_BLEND]);
            } else {
                set_gpr(c, 4, kind);
            }
        }
        c.ret(HERO_BLOCK);
        c.w8(HERO_BLOCK + RELEASE_CLEAR, 0);
        c.w8(HERO_BLOCK + RELEASE_LATCH, 0);
    });
    true
}

/// 0x202160 abs_diff_sum3(a, b) -> f32: sum over 3 rows of 3 floats (16-byte row stride) of |a - b|, added in
/// order; leaves v0 = 0, v1 = 3 and f12 = the last signed difference.
pub fn abs_diff_sum3(c: &mut Cpu) -> bool {
    let a = c.arg(0);
    let b = c.arg(1);
    let mut sum = F::ZERO;
    let mut last = F::ZERO;
    h::frame(
        c,
        0x80,
        &[(16, 0x0, SQ), (17, 0x10, SQ), (18, 0x20, SQ), (19, 0x30, SQ), (20, 0x40, SQ), (21, 0x50, SQ), (31, 0x60, SQ)],
        |c, sp| {
            let saved_f20 = c.f[20];
            c.w32(sp.wrapping_add(0x70), saved_f20);
            for row in 0..3u32 {
                for k in 0..3u32 {
                    let off = row * 16 + k * 4;
                    last = c.rf(a.wrapping_add(off)) - c.rf(b.wrapping_add(off));
                    sum = sum + last.abs();
                }
            }
            c.ret(0);
            set_gpr(c, 3, 3);
            c.f[12] = last.0;
        },
    );
    c.fret(sum);
    true
}

/// 0x203d28 knock_vec(v, xy_scale, z_scale): with a0 = 0 the xy length is set to 1 (vec_set_length_xy) and z to
/// z_scale; otherwise v.xy scales by xy_scale and v.z by z_scale.
pub fn knock_vec(c: &mut Cpu) -> bool {
    let v = c.arg(0);
    let zero_len = c.arg(1) == 0;
    let xy_scale = c.farg(0);
    let z_scale = c.farg(1);
    h::frame(c, 0x30, &[(16, 0x0, SQ), (31, 0x10, SQ)], |c, sp| {
        let saved_f20 = c.f[20];
        c.w32(sp.wrapping_add(0x20), saved_f20);
        if zero_len {
            h::run_keep(c, vecmath::vec_set_length_xy, &[v, v], &[xy_scale]);
            c.w32(v.wrapping_add(8), z_scale.0);
        } else {
            let x = c.rf(v) * xy_scale;
            let y = c.rf(v.wrapping_add(4)) * xy_scale;
            let z = c.rf(v.wrapping_add(8)) * z_scale;
            c.wf(v, x);
            c.wf(v.wrapping_add(4), y);
            c.wf(v.wrapping_add(8), z);
            c.f[0] = x.0;
            c.f[1] = y.0;
            c.f[2] = z.0;
        }
    });
    true
}
