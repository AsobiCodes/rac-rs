//! Slot-table helpers from the level-4 context block at 0x1842f0: the slot picker (0x239338) and the bit-mask row
//! blend (0x239498).

use crate::h;
use crate::misc_leaf;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// MIPS register numbers used below.
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const V1: usize = 3;
const T0: usize = 8;
const T1: usize = 9;
const T2: usize = 10;
const T3: usize = 11;

/// Retail byte-scoring helper called once per slot (0x2392e0).
const SLOT_SCORE: u32 = 0x0023_92e0;
/// Context block the slot tables live in (retail `s7 + 0x42f0`).
const CTX: u32 = 0x0018_42f0;
/// Word: pending request id; when non-zero the backward search is skipped.
const OFF_PENDING: u32 = 0x224;
/// Word: slot index that is excluded from the search.
const OFF_EXCLUDE: u32 = 0x2a0;
/// Flag words, one per slot (slot i at `OFF_FLAG + 4 * i`).
const OFF_FLAG: u32 = 0x278;
/// Entry words, one per slot (slot i at `OFF_ENTRY + 4 * i`).
const OFF_ENTRY: u32 = 0x28c;
/// Number of slots in the forward scan.
const SLOT_COUNT: u32 = 5;
/// Last slot index tested by the backward search (retail starts at 4).
const LAST_SEARCH_SLOT: i32 = 4;
/// Marks an empty entry.
const EMPTY: u32 = u32::MAX;
/// Sign-extends a 32-bit value to the 64-bit register image.
fn sx(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// Number of outer rows in the blend (retail loops while the row counter is at most 0x7fff).
const ROWS: u32 = 0x8000;
/// Bits per output row byte.
const BITS: u32 = 8;

/// 0x239338 pick_slot(): picks the slot to use for a request -> v0: the chosen slot index.
/// Searches slots from 4 down to 0 for a flagged, non-excluded slot whose entry is empty, returning it. If the
/// pending request is empty, scores every flagged slot against the pending entry and keeps the best one, which is
/// then marked empty. Returns 1 when the pending entry scores as -1.
pub fn pick_slot(c: &mut Cpu) -> bool {
    if c.r32(CTX + OFF_PENDING) == 0 {
        let exclude = c.r32(CTX + OFF_EXCLUDE);
        let mut i = LAST_SEARCH_SLOT;
        while i >= 0 {
            let slot = i as u32;
            if c.r32(CTX + OFF_FLAG + 4 * slot) != 0 && slot != exclude && c.r32(CTX + OFF_ENTRY + 4 * slot) == EMPTY {
                c.ret(slot);
                c.s(V1, sx(CTX + OFF_ENTRY + 4 * slot));
                c.s(A0, sx(slot));
                c.s(A1, sx(CTX));
                c.s(A2, sx(EMPTY));
                return true;
            }
            i -= 1;
        }
        // Exhausted: retail leaves v1 at the last slot address it formed, a0 = -1, a1/a2 as set on entry.
        c.s(V1, sx(CTX + OFF_ENTRY - 4));
        c.s(A0, sx(EMPTY));
        c.s(A1, sx(CTX));
        c.s(A2, sx(EMPTY));
    }

    let pending = c.r32(CTX + OFF_PENDING);
    let (base_score, _) = h::call_keep(c, SLOT_SCORE, &[pending], &[]);
    if base_score == EMPTY {
        c.ret(1);
        return true;
    }

    let exclude = c.r32(CTX + OFF_EXCLUDE);
    let mut best_diff: i32 = 0;
    let mut best_slot: i32 = -1;
    for slot in 0..SLOT_COUNT {
        if c.r32(CTX + OFF_FLAG + 4 * slot) == 0 || slot == exclude {
            continue;
        }
        let entry = c.r32(CTX + OFF_ENTRY + 4 * slot);
        if entry == EMPTY {
            c.ret(slot);
            c.s(A0, sx(EMPTY));
            return true;
        }
        let (score, _) = h::call_keep(c, SLOT_SCORE, &[entry & 0xff], &[]);
        let (dist, _) = h::run(c, misc_leaf::iabs, &[score.wrapping_sub(base_score)], &[]);
        c.s(V1, sx(dist));
        c.s(A0, sx(dist));
        if best_diff < dist as i32 {
            best_diff = dist as i32;
            best_slot = slot as i32;
        }
    }

    // Retail: nor / movz -- an index of -1 becomes slot 0.
    if best_slot == -1 {
        best_slot = 0;
    }
    let chosen = best_slot as u32;
    c.w32(CTX + OFF_ENTRY + 4 * chosen, EMPTY);
    c.ret(chosen);
    c.s(V1, sx(CTX + 4 * chosen));
    c.s(A0, sx(chosen << 2));
    c.s(A1, sx(EMPTY));
    true
}

/// 0x239498 mask_select_rows(dst, a, b, mask): blends 0x8000 rows of 8 bytes -> v0: 1.
/// Row `r` takes byte `k` of `a` where bit `k` of mask byte `r` is set, otherwise byte `k` of `b`; the mask pointer
/// advances one byte per row. Scratch registers are left as the retail loop leaves them.
pub fn mask_select_rows(c: &mut Cpu) -> bool {
    let mut dst = c.arg(0);
    let mut src_a = c.arg(1);
    let mut src_b = c.arg(2);
    let mut mask = c.arg(3);
    let mut row: u32 = 0;
    let mut t0: u32 = 1;
    for _ in 0..ROWS {
        let row_mask = mask;
        let mut bit: u32 = 1;
        for _ in 0..BITS {
            let set = c.r8(row_mask) as u32 & bit;
            let byte = if set != 0 { c.r8(src_a) } else { c.r8(src_b) };
            c.w8(dst, byte);
            src_a = src_a.wrapping_add(1);
            src_b = src_b.wrapping_add(1);
            dst = dst.wrapping_add(1);
            bit <<= 1;
        }
        t0 = bit;
        mask = mask.wrapping_add(1);
        row += 1;
    }
    c.set_arg(0, dst);
    c.set_arg(1, src_a);
    c.set_arg(2, src_b);
    c.set_arg(3, mask);
    c.s(T0, t0 as u64);
    c.s(T1, mask as u64);
    c.s(T2, row as u64);
    c.s(T3, 0x7fff);
    c.s(V1, row as u64);
    c.ret(1);
    true
}
