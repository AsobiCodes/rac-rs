//! Small leaf-like helpers: a flush chain, a stepped countdown, an OR-copy loop and an absolute value.

use crate::dma_setup;
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// MIPS register numbers used below.
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;
const RA: usize = 31;

/// Retail frame size of 0x1eefb0 (one saved quadword: ra).
const FRAME: u32 = 0x10;

/// Sign-extends a 32-bit result the way MIPS 32-bit ops leave a 64-bit register.
fn sext32(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// Reads the 16-byte quadword at `a` (low four address bits ignored) as one 128-bit value.
fn load_quad(c: &mut Cpu, a: u32) -> u128 {
    let base = a & !0xf;
    (0..4).fold(0u128, |acc, i| acc | (c.r32(base + 4 * i) as u128) << (32 * i))
}

/// Writes the 128-bit value `q` as a quadword at `a` (low four address bits ignored).
fn store_quad(c: &mut Cpu, a: u32, q: u128) {
    let base = a & !0xf;
    for i in 0..4 {
        c.w32(base + 4 * i, (q >> (32 * i)) as u32);
    }
}

/// 0x1eefb0 flush_chain() -> void: emits the DMA tag, then runs the two retail flush steps.
pub fn flush_chain(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    h::frame(c, FRAME, &[(RA, 0, h::SQ)], |c, _sp| {
        let _ = h::run(c, dma_setup::tag_emit_or_flush, &[], &[]);
        let _ = h::call(c, 0x1ea0b0, &[], &[]);
        let _ = h::call(c, 0x1ea1e8, &[], &[]);
    });
    true
}

/// 0x1f15b0 countdown_by_two(n) -> void: steps a0 down by 2 at least once, until the pre-step value is <= 0.
pub fn countdown_by_two(c: &mut Cpu) -> bool {
    // bgtz tests the full 64-bit a0 before the delay-slot addi (32-bit, sign-extended) runs.
    let mut n = c.g(A0) as i64;
    loop {
        let more = n > 0;
        n = (n as i32).wrapping_sub(2) as i64;
        if !more {
            break;
        }
    }
    c.s(A0, n as u64);
    true
}

/// 0x1f16e8 or_copy_quads(dst, src1, src2, bytes) -> void: dst[i] = src1[i] | src2[i] per quadword, the last
/// OR in v0 and last src2 quad in v1.
pub fn or_copy_quads(c: &mut Cpu) -> bool {
    let mut dst = c.g(A0) as u32;
    let mut src1 = c.g(A1) as u32;
    let mut src2 = c.g(A2) as u32;
    let mut left = c.g(A3) as i64;
    loop {
        let q1 = load_quad(c, src1);
        let q2 = load_quad(c, src2);
        src1 = src1.wrapping_add(16);
        src2 = src2.wrapping_add(16);
        dst = dst.wrapping_add(16);
        left = (left as i32).wrapping_sub(16) as i64;
        let or = q1 | q2;
        c.r[V0] = or;
        c.r[V1] = q2;
        store_quad(c, dst.wrapping_sub(16), or);
        if left <= 0 {
            break;
        }
    }
    c.s(A0, sext32(dst));
    c.s(A1, sext32(src1));
    c.s(A2, sext32(src2));
    c.s(A3, sext32(left as u32));
    true
}

/// 0x1f1798 iabs(x) -> int: |x| as a 32-bit value; a0 is negated in place when x < 0.
pub fn iabs(c: &mut Cpu) -> bool {
    let x = c.g(A0) as i64;
    if x >= 0 {
        c.ret(x as u32);
    } else {
        let n = (x as i32).wrapping_neg();
        c.s(A0, sext32(n as u32));
        c.ret(n as u32);
    }
    true
}
