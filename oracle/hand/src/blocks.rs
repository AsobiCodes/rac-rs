//! Display-list block builders: `fill_slot_words` splats a word over the slot words of a record table and
//! `build_lod_blocks` selects the visible items, then writes two 0x70-byte transfer blocks (high and low detail)
//! per selected item.
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// gp-relative game globals (gp = 0x1b0b00-ish; offsets from the disassembly).
const GP_FILL_SIZE: i32 = -0x6c88;
const GP_ITEM_COUNT: i32 = -0x6cc0;
const GP_FLAG_COUNT: i32 = -0x6cb8;
const GP_BLOCK_PTR: i32 = -0x5ac0;
const GP_BLOCK_CONSTS: i32 = -0x6460;
/// Fill table (16 bytes per entry), selection inputs and outputs.
const FILL_TABLE: u32 = 0x1a_c300;
const ITEM_A: u32 = 0x19_a380;
const ITEM_B_SPR: u32 = 0x7000_3a00;
const FLAG_SPR: u32 = 0x7000_3e00;
const KEPT_LIST: u32 = 0x19_a700;
const PART_LISTS: u32 = 0x19_aa80;
const ITEM_WORDS: u32 = 0x19_9b80;
const SLOT_WORDS: u32 = 0x19_9f80;
/// Marks an unused B entry.
const B_UNUSED: u32 = 0x4000_0000;
/// Block count limit: stop once the running offset passes this.
const OFFSET_LIMIT: u32 = 0x3ffb;

/// 0x248138 fill_slot_words(): for each 16-byte entry of the fill table (word value, record list, slot list, count),
/// stores the value into the destination word of every slot of every record (the number of words per record is
/// the byte at +0xf of the record, at least 2).
pub fn fill_slot_words(c: &mut Cpu) -> bool {
    let gp = c.g(28) as u32;
    let size = c.r32(gp.wrapping_add(GP_FILL_SIZE as u32));
    let end = FILL_TABLE.wrapping_add(size);
    let mut entry = FILL_TABLE;
    loop {
        let value = c.r32(entry);
        let (mut rec, mut slot) = (c.r32(entry + 4), c.r32(entry + 8));
        let mut left = c.r32(entry + 0xc) as i32;
        loop {
            let mut dst = c.r32(slot + 4);
            let mut n = c.r8(rec + 0xf) as i32;
            c.w32(dst, value);
            n -= 1;
            dst += 4;
            if n != 0 {
                loop {
                    c.w32(dst, value);
                    n -= 1;
                    dst += 4;
                    if n <= 0 {
                        break;
                    }
                    c.w32(dst, value);
                    n -= 1;
                    dst += 4;
                    if n == 0 {
                        break;
                    }
                }
            }
            left -= 1;
            slot += 8;
            rec += 0x10;
            if left <= 0 {
                break;
            }
        }
        entry += 0x10;
        if entry == end {
            break;
        }
    }
    true
}

/// MIPS register numbers of the scratch registers that the retail code leaves behind (read by the caller).
const AT: usize = 1;
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A3: usize = 7;
const T0: usize = 8;
const T1: usize = 9;
const T2: usize = 10;
const T3: usize = 11;
const T4: usize = 12;
const T5: usize = 13;
const T6: usize = 14;
const T7: usize = 15;
const T8: usize = 24;
const T9: usize = 25;
const GP: usize = 28;

/// One 0x70-byte block at `r[T8]`: five constant qwords (k), a code word 0x53, the log2 size `level`, the running
/// offset `r[T9]` and the source word `r[A0]` (the low block moves the source past the high one). Updates the
/// registers retail leaves: at = 0x5000_0000 + quarter, v1 = level, a0 = source, t9 = next offset.
fn write_block(c: &mut Cpu, r: &mut [u32; 32], k: &[V; 5], level: u32, low: bool) {
    let dst = r[T8];
    let off = r[T9];
    let src = r[A0];
    for i in 0..4 {
        c.wv(dst + 0x10 * i, k[i as usize]);
    }
    c.wv(dst + 0x40, V([0; 4]));
    c.w32(dst + 0x48, 0x53);
    c.wv(dst + 0x50, k[4]);
    c.w16(dst + 0x24, off as u16);
    let e = level as i32 - 6;
    if e > 0 {
        c.w8(dst + 0x26, (1u32 << (e & 31)) as u8);
    }
    let n = 1u32 << (level & 31);
    c.w32(dst + 0x30, n);
    c.w32(dst + 0x34, n);
    let area = n << (level & 31);
    let quarter = area >> 4;
    let src = if low { src.wrapping_add(area << 2) } else { src };
    c.w32(dst + 0x50, 0x8000 + quarter);
    c.wv(dst + 0x60, V([0; 4]));
    c.w32(dst + 0x64, src);
    c.w32(dst + 0x60, 0x3000_0000 + quarter);
    c.w32(dst + 0x6c, 0x5000_0000 + quarter);
    r[AT] = 0x5000_0000 + quarter;
    r[V1] = level;
    r[A0] = src;
    r[T9] = off.wrapping_add(quarter >> 4);
}

