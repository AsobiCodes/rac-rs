//! HUD pool bump allocator (pool header at 0x17e5d8) and the HUD slot initialiser that uses it.
use crate::h;
use crate::hud_counter;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Pool header: cursor at +0x10, end at +0x14.
const POOL_HDR: u32 = 0x0017_e5d8;
const POOL_CUR: u32 = POOL_HDR + 0x10;
const POOL_END: u32 = POOL_HDR + 0x14;
/// Lazily initialises the pool when its cursor is zero.
const POOL_INIT: u32 = 0x0022_8708;
/// v1 as left by `lui v1, 0x18`.
const LUI_18: u64 = 0x0018_0000;
/// v1 as left by `lui v1, 0xffff; ori v1, v1, 0xfff0` (64-bit).
const MASK_F0: u64 = 0xffff_ffff_ffff_fff0;
/// Argument to mathx::float_to_int_scaled (0xb4).
const SCALE_ARG: u32 = 0xb4;
/// Added to the scaled value before it is stored (0x1e).
const SLOT_OFFSET: u32 = 0x1e;
const SLOT_COUNTER_SIZE: u32 = 0x7c;

/// 0x228730 pool_grab(size) -> old cursor or 0: bumps the pool cursor by size rounded up to 16 bytes
/// (initialising the pool first if its cursor is zero); returns 0 when less than size bytes remain.
pub fn pool_grab(c: &mut Cpu) -> bool {
    let size = c.arg(0);
    c.s(3, LUI_18);
    if c.r32(POOL_CUR) == 0 {
        h::call_keep(c, POOL_INIT, &[size], &[]);
    }
    let cur = c.r32(POOL_CUR);
    let end = c.r32(POOL_END);
    c.s(4, cur as i32 as i64 as u64);
    if (end.wrapping_sub(cur) as i32) < size as i32 {
        c.ret(0);
        return true;
    }
    let padded = size.wrapping_add(15) & 0xffff_fff0;
    let next = cur.wrapping_add(padded);
    c.s(3, MASK_F0);
    c.s(5, size.wrapping_add(15) as i32 as i64 as u64);
    c.w32(POOL_CUR, next);
    c.s(4, next as i32 as i64 as u64);
    c.ret(cur);
    true
}

/// 0x228ec0 hud_slot_init(slot) -> counter size: clears the 16-bit fields at +0x48 and +0x4a, stores
/// the scaled float (180) plus 30 at +0x7c, then returns hud_counter::counter_size(slot).
pub fn hud_slot_init(c: &mut Cpu) -> bool {
    let slot = c.arg(0);
    let (scaled, _) = h::run_keep(c, mathx::float_to_int_scaled, &[SCALE_ARG], &[]);
    c.w16(slot + 0x48, 0);
    c.w16(slot + 0x4a, 0);
    c.w32(slot + SLOT_COUNTER_SIZE, scaled.wrapping_add(SLOT_OFFSET));
    let (size, _) = h::run_keep(c, hud_counter::counter_size, &[slot], &[]);
    c.ret(size);
    true
}
