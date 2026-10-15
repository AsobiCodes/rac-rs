//! Row and record helpers from the L4_06 and L4_07 overlay: a level-indexed pointer-table copy, a level flag gate with one
//! retail call, a record-list size sum, and a bitwise CRC over bytes.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::{uw4, w4, Cpu};

/// Level word in RAM (`lui 0x16` + `lw -0x127c`); it selects a row of the row table.
const LEVEL_WORD: u32 = 0x0015_ED84;
/// Number of rows in the row table.
const ROW_COUNT: u32 = 0x13;
/// Row table: two words per row, the first and end index into the source table (`lui 0x1c` + `addiu 0x4538`).
const ROW_TABLE: u32 = 0x001C_4538;
/// Source table of record pointers, one word each (`lui 0x18` + `addiu -0x6d48`).
const SRC_TABLE: u32 = 0x0017_92B8;
/// Destination array of 16-byte entries (`lui 0x14` + `addiu -0x2a50`).
const DST_TABLE: u32 = 0x0013_D5B0;
/// Offsets in a source record of the three words copied to the entry: x, y, z.
const SRC_X: u32 = 0x10;
const SRC_Y: u32 = 0x14;
const SRC_Z: u32 = 0x48;

/// 0x23d0a8 pair_table_copy(): for the current level's row (LEVEL_WORD), copies the x, y, z words of every non-null
/// source record in the row's index range into its 16-byte destination entry. No args, no result.
pub fn pair_table_copy(c: &mut Cpu) -> bool {
    let row = c.r32(LEVEL_WORD);
    if row >= ROW_COUNT {
        // Retail: the bound test fails, v0 = 0, v1 still holds the level word, and the delay slot left a0 = row + 1.
        c.s(2, 0);
        c.s(3, row as i32 as i64 as u64);
        c.s(4, row.wrapping_add(1) as u64);
        return true;
    }
    let row_ptr = ROW_TABLE + row * 4;
    let next_ptr = ROW_TABLE + (row + 1) * 4;
    let first = c.r32(row_ptr) as i32;
    let last = c.r32(next_ptr) as i32;
    if first >= last {
        c.s(2, 0);
        c.s(3, row_ptr as u64);
        c.s(4, next_ptr as u64);
        c.s(6, first as i64 as u64);
        c.s(7, last as i64 as u64);
        return true;
    }
    let (mut f0, mut f1) = (c.f[0], c.f[1]);
    let mut loaded = 0;
    for j in first..last {
        let j = j as u32;
        let src = c.r32(SRC_TABLE + j * 4);
        loaded = src;
        if src != 0 {
            let dst = DST_TABLE + j * 16;
            f0 = c.r32(src + SRC_X);
            c.w32(dst, f0);
            f1 = c.r32(src + SRC_Y);
            c.w32(dst + 4, f1);
            f0 = c.r32(src + SRC_Z);
            c.w32(dst + 8, f0);
        }
    }
    // Retail leaves the loop counter at zero, the pointers one past the last index, and v0 = the last pointer word.
    let end = last as u32;
    c.ret(loaded);
    c.s(3, SRC_TABLE as u64);
    c.s(4, DST_TABLE.wrapping_add(end * 16) as u64);
    c.s(5, SRC_TABLE.wrapping_add(end * 4) as u64);
    c.s(6, 0);
    c.s(7, last as i64 as u64);
    c.f[0] = f0;
    c.f[1] = f1;
    true
}

/// Level flag block (`lui 0x13` + `addiu -0x2d70`), with a count word at +0xd4, a signed word at +0xdc and a call flag
/// at +0xe4.
const FLAG_BLOCK: u32 = 0x0013_D290;
const FLAG_COUNT: u32 = 0xD4;
const FLAG_SIGN: u32 = 0xDC;
const FLAG_CALL: u32 = 0xE4;
/// Mode word (`lui 0x16` + `lw -0x1150`), written with 1, or with 0x15 after the call.
const MODE_WORD: u32 = 0x0015_EEB0;
/// Flags word read after the call (`lui 0x16` + `lw -0x114c`), ORed with 0x40 and stored at gp - 0x7d4c.
const FLAGS_WORD: u32 = 0x0015_EEB4;
const GP_FLAG_OFF: u32 = 0x7D4C;
/// Retail callee run with (3, 0) when the call flag is set.
const CALLEE: u32 = 0x001F_4840;
/// Frame size of the retail prologue (`addiu sp, sp, -0x10`), with ra saved at 0(sp).
const FRAME: u64 = 0x10;
/// The `at` register after `lui at, 0x16`.
const AT_HI: u64 = 0x0016_0000;

