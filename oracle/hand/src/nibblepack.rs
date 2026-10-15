//! Nibble-to-bit packing loop (0x23a7d0): for each of 256 indices, retail 0x23a920 fills a 512-byte scratchpad
//! block, and the 4-bit cells of that block are packed into 32 words of bits.
use crate::h;
use crate::ps2::*;

use ee_oracle::cpu::Cpu;

/// Retail helper that fills the scratchpad block for one index: (scratchpad, index, fill_a, fill_b).
const FILL_BLOCK: u32 = 0x0023_a920;
/// EE scratchpad base: where the helper writes and the packing reads.
const SCRATCH: u32 = 0x7000_0000;
/// Number of indices packed (the loop runs 0..0x100).
const INDICES: u32 = 0x100;
/// Scratchpad bytes per row (32 cells, two per byte) and rows per index.
const ROW_BYTES: u32 = 0x10;
const ROWS: u32 = 0x20;
/// Bytes between packed words of consecutive rows (16 words per row group).
const DST_ROW_STRIDE: u32 = 0x40;
/// Frame size of the retail prologue (`addiu sp,sp,-0x90`).
const FRAME: u32 = 0x90;

/// 0x23a7d0 pack_nibble_bits(dst, fill_a, fill_b) -> void: for each index 0..256, runs retail 0x23a920 to fill the
/// scratchpad block, then writes 32 words to dst: bit k of row r is set when cell k of row r is nonzero (even cells
/// are the low nibble of a byte, odd cells the high nibble). Index 0..256 is never negative, so retail's
/// round-toward-zero adjustment for the block index is a plain shift.
pub fn pack_nibble_bits(c: &mut Cpu) -> bool {
    let dst = c.arg(0);
    let fill_a = c.arg(1);
    let fill_b = c.arg(2);
    let saves = [
        (16, 0x00, h::SQ),
        (17, 0x10, h::SQ),
        (18, 0x20, h::SQ),
        (19, 0x30, h::SQ),
        (20, 0x40, h::SQ),
        (21, 0x50, h::SQ),
        (22, 0x60, h::SQ),
        (23, 0x70, h::SQ),
        (31, 0x80, h::SQ),
    ];
    h::frame(c, FRAME, &saves, |c, _sp| {
        let mut base = dst;
        for index in 0..INDICES {
            h::call_keep(c, FILL_BLOCK, &[SCRATCH, index, fill_a, fill_b], &[]);
            // Block index: 16 indices per group, each group 512 words apart; the 16 entries interleave by index & 15.
            base = dst.wrapping_add(4 * ((index & 0xf) + 512 * (index >> 4)));
            for row in 0..ROWS {
                let word = pack_row(c, row);
                c.w32(base.wrapping_add(row * DST_ROW_STRIDE), word);
            }
        }
        set_exit_regs(c, base.wrapping_add(ROWS * DST_ROW_STRIDE));
    });
    true
}

/// Leaves the scratch registers as retail's last pass of the loop does (index 255, row 31, cell 31): a1 is one past
/// the last row's word pointer, a2 and t3 = 0x20, t2 = 0x1f0, t4 = 0x100, v0 = 0, v1 = 1, a0 = 0x20, a3 = 0.
fn set_exit_regs(c: &mut Cpu, a1: u32) {
    c.s(2, 0);
    c.s(3, 1);
    c.s(4, ROWS as u64);
    c.s(5, a1 as i32 as i64 as u64);
    c.s(6, ROWS as u64);
    c.s(7, 0);
    c.s(8, 0x1f);
    c.s(9, 0);
    c.s(10, ((ROWS - 1) << 4) as u64);
    c.s(11, ROWS as u64);
    c.s(12, INDICES as u64);
}

/// Packs one scratchpad row of 32 cells into a word: bit k is set when cell k is nonzero.
fn pack_row(c: &mut Cpu, row: u32) -> u32 {
    let line = SCRATCH + ROW_BYTES * row;
    let mut word = 0u32;
    for k in 0..32u32 {
        let byte = c.r8(line + k / 2);
        // Even cells are the low nibble of the byte, odd cells the high nibble.
        let cell = if k & 1 == 0 { byte & 0xf } else { byte >> 4 };
        if cell != 0 {
            word |= 1 << k;
        }
    }
    word
}
