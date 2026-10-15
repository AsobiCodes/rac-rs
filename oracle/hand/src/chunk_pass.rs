//! Chunk-table pass and the frame counter tick (retail overlay code, Eudora addresses).
use crate::h::{self, SQ};
use crate::ps2::Ee;
use ee_oracle::cpu::Cpu;

/// Stack frame of the chunk pass (`addiu sp,-0xb0`).
const FRAME: u32 = 0xb0;
/// Registers the chunk pass saves in its frame (s0-s7, fp, ra), at the offsets retail uses.
const SAVES: [(usize, u32, u8); 10] = [
    (16, 0x10, SQ),
    (17, 0x20, SQ),
    (18, 0x30, SQ),
    (19, 0x40, SQ),
    (20, 0x50, SQ),
    (21, 0x60, SQ),
    (22, 0x70, SQ),
    (23, 0x80, SQ),
    (30, 0x90, SQ),
    (31, 0xa0, SQ),
];
/// Pass state record: the chunk count at +0xa8 and the pass count at +0xac.
const STATE: u32 = 0x001C_0000 - 0x6210;
const STATE_CHUNKS: u32 = 0xa8;
const STATE_COUNT: u32 = 0xac;
/// Table of 8-byte chunk entries: a kind-table index word, then a chunk count word.
const ENTRIES: u32 = 0x001C_0000 - 0x6428;
/// Word table read by pass index (advances by 4 per chunk).
const PASS_WORDS: u32 = 0x001C_0000 - 0x60c8;
/// Word table of chunk ids, indexed by the running chunk index plus the entry's flag.
const CHUNK_IDS: u32 = 0x001C_0000 - 0x6160;
/// Byte table indexed by the entry's kind word.
const KIND_BYTES: u32 = 0x0019_0000 + 0x7cc0;
/// Pointer table indexed by kind (4-byte entries), each pointing at a record whose +0x48 + 4*n word gets the chunk.
const KIND_PTRS: u32 = 0x0019_0000 + 0x7400;
/// Frame counter word, and the flag word that gates the tick.
const COUNTER: u32 = 0x0016_0000 - 0x11dc;
const TICK_GATE: u32 = 0x0016_0000 - 0x0a74;
/// GP-relative words read by the frame counter tick: the frame count (-0x7ddc) and the bonus flag (-0x7e80).
const GP_FRAMES: u32 = 0x7ddc;
const GP_BONUS_FLAG: u32 = 0x7e80;
/// The frame counter is checked against this period.
const PERIOD: i32 = 50;

/// Sets register `r` to `v` as a sign-extended 32-bit result (the upper 32 bits follow the sign).
fn set(c: &mut Cpu, r: usize, v: u32) {
    c.s(r, v as i32 as i64 as u64);
}

/// Sets the low 64 bits of HI and LO to the sign-extended `hi` and `lo` (a 32-bit `div` result).
fn set_hilo(c: &mut Cpu, hi: i32, lo: i32) {
    let keep = !(u64::MAX as u128);
    c.hi = (c.hi & keep) | (hi as i64 as u64 as u128);
    c.lo = (c.lo & keep) | (lo as i64 as u64 as u128);
}

/// Body of the chunk pass, run inside its stack frame `sp`.
fn chunk_pass_body(c: &mut Cpu, sp: u32, chunks: u32, count: u32) {
    // a2 = a0 and a3 = the state record, as retail sets them before the loop test.
    set(c, 6, chunks);
    set(c, 7, STATE);
    c.w32(STATE + STATE_CHUNKS, chunks);
    c.w32(sp, count);
    set(c, 2, count);
    c.w32(STATE + STATE_COUNT, count);
    if (count as i32) <= 0 {
        return;
    }
    let mut i: u32 = 0;
    let mut pass_off = chunks << 2;
    let mut entry = (chunks << 3).wrapping_add(ENTRIES);
    loop {
        let first = c.r32(entry);
        let kind = c.r8(first.wrapping_add(KIND_BYTES)) as u32;
        let second = c.r32(entry.wrapping_add(4));
        // The flag is set only for a two-chunk pass, on its second chunk.
        let flag = u32::from(count == 2 && i == 1);
        entry = entry.wrapping_add(8);
        let pass_word = c.r32(PASS_WORDS.wrapping_add(pass_off));
        pass_off = pass_off.wrapping_add(4);
        let (scaled, _) = h::call_keep(c, 0x0028_46f0, &[pass_word], &[]);
        let scaled = scaled << 4;
        let id = c.r32(CHUNK_IDS.wrapping_add((i + flag) << 2));
        i += 1;
        let (base, _) = h::call_keep(c, 0x0027_6818, &[id], &[]);
        let dest = id.wrapping_add(base).wrapping_sub(scaled);
        h::call_keep(c, 0x0028_45d8, &[dest, pass_word, 0, u32::MAX, 0], &[]);
        h::call_keep(c, 0x0023_fe50, &[dest, id], &[]);
        let slot = KIND_PTRS.wrapping_add(kind << 2);
        let record = c.r32(slot);
        c.w32(record.wrapping_add(second << 2).wrapping_add(0x48), id);
        let record = c.r32(slot);
        h::call_keep(c, 0x0023_2bb0, &[record, second], &[]);
        if (i as i32) >= (count as i32) {
            break;
        }
    }
    // The loop test left 0 in v0 and the pass count in v1.
    set(c, 2, 0);
    set(c, 3, count);
}

/// 0x2772d8 chunk_pass(chunks, count): for each of the `count` chunk entries, computes the chunk's address from
/// the id table, unpacks it (`spr_lzo_unpack`), stores it in the kind's record and rebases the record. Saves
/// `chunks` and `count` in the state record. v0 = 0 after a full pass, or `count` when `count` <= 0.
pub fn chunk_pass(c: &mut Cpu) -> bool {
    let chunks = c.arg(0);
    let count = c.arg(1);
    h::frame(c, FRAME, &SAVES, |c, sp| chunk_pass_body(c, sp, chunks, count));
    true
}

/// 0x277898 frame_counter_tick(): returns the gate word at 0x15f58c when it is set. Otherwise the counter at
/// 0x15ee24 becomes frames + 1 (frames = gp[-0x7ddc]); on a multiple of 50 it becomes frames + 11 when the bonus
/// flag gp[-0x7e80] is set. v0 = the gate, or the bonus flag off a boundary, or frames + 11 on a boundary.
pub fn frame_counter_tick(c: &mut Cpu) -> bool {
    let gp = c.g(28) as u32;
    let gate = c.r32(TICK_GATE);
    let period_word = c.r32(gp.wrapping_sub(GP_FRAMES));
    set(c, 5, period_word);
    if gate != 0 {
        set(c, 2, gate);
        return true;
    }
    let sum = period_word.wrapping_add(1) as i32;
    set(c, 3, PERIOD as u32);
    set_hilo(c, sum % PERIOD, sum / PERIOD);
    set(c, 1, 0x0016_0000);
    c.w32(COUNTER, sum as u32);
    let rem = sum % PERIOD;
    set(c, 4, rem as u32);
    let bonus_flag = c.r32(gp.wrapping_sub(GP_BONUS_FLAG));
    if rem != 0 {
        set(c, 2, bonus_flag);
        return true;
    }
    let bumped = period_word.wrapping_add(11);
    set(c, 2, bumped);
    if bonus_flag != 0 {
        c.w32(COUNTER, bumped);
    }
    true
}
