//! Actor slot lookup and per-frame hero tick helpers (retail 0x226fd8..0x227028 and 0x227b90..0x227c78).
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;
const RA: usize = 31;

fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// Current slot index cell (used when the caller passes -1).
const CUR_SLOT_INDEX: u32 = 0x0014_0408;
/// Slot record table: 0x18-byte records, halfword flag at +0xe.
const SLOT_RECORDS: u32 = 0x001c_4130;
const SLOT_RECORD_SIZE: i64 = 0x18;
const SLOT_RECORD_FLAG_OFF: u32 = 0x0e;
/// Word list of slot pointers, indexed by slot.
const SLOT_LIST: u32 = 0x0014_0000 - 0x2bd8;

/// 0x226fd8 slot_record(index) -> v0: the slot pointer for `index` (the current slot when index is -1), or 1 when
/// the slot's record has no flag. Exit: a0 = the record flag, a1 = index, v1 = the record (or slot-list) address,
/// HI/LO = the 0x18 * index product.
pub fn slot_record(c: &mut Cpu) -> bool {
    let a0 = c.g(4) as u64;
    let a1 = if a0 == u64::MAX { c.r32(CUR_SLOT_INDEX) as i32 as i64 as u64 } else { a0 };
    c.s(A1, a1);
    let index = a1 as u32 as i32;
    let product = SLOT_RECORD_SIZE * index as i64;
    c.lo = c.lo & !(u64::MAX as u128) | (product as i32 as i64 as u64) as u128;
    c.hi = c.hi & !(u64::MAX as u128) | ((product >> 32) as i32 as i64 as u64) as u128;
    let record = (product as i32 as u32).wrapping_add(SLOT_RECORDS);
    set_gpr(c, V1, record);
    let flag = c.r16(record + SLOT_RECORD_FLAG_OFF);
    c.s(A0, flag as u64);
    if flag == 0 {
        c.ret(1);
    } else {
        let slot = ((index as u32) << 2).wrapping_add(SLOT_LIST);
        set_gpr(c, V1, slot);
        let value = c.r32(slot);
        c.ret(value);
    }
    true
}

/// Hero tick state cell and its dispatch values.
const TICK_STATE: u32 = 0x0016_0000 - 0xa5c;
const TICK_STATE_TOP: u32 = 2;
const TICK_STATE_LOOP: u32 = 6;
/// Tick table: count halfword at +0x44, object pointer list at +0x178.
const TICK_TABLE: u32 = 0x0017_0000 - 0x36a0;
const TICK_COUNT_OFF: u32 = 0x44;
const TICK_LIST_OFF: u32 = 0x178;
/// Object pointer block: three pointers whose flag halfwords are cleared at the end.
const OBJ_BLOCK: u32 = 0x0014_0000 - 0xcb0;
const OBJ_A_OFF: u32 = 0x11d0;
const OBJ_B_OFF: u32 = 0x11d4;
const OBJ_C_OFF: u32 = 0x1220;
/// Object flag halfword at +0x34; bit 0x800 marks a held object, bit 0 is cleared at the end.
const OBJ_FLAG_OFF: u32 = 0x34;
const OBJ_HOLD_BIT: u16 = 0x0800;
const OBJ_RUN_BIT: u32 = 0xfffe;
/// Entry tag halfword at +0xa6: tags 0x509 and 0x50a get the hold bit.
const ENTRY_TAG_OFF: u32 = 0xa6;
const ENTRY_TAG_LO: u16 = 0x0509;
const ENTRY_TAG_HI: u16 = 0x050a;

/// Clears bit 0 of an object's flag halfword and returns the new value.
fn clear_run_bit(c: &mut Cpu, obj: u32) -> u32 {
    let v = c.r16(obj + OBJ_FLAG_OFF) as u32 & OBJ_RUN_BIT;
    c.w16(obj + OBJ_FLAG_OFF, v as u16);
    v
}

/// 0x227b90 hero_frame_tick(): when the tick state is 2 or 6, marks the listed entries held, runs the hand pose
/// chain, the 0x202628 retail step and the glow pulse, then clears the run bit of three objects. Returns nothing.
pub fn hero_frame_tick(c: &mut Cpu) -> bool {
    h::frame(c, 0x10, &[(RA, 0x00, h::SQ)], |c, _sp| {
        let state = c.r32(TICK_STATE);
        set_gpr(c, V0, if state == TICK_STATE_TOP { TICK_STATE_TOP } else { TICK_STATE_LOOP });
        set_gpr(c, V1, state);
        if state != TICK_STATE_TOP && state != TICK_STATE_LOOP {
            return;
        }
        let table = TICK_TABLE;
        set_gpr(c, V1, table);
        let count = c.r16(table + TICK_COUNT_OFF) as i16 as i32;
        set_gpr(c, V0, count as u32);
        if count > 0 {
            set_gpr(c, A2, 0);
            set_gpr(c, A3, table + TICK_LIST_OFF);
            let mut i = 0i32;
            loop {
                let entry = c.r32(table + TICK_LIST_OFF + 4 * i as u32);
                set_gpr(c, A1, entry);
                if matches!(c.r16(entry + ENTRY_TAG_OFF), ENTRY_TAG_LO | ENTRY_TAG_HI) {
                    let flag = c.r16(entry + OBJ_FLAG_OFF);
                    c.w16(entry + OBJ_FLAG_OFF, flag | OBJ_HOLD_BIT);
                }
                i += 1;
                set_gpr(c, A2, i as u32);
                set_gpr(c, A3, table + TICK_LIST_OFF + 4 * i as u32);
                if i >= count {
                    break;
                }
            }
            set_gpr(c, V0, 0);
        }
        h::run(c, crate::hero_hand::hand_pose_chain, &[], &[]);
        h::call(c, 0x0020_2628, &[], &[]);
        h::run(c, crate::glow::glow_pulse_update, &[], &[]);
        set_gpr(c, A0, OBJ_BLOCK);
        // retail `lui v0,0x14` leaves v0 = 0x140000 when no object pointer is set
        let mut v0 = 0x0014_0000;
        let a = c.r32(OBJ_BLOCK + OBJ_A_OFF);
        set_gpr(c, V1, a);
        if a != 0 {
            v0 = clear_run_bit(c, a);
        }
        let b = c.r32(OBJ_BLOCK + OBJ_B_OFF);
        set_gpr(c, V1, b);
        if b != 0 {
            v0 = clear_run_bit(c, b);
        }
        let obj_c = c.r32(OBJ_BLOCK + OBJ_C_OFF);
        set_gpr(c, V1, obj_c);
        if obj_c != 0 {
            v0 = clear_run_bit(c, obj_c);
        }
        set_gpr(c, V0, v0);
    });
    true
}
