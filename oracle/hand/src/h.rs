//! Shared plumbing for the hand-written functions: calling other hand Rust (or retail) with arguments, scratch
//! EE stack, and thin wrappers over the vector helpers.
use crate::mathx;
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;
use std::sync::atomic::Ordering;

/// Result of a nested call: v0 and f0.
pub type Ret = (u32, F);

/// RAM word of the current level index (the vblank callback compares it with the one the table was built for).
const LEVEL_WORD: usize = 0x15ed84;

/// Level index (Eudora) of the code the registered hands (the `EUDORA` table) were verified against.
const HAND_LEVEL: u32 = 4;

/// True when the recompiled/hand table is about to be rebuilt, so the whole Rust stack of the game thread is about to
/// be unwound (panic `RecRebuilt`). A rebuild happens (a) in the vblank callback when the level index (0x15ed84) or
/// the loader state (`rec::LOADER`) in RAM differs from the one the table was built for: the wait then unwinds; this is
/// the window from the write of the level / loader word to the next vblank; (b) after FlushCache (syscall 0x64, the
/// only site is the stub 0x118a80) when the code word at a registered function entry changed and the set of functions
/// in use changed: the overlay swap of a level change. Compiled retail frames lose nothing by an unwind (their state
/// is all in `Cpu` and on the EE stack), but a hand's Rust frame does: the code after its nested `c.call` / `h::call`
/// / `h::run` never runs, and the PC is left inside the callee with `ra` = the call-return sentinel. So a hand whose
/// retail body can reach FlushCache or the vblank wait (0x118cb0) through any callee, indirect calls included, must
/// decline first thing while this is true (`if crate::h::unwind_pending(c) { return false; }`, before it writes
/// anything): retail runs instead and unwinds losslessly. For (b) the stale word is not visible before the swap, but
/// the table is then built for another level than the code in RAM (the loader state 7 table of the new level is built
/// while the old overlay is still loaded), which `HAND_LEVEL` catches: the hands only match code of that level.
/// Always false without a table (fndiff, the interpreter alone).
///
/// ORACLE_HAND_NOGUARD=1 makes it always false, as if no hand carried the guard: the merge gate's negative control
/// (integrate.py `negctl`) runs the level-travel check that way and requires it to FAIL, so a gate that cannot see a
/// lost hand frame is caught. Never set it for play.
pub fn unwind_pending(c: &Cpu) -> bool {
    let Some(t) = c.rec.as_ref() else { return false };
    if guards_off() {
        return false;
    }
    let ld = u16::from_le_bytes([c.mem.ram[ee_oracle::rec::LOADER as usize], c.mem.ram[ee_oracle::rec::LOADER as usize + 1]]);
    t.level != c.mem.ram[LEVEL_WORD] as u32 || t.loader != ld || t.level != HAND_LEVEL
}

/// ORACLE_HAND_NOGUARD=1 (read once): the unwind guards never decline.
fn guards_off() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| std::env::var("ORACLE_HAND_NOGUARD").is_ok_and(|v| v == "1"))
}

/// Runs `body` as a nested call and puts every general/float register back afterwards (the result is returned
/// instead), so only the registers the hand function itself writes differ from the caller's.
fn nested(c: &mut Cpu, ints: &[u32], floats: &[F], body: impl FnOnce(&mut Cpu)) -> Ret {
    let (r, f) = (c.r, c.f);
    for (i, &a) in ints.iter().enumerate() {
        c.set_arg(i, a);
    }
    for (i, &x) in floats.iter().enumerate() {
        c.set_farg(i, x);
    }
    body(c);
    let ret = (c.result(), c.fresult());
    c.r = r;
    c.f = f;
    ret
}

/// Like `nested`, but puts back only what a retail callee preserves by the ABI: s0-s7, gp, sp, fp (all 128 bits),
/// ra (as before the call; `Cpu::call` sets it to CALL_RET) and f20-f31. Everything else stays as the callee left it
/// (v0/v1, at, a0-a3, t0-t9, k0/k1, f0-f19, HI/LO), which is what a retail `jal` leaves the caller.
fn nested_keep(c: &mut Cpu, ints: &[u32], floats: &[F], body: impl FnOnce(&mut Cpu)) -> Ret {
    let (r, f) = (c.r, c.f);
    for (i, &a) in ints.iter().enumerate() {
        c.set_arg(i, a);
    }
    for (i, &x) in floats.iter().enumerate() {
        c.set_farg(i, x);
    }
    body(c);
    for i in (16..24).chain(28..32) {
        c.r[i] = r[i];
    }
    for i in 20..32 {
        c.f[i] = f[i];
    }
    (c.result(), c.fresult())
}

