//! Init and bookkeeping helpers from the retail 0x268d68..0x26d388 block (MIPS, PS2 EE).
//! Each function is bit-exact with the retail code it replaces.

use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;
use crate::record_list_walk;

/// Scale-constant block (retail `lui 0x16` + `addiu 0x6d80`).
const SCALE_BLOCK: u32 = 0x16_6d80;
/// Quadword pads cleared next to the scale block.
const PAD_A: u32 = 0x16_70d0;
const PAD_B: u32 = 0x16_70e0;
const PAD_C: u32 = 0x16_70f0;
/// Pending flag word read by `tick_pending` (`lh` at this address).
const PENDING_FLAG: u32 = 0x15_16d8;
/// Table base loaded into $a3 by `tick_pending`.
const TICK_TABLE: u32 = 0x15_0000;
/// Slot table used by `tick_pending` (the index word sits at +0x2a0).
const SLOT_TABLE: u32 = 0x18_42f0;
/// Base of the state block whose byte at +0xcb gates the 0x258080 callback.
const PENDING_STATE: u32 = 0x1b_d9f0;
/// Upper half of the record data pool.
const POOL_HI: u32 = 0x7000_0000;
/// Destination block written by `build_record_tables`.
const DEST_HI: u32 = 0x1b_0000;
/// Upper half of the record-index pointer (`lw` at +0x4514).
const RECORD_INDEX_HI: u32 = 0x18_0000;

const FLOAT_256: u32 = 0x4380_0000;
const FLOAT_64: u32 = 0x4280_0000;
const FLOAT_ONE: u32 = 0x3f80_0000;

/// Sets a MIPS general register to a sign-extended 32-bit value, as `lui`/`addiu`/`lw` do.
fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// Stores 16 zero bytes, as retail's `sq $zero` does.
fn zero_quad(c: &mut Cpu, addr: u32) {
    for i in 0..4 {
        c.w32(addr.wrapping_add(i * 4), 0);
    }
}

/// 0x268d68 init_scale_block(void) -> void: stores 256.0, 64.0 and 1.0 into the scale block at 0x166d80 and clears
/// three quadwords around it.
pub fn init_scale_block(c: &mut Cpu) -> bool {
    c.f[1] = FLOAT_256;
    c.f[0] = FLOAT_64;
    c.w32(SCALE_BLOCK + 0x144, FLOAT_256);
    c.w32(SCALE_BLOCK + 0x148, FLOAT_64);
    c.w32(SCALE_BLOCK + 0x140, FLOAT_256);
    zero_quad(c, PAD_A);
    zero_quad(c, PAD_B);
    zero_quad(c, PAD_C);
    c.f[0] = FLOAT_ONE;
    c.w32(SCALE_BLOCK + 0x37c, FLOAT_ONE);
    c.w32(SCALE_BLOCK + 0x350, FLOAT_ONE);
    c.w32(SCALE_BLOCK + 0x364, FLOAT_ONE);
    c.w32(SCALE_BLOCK + 0x378, FLOAT_ONE);
    set_gpr(c, 1, FLOAT_ONE);
    set_gpr(c, 2, SCALE_BLOCK);
    set_gpr(c, 3, PAD_C);
    true
}

/// Releases the slot named by the index word (retail 0x26c63c..0x26c680, flag clear path).
fn release_slot(c: &mut Cpu) {
    set_gpr(c, 5, SLOT_TABLE);
    set_gpr(c, 6, u32::MAX);
    let idx = c.r32(SLOT_TABLE + 0x2a0);
    if idx != u32::MAX {
        let addr = SLOT_TABLE.wrapping_add(0x28c).wrapping_add(idx.wrapping_mul(4));
        set_gpr(c, 4, 0x1c_0000);
        c.w8(0x1c_0000 - 0x6145, 0);
        let v = c.r32(addr) ^ 0x1000;
        c.w32(addr, v);
        c.w32(SLOT_TABLE + 0x2a0, u32::MAX);
    }
}

/// 0x26c628 tick_pending(void) -> 0: when the pending flag is clear, releases the slot it names; when it is set and
/// the state byte is set, runs the callback at 0x258080, clears the byte and frees the slot.
pub fn tick_pending(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    h::frame(c, 0x20, &[(16, 0, h::SQ), (31, 0x10, h::SQ)], |c, _sp| {
        set_gpr(c, 7, TICK_TABLE);
        if c.r16(PENDING_FLAG) as i16 == 0 {
            release_slot(c);
            set_gpr(c, 3, 0);
            set_gpr(c, 2, 0x1c_0000);
            return;
        }
        set_gpr(c, 2, 0x1c_0000);
        set_gpr(c, 16, PENDING_STATE);
        let state = c.r8(PENDING_STATE + 0xcb);
        set_gpr(c, 3, state as u32);
        if state != 0 {
            h::call_keep(c, 0x25_8080, &[], &[]);
            c.w8(PENDING_STATE + 0xcb, 0);
            set_gpr(c, 2, SLOT_TABLE);
            set_gpr(c, 4, u32::MAX);
            let addr = SLOT_TABLE.wrapping_add(c.r32(SLOT_TABLE + 0x2a0).wrapping_mul(4));
            set_gpr(c, 3, addr);
            c.w32(addr + 0x28c, u32::MAX);
            c.w32(SLOT_TABLE + 0x2a0, u32::MAX);
        }
    });
    c.ret(0);
    true
}

/// 0x26cca8 build_record_tables(rec) -> 0: fills the record at a0 from the list walk and copies two pool words
/// into 0x1b3a74 and 0x1b3958 when the index entry is valid.
pub fn build_record_tables(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    h::frame(c, 0x20, &[(16, 0, h::SQ), (31, 0x10, h::SQ)], |c, _sp| {
        let rec = c.arg(0);
        set_gpr(c, 16, rec);
        h::call_keep(c, 0x24_0188, &[], &[]);
        h::run_keep(c, record_list_walk::walk_records, &[POOL_HI, 0, POOL_HI | 0x100, 1], &[]);
        let walk_v0 = c.g(2) as u32;
        c.w32(rec + 0x7c, walk_v0);
        set_gpr(c, 3, RECORD_INDEX_HI);
        let slot = rec.wrapping_add(0x30).wrapping_add(c.r32(RECORD_INDEX_HI + 0x4514).wrapping_mul(4));
        set_gpr(c, 16, slot);
        set_gpr(c, 4, u32::MAX);
        let entry = c.r32(slot);
        if entry != u32::MAX {
            let v1 = c.r32(entry.wrapping_mul(4).wrapping_add(POOL_HI));
            set_gpr(c, 3, v1);
            set_gpr(c, 5, DEST_HI);
            set_gpr(c, 4, POOL_HI | 0x100);
            set_gpr(c, 6, DEST_HI);
            c.w32(DEST_HI + 0x3a74, v1);
            let entry2 = c.r32(slot);
            let v1b = c.r32(entry2.wrapping_mul(4).wrapping_add(POOL_HI | 0x100));
            c.w32(DEST_HI + 0x3958, v1b);
            set_gpr(c, 3, v1b);
        }
    });
    c.ret(0);
    true
}

/// 0x26d368 run_init_step(void) -> 0: calls the setup routine at 0x276550 with a0 = 1.
pub fn run_init_step(c: &mut Cpu) -> bool {
    h::frame(c, 0x10, &[(31, 0, h::SQ)], |c, _sp| {
        h::call_keep(c, 0x27_6550, &[1], &[]);
    });
    c.ret(0);
    true
}
