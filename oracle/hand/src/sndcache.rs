//! Sound cache: slot binding, nibble run-length unpacking, and the 8-entry cache lookup.

use ee_oracle::cpu::Cpu;
use crate::ps2::*;

/// Index of the currently bound slot (signed; negative means none).
const SLOT_CUR: u32 = 0x0015_FF8C;
/// Number of valid entries in the slot id table.
const SLOT_COUNT: u32 = 0x0015_FF88;
/// Mode flag stored by `slot_bind`.
const LAST_MODE: u32 = 0x0015_FF90;
/// Tick word stamped into slot records by `slot_bind`.
const BIND_TICK: u32 = 0x0015_FF84;
/// Table of bound slot ids (one word each).
const SLOT_IDS: u32 = 0x001A_FCC0;
/// Per-slot instance pointers (one word each).
const SLOT_PTRS: u32 = 0x001A_FD20;
/// Per-slot 16-byte records.
const SLOT_REC: u32 = 0x001A_FDE0;
/// Argument block passed to the bind helper.
const SLOT_ARG: u32 = 0x001A_ECC0;
/// Per-slot table of 16 signed halfwords (32-byte stride).
const SLOT_HALVES: u32 = 0x001A_FF60;
/// Byte table: sound type of each key.
const KEY_TYPE: u32 = 0x0019_7CC0;
/// Per-type instance pointers (one word each).
const TYPE_INST: u32 = 0x0019_7400;
/// Per-type auxiliary words (one word each).
const TYPE_AUX: u32 = 0x0019_A380;
/// Key table: 37 entries of 0x4c bytes, key in the first word (retail base 0x179BD0).
const KEY_TAB: u32 = 0x0017_9BD0;
/// Byte flags parallel to the key table.
const KEY_FLAG: u32 = 0x0013_E520;
/// 8-byte configuration value copied into instance records.
const CFG_WORD: u32 = 0x0018_28F0;
/// 8-byte tail value copied into instance records.
const CFG_TAIL: u32 = 0x0018_2900;
/// Word read as the base of the bind scale.
const SCALE_BASE: u32 = 0x0017_3F10;
/// Multiplier applied to the mode flag (1.5 in 16.16).
const MODE_SCALE: u32 = 0x0001_8000;
/// Cache header: flag at +0x28, key/value entries from +0x30 with a 16-byte stride.
const CACHE: u32 = 0x0018_42F0;
/// Frame tick word used as the cache entry value.
const CACHE_TICK: u32 = 0x0015_F3F8;
/// Number of cache entries.
const CACHE_SLOTS: u32 = 8;
/// Retail helper called with 0 before and after a bind.
const HELPER_SYNC: u32 = 0x0011_8A80;
/// Retail helper taking the slot's instance pointer and the scale.
const HELPER_ATTACH: u32 = 0x0023_FE50;
/// Retail helper filling the slot's argument block.
const HELPER_FILL: u32 = 0x0023_2C08;

