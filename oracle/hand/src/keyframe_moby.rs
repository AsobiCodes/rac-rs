//! Keyframe blend (0x2d6890) and attached moby spawn (0x2d9bf0), bit-exact with the retail code.

use crate::actor;
use crate::h;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;
use crate::vecmath;

/// Word in RAM that holds the base address of the per-level tables (retail `lw` from 0x15ff58).
const TABLE_BASE_SLOT: u32 = 0x15ff58;
/// Table of key-list pointers, indexed by the word at actor-state + 8 (retail base 0x1b05b0).
const KEY_LIST_TABLE: u32 = 0x1b05b0;
/// Moby class spawned as the attached child.
const CHILD_CLASS: u32 = 0x326;
/// Byte offsets inside a moby record and its per-moby state.
const MOBY_STATE: u32 = 0x78;
const MOBY_POS: u32 = 0x10;
const MOBY_FLAGS: u32 = 0x34;
const MOBY_PARENT_LINK: u32 = 0xc;
const MOBY_TYPE: u32 = 0x30;
const MOBY_ACTIVE: u32 = 0x20;
const MOBY_EXTRA: u32 = 0x94;
/// Flags OR-ed into the moby flags halfword.
const CHILD_FLAGS: u32 = 0x41;
/// Type byte written to the child.
const CHILD_TYPE: u8 = 0x40;
/// Parent field that gates the spawn: the spawn runs only when this byte is non-negative.
const PARENT_GATE: u32 = 0x20;
/// Stack-frame offsets of the vectors that the blend builds (sp+0 and sp+0x10).
const VEC_A: u32 = 0x0;
const VEC_B: u32 = 0x10;
/// Frame offsets where retail saves f20, f21, f22.
const FSAVE: [u32; 3] = [0x80, 0x88, 0x90];

/// 0x2d6890 keyframe_blend(actor): blends the 16-byte keys of a list at time t = (1 - *time) * (count - 1) (linear,
/// weights 1-frac and frac on keys floor(t) and floor(t)+1), stores the sum at actor+0x10 and copies it to the slot
/// at the actor's second index.
pub fn keyframe_blend(c: &mut Cpu) -> bool {
    let actor = c.arg(0);
    h::frame(
        c,
        0xa0,
        &[
            (16, 0x20, h::SQ),
            (17, 0x30, h::SQ),
            (18, 0x40, h::SQ),
            (19, 0x50, h::SQ),
            (20, 0x60, h::SQ),
            (31, 0x70, h::SQ),
        ],
        |c, sp| {
            for (i, off) in FSAVE.iter().enumerate() {
                c.w32(sp + off, c.f[20 + i]);
            }
            let state = c.r32(actor + MOBY_STATE);
            let (k0, k1, k2) = (c.r32(state), c.r32(state + 4), c.r32(state + 8));
            let keys = c.r32(KEY_LIST_TABLE.wrapping_add(k2.wrapping_mul(4)));
            let base = c.r32(TABLE_BASE_SLOT);
            let time_ptr = c.r32(base.wrapping_add(k0.wrapping_mul(256)).wrapping_add(MOBY_STATE));
            let dest_block = base.wrapping_add(k1.wrapping_mul(256));
            c.s(7, base as i32 as i64 as u64);

            let count = c.r32(keys).wrapping_sub(1);
            let span = h::run_keep(c, mathx::int_to_float, &[count], &[]).1;
            let t = (F::ONE - c.rf(time_ptr)) * span;
            let frac = h::run_keep(c, mathx::fmod_q, &[], &[t, F::ONE]).1;
            let whole = h::run_keep(c, mathx::float_to_int, &[], &[t]).0;
            c.f[0] = 0;

            let mut second = whole;
            if frac != F(0) {
                second = second.wrapping_add(1);
            }
            let key_a = keys.wrapping_add(whole.wrapping_mul(16)).wrapping_add(16);
            let key_b = keys.wrapping_add(second.wrapping_mul(16)).wrapping_add(16);
            h::run_keep(c, vecmath::vec_scale, &[sp + VEC_A, key_a], &[F::ONE - frac]);
            h::run_keep(c, vecmath::vec_scale, &[sp + VEC_B, key_b], &[frac]);
            h::call_keep(c, 0x1f1810, &[actor + MOBY_POS, sp + VEC_A, sp + VEC_B], &[]);

            let blend = c.mem.rd128(actor + MOBY_POS);
            c.r[2] = blend;
            c.s(3, dest_block.wrapping_add(MOBY_POS) as u64);
            c.mem.wr128(dest_block.wrapping_add(MOBY_POS), blend);
        },
    );
    true
}

/// 0x2d9bf0 spawn_attached_moby(parent) -> child or 0: when parent+0x20 is non-negative, spawns moby class 0x326,
/// links it back to the parent, copies the parent's position into it and returns it (0 when there is no child).
pub fn spawn_attached_moby(c: &mut Cpu) -> bool {
    let parent = c.arg(0);
    let child = h::frame(c, 0x20, &[(16, 0x0, h::SQ), (31, 0x10, h::SQ)], |c, _sp| {
        if (c.r8(parent + PARENT_GATE) as i8) < 0 {
            return 0;
        }
        let (child, _) = h::run_keep(c, actor::alloc_moby, &[CHILD_CLASS], &[]);
        c.s(6, child as i32 as i64 as u64);
        if child == 0 {
            return 0;
        }
        let state = c.r32(child + MOBY_STATE);
        c.w8(child + MOBY_TYPE, CHILD_TYPE);
        c.w8(child + MOBY_ACTIVE, 0);
        c.w32(state + MOBY_PARENT_LINK, parent);
        c.w32(child + MOBY_EXTRA, 0);
        let flags = c.r16(child + MOBY_FLAGS) as u32 | CHILD_FLAGS;
        c.w16(child + MOBY_FLAGS, flags as u16);
        c.s(3, flags as u64);
        c.s(4, child.wrapping_add(MOBY_POS) as u64);
        c.s(5, parent.wrapping_add(MOBY_POS) as u64);
        let pos = c.mem.rd128(parent + MOBY_POS);
        c.mem.wr128(child + MOBY_POS, pos);
        child
    });
    c.ret(child);
    true
}
