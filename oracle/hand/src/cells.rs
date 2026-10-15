//! The 64 x 64 grid of 16 m moby cells: each cell lists the ids of the mobys whose bounding-sphere rectangle covers
//! it, in a pool of 32-byte blocks (16 ids each) handed out by a bitmap allocator.
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Cell table, 4 bytes per cell at 4 * x + 256 * y: u16 first block, u8 id count, u8 block count.
const CELLS: u32 = 0x19b8e0;
/// Block pool: block b holds 16 u16 moby ids at SLOTS + 32 * b.
const SLOTS: u32 = 0x19f8e0;
/// Allocation bitmap of the block pool, one bit per block.
const BLOCK_MAP: u32 = 0x1ab8e0;

fn cell_addr(x: u32, y: u32) -> u32 {
    CELLS + 4 * x + 256 * y
}

/// The cells of a cell rectangle key (bytes x0, y0, x1, y1), row by row.
fn cells(key: u32) -> impl Iterator<Item = (u32, u32)> {
    let [x0, y0, x1, y1] = key.to_le_bytes().map(u32::from);
    (y0..=y1).flat_map(move |y| (x0..=x1).map(move |x| (x, y)))
}

/// Whether cell (x, y) lies outside the rectangle key (byte-wise saturating compare, as retail's psubsb).
fn outside(x: u32, y: u32, key: u32) -> bool {
    let k = key.to_le_bytes();
    let d = |v: u32, b: u8| (v as u8 as i8).saturating_sub(b as i8);
    d(x, k[0]) < 0 || d(y, k[1]) < 0 || d(x, k[2]) > 0 || d(y, k[3]) > 0
}

/// Frees `n` blocks from `start` (at least one); stops at a block that is already free (retail traps there).
fn free_blocks(c: &mut Cpu, start: u32, n: u32) {
    let mut n = n as i32;
    for b in start.. {
        let a = BLOCK_MAP + (b >> 3);
        let old = c.r8(a);
        let new = old & !(1 << (b & 7));
        n -= 1;
        if new == old {
            return;
        }
        c.w8(a, new);
        if n <= 0 {
            return;
        }
    }
}

/// First-fit allocation of `n` blocks inside one 32-block bitmap word, at multiples of `n`; returns the first block.
fn alloc_blocks(c: &mut Cpu, n: u32) -> u32 {
    let mask = ((1u32 << (n & 31)) as i32).wrapping_sub(1) as i64 as u64;
    let shift = n & 63;
    let mut first = 0u32;
    let mut a = BLOCK_MAP;
    loop {
        let word = c.r32(a) as u64;
        a += 4;
        if word != 0xffff_ffff {
            let (mut bits, mut at) = (mask, 0u32);
            loop {
                let hit = word & bits;
                bits <<= shift;
                at += n;
                if hit == 0 {
                    break;
                }
            }
            at -= n;
            bits >>= shift;
            if at != 32 {
                c.w32(a - 4, (word | bits) as u32);
                return first + at;
            }
        }
        first += 32;
    }
}

/// Copies `n` blocks (at least one) from block `from` to block `to`.
fn copy_blocks(c: &mut Cpu, from: u32, to: u32, n: i32) {
    let (src, dst) = (SLOTS.wrapping_add(from << 5), SLOTS.wrapping_add(to << 5));
    for i in 0..n.max(1) as u32 {
        for q in [0, 16] {
            let v = c.rv(src + 32 * i + q);
            c.wv(dst + 32 * i + q, v);
        }
    }
}

/// Removes moby `id` from a cell (swap with the last id); halves the cell's blocks when it got sparse.
fn remove(c: &mut Cpu, cell: u32, id: u32) {
    let base = SLOTS + 32 * c.r16(cell) as u32;
    let end = base + 2 * c.r8(cell + 2) as u32;
    let Some(p) = (base..end).step_by(2).find(|&p| c.r16(p) as u32 == id) else {
        return; // retail traps: the moby is not listed
    };
    let last = c.r16(end - 2);
    c.w16(p, last);
    c.w16(end - 2, 0);
    let count = (c.r8(cell + 2) as i32) - 1;
    let blocks = c.r8(cell + 3) as i32;
    c.w8(cell + 2, count as u8);
    if count == 0 || (blocks != 1 && blocks * 8 - 4 - count > 0) {
        shrink(c, cell);
    }
}

/// Moves a cell's ids to half as many blocks (none when it had one).
fn shrink(c: &mut Cpu, cell: u32) {
    let blocks = c.r8(cell + 3) as u32;
    let start = c.r16(cell) as u32;
    free_blocks(c, start, blocks);
    let old = c.r16(cell) as i16 as i32 as u32;
    let half = blocks >> 1;
    c.w16(cell, 0);
    c.w8(cell + 3, half as u8);
    if half == 0 {
        return;
    }
    let new = alloc_blocks(c, half);
    c.w16(cell, new as u16);
    let n = c.r8(cell + 3) as i8 as i32;
    copy_blocks(c, old, new, n);
}

/// Appends moby `id` to a cell, first moving it to twice as many blocks (at least one) when it is full.
fn add(c: &mut Cpu, cell: u32, id: u32) {
    let blocks = c.r8(cell + 3) as i32;
    if blocks == 0 || blocks * 16 - c.r8(cell + 2) as i32 <= 0 {
        let grown = (blocks << 1).max(1) as u32;
        c.w8(cell + 3, grown as u8);
        let new = alloc_blocks(c, grown);
        let old = c.r16(cell) as u32;
        c.w16(cell, new as u16);
        if blocks != 0 {
            free_blocks(c, old, blocks as u32);
            let to = c.r16(cell) as u32;
            let n = (c.r8(cell + 3) >> 1) as i32;
            copy_blocks(c, old, to, n);
        }
    }
    let count = c.r8(cell + 2) as u32;
    let block = c.r16(cell) as u32;
    c.w8(cell + 2, count as u8 + 1);
    c.w16(SLOTS + 2 * count + 32 * block, id as u16);
}

/// Moves moby `m` from the cells of its stored rectangle key (+0xa0) to those of `key` (a negative key files it
/// nowhere); cells in both keep it.
pub fn refile_cells(c: &mut Cpu, m: u32, key: u32) {
    let id = c.r32(m + 0xac);
    let old = c.r32(m + 0xa0);
    if (old as i32) >= 0 {
        for (x, y) in cells(old) {
            if outside(x, y, key) {
                remove(c, cell_addr(x, y), id);
            }
        }
    }
    c.w32(m + 0xa0, key);
    if (key as i32) < 0 {
        return;
    }
    for (x, y) in cells(key) {
        if outside(x, y, old) {
            add(c, cell_addr(x, y), id);
        }
    }
}

/// 0x243308 free_blocks(start, n): frees n grid blocks.
pub fn free_blocks_hand(c: &mut Cpu) -> bool {
    let (start, n) = (c.arg(0), c.arg(1));
    free_blocks(c, start, n);
    true
}

/// 0x243368 alloc_blocks(n) -> first block: allocates n grid blocks.
pub fn alloc_blocks_hand(c: &mut Cpu) -> bool {
    let n = c.arg(0);
    let b = alloc_blocks(c, n);
    c.ret(b);
    true
}

/// 0x2433e8 actor_refile_cells(moby, key): refiles a moby in the grid cells of its sphere rectangle.
pub fn actor_refile_cells(c: &mut Cpu) -> bool {
    let (m, key) = (c.arg(0), c.arg(1));
    refile_cells(c, m, key);
    true
}
