//! Register-convention wrappers: call the vector, trig and math helpers (Rust or retail) with plain arguments, and
//! reserve stack frames so locals live where retail keeps them.
use crate::ps2::*;
use crate::{trig, vecmath};
use ee_oracle::cpu::Cpu;

/// The register file at entry. The checker counts every register a rewrite changes as a result, so helpers that
/// make retail calls or set arguments restore the registers (`restore`) before they write their real results.
pub struct Regs {
    r: [u128; 32],
    f: [u32; 32],
}

/// Snapshot of the general and float registers.
pub fn save(c: &Cpu) -> Regs {
    Regs { r: c.r, f: c.f }
}

/// Puts back the registers saved by `save` (call before writing results).
pub fn restore(c: &mut Cpu, regs: &Regs) {
    c.r = regs.r;
    c.f = regs.f;
}

/// Saves the registers and lowers the EE stack pointer by `size`; returns (saved registers, frame base).
pub fn push_frame(c: &mut Cpu, size: u32) -> (Regs, u32) {
    let saved = save(c);
    let base = (c.g(29) as u32).wrapping_sub(size);
    c.s(29, base as i32 as i64 as u64);
    (saved, base)
}

/// Restores the registers (and stack pointer) saved by `push_frame`.
pub fn pop_frame(c: &mut Cpu, saved: &Regs) {
    restore(c, saved);
}

/// 0x1f1810 vec_add(out, a, b).
pub fn vec_add(c: &mut Cpu, out: u32, a: u32, b: u32) {
    c.set_arg(0, out);
    c.set_arg(1, a);
    c.set_arg(2, b);
    vecmath::vec_add(c);
}

/// 0x1f1840 vec_sub(out, a, b).
pub fn vec_sub(c: &mut Cpu, out: u32, a: u32, b: u32) {
    c.set_arg(0, out);
    c.set_arg(1, a);
    c.set_arg(2, b);
    vecmath::vec_sub(c);
}

/// 0x1f1898 vec_scale(out, v, s).
pub fn vec_scale(c: &mut Cpu, out: u32, v: u32, s: F) {
    c.set_arg(0, out);
    c.set_arg(1, v);
    c.set_farg(0, s);
    vecmath::vec_scale(c);
}

/// 0x1f1a98 vec_set_length(out, v, len).
pub fn vec_set_length(c: &mut Cpu, out: u32, v: u32, len: F) {
    c.set_arg(0, out);
    c.set_arg(1, v);
    c.set_farg(0, len);
    vecmath::vec_set_length(c);
}

/// 0x1f19a0 length_xy(v).
pub fn length_xy(c: &mut Cpu, v: u32) -> F {
    c.set_arg(0, v);
    vecmath::length_xy(c);
    c.fresult()
}

/// 0x1f1a20 dist_xy(a, b).
pub fn dist_xy(c: &mut Cpu, a: u32, b: u32) -> F {
    c.set_arg(0, a);
    c.set_arg(1, b);
    vecmath::dist_xy(c);
    c.fresult()
}

/// 0x1f1e48 fast_atan2(a, b).
pub fn atan2(c: &mut Cpu, a: F, b: F) -> F {
    c.set_farg(0, a);
    c.set_farg(1, b);
    trig::fast_atan2(c);
    c.fresult()
}

/// 0x1f1778 sqrt_q(x).
pub fn sqrt_q(c: &mut Cpu, x: F) -> F {
    c.set_farg(0, x);
    crate::mathx::sqrt_q(c);
    c.fresult()
}

/// 0x1f2370 mat4_mul(out, a, b).
pub fn mat4_mul(c: &mut Cpu, out: u32, a: u32, b: u32) {
    c.set_arg(0, out);
    c.set_arg(1, a);
    c.set_arg(2, b);
    vecmath::mat4_mul(c);
}

/// 0x1f2400 quat_mul(out, a, b).
pub fn quat_mul(c: &mut Cpu, out: u32, a: u32, b: u32) {
    c.set_arg(0, out);
    c.set_arg(1, a);
    c.set_arg(2, b);
    vecmath::quat_mul(c);
}

/// 0x1e24d8 line_probe(from, to, flags, a3, t0): retail collision segment test; non-zero when it hits.
pub fn line_probe(c: &mut Cpu, from: u32, to: u32, flags: u32, a3: u32, t0: u32) -> u32 {
    c.set_arg(0, from);
    c.set_arg(1, to);
    c.set_arg(2, flags);
    c.set_arg(3, a3);
    c.set_arg(4, t0);
    c.call(0x1e_24d8);
    c.result()
}
