//! Claim table of five 8-byte records at 0x1ba2b8 (key word, then flag word): 0x2766a8 claims a record for a key,
//! 0x276768 releases it, 0x276818 looks a key up.
use crate::h;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Table base (lui 0x1c, addiu -0x5d48); record i starts at TABLE + 8 * i.
const TABLE: u32 = 0x001b_a2b8;
/// Records in the table.
const RECORDS: u32 = 5;
/// Bytes per record.
const STRIDE: u32 = 8;
/// Offset of the flag word inside a record.
const FLAG_OFF: u32 = 4;
/// Flag bit 0: selects the lookup code.
const FLAG_SELECT: u32 = 1;
/// Flag bit 1: record is claimed.
const FLAG_CLAIMED: u32 = 2;
/// Flag bit 2: release is deferred to the pending hook.
const FLAG_PENDING: u32 = 4;
/// Lookup code for a record whose select bit is set (lui 4 / ori 0xf000).
const CODE_SELECTED: u32 = 0x0004_f000;
/// Lookup code for a record whose select bit is clear (lui 1 / ori 0x1800).
const CODE_PLAIN: u32 = 0x0001_1800;
/// Lookup result when no record holds the key (-1).
const CODE_MISSING: u32 = u32::MAX;
/// Pattern that fill_words writes after a claim.
const FILL_PATTERN: u32 = 0xdead_beef;
/// Byte that gates the pending hook (0x1b9df0 + 0xcb).
const PENDING_BYTE: u32 = 0x001b_9ebb;
/// Pending hook (0x258080), run once by a release while PENDING_BYTE is set.
const PENDING_HOOK: u32 = 0x0025_8080;

/// Address of record `i`.
fn record(i: u32) -> u32 {
    TABLE + i * STRIDE
}

/// Lookup code of the record holding `key`, or CODE_MISSING.
fn lookup_code(c: &mut Cpu, key: u32) -> u32 {
    for i in 0..RECORDS {
        let rec = record(i);
        if c.r32(rec) == key {
            return if c.r32(rec + FLAG_OFF) & FLAG_SELECT != 0 { CODE_SELECTED } else { CODE_PLAIN };
        }
    }
    CODE_MISSING
}

/// 0x2766a8 claim(invert) -> key: claims the first record that is unclaimed, has a nonzero key and whose select bit
/// equals `invert` (a0 != 0); marks it, fills from its key with FILL_PATTERN and returns the key, or 0 if none.
pub fn claim_slot(c: &mut Cpu) -> bool {
    let invert = (c.arg(0) as u32 != 0) as u32;
    for i in 0..RECORDS {
        let rec = record(i);
        let flags = c.r32(rec + FLAG_OFF);
        if ((flags & FLAG_SELECT) ^ invert) != 0 {
            continue;
        }
        let key = c.r32(rec);
        if key == 0 || flags & FLAG_CLAIMED != 0 {
            continue;
        }
        c.w32(rec + FLAG_OFF, flags | FLAG_CLAIMED);
        let code = lookup_code(c, key);
        h::run_keep(c, mathx::fill_words, &[key, FILL_PATTERN, code], &[]);
        c.ret(key);
        return true;
    }
    c.ret(0);
    true
}

/// 0x276768 release(key) -> 0: for the claimed record holding `key`, drops the claim; a pending record first toggles
/// its pending bit, runs the pending hook once if PENDING_BYTE is set, then drops the claim.
pub fn release_record(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let key = c.arg(0) as u32;
    for i in 0..RECORDS {
        let rec = record(i);
        if c.r32(rec) != key {
            continue;
        }
        let flags = c.r32(rec + FLAG_OFF);
        if flags & FLAG_CLAIMED == 0 {
            continue;
        }
        if flags & FLAG_PENDING != 0 {
            c.w32(rec + FLAG_OFF, flags ^ FLAG_PENDING);
            if c.r8(PENDING_BYTE) != 0 {
                let _ = h::call_keep(c, PENDING_HOOK, &[key], &[]);
                c.w8(PENDING_BYTE, 0);
            }
        }
        let flags = c.r32(rec + FLAG_OFF);
        c.w32(rec + FLAG_OFF, flags & !FLAG_CLAIMED);
        break;
    }
    c.ret(0);
    true
}

/// 0x276818 lookup(key) -> code: the lookup code of the record holding `key` (CODE_SELECTED when its select bit is
/// set, else CODE_PLAIN), or CODE_MISSING.
pub fn lookup_entry(c: &mut Cpu) -> bool {
    let key = c.arg(0) as u32;
    let code = lookup_code(c, key);
    c.ret(code);
    true
}
