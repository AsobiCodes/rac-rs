use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const REG_AT: usize = 1;
const REG_V0: usize = 2;
const REG_V1: usize = 3;
const REG_A0: usize = 4;
const REG_A1: usize = 5;
const REG_A3: usize = 7;
const REG_T0: usize = 8;
const REG_GP: usize = 28;

/// Base of the slot pointer table (lui 0x1b, addiu -0x46c0 in retail).
const SLOT_TABLE: u32 = 0x1AB940;
/// Upper half of the scratch globals (lui 0x16) used by retail.
const GLOBAL_HI: u32 = 0x160000;
/// Word holding the highest valid slot index (lw -0x8c).
const SLOT_COUNT_WORD: u32 = GLOBAL_HI - 0x8C;
/// Word holding the base address added to a slot's offset (lw -0xa8).
const SLOT_BASE_WORD: u32 = GLOBAL_HI - 0xA8;
/// Halfword that receives the slot's 15-bit index (sh).
const SLOT_INDEX_HALF: u32 = GLOBAL_HI + 0x90;
/// Word that receives the current slot address (sw).
const SLOT_ADDR_WORD: u32 = GLOBAL_HI + 0x94;
/// Gp-relative word that keeps the raw table entry.
const GP_ENTRY_OFS: i32 = -0x6B74;
/// Gp-relative word passed as the second argument of the callee.
const GP_CALLBACK_ARG_OFS: i32 = -0x6B6C;
/// Retail callee taking (out, arg).
const CHAIN_CALLEE: u32 = 0x24BBF8;

/// Sign-extend a 32-bit value into a 64-bit register like lw does.
fn sx(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// Store a result slot: `out` gets the slot address and the callee sees it too.
fn chain(c: &mut Cpu, out: u32, a1: u32, v0: u32, v1: u32) -> bool {
    c.s(REG_V0, sx(v0));
    c.s(REG_V1, sx(v1));
    c.s(REG_A0, sx(out));
    c.s(REG_T0, sx(out));
    c.s(REG_A1, sx(a1));
    c.s(REG_AT, sx(GLOBAL_HI));
    let r = h::call_keep(c, CHAIN_CALLEE, &[out, a1], &[]);
    c.ret(r.0);
    true
}

/// 0x24bb10 lookup_slot(out*, idx, a2, a3) -> result: stores the slot address for idx into *out (0 when
/// invalid), optionally runs a retail callback for the slot, returns -1 for a bad index or empty slot
pub fn lookup_slot(c: &mut Cpu) -> bool {
    let out = c.arg(0);
    let idx = c.arg(1) as i32;
    let a2 = c.arg(2);
    let a3 = c.arg(3);
    let gp = c.g(REG_GP) as u32;

    c.s(REG_T0, sx(out));
    c.w32(out, 0);

    let count = c.r32(SLOT_COUNT_WORD) as i32;
    if idx < 0 || count < idx {
        c.s(REG_V0, sx(u32::MAX));
        return true;
    }

    c.s(REG_AT, sx(GLOBAL_HI));
    c.w32(SLOT_ADDR_WORD, 0);
    let table_entry = SLOT_TABLE.wrapping_add((idx as u32) << 2);
    c.s(REG_V1, sx(table_entry));
    let entry = c.r32(table_entry);
    c.w32(gp.wrapping_add(GP_ENTRY_OFS as u32), entry);
    if entry == 0 {
        c.s(REG_V0, sx(u32::MAX));
        return true;
    }

    // Slot found: index is the halfword at entry, masked to 15 bits; address = base + (index << 8).
    let index = (c.r16(entry) & 0x7FFF) as u32;
    let base = c.r32(SLOT_BASE_WORD);
    let addr = base.wrapping_add(index << 8);
    c.w16(SLOT_INDEX_HALF, index as u16);
    c.w32(SLOT_ADDR_WORD, addr);
    c.w32(out, addr);

    // Flag bit 7 of the byte at +0x20 (sign bit of lb).
    let flag = ((c.r8(addr.wrapping_add(0x20)) as u32) >> 7) & 1;
    c.s(REG_V1, sx(flag));
    c.s(REG_V0, sx(addr));

    let callback_arg = c.r32(gp.wrapping_add(GP_CALLBACK_ARG_OFS as u32));
    if a2 != 0 {
        if a3 == 0 || flag != 0 {
            c.s(REG_V0, 0);
            return true;
        }
        // a3 != 0 and flag == 0: callback with the slot address as its argument.
        c.s(REG_V0, 0);
        return chain(c, out, addr, 0, flag);
    }

    // a2 == 0: the callback argument comes from the gp global.
    c.s(REG_A1, sx(callback_arg));
    if a3 != 0 {
        return chain(c, out, callback_arg, addr, flag);
    }
    if flag == 0 {
        c.s(REG_V0, 0);
        return true;
    }
    chain(c, out, callback_arg, 0, flag)
}
