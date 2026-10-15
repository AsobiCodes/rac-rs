//! Level 4 (rec/l4_07) slot gather: marks the byte indices listed by each record in a 256-byte map at BUF, writes the
//! index list after the map, then copies the 64-byte scratchpad slots named by the list to the destination.
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Scratch buffer: the 0x80-byte presence map, then the index list.
const BUF: u32 = 0x0019_6f00;
const LIST_OFFSET: u32 = 0x80;
/// Scratchpad slot table; slot n is at SLOT_TABLE + n * 0x40.
const SLOT_TABLE: u32 = 0x7000_0000;
const SLOT_SHIFT: u32 = 6;
const SLOT_BYTES: u32 = 0x40;
/// Retail callee that consumes the buffer.
const CONSUME_FN: u32 = 0x0024_69d0;
/// The argument struct holds the record table pointer at +0x24; that table holds the index table at +0x1c.
const RECORD_TABLE_OFFSET: u32 = 0x24;
const INDEX_TABLE_OFFSET: u32 = 0x1c;
/// Byte written into the map for every listed index, and the end marker after the largest index.
const MAP_MARK: u8 = 1;
const END_MARK: u8 = 0xff;
/// Scratchpad base for the quadword copies.
const SCRATCHPAD: u32 = 0x7000_0000;

/// Sets a 32-bit result in a GPR, sign-extended as MIPS 32-bit ops leave it.
fn set32(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// Packs a quadword into the 128-bit register image (lane 0 is the lowest 32 bits).
fn quad(v: V) -> u128 {
    v.0.iter().enumerate().fold(0u128, |acc, (i, &w)| acc | (w as u128) << (32 * i))
}

/// 0x246868 gather_slots(a0 = struct, s4..s7 = callee arg / list count / index list / destination) -> (): marks
/// each listed byte index in the map, writes the index list, calls 0x2469d0 and copies the marked slots to s7.
pub fn gather_slots(c: &mut Cpu) -> bool {
    // The retail code parks a0..a3 in the upper halves of s4..s7 and restores them as the lower halves at exit.
    let arg0 = c.g(4);
    let arg1 = c.g(5);
    let arg2 = c.g(6);
    let arg3 = c.g(7);
    let ra_low = c.g(31);
    // pcpyld: each saved argument goes into the upper half of its register; the lower halves stay.
    for (reg, v) in [(20, arg0), (21, arg1), (22, arg2), (23, arg3), (31, ra_low)] {
        c.r[reg] = (c.r[reg] as u64 as u128) | (v as u128) << 64;
    }
    let s4 = c.g(20) as u32;
    let count = c.g(21) as u32 as i32;
    let list = c.g(22) as u32;
    let dst = c.g(23) as u32;

    let record_table = c.r32((arg0 as u32).wrapping_add(RECORD_TABLE_OFFSET));
    let index_table = c.r32(record_table.wrapping_add(INDEX_TABLE_OFFSET));

    for q in 0..8u32 {
        c.wv(BUF + q * 16, V([0; 4]));
    }

    // Outer loop: one record per listed index; the do-while runs at least once.
    let mut v0 = count;
    let mut v1 = BUF + LIST_OFFSET;
    let mut ptr = list;
    let mut t6: i32 = 0;
    let mut a2: u32 = 0;
    let mut a3: u32 = 0;
    loop {
        let idx = c.r32(ptr);
        ptr = ptr.wrapping_add(4);
        let record = c.r32((idx << 2).wrapping_add(index_table).wrapping_add(4));
        a2 = c.r16(record) as u32;
        let mut a1 = record;
        // Inner loop: mark each byte of the record's index run; runs at least once.
        loop {
            a3 = c.r8(a1.wrapping_add(4)) as u32;
            a1 = a1.wrapping_add(1);
            a2 = a2.wrapping_sub(1);
            c.w8(BUF.wrapping_add(a3), MAP_MARK);
            if (a2 as i32) <= 0 {
                break;
            }
        }
        c.w8(v1, a3 as u8);
        v1 = v1.wrapping_add(1);
        v0 = v0.wrapping_sub(1);
        t6 = t6.max(a3 as i32);
        if v0 <= 0 {
            break;
        }
    }

    // The map's end marker sits at the largest index, and the byte after the map holds largest + 1.
    let top = t6 as u32;
    c.w8(BUF + 0x7f, top.wrapping_add(1) as u8);
    let end_addr = top.wrapping_add(BUF);
    c.w8(end_addr, END_MARK);

    // Registers the retail jal leaves for the callee.
    set32(c, 1, END_MARK as u32);
    set32(c, 2, top.wrapping_add(1));
    set32(c, 3, v1);
    set32(c, 4, s4);
    set32(c, 5, BUF);
    set32(c, 6, a2);
    set32(c, 7, a3);
    set32(c, 8, a3.wrapping_add(BUF));
    // t8 still holds the index table pointer that the retail lw chain loaded.
    set32(c, 24, index_table);
    set32(c, 14, end_addr);
    set32(c, 15, BUF);
    // The callee is retail code: its own writes to s4..s7 and ra (both halves) stay, as after a jal.
    c.set_arg(0, s4);
    c.set_arg(1, BUF);
    c.call(CONSUME_FN);

    // Copy loop: one 64-byte slot per listed index, read from the scratchpad, written at dst + i * 0x40.
    let mut at = c.g(21) as u32 as i32;
    let mut out = (c.g(23) as u32).wrapping_sub(SLOT_BYTES);
    let mut read = BUF + LIST_OFFSET;
    let mut src: u32 = 0;
    let mut last = [V([0; 4]); 4];
    loop {
        let slot = c.r8(read) as u32;
        read = read.wrapping_add(1);
        at = at.wrapping_sub(1);
        src = (slot << SLOT_SHIFT).wrapping_add(SCRATCHPAD);
        last = [c.rv(src), c.rv(src.wrapping_add(16)), c.rv(src.wrapping_add(32)), c.rv(src.wrapping_add(48))];
        out = out.wrapping_add(SLOT_BYTES);
        for (i, q) in last.iter().enumerate() {
            c.wv(out.wrapping_add(16 * i as u32), *q);
        }
        if at <= 0 {
            break;
        }
    }

    // Final register state of the retail copy loop and the epilogue.
    set32(c, 1, at as u32);
    set32(c, 2, out);
    set32(c, 3, read);
    set32(c, 4, src);
    c.r[5] = quad(last[0]);
    c.r[6] = quad(last[1]);
    c.r[7] = quad(last[2]);
    c.r[8] = quad(last[3]);
    set32(c, 14, SCRATCHPAD);
    set32(c, 15, BUF);
    // The retail epilogue (pcpyud) moves each upper half down and zeroes the upper half.
    for reg in [20, 21, 22, 23, 31] {
        c.r[reg] >>= 64;
    }
    true
}
