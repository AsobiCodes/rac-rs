//! Small moby/object field helpers and the animation start routine.
use crate::actor;
use crate::ps2::*;
use crate::rcall::{restore, save};
use ee_oracle::cpu::Cpu;

/// Active-slot colour table: 16 bytes per active slot index, loaded from moby+0xf0 when a slot is taken.
const ACTIVE_TABLE: u32 = 0x19_6e00;
/// Retail routine that binds a moby to an active slot (`slot | 0x300`).
const BIND_SLOT_FN: u32 = 0x24_45b0;
/// Table of 64-byte records indexed by the byte at moby+0xa4.
const LINK_RECORDS: u32 = 0x17_8200;
/// Above this animation time (0.025) a new active slot is taken.
const START_TIME_LIMIT: F = F(0x3ccc_cccd);

/// 0x24a148 anim_start(moby, anim, last_frame_limit, speed): selects animation `anim` (clamped frame count
/// byte at +0x51), takes an active slot when the moby has been animating (time > 0.025) or holds
/// track state (+0x60 / +0x64), reloads the tracks and resets time (+0x54 = 0, +0x58 = 1, +0x5c = 1 / speed).
pub fn anim_start(c: &mut Cpu) -> bool {
    let (m, anim, limit, speed) = (c.arg(0), c.arg(1), c.arg(2) as i32, c.arg(3));
    let saved = save(c);
    let entry = |c: &mut Cpu| {
        let table = c.r32(m + 0x24).wrapping_add(anim << 2);
        c.r32(table + 0x48)
    };
    let e = entry(c);
    let frames = c.r8(e + 0x10) as i32;
    let last = (if limit < frames { limit } else { frames - 1 }) as u8;
    let busy = START_TIME_LIMIT < c.rf(m + 0x54) || c.r32(m + 0x60) != 0 || c.r32(m + 0x64) != 0;
    if busy {
        c.set_arg(0, m);
        actor::active_add(c);
        let slot = c.result() as i32;
        if slot >= 0 {
            c.set_arg(0, m);
            c.set_arg(1, slot as u32 | 0x300);
            c.call(BIND_SLOT_FN);
            let colour = c.rv(m + 0xf0);
            c.wv(ACTIVE_TABLE + ((slot as u32) << 4), colour);
            let prev = c.r8(m + 0x52);
            if prev != 0xff {
                c.w8(m + 0xa5, prev);
            }
            c.w8(m + 0x52, 0xff);
            c.w8(m + 0x50, slot as u8);
        }
    }
    c.w8(m + 0x51, last);
    c.w8(m + 0x53, anim as u8);
    c.set_arg(0, m);
    actor::anim_load_tracks(c);
    c.wf(m + 0x58, F::ONE);
    let rate = F::ONE / F::from_int(speed as i32);
    let flags = c.r8(m + 0x70) & 0xfd;
    c.w8(m + 0x70, flags);
    c.w32(m + 0x54, 0);
    c.wf(m + 0x5c, rate);
    let e = entry(c);
    let v = c.r8(e + 0x11);
    c.w8(m + 0x7c, v);
    restore(c, &saved);
    true
}

/// 0x24c490 link_active(obj) -> 1 when obj+0x94 points to a record whose word +8 is non-zero, else 0.
pub fn link_active(c: &mut Cpu) -> bool {
    let p = c.r32(c.arg(0) + 0x94);
    let r = p != 0 && c.r32(p + 8) != 0;
    c.ret(r as u32);
    true
}

/// 0x24c4e8 probe_init(obj, a, b, src, length): sets obj+0x10 = a, +0x14 = b, +0x1c = length, +0x20 = 1 and copies the
/// qword at src to obj+0.
pub fn probe_init(c: &mut Cpu) -> bool {
    let (obj, a, b, src, len) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3), c.farg(0));
    c.w32(obj + 0x10, a);
    c.w32(obj + 0x14, b);
    c.wf(obj + 0x1c, len);
    c.w32(obj + 0x20, 1);
    let q = c.rv(src);
    c.wv(obj, q);
    true
}

/// 0x24cfe0 link_record(obj, mask, keep) -> record pointer or 0: the 64-byte record indexed by obj+0xa4 when it
/// belongs to obj and its flags (+0x24) & mask != 0; otherwise 0, and unless `keep` the link byte is cleared.
pub fn link_record(c: &mut Cpu) -> bool {
    let (obj, mask, keep) = (c.arg(0), c.arg(1), c.arg(2));
    let idx = c.r8(obj + 0xa4);
    if idx == 0xff {
        c.ret(0);
        return true;
    }
    let rec = LINK_RECORDS + ((idx as u32) << 6);
    let owner = c.r32(rec + 0x34);
    if owner != obj {
        c.ret(0);
        return true;
    }
    if c.r32(rec + 0x24) & mask != 0 {
        c.ret(rec);
        return true;
    }
    if keep == 0 {
        c.w8(owner + 0xa4, 0xff);
    }
    c.ret(0);
    true
}

/// 0x24eef8 attached_value(obj) -> word at *(obj+0x78) when obj != 0 and (halfword obj+0x34 & 0x20), else 0.
pub fn attached_value(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    if obj == 0 || c.r16(obj + 0x34) & 0x20 == 0 {
        c.ret(0);
        return true;
    }
    let p = c.r32(obj + 0x78);
    let v = c.r32(p);
    c.ret(v);
    true
}
