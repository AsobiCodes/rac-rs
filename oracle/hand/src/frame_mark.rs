//! Per-frame end hook of the retail overlay: records the GS write pointer, runs the empty hook, and clears the
//! 8-byte head of every entry of a list record (Eudora addresses).
use crate::ps2::Ee;
use crate::{collision, h};
use ee_oracle::cpu::Cpu;

/// Current write pointer (a quadword pointer, advanced by the begin hook).
const WRITE_PTR: u32 = 0x0016_1140;
/// Copy of the write pointer taken by this hook.
const WRITE_PTR_END: u32 = 0x0016_04B0;
/// Argument block of the empty hook (a0 of its call here).
const HOOK_ARG_END: u32 = 0x0016_0490;
/// Retail empty hook (code at 0x1e9608, so the registers it leaves match retail's jal).
const EMPTY_HOOK: u32 = 0x001E_9608;
/// Stamp word copied to `STAMP_DST`.
const STAMP_SRC: u32 = 0x0015_EE78;
const STAMP_DST: u32 = 0x0015_EE74;
/// Entry list record pointer (count i16 at +0xc, base pointer at +0x10, 16-byte entries).
const ENTRY_REC: u32 = 0x0016_049C;
const REC_COUNT: u32 = 0xC;
const REC_BASE: u32 = 0x10;
const ENTRY_SIZE_SHIFT: u32 = 4;
/// GP-relative slots: the write pointer plus 0x10 is stored at -0x5ac0, and a zero word at -0x77e8.
const GP_END_PTR: i32 = -0x5AC0;
const GP_CLEAR: i32 = -0x77E8;

/// 0x27c2f8 clear_entry_list() -> void: records the write pointer, runs the empty hook, copies the stamp, then zeroes
/// the 8-byte head of every entry of the list record at 0x16049c (count at +0xc, base at +0x10, 16-byte stride).
pub fn clear_entry_list(c: &mut Cpu) -> bool {
    h::frame(c, 0x10, &[(31, 0, h::SQ)], |c, _sp| {
        let gp = c.g(28) as u32;
        let ptr = c.r32(WRITE_PTR);
        c.w32(WRITE_PTR_END, ptr);
        c.w32(gp.wrapping_add(GP_END_PTR as u32), ptr.wrapping_add(0x10));
        h::call_keep(c, EMPTY_HOOK, &[HOOK_ARG_END, 1], &[]);
        let stamp = c.r32(STAMP_SRC);
        c.w32(STAMP_DST, stamp);
        let rec = c.r32(ENTRY_REC);
        let count = c.r16(rec.wrapping_add(REC_COUNT)) as i16 as i32;
        c.w32(gp.wrapping_add(GP_CLEAR as u32), 0);
        c.set_arg(1, rec);
        if count > 0 {
            let mut last = 0u32;
            for i in 0..count as u32 {
                let base = c.r32(rec.wrapping_add(REC_BASE));
                last = base.wrapping_add(i << ENTRY_SIZE_SHIFT);
                c.w32(last, 0);
                c.w32(last.wrapping_add(4), 0);
            }
            c.set_arg(0, count as u32);
            c.ret(0);
            c.s(3, last as i32 as i64 as u64);
        } else {
            c.set_arg(0, 0);
            c.ret(stamp);
            c.s(3, count as i64 as u64);
        }
    });
    true
}