/// 0x23d860 level_flag_gate(): with the count word below 3 and the signed word at +0xdc negative, sets the mode word to
/// 1, or (when the call flag at +0xe4 is set) calls 0x1f4840 with (3, 0), sets the mode word to 0x15 and stores the
/// flags word | 0x40 at gp - 0x7d4c. No args, no result (v0 is set as retail leaves it).
pub fn level_flag_gate(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let sp = c.g(29);
    let frame = sp.wrapping_sub(FRAME);
    let slot = frame as u32;
    // sq ra, 0(sp) in the new frame; lq ra reloads it on every path below.
    for (k, w) in w4(c.r[31]).into_iter().enumerate() {
        c.w32(slot + 4 * k as u32, w);
    }
    c.s(29, frame);
    c.s(3, FLAG_BLOCK as u64);
    let count = c.r32(FLAG_BLOCK + FLAG_COUNT);
    let sign = c.r32(FLAG_BLOCK + FLAG_SIGN);
    let call = c.r32(FLAG_BLOCK + FLAG_CALL);
    if (count as i32) >= 3 {
        c.s(2, 0);
    } else if (sign as i32) >= 0 {
        c.ret(sign);
    } else if call == 0 {
        c.ret(1);
        c.s(1, AT_HI);
        c.s(4, 3);
        c.w32(MODE_WORD, 1);
    } else {
        h::call_keep(c, CALLEE, &[3, 0], &[]);
        let flags = c.r32(FLAGS_WORD) | 0x40;
        let gp_slot = (c.g(28) as u32).wrapping_sub(GP_FLAG_OFF);
        c.s(3, 0x15);
        c.s(1, AT_HI);
        c.ret(flags);
        c.w32(MODE_WORD, 0x15);
        c.w32(gp_slot, flags);
    }
    // lq ra, 0(sp) and the epilogue addiu sp, sp, 0x10.
    let reloaded = [c.r32(slot), c.r32(slot + 4), c.r32(slot + 8), c.r32(slot + 12)];
    c.r[31] = uw4(reloaded);
    c.s(29, sp);
    true
}

/// 0x23f4c0 record_list_bytes(list) -> u32: total size of a list of records (a nonzero word, then a size word at +4,
/// 16-byte stride) ended by a zero word. Each record adds 8 + size and the sum rounds up to 4; returns the total + 8.
pub fn record_list_bytes(c: &mut Cpu) -> bool {
    let mut p = c.arg(0);
    if c.r32(p) == 0 {
        c.s(5, 8);
        c.ret(16);
        return true;
    }
    let mut total: u32 = 8;
    let mut next = 0;
    loop {
        let size = c.r32(p + 4);
        total = total.wrapping_add(8).wrapping_add(size);
        p = p.wrapping_add(0x10);
        next = c.r32(p);
        total = total.wrapping_add(3) & !3;
        if next == 0 {
            break;
        }
    }
    c.s(3, next as i32 as i64 as u64);
    c.s(4, p as i32 as i64 as u64);
    c.s(5, total as i32 as i64 as u64);
    c.s(6, -4i64 as u64);
    c.ret(total.wrapping_add(8));
    true
}

/// Largest byte count the CRC handles; a length at or above it returns 0 without touching the bytes.
const CRC_MAX_LEN: i32 = 0x1801;
/// Seed of the CRC register (`lui 0xedb8` + `ori 0x8320`).
const CRC_SEED: u32 = 0xEDB8_8320;
/// Feedback term applied when the top bit of the working word is set (`xori 0x1f45`).
const CRC_TAP: u32 = 0x1F45;

/// 0x23f4f8 crc_bytes(start, len) -> u32: bitwise CRC over the len bytes at start, seeded with 0xedb88320, returning the
/// low 16 bits. a1 is left as start + len and a0 as the end pointer. len >= 0x1801 returns 0.
pub fn crc_bytes(c: &mut Cpu) -> bool {
    let start = c.arg(0);
    let len = c.arg(1);
    let end = start.wrapping_add(len);
    c.s(5, end as i32 as i64 as u64);
    if (len as i32) >= CRC_MAX_LEN {
        c.ret(0);
        return true;
    }
    let mut crc = CRC_SEED;
    let mut p = start;
    if p < end {
        loop {
            let byte = c.r8(p) as u32;
            p = p.wrapping_add(1);
            crc ^= byte << 8;
            for _ in 0..8 {
                crc = if crc & 0x8000 != 0 { (crc << 1) ^ CRC_TAP } else { crc << 1 };
            }
            if p >= end {
                break;
            }
        }
        c.s(6, u64::MAX);
    }
    c.s(3, crc as i32 as i64 as u64);
    c.s(4, p as i32 as i64 as u64);
    c.ret(crc & 0xFFFF);
    true
}
