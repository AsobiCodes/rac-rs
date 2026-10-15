//! Small helpers shared by the hero modules: calling retail functions and reserving their stack frames.
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// The hero state block (lui 0x14, -0xcb0): pointer table, flags and vectors the hero code works on.
pub const HERO: u32 = 0x13_f350;

/// Calls the retail (or already rewritten) function at `addr` with integer and float arguments.
pub fn call(c: &mut Cpu, addr: u32, args: &[u32], fargs: &[F]) {
    for (i, v) in args.iter().enumerate() {
        c.set_arg(i, *v);
    }
    for (i, v) in fargs.iter().enumerate() {
        c.set_farg(i, *v);
    }
    c.call(addr);
}

/// Runs `f` with `buf` = the start of a `frame`-byte stack frame, so locals keep retail's stack addresses.
pub fn with_frame<R>(c: &mut Cpu, frame: u32, f: impl FnOnce(&mut Cpu, u32) -> R) -> R {
    let sp = c.g(29) as u32;
    let buf = sp - frame;
    c.s(29, buf as i32 as i64 as u64);
    let r = f(c, buf);
    c.s(29, sp as i32 as i64 as u64);
    r
}

/// Runs a hand function whose retail version leaves only `v0` / `f0` as results: every other register the
/// nested calls clobbered is put back (retail's leftover scratch values are not results).
pub fn preserved(c: &mut Cpu, keep_v0: bool, keep_f0: bool, f: fn(&mut Cpu) -> bool) -> bool {
    let (r, fr) = (c.r, c.f);
    let ok = f(c);
    let (v0, f0) = (c.r[2], c.f[0]);
    for i in 1..26 {
        c.r[i] = r[i];
    }
    for i in 0..20 {
        c.f[i] = fr[i];
    }
    if keep_v0 {
        c.r[2] = v0;
    }
    if keep_f0 {
        c.f[0] = f0;
    }
    ok
}
