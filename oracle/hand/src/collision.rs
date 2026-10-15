//! Collision data lookups.
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// The cell at (i, j, k) of a three-level sparse grid, 0 when outside or empty. Each level is a u16 start, a u16
/// count and the entries for start..start+count: level 1 u16 word offsets of the rows, levels 2 and 3 u32 byte
/// offsets (from `grid`) of the columns and the cell values.
pub fn grid_cell(c: &mut Cpu, grid: u32, i: i32, j: i32, k: i32) -> u32 {
    let level = |c: &mut Cpu, a: u32, n: i32| -> Option<u32> {
        let (start, count) = (c.r16(a) as i32, c.r16(a + 2) as i32);
        let n = n.wrapping_sub(start);
        (n >= 0 && count - n > 0).then_some(n as u32)
    };
    let Some(n) = level(c, grid, i) else { return 0 };
    let row = 4 * c.r16(grid + 4 + 2 * n) as u32;
    if row == 0 {
        return 0;
    }
    let row = grid + row;
    let Some(n) = level(c, row, j) else { return 0 };
    let col = c.r32(row + 4 + 4 * n);
    if col == 0 {
        return 0;
    }
    let col = grid.wrapping_add(col);
    let Some(n) = level(c, col, k) else { return 0 };
    c.r32(col + 4 + 4 * n)
}

/// 0x1e2438 (register call: grid t8, i v1, j v0, k at; result in at).
pub fn grid_cell_reg(c: &mut Cpu) -> bool {
    let (grid, i, j, k) = (c.g(24) as u32, c.g(3) as i32, c.g(2) as i32, c.g(1) as i32);
    let v = grid_cell(c, grid, i, j, k);
    c.s(1, v as i32 as i64 as u64);
    true
}

/// Collision result block: word +0x1c holds the surface flags of the last hit (negative = none).
const HIT_FLAGS: u32 = 0x173f5c;

/// 0x1e5e40 hit_surface_class() -> flags & 0x1f, or -1 when there was no hit or the class is 0x1f.
pub fn hit_surface_class(c: &mut Cpu) -> bool {
    let v = c.r32(HIT_FLAGS) as i32;
    let class = v & 0x1f;
    c.ret(if v < 0 || class == 0x1f { -1i32 as u32 } else { class as u32 });
    true
}

/// 0x1e5e70 hit_surface_group() -> (flags & 0x60) >> 5, or 0 when there was no hit or both bits are set.
pub fn hit_surface_group(c: &mut Cpu) -> bool {
    let v = c.r32(HIT_FLAGS) as i32;
    let group = v & 0x60;
    c.ret(if v < 0 || group == 0x60 { 0 } else { (group >> 5) as u32 });
    true
}

/// 0x1e5f60 hit_queue_flush(): calls 0x1e5fd8 for each queued 16-byte record at 0x16a940 (count at gp-0x7c3c,
/// re-read each time), then empties the queue and resets the write pointer (gp-0x7c40) to 0x16a140.
pub fn hit_queue_flush(c: &mut Cpu) -> bool {
    let sv = crate::camobj::save(c);
    let gp = c.g(28) as u32;
    let count = gp.wrapping_sub(0x7c3c);
    let mut i = 0;
    while (i as i32) < c.r32(count) as i32 {
        let rec = 0x16a940 + 16 * i;
        for k in 0..4 {
            let w = c.r32(rec + 4 * k);
            c.set_arg(k as usize, w);
        }
        c.call(0x1e5fd8);
        i += 1;
    }
    c.w32(count, 0);
    c.w32(gp.wrapping_sub(0x7c40), 0x16a140);
    crate::camobj::restore(c, &sv);
    true
}

/// 0x1e9488 / 0x1e9600 / 0x1e9608: empty hooks (return immediately).
pub fn empty_hook(_c: &mut Cpu) -> bool {
    true
}

/// 0x1e95d8 move_pending_flag(): moves the word at +0x364 of block 0x169d80 to +0x368, clearing it and the word at +0x1b0.
pub fn move_pending_flag(c: &mut Cpu) -> bool {
    let v = c.r32(0x169d80 + 0x364);
    c.w32(0x169d80 + 0x364, 0);
    c.w32(0x169d80 + 0x368, v);
    c.w32(0x169d80 + 0x1b0, 0);
    true
}
