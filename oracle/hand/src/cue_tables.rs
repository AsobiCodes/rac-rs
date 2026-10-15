//! Retail routines from the L4_09 record: a slot store with two helper calls, and two save-data counters that clamp
//! a nonzero-byte count.
use crate::h;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Global the slot store writes (`lui 0x16` + `addiu -0x9e0`).
const SLOT_VALUE: u32 = 0x0015_F620;
/// Scale argument the slot store passes to the float helper when its slot argument is -1.
const SCALE_ARG: u32 = 180;
/// Retail helper that takes the object (`lui 0x1f6b80`).
const SLOT_OBJECT_FN: u32 = 0x001F_6B80;
/// Retail helper that takes the frame base, the object result and the slot argument (`jal 0x116248`).
const SLOT_STORE_FN: u32 = 0x0011_6248;
/// Base of the frame-base global (`lui 0x18` + `addiu -0x6de8`).
const FRAME_BASE: u32 = 0x0017_9218;

/// Gold-bolt table: 37 bytes at `lui 0x14` + `addiu -0x1ae0`.
const GOLD_TABLE: u32 = 0x0013_E520;
const GOLD_TABLE_LEN: u32 = 37;
const GOLD_MAX: u32 = 10;
/// Second save-data table: 32 bytes at `lui 0x14` + `addiu -0x2bf8`.
const SHIFT_TABLE: u32 = 0x0013_D408;
const SHIFT_TABLE_LEN: u32 = 32;
const SHIFT_MAX: u32 = 30;

/// Sets GPR `reg` to a sign-extended 32-bit value, as the 32-bit MIPS ops do.
fn put(c: &mut Cpu, reg: usize, v: u32) {
    c.r[reg] = v as i32 as i64 as u64 as u128;
}

/// SQ: stores a 128-bit register at `addr` (lane 0 at the lowest address).
fn store_q(c: &mut Cpu, addr: u32, v: u128) {
    for i in 0..4u32 {
        c.w32(addr.wrapping_add(4 * i), (v >> (32 * i)) as u32);
    }
}

/// 0x2560d0 store_slot(obj, x, slot) -> none: when slot is -1 it takes the float helper's result (scale 180)
/// instead, runs two retail helpers on obj and x, and stores the slot value to SLOT_VALUE.
pub fn store_slot(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let obj = c.arg(0);
    let x = c.arg(1);
    let mut slot = c.arg(2);
    let sp = (c.r[29] as u32).wrapping_sub(0x50);
    store_q(c, sp + 0x40, c.r[31]);
    store_q(c, sp + 0x30, c.r[19]);
    store_q(c, sp + 0x20, c.r[18]);
    store_q(c, sp + 0x10, c.r[17]);
    store_q(c, sp, c.r[16]);
    if slot == 0xFFFF_FFFF {
        slot = h::run_keep(c, mathx::float_to_int_scaled, &[SCALE_ARG], &[]).0;
    }
    let (obj_value, _) = h::call_keep(c, SLOT_OBJECT_FN, &[obj], &[]);
    h::call_keep(c, SLOT_STORE_FN, &[FRAME_BASE, obj_value, x], &[]);
    c.w32(SLOT_VALUE, slot);
    true
}

/// Counts the nonzero bytes of a table and leaves the scan registers as the retail loop does.
fn nonzero_count(c: &mut Cpu, table: u32, len: u32) -> u32 {
    let mut count = 0u32;
    for i in 0..len {
        if c.r8(table + i) != 0 {
            count += 1;
        }
    }
    put(c, 3, 1);
    put(c, 5, len);
    put(c, 6, count);
    put(c, 7, table);
    put(c, 4, (count <= max_of(len)) as u32);
    count
}

/// Upper clamp of the table scan: the limit the retail `slti` compares against is one above it.
fn max_of(len: u32) -> u32 {
    if len == GOLD_TABLE_LEN {
        GOLD_MAX
    } else {
        SHIFT_MAX
    }
}

/// Count of nonzero bytes in a table, clamped to `max` (the retail result in v0).
fn clamped_count(c: &mut Cpu, table: u32, len: u32) -> u32 {
    let count = nonzero_count(c, table, len);
    let max = max_of(len);
    if count <= max {
        count
    } else {
        max
    }
}

/// 0x2569a8 gold_bolts_spent() -> u32: counts the nonzero bytes of the 37-entry gold table, clamped to 0..10.
pub fn gold_bolts_spent(c: &mut Cpu) -> bool {
    let v = clamped_count(c, GOLD_TABLE, GOLD_TABLE_LEN);
    c.ret(v);
    true
}

/// 0x2569f0 shift_count() -> u32: counts the nonzero bytes of the 32-entry table, clamped to 0..30.
pub fn shift_count(c: &mut Cpu) -> bool {
    let v = clamped_count(c, SHIFT_TABLE, SHIFT_TABLE_LEN);
    c.ret(v);
    true
}
