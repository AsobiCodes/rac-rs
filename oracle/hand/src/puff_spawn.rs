//! Spawns a type-0x12 trail puff: allocates a particle, scatters its offset through the source matrix, and fills its
//! tag bytes, timer and scale.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Particle type passed to the allocator.
const PUFF_TYPE: u32 = 0x12;
/// Retail `jal` targets, run as retail code so scratch registers match.
const PARTICLE_ALLOC_MODE0: u32 = 0x259a38;
const RAND_FLOAT: u32 = 0x24a4b0;
const VEC4_MUL_MAT4: u32 = 0x1f1c90;
const FLOAT_TO_INT: u32 = 0x1f27e8;
const FLOAT_TO_INT_SCALED: u32 = 0x1f14b8;
/// Retail `jal` target: out-of-line vector add (out, a, b).
const VEC_ADD: u32 = 0x1f1810;
/// Retail `jal` target: timer/tag helper, no args, leaves v0 as the next tag byte source.
const RETAIL_TAG_HELPER: u32 = 0x1160d8;
/// Retail `lui v0,0x1b` base; the global pointer slot is at base + 0x21c8.
const TAG_PTR_BASE: u32 = 0x001b_0000;
/// Retail global: pointer to a byte whose value becomes the puff's tag byte.
const TAG_PTR_SLOT: u32 = TAG_PTR_BASE + 0x21c8;
/// Retail `lui v0,0x80` + `ori 0x6040`: word stored at puff+4.
const PUFF_WORD4: u32 = 0x0080_6040;
/// Single 4.0 (argument to the float-to-int helper).
const FOUR: F = F(0x4080_0000);
/// Single -0.1 / 0.1 (first scatter range).
const SCATTER_LO: F = F(0xbdcc_cccd);
const SCATTER_HI: F = F(0x3dcc_cccd);
/// Single -0.3 / 0.3 (second and third scatter ranges).
const SCATTER_WIDE_LO: F = F(0xbe99_999a);
const SCATTER_WIDE_HI: F = F(0x3e99_999a);
/// Single 0.08 (stored at puff+0x24, also the timer's lower bound stand-in).
const STEP: F = F(0x3da3_d70a);
/// Timer range bounds (random between these).
const TIMER_LO: F = F(0x46da_b201);
const TIMER_HI: F = F(0x4743_3c00);
/// Integer argument to the scaled float-to-int helper (scale source).
const SCALE_ARG: u32 = 0xd2;
/// Single 1.0 (final puff scale).
const ONE: F = F(0x3f80_0000);

/// 0x25e670 particle_spawn_puff(owner, src) -> puff: allocates a type-0x12 particle (returns 0 when none); the source
/// block at `src` holds three basis columns that scatter the puff's offset, with tag bytes, a timer and scale set.
pub fn particle_spawn_puff(c: &mut Cpu) -> bool {
    let (owner, src) = (c.arg(0), c.arg(1));
    let saved_f20 = c.f[20];
    let saved_f21 = c.f[21];
    h::frame(
        c,
        0xc0,
        &[
            (16, 0x60, h::SQ),
            (17, 0x70, h::SQ),
            (18, 0x80, h::SQ),
            (19, 0x90, h::SQ),
            (31, 0xa0, h::SQ),
        ],
        |c, sp| {
            c.w32(sp + 0xb0, saved_f20);
            c.w32(sp + 0xb8, saved_f21);
            let puff = h::call_keep(c, PARTICLE_ALLOC_MODE0, &[PUFF_TYPE], &[]).0 as u32;
            if puff == 0 {
                return;
            }
            let timer_slot = puff + 0x20;
            c.mem.wr128(sp + 0x50, 0);
            c.r[2] = c.mem.rd128(src);
            c.r[3] = c.mem.rd128(src + 0x10);
            c.r[4] = c.mem.rd128(src + 0x20);
            c.mem.wr128(sp + 0x20, c.r[2]);
            c.mem.wr128(sp + 0x30, c.r[3]);
            c.mem.wr128(sp + 0x40, c.r[4]);

            let x = h::call_keep(c, RAND_FLOAT, &[], &[SCATTER_LO, SCATTER_HI]).1;
            c.w32(sp + 0x10, x.0);
            let y = h::call_keep(c, RAND_FLOAT, &[], &[SCATTER_WIDE_LO, SCATTER_WIDE_HI]).1;
            c.w32(sp + 0x14, y.0);
            let z = h::call_keep(c, RAND_FLOAT, &[], &[SCATTER_WIDE_LO, SCATTER_WIDE_HI]).1;
            c.w32(sp + 0x18, z.0);

            // Offset = matrix * scatter vector, added onto the puff's position.
            h::call_keep(c, VEC4_MUL_MAT4, &[sp, sp + 0x10, sp + 0x20], &[]);
            h::call_keep(c, VEC_ADD, &[puff + 0x10, sp, owner], &[]);

            c.w32(puff + 4, PUFF_WORD4);
            c.s(2, PUFF_WORD4 as u64);
            h::call_keep(c, FLOAT_TO_INT, &[], &[FOUR]);
            let v0 = (c.g(2) as u32).wrapping_add(0x40);
            c.s(2, v0 as i32 as i64 as u64);
            c.s(3, 0x48);
            c.w8(puff + 1, 0);
            c.w8(puff + 3, 0x48);
            c.w8(puff + 9, v0 as u8);
            h::call_keep(c, RETAIL_TAG_HELPER, &[], &[]);
            c.w8(puff + 8, c.g(2) as u8);

            c.s(2, TAG_PTR_BASE as u64);
            let tag_ptr = c.r32(TAG_PTR_SLOT) as u32;
            c.s(3, tag_ptr as i32 as i64 as u64);
            let tag = c.r8(tag_ptr);
            c.s(2, tag as u64);
            c.w8(puff + 2, tag);
            c.f[0] = STEP.0;
            c.w32(timer_slot + 4, STEP.0);
            let timer = h::call_keep(c, RAND_FLOAT, &[], &[TIMER_LO, TIMER_HI]).1;
            c.w32(puff + 0xc, timer.0);
            let scale = h::call_keep(c, FLOAT_TO_INT_SCALED, &[SCALE_ARG], &[]).0 as u16;
            c.w16(puff + 0xa, scale);
            c.s(1, ONE.0 as u64);
            c.f[0] = ONE.0;
            c.w32(timer_slot + 0xc, ONE.0);
        },
    );
    true
}
