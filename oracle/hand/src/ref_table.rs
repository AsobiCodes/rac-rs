//! GS/VIF reference record writer and the 16-byte slot table lookup at 0x179510.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Pointer word (in the 0x16 page) to the 11-qword GS state that receives the VIF refs.
const GS_STATE_PTR: u32 = 0x0016_1140;
/// VIF ref word stored at state+0.
const VIF_REF_A: u32 = 0x3000_0029;
/// Address word (0x151b60) stored at state+4.
const VIF_REF_B: u32 = 0x0015_1B60;
/// VIF ref word stored at state+0xc (also left in a0).
const VIF_REF_C: u32 = 0x5000_0029;
/// gp-relative offset of the word that receives the cursor (state+0x10).
const GP_CURSOR_OFFSET: u32 = 0x5AC0;
/// Pointer word (0x15f640) to the slot list the lookup scans.
const SLOT_LIST_PTR: u32 = 0x0015_F640;
/// Slot table base (0x179510): entry count at +0x2c, 16-byte entries.
const SLOT_TABLE: u32 = 0x0017_9510;
/// Offset of the entry count inside the slot table.
const SLOT_COUNT_OFFSET: u32 = 0x2C;
/// Offset of the key word inside each slot entry.
const SLOT_KEY_OFFSET: u32 = 4;
/// Value retail leaves in t0 (0x180000).
const HI_BASE: u32 = 0x0018_0000;
/// Value returned by find_slot_value when no slot matches (0x1797a8).
const DEFAULT_ENTRY: u32 = 0x0017_97A8;
/// gp-relative offset of the base pointer used by find_slot_value.
const GP_BASE_OFFSET: u32 = 0x75C0;
/// Size in bytes of one slot entry.
const ENTRY_SHIFT: u32 = 4;

/// Sets a general register to a 32-bit value, sign-extended the way the MIPS 32-bit ops leave it.
fn set_s32(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// 0x1f4460 append_vif_ref() -> v0: writes the two VIF refs and the terminator into the GS state at
/// `*0x161140`, stores the cursor (state+0x10) at gp-0x5ac0 and returns it in v0.
pub fn append_vif_ref(c: &mut Cpu) -> bool {
    let state = c.r32(GS_STATE_PTR);
    c.w32(state, VIF_REF_A);
    c.w32(state.wrapping_add(4), VIF_REF_B);
    c.w32(state.wrapping_add(8), 0);
    c.w32(state.wrapping_add(0xC), VIF_REF_C);
    let cursor = state.wrapping_add(0x10);
    let gp = c.g(28) as u32;
    c.w32(gp.wrapping_sub(GP_CURSOR_OFFSET), cursor);
    set_s32(c, 2, cursor);
    set_s32(c, 3, state);
    set_s32(c, 4, VIF_REF_C);
    set_s32(c, 5, state);
    true
}

/// 0x1f6b10 find_slot_index(key a0) -> v0: index of the first slot whose key word equals `key`
/// (index 0 is the list head, then the table scan from 1), or -1 when none matches.
pub fn find_slot_index(c: &mut Cpu) -> bool {
    let key = c.arg(0) as u32;
    set_s32(c, 8, HI_BASE);
    set_s32(c, 6, u32::MAX);
    let count = c.r32(SLOT_TABLE + SLOT_COUNT_OFFSET) as i32;
    if count <= 0 {
        set_s32(c, 3, count as u32);
        set_s32(c, 5, 0);
        set_s32(c, 2, u32::MAX);
        return true;
    }
    set_s32(c, 5, 0);
    let list = c.r32(SLOT_LIST_PTR);
    set_s32(c, 7, list);
    if c.r32(list.wrapping_add(SLOT_KEY_OFFSET)) == key {
        set_s32(c, 3, list);
        set_s32(c, 6, 0);
        set_s32(c, 2, 0);
        return true;
    }
    set_s32(c, 3, SLOT_TABLE);
    let mut index: u32 = 1;
    let mut found = u32::MAX;
    while (index as i32) < count {
        let entry = list.wrapping_add(index << ENTRY_SHIFT);
        if c.r32(entry.wrapping_add(SLOT_KEY_OFFSET)) == key {
            found = index;
            break;
        }
        index += 1;
    }
    set_s32(c, 5, index);
    set_s32(c, 6, found);
    set_s32(c, 2, found);
    true
}

/// 0x1f6b80 find_slot_value(key a0) -> v0: the first word of the slot entry found by `find_slot_index`
/// (entry base taken from gp-0x75c0), or the address 0x1797a8 when the key is not found.
pub fn find_slot_value(c: &mut Cpu) -> bool {
    let frame = (c.g(29) as u32).wrapping_sub(0x10);
    let ra = c.r[31];
    c.mem.wr128(frame & !15, ra);
    let index = h::run(c, find_slot_index, &[], &[]).0;
    c.r[31] = ra;
    if (index as i32) >= 0 {
        let gp = c.g(28) as u32;
        let base = c.r32(gp.wrapping_sub(GP_BASE_OFFSET));
        let entry = (index << ENTRY_SHIFT).wrapping_add(base);
        let value = c.r32(entry);
        set_s32(c, 3, entry);
        set_s32(c, 2, value);
    } else {
        set_s32(c, 3, index);
        set_s32(c, 2, DEFAULT_ENTRY);
    }
    true
}
