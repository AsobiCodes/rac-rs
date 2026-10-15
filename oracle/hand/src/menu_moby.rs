//! Menu/pause moby pass: scratchpad save/restore around the pass, and the moby draw range.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Scratchpad moby copy area (0x70003a00) and its RAM save slot (0x197080).
const SCRATCH: u32 = 0x7000_3a00;
const SAVE: u32 = 0x0019_7080;
const COPY_LEN: u32 = 0x380;
/// Menu slot counter at 0x160000 - 0xac, and its gp-relative pointer slot.
const COUNTER: u32 = 0x0015_ff54;
const SLOT_PTR_GP: u32 = (-0x6cac_i32) as u32;
const MOBY_SLOT_DRAW: u32 = 0x0024_8288;
const FLAG_GP: u32 = (-0x7e7c_i32) as u32;
const SLOT_TABLE: u32 = 0x0015_0000 - 0x3fb0;
const SLOT_NONE: u32 = 0xff;
const DECLINE_DRAW_RANGE: bool = true;
/// Return addresses of the retail jal calls to the two copies in moby_draw_range.
const RA_SCRATCH_SAVE: u128 = 0x0024_2804;
const RA_SCRATCH_RESTORE: u128 = 0x0024_2824;
/// retail copy_qwords(dst, src, len): called as a retail jal, so its leftover scratch registers match retail.
const COPY_QWORDS: u32 = 0x001f_16c0;

/// Copies the 0x380-byte area `from` to `to` through retail copy_qwords. Retail's wrapper first stores its return
/// address (`ra`) at its own frame (sp - 0x10), and that word stays in the stack, so it is stored here too.
fn copy_area(c: &mut Cpu, ra: u128, from: u32, to: u32) {
    let frame = (c.g(29) as u32).wrapping_sub(0x10);
    for i in 0..4 {
        c.w32(frame.wrapping_add(4 * i), (ra >> (32 * i)) as u32);
    }
    h::call_keep(c, COPY_QWORDS, &[from, to, COPY_LEN], &[]);
}

/// 0x2426b8 moby_scratch_restore(): copies the 0x380-byte save area at 0x197080 back to scratchpad 0x70003a00.
pub fn moby_scratch_restore(c: &mut Cpu) -> bool {
    let ra = c.r[31];
    copy_area(c, ra, SAVE, SCRATCH);
    true
}

/// 0x2426e8 moby_scratch_save(): copies the 0x380-byte scratchpad area at 0x70003a00 to the save area at 0x197080.
pub fn moby_scratch_save(c: &mut Cpu) -> bool {
    let ra = c.r[31];
    copy_area(c, ra, SCRATCH, SAVE);
    true
}

/// 0x2427d0 moby_draw_range(a0 = first, a1 = count): menu/pause moby pass; draws the slot range and retires one
/// menu slot from the counter (returns the new counter).
pub fn moby_draw_range(c: &mut Cpu) -> bool {
    // Declined: the retail callee 0x248288 stores scratchpad 0x3ff8 differently from this Rust (cause not found), so
    // retail runs. Nothing is written before this return.
    if DECLINE_DRAW_RANGE {
        return false;
    }
    let first = c.arg(0) as u32;
    let count = c.arg(1) as u32;
    h::frame(c, 0x30, &[(17, 0x10, h::SQ), (16, 0x00, h::SQ), (31, 0x20, h::SQ)], |c, _sp| {
        c.s(17, first as i32 as i64 as u64);
        c.s(16, count as i32 as i64 as u64);
        // ra is set to the retail return address before each jal, so a callee's stored ra matches retail's.
        c.s(31, 0x0024_27f4);
        h::call_keep(c, 0x2913c8, &[0x47, 0x5_360b], &[]);
        c.s(31, 0x0024_27fc);
        h::call_keep(c, 0x118a80, &[0], &[]);
        copy_area(c, RA_SCRATCH_SAVE, SCRATCH, SAVE);
        let slot = c.r32(COUNTER);
        c.s(31, 0x0024_281c);
        h::call_keep(c, MOBY_SLOT_DRAW, &[first, slot, count, 0], &[]);
        let v0 = c.g(2) as u32;
        let gp = c.g(28) as u32;
        c.w32(gp.wrapping_add(SLOT_PTR_GP), v0);
        copy_area(c, RA_SCRATCH_RESTORE, SAVE, SCRATCH);
        let next = c.r32(COUNTER).wrapping_sub(0x10);
        c.w32(COUNTER, next);
        c.s(1, 0x0016_0000);
        c.ret(next);
    });
    true
}

/// 0x242b68 set_slot_byte(slot, val): when slot != 0xff, stores val (0xff) as a byte in the slot table at
/// 0x14c050 + slot + (gp-relative index << 4).
pub fn set_slot_byte(c: &mut Cpu) -> bool {
    let slot = c.arg(0) as u32;
    let gp = c.g(28) as u32;
    let index = c.r32(gp.wrapping_add(FLAG_GP));
    let addr = slot.wrapping_add(index << 4).wrapping_add(SLOT_TABLE);
    c.s(5, SLOT_NONE as u64);
    if slot == SLOT_NONE {
        c.s(2, index as i32 as i64 as u64);
    } else {
        c.w8(addr, SLOT_NONE as u8);
        c.s(2, addr as i32 as i64 as u64);
        c.s(3, SLOT_TABLE as i32 as i64 as u64);
    }
    true
}
