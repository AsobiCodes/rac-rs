//! Level 4 helpers: a DMA list builder, a colour-slot claim, and the empty function next to them.

use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Sign-extends a 32-bit value the way MIPS 32-bit ops leave a 64-bit register.
fn sx(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// Word whose value seeds the DMA list addresses.
const DMA_BASE_WORD: u32 = 0x0016_114c;
/// Start of the 10-word DMA list.
const DMA_LIST: u32 = 0x0017_3f00;
/// First list address before masking: 0x300000 - 0x7c71.
const DMA_ADDR_START: u32 = 0x0030_0000 - 0x7c71;

/// 0x230668 dma_block_setup(): writes a 10-word DMA list at 0x173f00 from the base word at 0x16114c.
pub fn dma_block_setup(c: &mut Cpu) -> bool {
    let base = c.r32(DMA_BASE_WORD);
    let a0 = DMA_ADDR_START & 0xffff_c000;
    let t2 = 0x0017_0000u32;
    let v0 = DMA_LIST;
    let t3 = a0.wrapping_add(base);
    let a3 = t3.wrapping_add(base);
    let a1 = a3.wrapping_add(0x0006_4000);
    let a2 = a1.wrapping_add(0x0003_0000);
    let v1 = 0x0720_0000u32;
    let t0 = 0x0700_0000u32;
    let t1 = 0x0710_0000u32;
    c.w32(v0 + 0x28, v1);
    c.w32(v0, a0);
    c.w32(v0 + 0x18, a2);
    c.w32(v0 + 0x20, t0);
    c.w32(v0 + 0x24, t1);
    c.w32(v0 + 0x04, a0);
    c.w32(v0 + 0x08, t3);
    c.w32(v0 + 0x0c, a3);
    c.w32(v0 + 0x10, a1);
    c.w32(v0 + 0x14, a2);
    c.s(2, sx(v0));
    c.s(3, sx(v1));
    c.s(4, sx(a0));
    c.s(5, sx(a1));
    c.s(6, sx(a2));
    c.s(7, sx(a3));
    c.s(8, sx(t0));
    c.s(9, sx(t1));
    c.s(10, sx(t2));
    c.s(11, sx(t3));
    true
}

/// 0x230b30 empty_fn(): returns at once (jr ra; nop).
pub fn empty_fn(_c: &mut Cpu) -> bool {
    true
}

/// Eight 0x30-byte slot records at 0x1805c0; a record is free while the word at its +0x10 is zero.
const SLOT_TABLE: u32 = 0x0018_05c0;
const SLOT_COUNT: u32 = 8;
const SLOT_STRIDE: u32 = 0x30;
const SLOT_FLAG: u32 = 0x10;
/// Per-slot 32-byte vector area at 0x1803c0 and the 1 KiB block area at 0x180740.
const SLOT_VEC: u32 = 0x0018_03c0;
const SLOT_BLOCK: u32 = 0x0018_0740;
/// Threshold 0.8 (bits), compared against the float at 0x15f5b4.
const THRESH_BITS: u32 = 0x3f4c_cccd;
const THRESH_ADDR: u32 = 0x0015_f5b4;
/// 1/128 (bits), the colour scale.
const INV_128_BITS: u32 = 0x3c00_0000;

/// 0x230b38 claim_slot_color(src, rgb, f12, f13) -> first free slot index, or -1.
/// Below the threshold it returns -1 at once. Otherwise it copies the 16-byte vector at `src` into the slot,
/// stores the three colour bytes as floats scaled by 1/128, the f12 value and the f13 value, zeroes the record and
/// marks it used.
pub fn claim_slot_color(c: &mut Cpu) -> bool {
    let src = c.arg(0);
    let rgb = c.arg(1);
    let alpha = c.farg(1);
    let thresh = F(THRESH_BITS);
    let limit = c.rf(THRESH_ADDR);
    c.f[1] = limit.0;
    c.s(6, sx(src));
    c.s(1, sx(THRESH_BITS));
    c.fret(thresh);
    if thresh < limit {
        c.ret(u32::MAX);
        return true;
    }

    let mut s2: u32 = 0;
    while s2 < SLOT_COUNT && c.r32(SLOT_TABLE + SLOT_FLAG + SLOT_STRIDE * s2) != 0 {
        s2 += 1;
    }
    if s2 == SLOT_COUNT {
        c.s(4, sx(0x0018_0000));
        // retail's loop exit steps v1 once more (delay slot of the beqz at 0x230ba4): flag of slot 8.
        c.s(3, sx(SLOT_TABLE + SLOT_FLAG + SLOT_STRIDE * SLOT_COUNT));
        c.ret(u32::MAX);
        return true;
    }

    let vec = SLOT_VEC + (s2 << 5);
    let rec = SLOT_TABLE + SLOT_STRIDE * s2;
    let inv = F(INV_128_BITS);
    let quad = c.rv(src);
    c.wv(vec + 0x10, quad);
    c.wf(vec + 0x1c, c.farg(0));
    let mut last = F(0);
    for i in 0..3u32 {
        last = F::from_int(((rgb >> (8 * i)) & 0xff) as i32) * inv;
        c.wf(vec + 4 * i, last);
    }
    c.wf(vec + 0x0c, alpha);
    for i in 0..3u32 {
        c.wv(rec + 16 * i, V([0; 4]));
    }
    let block = SLOT_BLOCK + (s2 << 10);
    c.w32(rec + 0x10, 1);
    c.w32(rec + 0x0c, block);

    c.fret(last);
    c.f[1] = limit.0;
    c.s(1, sx(INV_128_BITS));
    c.s(3, sx(block));
    c.s(4, 1);
    c.s(5, 0);
    c.s(6, sx(src));
    c.ret(s2);
    true
}
