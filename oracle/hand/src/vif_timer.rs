//! VIF0 DMA chain start and the timer record ring helpers at 0x290f20..0x2910e8 (rec/l4_13). The ring is three
//! words at 0x161138 with a 0/1 index at 0x161150; the record block is at 0x173f00.
use crate::h;
use crate::misc_leaf;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const AT: usize = 1;
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;
const T0: usize = 8;
const GP: usize = 28;

/// VIF0 DMA channel registers: CHCR (busy bit in CHCR_STR), QWC and TADR.
const VIF0_CHCR: u32 = 0x1000_8000;
const VIF0_QWC: u32 = 0x1000_8020;
const VIF0_TADR: u32 = 0x1000_8030;
/// CHCR start/busy bit.
const CHCR_STR: u32 = 0x100;
/// CHCR value that starts a chain transfer from memory.
const CHCR_START_CHAIN: u32 = 0x145;
/// Mask on the tag address handed to the DMA channel.
const TAG_MASK: u32 = 0x0fff_ffff;
/// Argument passed to countdown_by_two while the channel is busy.
const WAIT_STEP: u32 = 0x10;

/// Upper half of the 0x16 data page (`lui 0x16`).
const PAGE16: u32 = 0x0016_0000;
/// Word holding the table index (`lui 0x16; -0x127c`).
const TABLE_INDEX: u32 = PAGE16 - 0x127c;
/// Index limit: indices of 19 and up read entry 0.
const TABLE_LIMIT: i32 = 0x13;
/// Table of words indexed by TABLE_INDEX (`lui 0x1c; +0x21b8`).
const TABLE_BASE: u32 = 0x001c_21b8;
/// Gp-relative offset of the word stored by indexed_table_word.
const GP_TABLE_WORD: u32 = 0x5ab4;

/// Record block (`lui 0x17; +0x3f00`): the words at +4 and +8 feed the ring.
const RECORD: u32 = 0x0017_3f00;
/// Word added to the first record field (`lui 0x16; +0x114c`).
const BASE_WORD: u32 = PAGE16 + 0x114c;
/// Word subtracted from the sum (`lui 0x16; -0xaa8` from 0x160000 is 0x15f558).
const SUB_WORD: u32 = PAGE16 - 0xaa8;
/// Three-word ring at 0x161138.
const RING: u32 = PAGE16 + 0x1138;
/// Ring index (0 or 1) at 0x161150.
const RING_INDEX: u32 = PAGE16 + 0x1150;
/// Last value at 0x161140.
const LAST: u32 = PAGE16 + 0x1140;
/// Copy of the previous last value at 0x161144.
const LAST_COPY: u32 = PAGE16 + 0x1144;
/// Output word written by timer_record_load (`lui 0x16; -0xa24`).
const OUT_LOAD: u32 = PAGE16 - 0xa24;
/// Output word written by timer_ring_advance (`lui 0x16; -0xa28`).
const OUT_ADVANCE: u32 = PAGE16 - 0xa28;
/// Gp-relative offsets of the two words stored by the timer helpers.
const GP_LOAD_WORD: u32 = 0x7628;
const GP_ADVANCE_WORD: u32 = 0x7624;
/// Bias subtracted from the timer sums (`addiu -0x2000`).
const TIMER_BIAS: u32 = 0x2000;

/// Sign-extends a 32-bit MIPS word into the 64-bit GPR `n`.
fn sx(c: &mut Cpu, n: usize, v: u32) {
    c.s(n, v as i32 as i64 as u64);
}

/// The retail spin: while the VIF0 channel is busy, call countdown_by_two(16) and look again.
fn wait_channel_idle(c: &mut Cpu) {
    while c.r32(VIF0_CHCR) & CHCR_STR != 0 {
        h::run_keep(c, misc_leaf::countdown_by_two, &[WAIT_STEP], &[]);
    }
}

