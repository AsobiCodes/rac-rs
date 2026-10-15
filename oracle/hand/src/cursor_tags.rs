//! Write-cursor packet helpers: `tag_ring_fill` writes the paired tag blocks for a run of 16 KiB chunks through the
//! global write cursor (calling the retail chunk writer for each chunk), and `cursor_put_tag` appends one tag
//! quadword at the cursor.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const AT: usize = 1;
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const GP: usize = 28;
const SP: usize = 29;
const RA: usize = 31;
/// Callee-saved registers the prologue spills, with their frame offsets (s0-s7, ra).
const SAVED: [(usize, u32); 9] = [
    (16, 0x00),
    (17, 0x10),
    (18, 0x20),
    (19, 0x30),
    (20, 0x40),
    (21, 0x50),
    (22, 0x60),
    (23, 0x70),
    (RA, 0x80),
];

/// Global write cursor (0x160000 + 0x1140): the address of the next free 16-byte slot.
const CURSOR: u32 = 0x0016_1140;
/// The cursor's upper half, left in `at` by the `lui` that precedes each cursor store.
const CURSOR_HI: u64 = 0x0016_0000;
/// Retail chunk writer, called with the tag pointer and seven more arguments.
const CHUNK_WRITER: u32 = 0x0012_2330;
/// Stack frame size of `tag_ring_fill` (`addiu sp, sp, -0x90`).
const FRAME: u32 = 0x90;
/// Gp-relative word that mirrors the tag pointer (gp - 0x5ac0).
const GP_PTR: u32 = 0x5ac0;
/// Uncached-alias bit added to the packet word by `cursor_put_tag`.
const UNCACHED: u32 = 0x9000_0000;

/// Tag words: head of the first block, count tag of the first block, and the two tags of the second block.
const TAG_HEAD: u32 = 0x1000_0006;
const TAG_FIRST: u32 = 0x5000_0006;
const TAG_SECOND: u32 = 0x3000_0400;
const TAG_THIRD: u32 = 0x5000_0400;
/// One chunk is 16 KiB; the length is rounded up to whole chunks.
const CHUNK: u32 = 0x4000;
const CHUNK_MASK: u32 = 0xffff_c000;
/// Bytes the cursor advances per chunk (two 16-byte blocks of tags are written at +0 and +0x60).
const BLOCK: u32 = 0x60;
const QUAD: u32 = 0x10;

/// Sign-extends a 32-bit result the way a MIPS 64-bit register holds it.
fn sx(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// 0x2917e8 tag_ring_fill(base, seed, len) -> v0 = the cursor plus 0x10 after the last chunk (the chunk mask when
/// `len` rounds up to no chunk). Spills s0-s7 and ra to the frame. For each 16 KiB chunk of `len` (rounded up):
/// writes a tag block at the write cursor, calls the retail chunk writer on the block's data (a0 = block + 0x10),
/// then writes a second block at cursor + 0x60 whose word at +4 is `base + chunk offset`.
pub fn tag_ring_fill(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let base = c.g(A0) as u32;
    let seed = c.g(A1) as u32;
    let len = (c.g(A2) as u32).wrapping_add(CHUNK - 1) & CHUNK_MASK;
    let sp = c.g(SP) as u32;
    let gp = c.g(GP) as u32;
    let frame = sp.wrapping_sub(FRAME);
    for (reg, off) in SAVED {
        let v = c.r[reg];
        for k in 0..4 {
            c.w32(frame.wrapping_add(off + 4 * k), (v >> (32 * k)) as u32);
        }
    }

    c.s(V0, sx(CHUNK_MASK));
    let mut remaining = len as i32;
    let mut offset: u32 = 0;
    while remaining > 0 {
        let a1 = ((seed.wrapping_add(offset) << 8) as i32 >> 16) as u32;
        let p = c.r32(CURSOR);
        c.w32(p, TAG_HEAD);
        let p = c.r32(CURSOR);
        c.w32(p.wrapping_add(4), 0);
        remaining -= CHUNK as i32;
        let p = c.r32(CURSOR);
        c.w32(p.wrapping_add(8), 0);
        let p = c.r32(CURSOR);
        c.w32(p.wrapping_add(0xc), TAG_FIRST);
        let arg = c.r32(CURSOR).wrapping_add(QUAD);
        c.s(V0, sx(arg));
        c.s(A0, sx(arg));
        c.w32(gp.wrapping_sub(GP_PTR), arg);
        // Chunk writer arguments: a0 = block + 0x10, a1 = shifted seed, a2 = 1, a3 = t0 = t1 = 0, t2 = t3 = 0x40.
        h::call_keep(c, CHUNK_WRITER, &[arg, a1, 1, 0, 0, 0, 0x40, 0x40], &[]);

        // Second block at the cursor's value +0x60 (the cursor moves there first).
        let p1 = c.r32(CURSOR);
        let a0 = base.wrapping_add(offset);
        offset = offset.wrapping_add(CHUNK);
        c.s(V1, sx(p1));
        let next = p1.wrapping_add(BLOCK);
        c.s(V0, sx(next));
        c.s(AT, CURSOR_HI);
        c.w32(CURSOR, next);
        c.w32(p1.wrapping_add(BLOCK), TAG_SECOND);
        let p2 = c.r32(CURSOR);
        c.w32(p2.wrapping_add(4), a0);
        let p3 = c.r32(CURSOR);
        c.s(V1, sx(p3));
        c.w32(p3.wrapping_add(8), 0);
        let p4 = c.r32(CURSOR);
        c.s(A0, sx(p4));
        c.w32(p4.wrapping_add(0xc), TAG_THIRD);
        let p5 = c.r32(CURSOR).wrapping_add(QUAD);
        c.s(V0, sx(p5));
        c.w32(gp.wrapping_sub(GP_PTR), p5);
    }
    true
}

/// 0x291bc0 cursor_put_tag(word) -> v0 = the new cursor: stores `word + 0x9000_0000` and three zero words as one
/// 16-byte tag at the write cursor (0x161140) and advances the cursor by 0x10.
pub fn cursor_put_tag(c: &mut Cpu) -> bool {
    let p = c.r32(CURSOR);
    let w = (c.g(A0) as u32).wrapping_add(UNCACHED);
    c.s(V1, sx(UNCACHED));
    c.s(A0, sx(w));
    c.s(A1, sx(p));
    let next = p.wrapping_add(QUAD);
    c.s(V0, sx(next));
    c.s(AT, CURSOR_HI);
    c.w32(CURSOR, next);
    c.w32(p, w);
    c.w32(p.wrapping_add(0xc), 0);
    c.w32(p.wrapping_add(4), 0);
    c.w32(p.wrapping_add(8), 0);
    true
}
