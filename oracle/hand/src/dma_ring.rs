//! DMA packet-ring helpers from level 4 (rec/l4_01): tag writes at the ring head and the head pointer global.
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// The ring head pointer lives at gp - 0x5ac0.
const RING_HEAD_OFFSET: u32 = 0x5ac0;
/// Tag word values: base 0x10000000, the 0x40000000 flag, and the combined 0x50000000 used for the high word.
const TAG_BASE: u32 = 0x1000_0000;
const TAG_FLAG: u32 = 0x4000_0000;
const TAG_HIGH: u32 = 0x5000_0000;
/// Ring span is the quadword count times 16, plus one 16-byte header.
const HEADER_BYTES: u32 = 0x10;

/// Sign-extends a 32-bit MIPS result into the low 64 bits of a register.
fn sext(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// Reads a little-endian doubleword (LD).
fn read64(c: &mut Cpu, a: u32) -> u64 {
    c.r32(a) as u64 | (c.r32(a.wrapping_add(4)) as u64) << 32
}

/// Appends a tag for `n` quadwords at the ring head, advances the head by `n * 16 + 16` and leaves the
/// temporaries the retail code leaves behind.
fn push_ring_entry(c: &mut Cpu, n: u32) {
    let head_slot = (c.g(28) as u32).wrapping_sub(RING_HEAD_OFFSET);
    let head = c.r32(head_slot);
    let span = (n << 4).wrapping_add(HEADER_BYTES);
    let tag_low = TAG_BASE | n;
    let tag_high = TAG_HIGH | n;
    c.wv(head, V([tag_low, 0, 0, tag_high]));
    let new_head = head.wrapping_add(span);
    c.s(25, sext(span));
    c.s(1, sext(head.wrapping_add(HEADER_BYTES)));
    c.s(8, sext(tag_high));
    c.s(9, sext(TAG_FLAG));
    c.s(10, sext(new_head));
    c.w32(head_slot, new_head);
}

/// 0x1efdf4 ring_entry(n) -> void: writes a 16-byte tag for n quadwords at the DMA ring head and advances the head.
pub fn ring_entry(c: &mut Cpu) -> bool {
    let n = c.arg(0);
    push_ring_entry(c, n);
    true
}

/// 0x1efd74 setup_quads() -> void: pushes a 5-quadword ring entry, then writes five header quadwords after it
/// from the state block in s2.
pub fn setup_quads(c: &mut Cpu) -> bool {
    const FIVE: u32 = 5;
    let block = c.g(18) as u32;
    push_ring_entry(c, FIVE);
    let ring = c.g(1) as u32;
    let first = (0x68u128 << 64) | 0x2400_0000_0000_0001u128;
    let second = ((read64(c, block + 0x78) as u128) << 64) | read64(c, block + 0x70) as u128;
    let third = (0xeeu128 << 64) | 0x2000_0000_0000_8001u128;
    let fourth = ((0x14u128) << 64) | read64(c, block + 0x80) as u128;
    let fifth = ((0x42u128) << 64) | read64(c, block + 0x88) as u128;
    c.mem.wr128(ring, first);
    c.mem.wr128(ring.wrapping_add(0x10), second);
    c.mem.wr128(ring.wrapping_add(0x20), third);
    c.mem.wr128(ring.wrapping_add(0x30), fourth);
    c.mem.wr128(ring.wrapping_add(0x40), fifth);
    c.r[8] = third;
    c.r[10] = fifth;
    c.r[12] = fourth;
    c.s(9, 0xee);
    c.s(11, 0x42);
    c.s(4, FIVE as u64);
    // The retail pexew pair on $ra leaves the return address in the low doubleword and 0x1efd84 above it.
    c.r[31] = ((0x1efd84u128) << 64) | (c.g(31) as u128);
    true
}

/// 0x1efe34 clip_loop(): VU0 clipping pass over the vertex batch, driven by computed jump tables. Not rewritten.
pub fn clip_loop(_c: &mut Cpu) -> bool {
    false
}
