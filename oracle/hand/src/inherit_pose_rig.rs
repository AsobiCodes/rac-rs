//! Class-0 camera rig rebuilt from the inherited camera pose: the rig block at `obj+0x70` gets its offset vectors, and
//! the boom and pivot vectors are derived from the hero position at `obj+0x30`.
use crate::euler_basis;
use crate::h;
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Retail constant vector (xyz scaled by -1.0 into the frame's first slot).
const NEG_SRC: u32 = 0x13_f5e0;
/// Retail constant vector copied into the rig block at `base+0x40`.
const OFFSET_SRC: u32 = 0x13_f3d0;
/// Field at `obj+0x86`: the rig is only rebuilt while this halfword is zero.
const ACTIVE_HALF: u32 = 0x86;
/// Field at `obj+0x70`: pointer to the rig block.
const RIG_PTR: u32 = 0x70;
/// Hero position vector at `obj+0x30`.
const HERO_POS: u32 = 0x30;

/// The 128 bits of a VU register as a GPR value (qmfc2 / lq).
fn quad(v: V) -> u128 {
    v.0.iter().enumerate().fold(0u128, |acc, (i, &w)| acc | (w as u128) << (32 * i))
}

/// Copies one 16-byte quadword from `src` to `dst` (retail `lq` / `sq`), leaving the value in $v0.
fn copy_quad(c: &mut Cpu, dst: u32, src: u32) {
    let v = c.rv(src);
    c.wv(dst, v);
    c.r[2] = quad(v);
}

/// 0x2e9e68 follow_rig_from_inherited_pose(obj): when the halfword at obj+0x86 is zero, rebuilds the rig block at
/// obj+0x70 (offset, pivot and boom vectors, zeroed slots and the flag at +0x220) from the hero pose at obj+0x30.
pub fn follow_rig_from_inherited_pose(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    h::frame(
        c,
        0xa0,
        &[
            (22, 0x80, h::SQ),
            (31, 0x90, h::SQ),
            (21, 0x70, h::SQ),
            (20, 0x60, h::SQ),
            (19, 0x50, h::SQ),
            (18, 0x40, h::SQ),
            (17, 0x30, h::SQ),
            (16, 0x20, h::SQ),
        ],
        |c, sp| {
            if c.r16(obj + ACTIVE_HALF) as i16 != 0 {
                return;
            }
            let base = c.r32(obj + RIG_PTR);
            let offset = base + 0x40;
            let pivot = base + 0x90;
            let boom = base + 0x130;

            // sp[0..16] = -1.0 * NEG_SRC.xyz
            h::run(c, vecmath::vec_scale, &[sp, NEG_SRC], &[F(0xbf80_0000)]);
            copy_quad(c, offset, OFFSET_SRC);
            copy_quad(c, base + 0xa0, offset);
            let (_, d) = h::run(c, vecmath::dot3, &[offset, sp], &[]);
            h::run(c, vecmath::vec_scale, &[base + 0x50, sp], &[d]);
            h::run(c, euler_basis::zero_vec4, &[base + 0xb0], &[]);
            h::run(c, euler_basis::zero_vec4, &[base + 0xc0], &[]);
            h::run(c, euler_basis::zero_vec4, &[base + 0xe0], &[]);

            // sp+0x10 = sp.xyz * boom.w-slot scalar (rig +0x160)
            let tmp = sp + 0x10;
            let s_boom = c.rf(base + 0x160);
            h::run(c, vecmath::vec_scale, &[tmp, sp], &[s_boom]);
            h::call(c, 0x1f1810, &[pivot, tmp, offset], &[]);

            let s_b0 = c.rf(base + 0xf0);
            h::run(c, vecmath::vec_scale, &[tmp, sp], &[s_b0]);
            h::call(c, 0x1f1810, &[base + 0xd0, tmp, offset], &[]);
            copy_quad(c, base + 0x80, base + 0xd0);

            h::call(c, 0x1f1840, &[boom, obj + HERO_POS, pivot], &[]);
            copy_quad(c, base + 0x140, boom);
            copy_quad(c, base, obj + HERO_POS);
            h::call(c, 0x1f1810, &[base + 0x1f0, pivot, boom], &[]);
            c.w32(base + 0x220, 0);
        },
    );
    true
}
