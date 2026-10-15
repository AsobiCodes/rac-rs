//! Plane-test bit mask over a record of float bounds (retail 0x291fb0) and the lookup that calls it (0x2920f8).
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const V0: usize = 2;
const V1: usize = 3;
const AT: usize = 1;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;
const T0: usize = 8;
const RA: usize = 31;

/// Float 1.0.
const ONE: F = F(0x3f80_0000);
/// Record offset of the first of the two float planes tested (word-spaced, one per pass).
const PLANE0: u32 = 0x30;
/// Record offsets of the float bound pair tested after the planes.
const BOUND_LO: u32 = 0x18;
const BOUND_HI: u32 = 0x1c;
/// Object offsets: the key byte, the thirteen word pointers, and the result byte of the table entry.
const KEY_OFF: u32 = 0x52;
const TABLE: u32 = 0x70;
const TABLE_LEN: u32 = 13;
const RESULT_OFF: u32 = 0xb6;

/// Sets general register `n` to the 32-bit value `v`, sign-extended as a MIPS 32-bit op leaves it.
fn set_gpr(c: &mut Cpu, n: usize, v: u32) {
    c.s(n, v as i32 as i64 as u64);
}

/// One plane pass. `x` is the plane at +0x10, `y` at +0x18 (the bound), `z` at +0x0, `w` at -8 (the previous
/// word). Each comparison that holds ORs the current bit into `mask`; `bits` doubles twice per pass.
fn plane_step(x: F, y: F, z: F, w: F, t: F, bits: &mut u32, mask: &mut u32) {
    // Pass one: is `x` at or below the bound `t`, and where does `t` sit against `y`.
    let below_t = if x <= y {
        x <= t
    } else if x <= t {
        *mask |= *bits;
        false
    } else {
        true
    };
    if below_t && t <= y {
        *mask |= *bits;
    }
    *bits <<= 1;
    // Pass two: the position of `t` against the pair (z, w).
    if w <= z {
        if w <= t && t <= z {
            *mask |= *bits;
        }
    } else if w <= t || t <= z {
        *mask |= *bits;
    }
    *bits <<= 1;
}

/// 0x291fb0 plane_mask(rec, t) -> mask: two plane passes over rec+0x30 and rec+0x34 (bits from 1 upward), then, when
/// the mask is still 0, the bound pair at rec+0x18 / rec+0x1c against t. Returns the mask.
pub fn plane_mask(c: &mut Cpu) -> bool {
    let rec = c.arg(0);
    let t = c.farg(0);
    let mut bits: u32 = 1;
    let mut mask: u32 = 0;
    let mut plane = rec + PLANE0;
    let (mut last_w, mut last_z) = (F(0), F(0));
    for _ in 0..2 {
        let x = c.rf(plane + 0x10);
        let y = c.rf(plane + 0x18);
        let z = c.rf(plane);
        let w = c.rf(plane - 8);
        plane_step(x, y, z, w, t, &mut bits, &mut mask);
        last_w = w;
        last_z = z;
        plane += 4;
    }
    set_gpr(c, V1, plane);
    set_gpr(c, A2, u32::MAX);
    if mask != 0 {
        c.fret(last_w);
        set_f1(c, last_z);
        c.ret(mask);
        set_gpr(c, A1, mask);
        return true;
    }
    // Bound test: f0 keeps the last previous word unless a 1.0 sum is formed below.
    let mut f0 = last_w;
    let lo = c.rf(rec + BOUND_LO);
    let mut at_set = false;
    if t < lo {
        mask = bits;
    } else {
        at_set = true;
        f0 = lo + ONE;
        if f0 < t {
            mask = bits;
        }
    }
    let hi = c.rf(rec + BOUND_HI);
    bits <<= 1;
    if t < hi {
        mask |= bits;
    } else {
        at_set = true;
        f0 = hi + ONE;
        if f0 < t {
            mask |= bits;
        }
    }
    if at_set {
        set_gpr(c, AT, 0x3f80_0000);
    }
    c.fret(f0);
    set_f1(c, hi);
    c.ret(mask);
    set_gpr(c, A1, mask);
    true
}

/// 0x2920f8 plane_find(obj, table) -> v0: the key byte of `obj` (+0x52) is compared with the word each of the thirteen
/// pointers at obj+0x70 leads to; the last match is run through plane_mask. With no match returns the byte at
/// obj+0xb6 instead.
pub fn plane_find(c: &mut Cpu) -> bool {
    let key_src = c.arg(0);
    let obj = c.arg(1);
    h::frame(c, 0x10, &[(RA, 0, h::SQ)], |c, _sp| {
        let key = c.r8(key_src + KEY_OFF) as u32;
        set_gpr(c, T0, key);
        let mut hit = 0;
        let mut slot = obj + TABLE;
        let mut last_ptr = 0;
        let mut last_x = 0;
        for _ in 0..TABLE_LEN {
            let ptr = c.r32(slot);
            slot += 4;
            let x = c.r32(ptr) ^ key;
            if x == 0 {
                hit = ptr;
            }
            last_ptr = ptr;
            last_x = x;
        }
        set_gpr(c, V0, last_x);
        set_gpr(c, V1, last_ptr);
        set_gpr(c, A2, slot);
        set_gpr(c, A3, u32::MAX);
        set_gpr(c, A0, hit);
        if hit != 0 {
            plane_mask(c);
        } else {
            let flag = c.r8(obj + RESULT_OFF) as u32;
            c.ret(flag);
        }
        true
    })
}

/// Sets float register f1 (the bound compare leaves it as retail does).
fn set_f1(c: &mut Cpu, f: F) {
    c.f[1] = f.0;
}