/// 0x237cc8 slot_bind(key, mode): binds `key` to a cache slot (move-to-front), fills the per-type instance.
pub fn slot_bind(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let key = c.arg(0);
    let mode = c.arg(1);
    let cur = c.r32(SLOT_CUR) as i32;
    if cur >= 0 && c.r32(SLOT_IDS + 4 * cur as u32) == key {
        return true;
    }
    let count = c.r32(SLOT_COUNT) as i32;
    c.w32(SLOT_CUR, 0);
    if count > 0 && c.r32(SLOT_IDS) != key {
        let mut i = 1;
        while i < count && c.r32(SLOT_IDS + 4 * i as u32) != key {
            i += 1;
        }
        c.w32(SLOT_CUR, i as u32);
    }
    let mode = if mode == u32::MAX {
        (c.r32(LAST_MODE) == 0) as u32
    } else {
        mode
    };
    c.w32(LAST_MODE, mode);
    let s1 = c.r32(SCALE_BASE).wrapping_add(mode.wrapping_mul(MODE_SCALE));
    let _ = crate::h::call(c, HELPER_SYNC, &[0], &[]);
    let cur = c.r32(SLOT_CUR);
    let slot_ptr = c.r32(SLOT_PTRS + 4 * cur);
    let _ = crate::h::call(c, HELPER_ATTACH, &[slot_ptr, s1], &[]);
    let _ = crate::h::call(c, HELPER_SYNC, &[0], &[]);

    let ty = c.r8(KEY_TYPE + key) as u32;
    let cur = c.r32(SLOT_CUR);
    c.w32(TYPE_INST + 4 * ty, s1);
    let aux = c.r32(s1 + 0x2C);
    c.w32(TYPE_AUX + 4 * ty, aux);
    let rec = SLOT_REC + 16 * cur;
    let _ = crate::h::call(c, HELPER_FILL, &[s1, SLOT_ARG, rec, key], &[]);

    // Stamp the slot's 16 halfword records into the instance's sub-table.
    let tick = c.r32(BIND_TICK);
    let cur = c.r32(SLOT_CUR);
    for a3 in 0..16u32 {
        let v = c.r16(SLOT_HALVES + 32 * cur + 2 * a3) as i16;
        if v >= 0 {
            let inst = c.r32(TYPE_INST + 4 * ty);
            let sub = c.r32(inst + 0x28);
            let dst = sub.wrapping_add(a3 << 5);
            c.w16(dst + 0x1A, v as u16);
            c.w32(dst + 0x1C, tick);
        }
    }

    // Look the key up in the key table and copy the config block into the instance.
    let mut found = None;
    for j in 0..0x25u32 {
        if c.r32(KEY_TAB + 0x4C * j) == key {
            found = Some(j);
            break;
        }
    }
    let Some(j) = found else {
        return true;
    };
    if c.r8(KEY_FLAG + j) == 0 {
        return true;
    }
    let inst = c.r32(TYPE_INST + 4 * ty);
    if c.r8(inst + 6) == 0 {
        return true;
    }
    let a = c.r32(inst).wrapping_add((c.r8(inst + 7) as u32) << 4);
    let dst = c.r32(a).wrapping_add(c.r32(a + 4).wrapping_sub(4) << 4);
    for k in 0..2 {
        let lo = c.r32(CFG_WORD + 4 * k);
        c.w32(dst + 0x20 + 4 * k, lo);
    }
    for k in 0..2 {
        let lo = c.r32(CFG_TAIL + 4 * k);
        c.w32(dst + 0x30 + 4 * k, lo);
    }
    true
}

/// 0x23a920 nibble_rle(out, n, base, table): run-length nibble unpacker. Its decode loop (the 0x23a954/0x23a958
/// words are not shown by fninfo) is not verified, so retail runs it.
pub fn nibble_rle(_c: &mut Cpu) -> bool {
    false
}

/// Swaps two 16-byte cache entries.
fn swap_entries(c: &mut Cpu, a: u32, b: u32) {
    for k in 0..4 {
        let x = c.r32(a + 4 * k);
        let y = c.r32(b + 4 * k);
        c.w32(a + 4 * k, y);
        c.w32(b + 4 * k, x);
    }
}

/// 0x23c2d8 cache_fetch(key) -> 0 on a hit, 1 on a miss: finds `key` in the 8-entry cache and moves the entry
/// to the front. A miss decodes through `nibble_rle`, which is unverified, so a miss returns false (retail runs
/// it) before any write is made.
pub fn cache_fetch(c: &mut Cpu) -> bool {
    let key = c.arg(0);
    if c.r32(CACHE + 0x28) == 0 {
        c.ret(0);
        return true;
    }
    let entry = |k: u32| CACHE + 0x30 + 16 * k;
    let mut victim = 0u32;
    let mut hit: Option<u32> = if c.r32(entry(0)) == key { Some(0) } else { None };
    if hit.is_none() {
        let mut j = 0u32;
        while j < CACHE_SLOTS {
            if (c.r32(entry(j) + 4) as i32) < (c.r32(entry(victim) + 4) as i32) {
                victim = j;
            }
            j += 1;
            if j < CACHE_SLOTS && c.r32(entry(j)) == key {
                hit = Some(j);
                break;
            }
        }
    }
    let Some(idx) = hit else {
        return false;
    };
    if idx != 0 {
        swap_entries(c, entry(0), entry(idx));
    }
    let tick = c.r32(CACHE_TICK);
    c.w32(entry(0) + 4, tick);
    c.ret(0);
    true
}
