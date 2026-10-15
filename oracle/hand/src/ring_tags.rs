//! Tag ring writer (retail 0x2916d8..0x291734) and the 16 KB block loop (retail 0x291738..0x2917e4).
//! The ring pointer lives at a global word; the writer stores two 16-byte tag pairs there and advances the pointer.
//! The loop splits a size, rounded up to 16 KB, into blocks and calls four retail helpers per block.

use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Global word holding the current write position of the tag ring.
const RING_PTR: u32 = 0x0016_1140;
/// Tag words written at the ring position: head tag, address word, zero, tail tag.
const TAG_HEAD: u32 = 0x3000_000b;
const TAG_ADDR: u32 = 0x0014_0000 - 0x30f0;
const TAG_TAIL: u32 = 0x5000_000b;
/// Bytes the write position advances past the tags.
const RING_STRIDE: u32 = 0x10;
/// gp-relative slot that receives the new write position.
const GP_RING_SLOT: i32 = -0x5ac0;

/// Stack frame of the block loop: s0..s3 and ra saved at 0x70..0xb0.
const FRAME_SIZE: u32 = 0xc0;
/// Block size and its mask (retail `and` with -0x4000 after adding 0x3fff).
const BLOCK_SIZE: i32 = 0x4000;
const BLOCK_ROUND: u32 = 0x3fff;
const BLOCK_MASK: u32 = 0xffff_c000;

/// Retail helpers called once per block.
const HELPER_SPLIT: u32 = 0x0012_2518;
const HELPER_RESET: u32 = 0x0011_8a80;
const HELPER_COPY: u32 = 0x0012_27d8;
const HELPER_FLUSH: u32 = 0x0012_0558;

/// Stores a 32-bit value as the sign-extended 64-bit register contents the MIPS code leaves.
fn set32(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// 0x2916d8 ring_tag_pair() -> ring position: writes the tag words at the ring write position, stores the position
/// after them (plus 0x10) in the gp slot and returns it.
pub fn ring_tag_pair(c: &mut Cpu) -> bool {
    let p = c.r32(RING_PTR);
    c.w32(p, TAG_HEAD);
    let p = c.r32(RING_PTR);
    c.w32(p.wrapping_add(4), TAG_ADDR);
    let p = c.r32(RING_PTR);
    c.w32(p.wrapping_add(8), 0);
    let p = c.r32(RING_PTR);
    c.w32(p.wrapping_add(0xc), TAG_TAIL);

    // Scratch registers as the retail code leaves them. The gp slot is the ring pointer itself, so it is stored last
    // (retail's delay-slot store), after the registers read the old position.
    let ring = c.r32(RING_PTR);
    let next = ring.wrapping_add(RING_STRIDE);
    set32(c, 4, TAG_TAIL);
    set32(c, 5, ring);
    set32(c, 3, ring);
    let gp = c.g(28) as u32;
    c.w32(gp.wrapping_add(GP_RING_SLOT as u32), next);
    c.ret(next);
    true
}

/// 0x291738 chunk_blocks(dst, src, size): rounds size up to 16 KB and, per 16 KB block, calls the split, reset,
/// copy and flush helpers. The loop runs while the remaining size is positive.
pub fn chunk_blocks(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let dst = c.arg(0);
    let src = c.arg(1);
    let size = c.arg(2);
    let saves = [(16, 0x70, h::SQ), (17, 0x80, h::SQ), (18, 0x90, h::SQ), (19, 0xa0, h::SQ), (31, 0xb0, h::SQ)];
    h::frame(c, FRAME_SIZE, &saves, |c, sp| {
        let rounded = size.wrapping_add(BLOCK_ROUND);
        set32(c, 6, rounded);
        set32(c, 2, (-BLOCK_SIZE) as u32);
        let mut remaining = (rounded & BLOCK_MASK) as i32;
        let mut offset: u32 = 0;
        while remaining > 0 {
            // Block source index: bits 8..23 of (src + offset), sign-extended.
            let index = (src.wrapping_add(offset) << 8) as i32 >> 16;
            h::call_keep(c, HELPER_SPLIT, &[sp, index as u32, 1, 0, 0, 0, 0x40, 0x40], &[]);
            remaining -= BLOCK_SIZE;
            h::call_keep(c, HELPER_RESET, &[0], &[]);
            h::call_keep(c, HELPER_COPY, &[sp, dst.wrapping_add(offset)], &[]);
            offset = offset.wrapping_add(BLOCK_SIZE as u32);
            h::call_keep(c, HELPER_FLUSH, &[0, 0], &[]);
        }
    });
    true
}