/// 0x290f20 vif0_chain_start(tag) -> void: waits for VIF0 DMA to go idle, sets QWC 0 and TADR to tag & 0x0fffffff,
/// starts a chain transfer (CHCR 0x145) and waits for it to finish.
pub fn vif0_chain_start(c: &mut Cpu) -> bool {
    let tag = c.g(A0) as u32 & TAG_MASK;
    wait_channel_idle(c);
    c.w32(VIF0_QWC, 0);
    c.w32(VIF0_TADR, tag);
    c.w32(VIF0_CHCR, CHCR_START_CHAIN);
    c.s(A0, CHCR_START_CHAIN as u64);
    c.s(V1, VIF0_CHCR as u64);
    c.s(A1, VIF0_TADR as u64);
    wait_channel_idle(c);
    c.s(V0, 0);
    true
}

/// 0x290fe8 indexed_table_word() -> v0 = table entry address, v1 = its word: the index word at 0x15ed84 is used
/// as is below 19 and as 0 otherwise; the word is also stored at gp-0x5ab4.
pub fn indexed_table_word(c: &mut Cpu) -> bool {
    let index = c.r32(TABLE_INDEX) as i32;
    let index = if index < TABLE_LIMIT { index } else { 0 };
    let addr = (index as u32).wrapping_shl(2).wrapping_add(TABLE_BASE);
    let word = c.r32(addr);
    sx(c, V0, addr);
    sx(c, V1, word);
    sx(c, A0, TABLE_BASE);
    let gp = c.g(GP) as u32;
    c.w32(gp.wrapping_sub(GP_TABLE_WORD), word);
    true
}

/// 0x291018 timer_record_load() -> void: copies the record at 0x173f00 into the ring at 0x161138, resets the ring
/// index to 0, and stores the timer sums at 0x15f5dc (sum - 0x2000) and gp-0x7628 (sum).
pub fn timer_record_load(c: &mut Cpu) -> bool {
    let rec_a = c.r32(RECORD + 4);
    let rec_b = c.r32(RECORD + 8);
    let base = c.r32(BASE_WORD);
    let sub = c.r32(SUB_WORD);
    c.w32(RING, rec_a);
    let sum = base.wrapping_add(rec_a).wrapping_sub(sub);
    c.w32(RING + 4, rec_b);
    let out = sum.wrapping_sub(TIMER_BIAS);
    c.w32(RING_INDEX, 0);
    c.w32(OUT_LOAD, out);
    c.w32(LAST, rec_a);
    let gp = c.g(GP) as u32;
    c.w32(gp.wrapping_sub(GP_LOAD_WORD), sum);
    sx(c, V0, out);
    sx(c, V1, sum);
    sx(c, A0, PAGE16);
    sx(c, A1, rec_a);
    sx(c, A2, RING);
    sx(c, A3, rec_b);
    sx(c, T0, sub);
    sx(c, AT, PAGE16);
    true
}

/// 0x291078 timer_ring_advance() -> void: flips the ring index (1 - index), stores the ring word it selects as the
/// last value (the old last value goes to 0x161144), and stores the sums at 0x15f5d8 and gp-0x7624.
pub fn timer_ring_advance(c: &mut Cpu) -> bool {
    let index = c.r32(RING_INDEX) as i32;
    let next = 1i32.wrapping_sub(index) as u32;
    let slot = next.wrapping_shl(2).wrapping_add(RING);
    let word = c.r32(slot);
    let base = c.r32(BASE_WORD);
    let sub = c.r32(SUB_WORD);
    let last = c.r32(LAST);
    c.w32(LAST_COPY, last);
    c.w32(RING_INDEX, next);
    c.w32(LAST, word);
    let sum = word.wrapping_add(base).wrapping_sub(sub);
    c.w32(OUT_ADVANCE, sum);
    let out = sum.wrapping_sub(TIMER_BIAS);
    let gp = c.g(GP) as u32;
    c.w32(gp.wrapping_sub(GP_ADVANCE_WORD), out);
    sx(c, V0, out);
    sx(c, V1, word);
    sx(c, A0, sum);
    sx(c, A1, last);
    sx(c, A2, next);
    sx(c, A3, sub);
    sx(c, AT, PAGE16);
    true
}
