//! Actor-slot registries: a u16 count followed by u16 entries, each entry a slot index into the actor pool.
//! The pool base pointer lives in a global; a slot index is the byte offset from that base divided by 256.

use crate::h;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Global holding the actor pool base (`lui 0x16` then `lw` at offset -0xa8).
const ACTOR_BASE_PTR: u32 = 0x0015_ff58;
/// Actor pool stride as a shift: a slot index times 256 is the byte offset from the base.
const SLOT_SHIFT: u32 = 8;
/// 255.0f, the colour scale.
const SCALE_255_BITS: u32 = 0x437f_0000;

/// Writes a 32-bit result to a GPR, sign-extended as the MIPS 32-bit ops leave it.
fn set(c: &mut Cpu, reg: usize, v: u32) {
    c.r[reg] = v as i32 as i128 as u128;
}

// MIPS register numbers used here.
const AT: usize = 1;
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;
const T0: usize = 8;

/// 0x24eba0 pack_rgba_f(r, g, b, a) -> u32: four 0..1 floats, each x255 truncated, packed as bytes r, g, b, a (low to high).
pub fn pack_rgba_f(c: &mut Cpu) -> bool {
    let scale = F(SCALE_255_BITS);
    set(c, AT, SCALE_255_BITS);
    let channels = [c.farg(0), c.farg(1), c.farg(2), c.farg(3)];
    let mut bytes = [0u32; 4];
    for (i, ch) in channels.iter().enumerate() {
        let (v, _) = h::run_keep(c, mathx::float_to_int, &[], &[*ch * scale]);
        bytes[i] = v;
    }
    // retail leaves f12 holding the integer bits of the last converted channel (alpha)
    c.set_farg(0, F(bytes[3]));
    let word = (bytes[0] & 0xff) | ((bytes[1] & 0xff) << 8) | ((bytes[2] & 0xff) << 16) | (bytes[3] << 24);
    c.ret(word);
    true
}

/// 0x252340 rocket_registry_insert(actor, list, cap) -> u32: append the actor's slot to the u16 count+list registry
/// if it is absent and the count is below cap; returns the new entry's address, the existing slot, or 0 when full.
pub fn rocket_registry_insert(c: &mut Cpu) -> bool {
    let base = c.r32(ACTOR_BASE_PTR);
    let slot = (((c.arg(0).wrapping_sub(base)) << SLOT_SHIFT) as i32) >> 16;
    let list = c.arg(1);
    let cap = c.arg(2) as u16 as i16 as i32;
    let count = c.r16(list) as i16 as i32;

    let mut t0 = if count > 0 { count as u32 } else { (c.r[T0] as u32) };
    let mut hit = None;
    for k in 0..count.max(0) {
        if c.r16(list + 2 + 2 * k as u32) as i16 as i32 == slot {
            hit = Some(k);
            break;
        }
    }
    let (ret, v1, a3) = match hit {
        Some(k) => {
            t0 = count as u32;
            // retail: the beq delay slot (addiu a3) runs on the matching iteration too, so a3 = k + 2
            (slot as u32, list + 2 + 2 * k as u32, k as u32 + 2)
        }
        None => {
            let (v1, a3) = if count > 0 { (list + 2 + 2 * count as u32, count as u32 + 1) } else { (count as u32, 1) };
            if count < cap {
                let new_count = c.r16(list) as u32 + 1;
                c.w16(list, new_count as u16);
                let offset = (((new_count << 16) as i32) >> 15) as u32;
                let addr = list.wrapping_add(offset);
                c.w16(addr, slot as u16);
                (addr, new_count, a3)
            } else {
                (0, v1, a3)
            }
        }
    };
    set(c, A0, slot as u32);
    set(c, A2, cap as u32);
    set(c, A3, a3);
    set(c, T0, t0);
    set(c, V1, v1);
    c.ret(ret);
    true
}

/// 0x2523c0 rocket_registry_remove(actor, list) -> i32: remove the actor's slot from the u16 registry, the last entry
/// fills the gap; returns count-1 when found, 1 when absent, and the (non-positive) count when empty.
pub fn rocket_registry_remove(c: &mut Cpu) -> bool {
    let base = c.r32(ACTOR_BASE_PTR);
    let actor = c.arg(0);
    let list = c.arg(1);
    let count = c.r16(list) as i16 as i32;
    let count_u = c.r16(list) as u32;
    set(c, T0, 1);
    set(c, A3, count_u);
    let (ret, v1, a2, t0) = if count <= 0 {
        (count as u32, (c.r[V1] as u32), (c.r[A2] as u32), 1)
    } else {
        let mut found = None;
        let mut last_p = 0;
        for k in 0..count_u {
            let entry = c.r16(list + 2 + 2 * k) as i16 as i32;
            last_p = base.wrapping_add((entry as u32) << SLOT_SHIFT);
            if last_p == actor {
                found = Some(k);
                break;
            }
        }
        match found {
            Some(k) => {
                let last = c.r16(list + 2 * count_u);
                let hit_addr = list + 2 + 2 * k;
                c.w16(hit_addr, last);
                c.w16(list, (count_u - 1) as u16);
                (count_u - 1, last as u32, hit_addr, k + 1)
            }
            None => (1, last_p, list + 2 + 2 * count_u, count_u + 1),
        }
    };
    set(c, V1, v1);
    set(c, A2, a2);
    set(c, T0, t0);
    c.ret(ret);
    true
}
