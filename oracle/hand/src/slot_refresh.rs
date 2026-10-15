//! Actor slot refresh (retail 0x25fe80..0x25ffbc): releases a blocked slot, otherwise repacks the slot's
//! 0x00ffffff/0xff000000 field from its scale terms and either copies the 16-byte pose block or poses the node.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;
const AT: usize = 1;
const S0: usize = 16;
const S1: usize = 17;
const RA: usize = 31;

/// Helper (retail 0x1f1530): nonzero while the slot at `node + 0x24` is still in use.
const SLOT_BUSY: u32 = 0x001f_1530;
/// Retail slot release (0x259b88), `slot_bits::slot_release`.
const SLOT_RELEASE: u32 = 0x0025_9b88;
/// Helper (retail 0x1f1840): fills a pose temp from the table at 0x166ec0.
const POSE_TEMP_INIT: u32 = 0x001f_1840;
/// Pose table address passed to `POSE_TEMP_INIT`.
const POSE_TABLE: u32 = 0x0016_6ec0;
/// Helper (retail 0x1f1810): adds two vec4 temps into the slot's pose block.
const POSE_BLEND: u32 = 0x001f_1810;
/// Length the pose vector is normalised to (0.4).
const POSE_LENGTH: F = F(0x3ecc_cccd);
/// Stack frame size of the retail function.
const FRAME: u32 = 0x50;

/// The R5900 `div`: (quotient, remainder) as LO and HI are left, including its divide-by-zero and overflow results.
fn div32(n: i32, d: i32) -> (i32, i32) {
    if d == 0 {
        (if n < 0 { 1 } else { -1 }, n)
    } else if n == i32::MIN && d == -1 {
        (i32::MIN, 0)
    } else {
        (n / d, n % d)
    }
}

/// Writes LO and HI (their low 64 bits, sign-extended from 32).
fn set_hilo(c: &mut Cpu, hi: i32, lo: i32) {
    c.lo = c.lo & !(u64::MAX as u128) | (lo as i64 as u64) as u128;
    c.hi = c.hi & !(u64::MAX as u128) | (hi as i64 as u64) as u128;
}

/// Writes a 32-bit result into a GPR, sign-extended as `addiu`/`lw` leave it.
fn put(c: &mut Cpu, r: usize, v: i32) {
    c.s(r, v as i64 as u64);
}

/// 0x25fe80 slot_refresh(slot) -> void: if the slot is busy or has no valid owner, releases it; otherwise
/// repacks the slot's scale field and copies or poses its pose block.
pub fn slot_refresh(c: &mut Cpu) -> bool {
    h::frame(c, FRAME, &[(S0, 0x20, h::SQ), (S1, 0x30, h::SQ), (RA, 0x40, h::SQ)], |c, sp| {
        let slot = c.g(A0) as u32;
        put(c, S1, slot as i32);
        if slot != 0 {
            refresh(c, slot, sp);
        }
    });
    true
}

/// The body for a non-null slot; `sp` is the frame's stack pointer.
fn refresh(c: &mut Cpu, slot: u32, sp: u32) {
    put(c, S0, slot.wrapping_add(0x20) as i32);
    let busy = h::call_keep(c, SLOT_BUSY, &[slot.wrapping_add(0x24)], &[]).0;
    let mut owner_ok = false;
    if busy == 0 {
        let owner = c.r32(slot.wrapping_add(0x20));
        put(c, V0, owner as i32);
        if owner != 0 {
            let flag = c.r8(owner.wrapping_add(0x20)) as i8;
            put(c, V0, flag as i32);
            owner_ok = flag >= 0;
        }
    }
    if !owner_ok {
        put(c, A0, slot as i32);
        h::run_keep(c, crate::slot_bits::slot_release, &[slot], &[]);
        return;
    }
    let scale_a = c.r32(slot.wrapping_add(0x28)) as i32;
    put(c, A2, scale_a);
    let rate = c.r16(slot.wrapping_add(0x24)) as i16 as i32;
    put(c, A3, rate);
    let (q5, r5) = div32(scale_a, 5);
    set_hilo(c, r5, q5);
    put(c, V0, q5);
    let repack = rate < q5;
    put(c, V0, repack as i32);
    if repack {
        put(c, A1, 3);
        let word = c.r32(slot.wrapping_add(0x2c)) as i32;
        put(c, V1, word);
        let (q3, _) = div32(scale_a, 3);
        put(c, A2, q3);
        let field = c.r32(slot.wrapping_add(4)) & 0x00ff_ffff;
        put(c, V0, 0x00ff_ffff);
        let prod = word.wrapping_mul(rate);
        put(c, V1, prod);
        let (lo, hi) = div32(prod, q3);
        set_hilo(c, hi, lo);
        put(c, V1, lo);
        let top = (lo as u32).wrapping_shl(24) as i32;
        put(c, V1, top);
        let packed = field.wrapping_add(top as u32);
        put(c, A0, packed as i32);
        c.w32(slot.wrapping_add(4), packed);
    }
    let blend_len = c.r16(slot.wrapping_add(0x26)) as i16 as i32;
    put(c, A1, blend_len);
    put(c, V0, -1);
    if blend_len != -1 {
        let owner = c.r32(slot.wrapping_add(0x20));
        put(c, A0, owner as i32);
        put(c, A2, sp as i32);
        h::run_keep(c, crate::actor::pose_node_pos, &[owner, blend_len as u32, sp], &[]);
        let temp = sp.wrapping_add(0x10);
        put(c, S0, temp as i32);
        put(c, A1, POSE_TABLE as i32);
        put(c, A2, sp as i32);
        h::call_keep(c, POSE_TEMP_INIT, &[temp, POSE_TABLE, sp], &[]);
        put(c, AT, 0x3ecc_cccd);
        h::run_keep(c, crate::vu_vec::vec_set_length, &[temp, temp], &[POSE_LENGTH]);
        h::call_keep(c, POSE_BLEND, &[slot.wrapping_add(0x10), sp, temp], &[]);
    } else {
        let owner = c.r32(slot.wrapping_add(0x20));
        put(c, V1, owner.wrapping_add(0x10) as i32);
        put(c, A0, slot.wrapping_add(0x10) as i32);
        let src = owner.wrapping_add(0x10) & !0xf;
        c.r[V0] = c.mem.rd128(src);
        c.mem.wr128(slot.wrapping_add(0x10) & !0xf, c.r[V0]);
    }
}
