//! Record-pool slot helpers: alloc, release and pointer rebase (retail 0x230c90..0x230de8, 0x232bb0..0x232c08).

use crate::h;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Threshold float at 0x15f5b4 (retail `lwc1 $f1` from `lui 0x16` and offset -0xa4c).
const LIMIT_ADDR: u32 = 0x15_f5b4;
/// 0.8f, the value compared against the threshold (`mtc1` 0x3f4ccccd).
const SCALE: F = F(0x3f4c_cccd);
/// Slot blocks: 8 blocks of 0x30 bytes from 0x1805c0; each block's flag word sits at +0x10.
const SLOT_BASE: u32 = 0x18_05c0;
const SLOT_STRIDE: u32 = 0x30;
const SLOT_FLAG: u32 = 0x10;
const SLOT_COUNT: u32 = 8;
/// Per-slot 32-byte entries from 0x1803c0 (quadword plus five floats).
const ENTRY_BASE: u32 = 0x18_03c0;
const ENTRY_STRIDE: u32 = 0x20;
/// Buffer pointer stored in a claimed block: 0x180740 plus 0x400 per slot.
const BUF_BASE: u32 = 0x18_0740;
const BUF_STRIDE: u32 = 0x400;
/// Retail routine that runs before a slot is released.
const PRE_RELEASE: u32 = 0x23_1348;

/// First free slot: slot 0 if its flag is clear, else the first clear flag in 1..8; `SLOT_COUNT` when all are set.
fn find_free_slot(c: &mut Cpu) -> u32 {
    let mut slot = 0;
    if c.r32(SLOT_BASE + SLOT_FLAG) != 0 {
        slot = 1;
        while slot < SLOT_COUNT && c.r32(SLOT_BASE + SLOT_FLAG + slot * SLOT_STRIDE) != 0 {
            slot += 1;
        }
    }
    slot
}

/// 0x230c90 alloc_record(src) -> v0: claims the first free slot (-1 if none, or if 0.8 is below the
/// float at 0x15f5b4); stores src's quadword and floats f12..f16 in its entry, clears its block, sets the buffer
/// pointer and flag; returns the slot index.
pub fn alloc_record(c: &mut Cpu) -> bool {
    let src = c.arg(0);
    let saves = [(16, 0x00, h::SQ), (17, 0x10, h::SQ), (31, 0x20, h::SQ)];
    let result = h::frame(c, 0x30, &saves, |c, _sp| {
        if SCALE < c.rf(LIMIT_ADDR) {
            return None;
        }
        let slot = find_free_slot(c);
        if slot == SLOT_COUNT {
            return None;
        }
        let entry = ENTRY_BASE + slot * ENTRY_STRIDE;
        let (f12, f13, f14, f15, f16) = (c.farg(0), c.farg(1), c.farg(2), c.farg(3), c.farg(4));
        let quad = c.rv(src);
        c.wv(entry + 0x10, quad);
        // Retail store order matters: f12 overwrites the last word of the quadword.
        c.wf(entry + 0x0c, f13);
        c.wf(entry + 0x1c, f12);
        c.wf(entry, f14);
        c.wf(entry + 0x04, f15);
        c.wf(entry + 0x08, f16);
        let block = SLOT_BASE + slot * SLOT_STRIDE;
        h::run(c, mathx::zero_qwords, &[block, SLOT_STRIDE], &[]);
        c.w32(block + 0x0c, BUF_BASE + slot * BUF_STRIDE);
        c.w32(block + SLOT_FLAG, 1);
        Some(slot)
    });
    c.ret(result.unwrap_or(u32::MAX));
    true
}

/// 0x230d90 release_record(slot) -> v0: calls 0x231348, clears the slot's 0x20-byte entry and its 0x30-byte block.
pub fn release_record(c: &mut Cpu) -> bool {
    let slot = c.arg(0);
    let saves = [(16, 0x00, h::SQ), (31, 0x10, h::SQ)];
    h::frame(c, 0x20, &saves, |c, _sp| {
        h::call(c, PRE_RELEASE, &[slot], &[]);
        h::run(c, mathx::zero_qwords, &[ENTRY_BASE + slot * ENTRY_STRIDE, 0x20], &[]);
        h::run(c, mathx::zero_qwords, &[SLOT_BASE + slot * SLOT_STRIDE, SLOT_STRIDE], &[]);
    });
    c.ret(SLOT_STRIDE);
    true
}

/// 0x232bb0 rebase_record(table, index): table[index] (word at +0x48 + 4*index) is a record; its word at +0x14 becomes
/// relative-to-absolute (record + offset) when nonzero, and each of the `count` (byte at +0x10) words from +0x1c
/// gets the record base added.
pub fn rebase_record(c: &mut Cpu) -> bool {
    let table = c.arg(0);
    let index = c.arg(1).wrapping_shl(2);
    let rec = c.r32(table.wrapping_add(index).wrapping_add(0x48));
    let off = c.r32(rec.wrapping_add(0x14));
    if off != 0 {
        c.w32(rec.wrapping_add(0x14), rec.wrapping_add(off));
    }
    let count = c.r8(rec.wrapping_add(0x10)) as u32;
    c.s(4, rec as u64);
    if count == 0 {
        c.s(5, 0);
    } else {
        for i in 0..count {
            let at = rec.wrapping_add(0x1c).wrapping_add(4 * i);
            let v = c.r32(at);
            c.w32(at, rec.wrapping_add(v));
        }
        c.s(5, count as u64);
        c.s(3, rec.wrapping_add(0x1c).wrapping_add(4 * count) as u64);
    }
    c.s(2, 0);
    true
}