/// 0x247e88 build_lod_blocks(first_offset) -> final offset << 8: selects items (B != unused and 2A - B > 0), flags
/// the parts of each selected item (1 = low detail only, 3 when A > B too), then for each flagged part writes
/// a high-detail block (flag bit 1) and a low-detail block (one level smaller) at the block pointer, recording
/// the running offset in the part's slot word; stops when the offset passes 0x3ffb.
/// The scratch registers are kept as retail leaves them (their values depend on the last path taken).
pub fn build_lod_blocks(c: &mut Cpu) -> bool {
    let mut r = [0u32; 32];
    for (n, v) in r.iter_mut().enumerate().skip(1) {
        *v = c.g(n) as u32;
    }
    let gp = r[GP];
    let gpr = |off: i32| gp.wrapping_add(off as u32);
    r[T9] = r[A0] >> 8;
    r[T8] = c.r32(gpr(GP_BLOCK_PTR));
    r[T0] = B_UNUSED;
    r[T1] = 0;
    r[T3] = PART_LISTS;
    r[T4] = KEPT_LIST;
    r[T5] = FLAG_SPR;
    r[T6] = ITEM_A;
    r[T7] = ITEM_B_SPR;
    r[T2] = c.r32(gpr(GP_ITEM_COUNT));
    for i in 0..0x10 {
        c.wv(FLAG_SPR + 0x10 * i, V([0; 4]));
    }
    r[AT] = FLAG_SPR + 0x100;
    r[V0] = FLAG_SPR + 0x100;

    // Select items and mark the parts they use.
    while r[T1] != r[T2] {
        r[T1] = r[T1].wrapping_add(1);
        r[AT] = c.r32(r[T7]);
        r[T7] = r[T7].wrapping_add(4);
        r[V0] = c.r32(r[T6]);
        r[T6] = r[T6].wrapping_add(4);
        let (b, a) = (r[AT], r[V0]);
        r[T3] = r[T3].wrapping_add(0x10);
        if b == r[T0] {
            continue;
        }
        r[A0] = r[T1].wrapping_sub(1);
        c.w32(r[T4], r[A0]);
        let two_a_minus_b = (a << 1).wrapping_sub(b);
        r[V1] = a.wrapping_sub(b);
        if two_a_minus_b as i32 <= 0 {
            continue;
        }
        r[V1] = if (a.wrapping_sub(b) as i32) > 0 { 3 } else { 1 };
        r[T4] = r[T4].wrapping_add(4);
        r[AT] = r[T3].wrapping_sub(0x10);
        r[V0] = 0xff;
        loop {
            let part = c.r8(r[AT]) as u32;
            r[AT] = r[AT].wrapping_add(1);
            r[A0] = part.wrapping_add(r[T5]);
            if part == 0xff {
                break;
            }
            r[A1] = (c.r8(r[A0]) as u32) | r[V1];
            c.w8(r[A0], r[A1] as u8);
        }
    }
    c.w32(r[T4], 0xffff_ffff);
    r[AT] = 0xffff_ffff;

    // Write the blocks.
    let mut k = [V([0; 4]); 5];
    let mut kq = [0u128; 5];
    for i in 0..5 {
        let addr = gpr(GP_BLOCK_CONSTS + 0x10 * i as i32);
        k[i] = c.rv(addr);
        kq[i] = c.mem.rd128(addr);
    }
    for i in 0..5 {
        c.r[T3 + i] = kq[i];
    }
    r[T2] = SLOT_WORDS;
    r[A3] = ITEM_WORDS;
    r[T0] = FLAG_SPR;
    r[T1] = c.r32(gpr(GP_FLAG_COUNT)).wrapping_add(FLAG_SPR);
    loop {
        // retail's beq delay slot (lbu at,(t0)) runs on the exit branch too
        r[AT] = c.r8(r[T0]) as u32;
        if r[T0] == r[T1] {
            break;
        }
        r[T0] = r[T0].wrapping_add(1);
        r[A0] = c.r32(r[A3]);
        r[A3] = r[A3].wrapping_add(4);
        c.w32(r[T2], 0);
        r[T2] = r[T2].wrapping_add(4);
        let flags = r[AT];
        if flags == 0 {
            continue;
        }
        r[V0] = flags & 2;
        r[V1] = r[A0] >> 28;
        r[A0] = (r[A0] << 4) >> 4;
        let level = r[V1];
        if r[V0] != 0 {
            let used = r[T9];
            write_block(c, &mut r, &k, level, false);
            c.w16(r[T2].wrapping_sub(4), used as u16);
            if (r[T9].wrapping_sub(OFFSET_LIMIT) as i32) > 0 {
                break;
            }
            r[T8] = r[T8].wrapping_add(0x70);
        }
        let used = r[T9];
        write_block(c, &mut r, &k, level.wrapping_sub(1), true);
        c.w16(r[T2].wrapping_sub(2), used as u16);
        if (r[T9].wrapping_sub(OFFSET_LIMIT) as i32) > 0 {
            break;
        }
        r[T8] = r[T8].wrapping_add(0x70);
    }
    c.w32(gpr(GP_BLOCK_PTR), r[T8]);
    for n in [AT, V1, A0, A1, A3, T0, T1, T2, T8, T9] {
        c.s(n, r[n] as i32 as i64 as u64);
    }
    c.ret(r[T9] << 8);
    true
}
