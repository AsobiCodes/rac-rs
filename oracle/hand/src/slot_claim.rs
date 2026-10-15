//! Slot control block (0x1516d0) from the L4_09 record: the state poll at 0x257418 and the slot claim at 0x257548.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Control block base (`lui 0x15` + `addiu 0x16d0`).
const BLOCK: u32 = 0x0015_16D0;
/// Block word: busy flag. The claim sets it to -1 when it launches a slot.
const F_BUSY: u32 = 0x34;
/// Block pointer the poll tests (non-null means active).
const F_PTR: u32 = 0x50;
/// Block halfword: state. The poll moves it from 3 to 4 after its callee runs.
const F_STATE: u32 = 0x5A;
/// Callee the poll runs when the state is 3.
const POLL_CALLEE: u32 = 0x0012_ECA0;
/// Callee the claim launches the slot entry through.
const LAUNCH_CALLEE: u32 = 0x0012_EC08;
/// Slot table base (`lui 0x13` + `addiu 0x7b80` + `addiu 0x2aa8`): one word per slot number.
const TABLE: u32 = 0x0013_A628;
/// Ninth launch argument, stored in the caller's outgoing area.
const LAUNCH_ARG9: u32 = 0x20;
/// Tenth launch argument (`lui 0x26` + `addiu -0x7b40`), stored in the outgoing area.
const LAUNCH_ARG10: u32 = 0x0025_84C0;
/// Frame size of the poll (saves s0 and ra).
const FRAME_POLL: u32 = 0x20;
/// Frame size of the claim (saves ra, holds the launch arguments).
const FRAME_CLAIM: u32 = 0x30;

/// Sets a general register to a 32-bit value, sign-extended as the MIPS 32-bit ops leave it.
fn gpr(c: &mut Cpu, n: usize, v: u32) {
    c.r[n] = v as i32 as i64 as u64 as u128;
}

/// 0x257418 poll_slot() -> int: when the block pointer (+0x50) is set and the state (+0x5a) is 3, runs 0x12eca0 and
/// sets the state to 4. Returns 1 when the callee ran, else 0 (in v0).
pub fn poll_slot(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let caller = c.r[29];
    let sp = (caller as u32).wrapping_sub(FRAME_POLL);
    c.mem.wr128(sp, c.r[16]);
    c.mem.wr128(sp + 0x10, c.r[31]);
    let ptr = c.r32(BLOCK + F_PTR);
    if ptr == 0 {
        gpr(c, 2, 0);
        gpr(c, 4, ptr);
        return true;
    }
    let state = c.r16(BLOCK + F_STATE) as i16;
    gpr(c, 3, state as i32 as u32);
    if state != 3 {
        gpr(c, 2, 0);
        gpr(c, 4, ptr);
        return true;
    }
    gpr(c, 2, 0);
    gpr(c, 4, ptr);
    c.r[29] = sp as u128;
    h::call_keep(c, POLL_CALLEE, &[ptr], &[]);
    c.w16(BLOCK + F_STATE, 4);
    gpr(c, 3, 4);
    gpr(c, 2, 1);
    c.r[29] = caller;
    true
}

/// 0x257548 claim_slot(slot, a1, a2) -> void: when the busy word (+0x34) is clear and the slot's table entry is set,
/// fills the block fields for the slot and launches the entry through 0x12ec08.
pub fn claim_slot(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let caller = c.r[29];
    let sp = (caller as u32).wrapping_sub(FRAME_CLAIM);
    c.mem.wr128(sp + 0x20, c.r[31]);
    let slot = c.arg(0) as u32;
    let a1 = c.arg(1) as u32;
    let a2 = c.arg(2) as u32;
    let busy = c.r32(BLOCK + F_BUSY);
    gpr(c, 12, BLOCK);
    c.r[11] = c.r[4];
    c.r[14] = c.r[5];
    c.r[13] = c.r[6];
    if busy != 0 {
        gpr(c, 2, busy);
        c.r[29] = caller;
        return true;
    }
    let addr = TABLE.wrapping_add(slot << 2);
    let entry = c.r32(addr);
    gpr(c, 2, TABLE);
    gpr(c, 4, addr);
    gpr(c, 3, BLOCK + F_BUSY);
    if entry == 0 {
        c.r[29] = caller;
        return true;
    }
    c.w16(BLOCK + 0x3E, 1);
    c.w16(BLOCK + 0x38, slot as u16);
    c.w16(BLOCK + 0x3C, a1 as u16);
    c.w16(BLOCK + 0x3A, a2 as u16);
    c.w16(BLOCK + 0x44, 0);
    c.w32(BLOCK + 0x48, 10);
    c.w32(BLOCK + 0x4C, 0xBB80);
    c.w32(BLOCK + F_BUSY, u32::MAX);
    c.w32(sp, LAUNCH_ARG9);
    c.w32(sp + 8, LAUNCH_ARG10);
    c.w32(sp + 0x10, BLOCK + F_BUSY);
    c.w32(sp + 0x14, 0);
    c.r[2] = u64::MAX as u128;
    c.r[29] = sp as u128;
    let t0 = a2 as u16 as i16 as i32 as u32;
    let ints = [entry, 0, 0, 0, t0, 0, 1, 0];
    h::call_keep(c, LAUNCH_CALLEE, &ints, &[]);
    c.r[29] = caller;
    true
}
