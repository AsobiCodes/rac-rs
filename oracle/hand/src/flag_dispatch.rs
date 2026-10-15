//! Flag-gated dispatch at 0x2721f8 and the link release at 0x272318 (rec/l4_11 region).

use crate::claim_table;
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Flag word in RAM (lui 0x14, lw -0x34fc).
const FLAG_WORD: u32 = 0x0013_CB04;
/// State block (lui 0x1c, addiu -0x6210); its +4 word points at a record.
const STATE: u32 = 0x001B_9DF0;
/// Saved word written by the 0x800 path and read by the 0x20 path (lui 0x18, +0x4514).
const SAVED_WORD: u32 = 0x0018_4514;
/// Retail helper called with (0, 0x11, field of the argument).
const NOTIFY_FN: u32 = 0x0027_E470;
/// gp-relative word read by the 0x800 path (gp - 0x7e7c).
const GP_WORD_OFF: u32 = 0x7E7C;

/// Sign-extends a 32-bit word the way lw and addiu leave it in a 64-bit GPR.
fn sx(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// 0x2721f8 dispatch_flags(obj) -> v0: 1 on the 0x800 and 0x40 paths, 0 or -1 on the 0x10 path, else 0; the 0x20 and
/// 0x40 paths also call the retail helper 0x27e470 with (0, 0x11, obj->0x14).
pub fn dispatch_flags(c: &mut Cpu) -> bool {
    h::frame(c, 0x10, &[(31, 0, h::SQ)], |c, _| dispatch_body(c));
    true
}

fn dispatch_body(c: &mut Cpu) {
    let obj = c.arg(0);
    let flags = c.r32(FLAG_WORD);
    let a0 = c.g(4);
    c.s(7, a0);
    c.s(3, sx(flags));
    if flags & 0x10 != 0 {
        let rec = c.r32(STATE + 4);
        let v1 = c.r32(rec.wrapping_add(0x38));
        c.s(3, sx(v1));
        if v1 == 0 {
            // beql taken: its delay slot loads the link word, and the branch tests that word.
            let link = c.r32(STATE + 0x124);
            c.s(2, sx(link));
            c.ret(if link != 0 { 0 } else { u32::MAX });
        } else {
            c.w32(STATE + 8, v1);
            c.ret(0);
        }
        return;
    }
    if flags & 0x800 != 0 {
        let gp = c.g(28) as u32;
        let saved = c.r32(gp.wrapping_sub(GP_WORD_OFF));
        c.s(4, sx(saved));
        c.s(3, 0x0018_0000);
        c.w32(SAVED_WORD, saved);
        c.ret(1);
        return;
    }
    if flags & 0x40 != 0 {
        let field = c.r32(obj.wrapping_add(0x14));
        c.s(6, sx(field));
        h::call_keep(c, NOTIFY_FN, &[0, 0x11, field], &[]);
        c.ret(1);
        return;
    }
    if flags & 0x20 != 0 {
        let saved = c.r32(SAVED_WORD);
        let rec = c.r32(STATE + 4);
        c.w32(STATE + 0xe4, saved);
        c.w32(STATE + 0xf0, rec);
        c.w32(STATE + 0xf4, 0xf);
        c.w32(STATE + 0xc, 3);
        c.s(3, 0xf);
        let field = c.r32(obj.wrapping_add(0x14));
        c.s(6, sx(field));
        h::call_keep(c, NOTIFY_FN, &[0, 0x11, field], &[]);
        c.ret(0);
        return;
    }
    c.ret(0);
}

/// 0x272318 release_link(rec) -> 0: drops the claim held by rec+0x54 (claim_table::release_record) and stores the
/// result back into rec+0x54.
pub fn release_link(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let rec = c.arg(0);
    h::frame(c, 0x20, &[(16, 0, h::SQ), (31, 0x10, h::SQ)], |c, _| {
        let link = c.r32(rec.wrapping_add(0x54));
        let (v0, _) = h::run_keep(c, claim_table::release_record, &[link], &[]);
        c.w32(rec.wrapping_add(0x54), v0);
    });
    c.ret(0);
    true
}
