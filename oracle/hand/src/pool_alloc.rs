//! Particle pool allocation: the body of `ALLOC_PARTICLE` (0x259a40). It takes the first free slot at the pool's hint,
//! marks it in the used-slot bitmap, moves the hint past the run of used slots, raises the top and the live count, and
//! returns the slot with its type byte set. A hint past the pool's end returns 0 without changing anything.
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// gp-relative pool state: base of the 0x40-byte slots, first free slot hint, highest used slot, live count.
const POOL_BASE: i32 = -0x6b14;
const POOL_FIRST_FREE: i32 = -0x6b10;
const POOL_TOP: i32 = -0x6b0c;
const POOL_COUNT: i32 = -0x6b08;
/// Used-slot bitmap: one bit per slot, eight slots per byte.
const SLOT_BITS: u32 = 0x1b_1980;
/// Slot indices from this one on are past the end of the pool.
const POOL_SLOTS: u32 = 0x800;
/// Slot size is 0x40 bytes.
const SLOT_SHIFT: u32 = 6;

/// GPR numbers of the registers retail's code writes.
const R_AT: usize = 1;
const R_V0: usize = 2;
const R_V1: usize = 3;
const R_A2: usize = 6;
const R_T2: usize = 10;
const R_T6: usize = 14;
const R_T7: usize = 15;
const R_T8: usize = 24;
const R_T9: usize = 25;

/// 32-bit register write: the low doubleword gets the sign-extended result, as the MIPS 32-bit ops leave it.
fn set32(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// pmaxw: signed maximum of each of the four 32-bit words of a and b.
fn pmaxw(a: u128, b: u128) -> u128 {
    let mut out = 0u128;
    for lane in 0..4 {
        let sh = lane * 32;
        let x = (a >> sh) as u32 as i32;
        let y = (b >> sh) as u32 as i32;
        out |= (x.max(y) as u32 as u128) << sh;
    }
    out
}

/// 0x259a40 particle_alloc(kind) -> slot or 0: claims the first free slot at the hint (a0 = kind goes to its first
/// byte), sets its bit in the bitmap, moves the hint past the used slots after it, raises the top and counts it.
pub fn particle_alloc(c: &mut Cpu) -> bool {
    let kind = c.arg(0) as u8;
    let gp = c.g(28) as u32;
    let base = c.r32(gp.wrapping_add(POOL_BASE as u32));
    let hint = c.r32(gp.wrapping_add(POOL_FIRST_FREE as u32));
    let top = c.r32(gp.wrapping_add(POOL_TOP as u32));
    let past_end = hint.wrapping_sub(POOL_SLOTS);
    if (past_end as i32) >= 0 {
        // Pool full: retail jumps to 0x259b80, outside this range, so retail runs this call (no writes made yet).
        return false;
    }
    set32(c, R_T7, SLOT_BITS);
    set32(c, R_T6, base);
    set32(c, R_T8, hint);
    set32(c, R_T9, top);
    set32(c, R_A2, hint);
    set32(c, R_AT, past_end);

    // Mark the slot: bit (p & 7) of byte (p >> 3) of the bitmap.
    let p0 = hint;
    let mut at = p0 >> 3;
    let mut v0: u32 = 1;
    at = at.wrapping_add(SLOT_BITS);
    let mut v1 = p0 & 7;
    v0 = v0.wrapping_shl(v1 & 31);
    v1 = c.r8(at) as u32 | v0;
    c.w8(at, v1 as u8);

    // Hint + 1; past the top, the hint is just p0 + 1.
    let mut t2 = 0u32;
    let mut t8 = p0.wrapping_add(1);
    let above_top = p0.wrapping_sub(top) as i32 > 0;
    let mut found = above_top;
    if !found {
        // Re-align the hint to its byte and test it against the pool end.
        at = t8 >> 3;
        t8 = at << 3;
        t2 = t8.wrapping_sub(POOL_SLOTS);
        v1 = c.r8(at.wrapping_add(SLOT_BITS)) as u32;
        found = (t2 as i32) >= 0;
        if !found {
            v0 = 1;
            // Walk the used bits from the hint; a byte of all-used bits moves on to the next byte.
            loop {
                let used = v1 & v0 != 0;
                v0 = v0.wrapping_shl(1);
                t2 = v0 & 0xff;
                if !used {
                    break;
                }
                t8 = t8.wrapping_add(1);
                if t2 != 0 {
                    continue;
                }
                at = at.wrapping_add(1);
                t2 = t8.wrapping_sub(POOL_SLOTS);
                v1 = c.r8(at.wrapping_add(SLOT_BITS)) as u32;
                if (t2 as i32) >= 0 {
                    break;
                }
                v0 = 1;
            }
        }
    }

    // Store the new hint, raise the top to max(top, p0), and count the slot.
    c.w32(gp.wrapping_add(POOL_FIRST_FREE as u32), t8);
    let t9 = pmaxw(c.r[R_T9], c.r[R_A2]);
    c.r[R_T9] = t9;
    c.w32(gp.wrapping_add(POOL_TOP as u32), t9 as u32);
    let count = c.r32(gp.wrapping_add(POOL_COUNT as u32)).wrapping_add(1);
    c.w32(gp.wrapping_add(POOL_COUNT as u32), count);

    let slot = (p0 << SLOT_SHIFT).wrapping_add(base);
    // Retail clears the slot's first 0x20 bytes (four quadword stores) before it writes the kind byte.
    c.wv(slot, V([0; 4]));
    c.wv(slot.wrapping_add(0x10), V([0; 4]));
    c.w8(slot, kind);
    set32(c, R_T8, t8);
    if !above_top {
        set32(c, R_T2, t2);
    }
    set32(c, R_V1, v1);
    set32(c, R_AT, p0 >> 3);
    set32(c, R_V0, slot);
    true
}