/// Runs a hand function `f` with integer arguments `ints` and float arguments `floats`. When `f` declines (returns
/// false), the retail function it replaces runs instead (its entry from the `EUDORA` table, code matching RAM), so a
/// nested call never silently does nothing; a declining `f` with no such entry counts in `ps2::LOST_DECLINES`.
pub fn run(c: &mut Cpu, f: fn(&mut Cpu) -> bool, ints: &[u32], floats: &[F]) -> Ret {
    nested(c, ints, floats, |c| hand_or_retail(c, f))
}

/// Like `run`, but afterwards puts back only what a retail callee preserves by the ABI (s0-s7, gp, sp, fp, ra as before
/// the call, f20-f31); the scratch registers the callee leaves (v0/v1, at, a0-a3, t0-t9, k0/k1, f0-f19, HI/LO) stay as
/// it left them, as after a retail `jal`. Use it when retail code after the call reads a scratch register the callee
/// left, or when the caller must leave the callee's scratch values as retail does. Caveat: a callee hand's own soft
/// differences (registers retail's callee writes that the hand callee leaves alone) become differences of the caller,
/// so use it only where needed.
pub fn run_keep(c: &mut Cpu, f: fn(&mut Cpu) -> bool, ints: &[u32], floats: &[F]) -> Ret {
    nested_keep(c, ints, floats, |c| hand_or_retail(c, f))
}

/// Runs the hand function `f`; when it declines, its retail entry runs instead (none: `lost_decline`).
fn hand_or_retail(c: &mut Cpu, f: fn(&mut Cpu) -> bool) {
    if !f(c) {
        match retail_entry(c, f) {
            Some(entry) => c.call(entry),
            None => lost_decline(f),
        }
    }
}

/// The retail entry a hand function replaces: a `EUDORA` row with this function whose code matches RAM.
fn retail_entry(c: &Cpu, f: fn(&mut Cpu) -> bool) -> Option<u32> {
    crate::EUDORA
        .iter()
        .filter(|h| h.f as usize == f as usize)
        .find(|h| ee_oracle::rec::hash(&c.mem, h.entry, h.end) == h.hash)
        .map(|h| h.entry)
}

fn lost_decline(f: fn(&mut Cpu) -> bool) {
    if LOST_DECLINES.fetch_add(1, Ordering::Relaxed) == 0 {
        eprintln!("ee-hand: nested hand fn {:#x} declined and has no retail entry: the call did nothing", f as usize);
    }
}

/// Calls the retail function at `addr` with integer arguments `ints` and float arguments `floats`.
pub fn call(c: &mut Cpu, addr: u32, ints: &[u32], floats: &[F]) -> Ret {
    nested(c, ints, floats, |c| c.call(addr))
}

/// Like `call`, but keeps the callee's scratch registers as `run_keep` does (same use and caveat; a retail callee's
/// scratch values are exactly what a retail `jal` leaves).
pub fn call_keep(c: &mut Cpu, addr: u32, ints: &[u32], floats: &[F]) -> Ret {
    nested_keep(c, ints, floats, |c| c.call(addr))
}

/// Runs `f` with `size` bytes of EE stack reserved below sp (scratch base passed in).
pub fn with_stack<R>(c: &mut Cpu, size: u32, f: impl FnOnce(&mut Cpu, u32) -> R) -> R {
    let sp = c.g(29) as u32;
    let buf = sp - size;
    c.s(29, buf as i32 as i64 as u64);
    let r = f(c, buf);
    c.s(29, sp as i32 as i64 as u64);
    r
}

/// `frame` save width: `sd` (low 64 bits; `ld` puts them back, upper half kept).
pub const SD: u8 = 8;
/// `frame` save width: `sq` (all 128 bits; `lq` puts them back).
pub const SQ: u8 = 16;

/// Runs `body` inside a retail-style stack frame, for hands that call retail code (callbacks) which retail calls
/// with its own frame in place: `addiu sp, sp, -size`, each `(register, offset, SD | SQ)` stored at sp + offset,
/// `body(c, sp)`, then the registers reloaded from the frame and sp put back, as retail's epilogue does. Inside
/// `body` the saved registers may be set as retail sets them (s0 = a0, ...) before calling retail; `frame` restores
/// them. Example (prologue `addiu sp,-0x40; sq s0,0x20(sp); sq ra,0x30(sp)`):
/// `h::frame(c, 0x40, &[(16, 0x20, h::SQ), (31, 0x30, h::SQ)], |c, sp| { ... })`.
pub fn frame<R>(c: &mut Cpu, size: u32, saves: &[(usize, u32, u8)], body: impl FnOnce(&mut Cpu, u32) -> R) -> R {
    let sp0 = c.g(29) as u32;
    let sp = sp0.wrapping_sub(size);
    c.s(29, sp as i32 as i64 as u64);
    for &(r, off, w) in saves {
        let a = sp.wrapping_add(off);
        match w {
            SQ => c.mem.wr128(a, c.r[r]),
            SD => c.mem.wr64(a, c.r[r] as u64),
            _ => panic!("h::frame: width {w} (use h::SD or h::SQ)"),
        }
    }
    let out = body(c, sp);
    for &(r, off, w) in saves {
        let a = sp.wrapping_add(off);
        if r == 0 {
            continue;
        }
        if w == SQ {
            c.r[r] = c.mem.rd128(a);
        } else {
            let v = c.mem.rd64(a);
            c.s(r, v);
        }
    }
    c.s(29, sp.wrapping_add(size) as i32 as i64 as u64);
    out
}

