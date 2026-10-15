//! SPR DMA tag-chain flush (0x27c388) and the per-slot list dispatch (0x27c4c0).

use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;
use crate::spr_stream;

const AT: usize = 1;
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const S0: usize = 16;
const SP: usize = 29;
const GP: usize = 28;
const RA: usize = 31;

/// Pointer to the first SPR list header.
const LIST_A: u32 = 0x1604b0;
/// Pointer to the second SPR list header.
const LIST_B: u32 = 0x1604b4;
/// Cursor: the next free 16-byte slot of the SPR area.
const CURSOR: u32 = 0x161140;
/// Flag word: nonzero when the SPR packet emitter runs.
const EMIT_FLAG: u32 = 0x16a0f4;
/// Offset from gp of the hold pointer (the tag written when no emitter runs).
const HOLD_OFF: u32 = 0x5ac0;
/// Word copied at the end of the flush: source and destination.
const COPY_SRC: u32 = 0x15ee78;
const COPY_DST: u32 = 0x15ee74;
/// Value of the upper half of the 0x16xxxx base (lui 0x16).
const BASE_HI: u32 = 0x160000;
/// Value of the 0x17xxxx base (lui 0x17).
const BASE_17: u32 = 0x170000;
/// DMA tag written into each list head.
const TAG_END: u32 = 0x2000_0000;
/// Table of SPR slot list pointers; count (halfword) at +6.
const SLOT_TABLE: u32 = 0x16049c;
/// Size of one tag entry in the chain (the cursor step).
const TAG_STEP: u32 = 0x10;

/// Writes a register as a sign-extended 32-bit value, the way the EE's 32-bit ops leave it.
fn set(c: &mut Cpu, r: usize, v: u32) {
    c.s(r, v as i32 as i64 as u64);
}

/// 0x27c388 spr_chain_flush(): closes the two SPR DMA tag lists. Each list head gets the end tag, the cursor advances
/// by one 16-byte slot per list, the emitter (0x23fce0) and 0x2915b0 run when the flag word is set, and the list-B
/// head is copied to the 0x15ee74 word. Returns v0 = list B's head address.
pub fn spr_chain_flush(c: &mut Cpu) -> bool {
    let cursor0 = c.r32(CURSOR);
    c.w32(LIST_B, cursor0);
    h::frame(c, 0x20, &[(S0, 0, h::SQ), (RA, 0x10, h::SQ)], |c, _sp| {
        set(c, S0, TAG_END);
        set(c, A1, BASE_17);
        let list_a = c.r32(LIST_A);
        c.w32(CURSOR, cursor0.wrapping_add(TAG_STEP));
        c.w32(list_a, TAG_END);
        c.w32(list_a + 4, cursor0.wrapping_add(TAG_STEP));
        c.w32(list_a + 8, 0);
        c.w32(list_a + 0xc, 0);
        let hold = c.r32((c.g(GP) as u32).wrapping_sub(HOLD_OFF));
        let target = if c.r32(EMIT_FLAG) != 0 {
            set(c, A0, list_a);
            set(c, V0, list_a);
            set(c, V1, hold);
            h::run_keep(c, spr_stream::emit_spr_packets, &[list_a], &[]);
            h::call_keep(c, 0x2915b0, &[], &[]);
            c.r32(CURSOR)
        } else {
            hold
        };
        c.w32(target, TAG_END);
        let list_a = c.r32(LIST_A);
        let cur = c.r32(CURSOR);
        c.w32(cur + 4, list_a.wrapping_add(TAG_STEP));
        let cur = c.r32(CURSOR);
        c.w32(cur + 8, 0);
        let cur = c.r32(CURSOR);
        c.w32(cur + 0xc, 0);
        let cur = c.r32(CURSOR).wrapping_add(TAG_STEP);
        c.w32(CURSOR, cur);
        let list_b = c.r32(LIST_B);
        c.w32(list_b, TAG_END);
        let cur = c.r32(CURSOR);
        c.w32(list_b + 4, cur);
        c.w32(list_b + 8, 0);
        c.w32(list_b + 0xc, 0);
        let copy = c.r32(COPY_SRC);
        c.w32(COPY_DST, copy);
        set(c, V0, list_b);
        set(c, A0, list_b);
        set(c, V1, copy);
        set(c, AT, BASE_HI);
    });
    true
}

/// 0x27c4c0 spr_slot_dispatch(index): when index is below the slot table count, flushes the list in slot `index`
/// (0x27c758 when it holds packets, else 0x27c518). Otherwise returns with v0 = index << 2.
pub fn spr_slot_dispatch(c: &mut Cpu) -> bool {
    let index = c.arg(0);
    h::frame(c, 0x10, &[(RA, 0, h::SQ)], |c, _sp| {
        let table = c.r32(SLOT_TABLE);
        set(c, V1, table);
        let count = c.r16(table.wrapping_add(6)) as i16 as i32;
        let scaled = index << 2;
        set(c, V0, scaled);
        if (index as i32) < count {
            let addr = table.wrapping_add(scaled);
            set(c, V0, addr);
            let entry = c.r32(addr.wrapping_add(0x20));
            set(c, A0, entry);
            let list = c.r32(entry.wrapping_add(4));
            set(c, V1, list);
            let callee = if list == 0 { 0x27c518 } else { 0x27c758 };
            h::call_keep(c, callee, &[entry], &[]);
        }
    });
    true
}
