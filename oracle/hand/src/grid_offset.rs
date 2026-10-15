//! Grid-cell offset vectors for the camera (retail 0x2e73f0).

use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Word holding the base address of the cell table.
const CELL_TABLE_PTR: u32 = 0x0016_0074;
/// First scratch vector (the scaled offset); the second one sits 0x10 below it.
const OUT_A: u32 = 0x0017_3f70;
const OUT_B: u32 = OUT_A - 0x10;
/// Grid entry field holding the cell index.
const ENTRY_INDEX: u32 = 0x14;
/// Cell records are 128 bytes; the cell centre sits at +0x30 in a record.
const CELL_SHIFT: u32 = 7;
const CELL_CENTRE: u32 = 0x30;
/// Stack frame of the retail function: scratch quads at +0x00 (grid query) and +0x10 (difference), saved
/// registers at +0x20..+0x60, the caller's f20-f22 at +0x70..+0x80.
const FRAME_SIZE: u32 = 0x90;
const SCRATCH_QUAD: u32 = 0x00;
const DIFF: u32 = 0x10;
const SAVED_F: u32 = 0x70;
const ONE: F = F(0x3f80_0000);
/// Retail vector helpers: VU0 residue matters to the caller, so they run as retail code.
const VEC_SUB: u32 = 0x001f_1840;
const VEC_ADD: u32 = 0x001f_1810;
const VEC_SCALE: u32 = 0x001f_1898;
const LENGTH3: u32 = 0x001f_1970;
/// Grid-cell lookup (camera_tick::grid_find_entry is its hand).
const GRID_FIND_ENTRY: u32 = 0x001e_0a18;

/// 0x2e73f0 cell_offset(pos*, radius) -> 1 when a grid entry holds pos, else 0: writes two scaled offsets from
/// the cell centre to pos into OUT_A and OUT_B.
pub fn cell_offset(c: &mut Cpu) -> bool {
    let pos = c.arg(0);
    let radius = c.farg(0);
    let saved_f = [c.f[20], c.f[21], c.f[22]];
    let saves = [(16, 0x20, h::SQ), (17, 0x30, h::SQ), (18, 0x40, h::SQ), (19, 0x50, h::SQ), (31, 0x60, h::SQ)];
    h::frame(c, FRAME_SIZE, &saves, |c, sp| {
        for (i, bits) in saved_f.iter().enumerate() {
            c.w32(sp + SAVED_F + 8 * i as u32, *bits);
        }
        let quad = c.rv(pos);
        c.wv(sp + SCRATCH_QUAD, quad);
        let entry = h::call_keep(c, GRID_FIND_ENTRY, &[sp + SCRATCH_QUAD], &[radius]).0;
        if entry == 0 {
            c.ret(0);
            return true;
        }
        let index = c.r32(entry + ENTRY_INDEX);
        let cell = c.r32(CELL_TABLE_PTR).wrapping_add(index << CELL_SHIFT);
        let centre = cell + CELL_CENTRE;
        let diff = sp + DIFF;
        h::call_keep(c, VEC_SUB, &[diff, pos, centre], &[]);
        let len_cell = h::call_keep(c, LENGTH3, &[cell], &[]).1;
        let len_diff = h::call_keep(c, LENGTH3, &[diff], &[]).1;
        let inv = ONE / len_diff;
        h::call_keep(c, VEC_SCALE, &[OUT_A, diff], &[(len_cell + radius) * inv]);
        h::call_keep(c, VEC_SCALE, &[OUT_B, diff], &[len_cell * inv]);
        h::call_keep(c, VEC_ADD, &[OUT_A, OUT_A, centre], &[]);
        h::call_keep(c, VEC_ADD, &[OUT_B, OUT_B, centre], &[]);
        c.ret(1);
        true
    })
}