/// 0x1f14b8: (int)(n * scale + 0.5).
pub fn scale_int(c: &mut Cpu, n: i32) -> i32 {
    run(c, mathx::float_to_int_scaled, &[n as u32], &[]).0 as i32
}

/// 0x1f17b0 fabs.
pub fn fabs(x: F) -> F {
    x.abs()
}

/// 0x1f1840 vec_sub(out, a, b).
pub fn vec_sub(c: &mut Cpu, out: u32, a: u32, b: u32) {
    run(c, vecmath::vec_sub, &[out, a, b], &[]);
}

/// 0x1f1a98 vec_set_length(out, v, len).
pub fn vec_set_length(c: &mut Cpu, out: u32, v: u32, len: F) {
    run(c, vecmath::vec_set_length, &[out, v], &[len]);
}

/// 0x1f1898 vec_scale(out, v, s).
pub fn vec_scale(c: &mut Cpu, out: u32, v: u32, s: F) {
    run(c, vecmath::vec_scale, &[out, v], &[s]);
}

/// 0x1f1810 vec_add(out, a, b).
pub fn vec_add(c: &mut Cpu, out: u32, a: u32, b: u32) {
    run(c, vecmath::vec_add, &[out, a, b], &[]);
}

/// 0x1f1930 dot3(a, b).
pub fn dot3(c: &mut Cpu, a: u32, b: u32) -> F {
    run(c, vecmath::dot3, &[a, b], &[]).1
}

/// 0x1f1970 length3(v).
pub fn length3(c: &mut Cpu, v: u32) -> F {
    run(c, vecmath::length3, &[v], &[]).1
}

/// 0x1f19e8 dist3(a, b).
pub fn dist3(c: &mut Cpu, a: u32, b: u32) -> F {
    run(c, vecmath::dist3, &[a, b], &[]).1
}

/// 0x1f1a20 dist_xy(a, b).
pub fn dist_xy(c: &mut Cpu, a: u32, b: u32) -> F {
    run(c, vecmath::dist_xy, &[a, b], &[]).1
}

/// 0x1f2400 quat_mul(out, a, b).
pub fn quat_mul(c: &mut Cpu, out: u32, a: u32, b: u32) {
    run(c, vecmath::quat_mul, &[out, a, b], &[]);
}

/// Copies a quadword (lq/sq).
pub fn copy_q(c: &mut Cpu, dst: u32, src: u32) {
    let v = c.rv(src);
    c.wv(dst, v);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ee_oracle::mem::Mem;

    #[test]
    fn frame_saves_and_restores() {
        let mut c = Cpu::new(Mem::new());
        let sp0 = 0x0010_0000u32;
        c.s(29, sp0 as i32 as i64 as u64);
        let s0 = 0x1111_2222_3333_4444_5555_6666_7777_8888u128;
        let ra = 0xaaaa_bbbb_cccc_dddd_0000_0000_0012_3456u128;
        c.r[16] = s0;
        c.r[31] = ra;
        let got = frame(&mut c, 0x40, &[(16, 0x20, SQ), (31, 0x30, SD)], |c, sp| {
            assert_eq!(sp, sp0 - 0x40);
            assert_eq!(c.g(29) as u32, sp0 - 0x40);
            assert_eq!(c.mem.rd128(sp + 0x20), s0);
            assert_eq!(c.mem.rd64(sp + 0x30), ra as u64);
            assert_eq!(c.mem.rd64(sp + 0x38), 0, "sd stores 8 bytes only");
            c.r[16] = 7;
            c.r[31] = 0xffff_0000_0000_0000_0000_0000_0000_0010;
            42
        });
        assert_eq!(got, 42);
        assert_eq!(c.r[16], s0, "sq/lq: all 128 bits back");
        // ld: low 64 bits back, the upper half as the body left it.
        assert_eq!(c.r[31], 0xffff_0000_0000_0000_0000_0000_0012_3456);
        assert_eq!(c.g(29) as u32, sp0);
    }
}
