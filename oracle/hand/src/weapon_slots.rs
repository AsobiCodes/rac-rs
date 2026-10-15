//! Weapon slot sweep helpers (retail 0x2dc318..0x2df970): the per-frame walk over the weapon slot table, the component
//! lookup for a slot record, and the ready-flag setter.

use crate::camswitch;
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const GP: usize = 28;
const S0: usize = 16;
const S1: usize = 17;
const S2: usize = 18;
const S3: usize = 19;
const S4: usize = 20;
const RA: usize = 31;
const V0: usize = 2;
const V1: usize = 3;

/// Base of the weapon data block (retail's s3 - 0xcb0 with s3 = 0x140000).
const DATA_BASE: u32 = 0x0014_0000 - 0x0cb0;
/// Table of 32-bit slot record pointers, indexed by the slot number.
const SLOT_TABLE: u32 = DATA_BASE + 0x2050;
/// Hit counters for record kind 4 and kind 3.
const HITS_KIND4: u32 = DATA_BASE + 0x2078;
const HITS_KIND3: u32 = DATA_BASE + 0x207c;
/// Byte flag that selects the long slot walk (10 slots instead of 5).
const LONG_WALK_FLAG: u32 = 0x0014_0000 - 0x1ae0 + 9;
/// First argument of the camera-arg spill for a slot (0x1dc4b8).
const SPILL_BLOCK: u32 = 0x001e_0000 - 0x3b48;
/// Halfword cleared by `set_ready_flags`.
const READY_HALF: u32 = 0x001e_0000 - 0x79e8;
/// Record types that `comp_lookup` passes through without a call.
const TYPE_SKIP_FE: u8 = 0xfe;
const TYPE_SKIP_FD: u8 = 0xfd;
/// Slot walk lengths.
const SLOTS_SHORT: u32 = 5;
const SLOTS_LONG: u32 = 10;

/// 0x2dd548 comp_lookup(rec) -> v0: the component pointer of a slot record. Null, type 0xfe and 0xfd records and records
/// whose object or vtable slot +0x10 is empty give 0; otherwise the vtable function runs with the record and its result
/// is v0.
pub fn comp_lookup(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    h::frame(c, 0x10, &[(RA, 0x00, h::SQ)], |c, _sp| {
        let rec = c.arg(0);
        if rec == 0 {
            c.ret(0);
            return true;
        }
        let kind = c.r8(rec + 0x20);
        c.s(V1, kind as u64);
        if kind == TYPE_SKIP_FE || kind == TYPE_SKIP_FD {
            c.ret(0);
            return true;
        }
        if c.r32(rec + 0x78) == 0 {
            c.ret(0);
            return true;
        }
        let obj = c.r32(rec + 0x24);
        if obj == 0 {
            c.ret(0);
            return true;
        }
        let class = c.r32(obj + 0x2c);
        let func = c.r32(class + 0x10);
        if func == 0 {
            c.ret(0);
            return true;
        }
        // jalr func with the record still in a0; the callee's v0 is the result and its scratch registers stay.
        h::call_keep(c, func, &[rec], &[]);
        true
    })
}

/// 0x2dc318 weapon_slot_sweep() -> (): clears the two hit counters, then walks the slot table (5 slots, or 10 while
/// the flag byte is set). A slot's record of kind 3 or 4 whose field +0x6a is not -1 counts a hit; any other non-empty
/// slot spills its camera args and is cleared.
pub fn weapon_slot_sweep(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let saves = [
        (S0, 0x00, h::SQ),
        (S1, 0x10, h::SQ),
        (S2, 0x20, h::SQ),
        (S3, 0x30, h::SQ),
        (S4, 0x40, h::SQ),
        (RA, 0x50, h::SQ),
    ];
    h::frame(c, 0x60, &saves, |c, _sp| {
        c.w32(HITS_KIND4, 0);
        c.w32(HITS_KIND3, 0);
        let mut i: u32 = 0;
        loop {
            let limit = if c.r8(LONG_WALK_FLAG) != 0 { SLOTS_LONG } else { SLOTS_SHORT };
            if i >= limit {
                break;
            }
            let slot = SLOT_TABLE + 4 * i;
            let record = c.r32(slot);
            if record == 0 {
                i += 1;
                continue;
            }
            c.set_arg(0, record);
            comp_lookup(c);
            let rec = c.result();
            if rec != 0 {
                let kind = c.r16(rec + 0x68) as i16;
                if (3..5).contains(&kind) {
                    let field = c.r16(rec + 0x6a) as i16;
                    if field != -1 {
                        let hits = if kind == 3 { HITS_KIND3 } else { HITS_KIND4 };
                        let n = c.r32(hits).wrapping_add(1);
                        c.w32(hits, n);
                        i += 1;
                        continue;
                    }
                }
            }
            // Spill the slot's record args (a1 = +0xb2, a2 = +0xa6) and clear the slot.
            let spill_rec = c.r32(slot);
            let a2 = c.r16(spill_rec + 0xa6) as i16 as i32 as u32;
            let a1 = c.r16(spill_rec + 0xb2) as i16 as i32 as u32;
            c.set_arg(0, SPILL_BLOCK);
            c.set_arg(1, a1);
            c.set_arg(2, a2);
            camswitch::spill_cam_args(c);
            c.w32(slot, 0);
            i += 1;
        }
        c.ret(0);
        let flag = c.r8(LONG_WALK_FLAG);
        c.s(V1, flag as u64);
        true
    })
}

/// 0x2df958 set_ready_flags() -> v0: sets the two gp-relative words at gp-0x4fec and gp-0x4ff0 to 1 and clears the
/// halfword at 0x1d8618; v0 = 0x1e0000.
pub fn set_ready_flags(c: &mut Cpu) -> bool {
    let gp = c.g(GP) as u32;
    c.s(V1, 1);
    c.w32(gp.wrapping_sub(0x4fec), 1);
    c.w16(READY_HALF, 0);
    c.w32(gp.wrapping_sub(0x4ff0), 1);
    c.s(V0, 0x001e_0000);
    true
}
