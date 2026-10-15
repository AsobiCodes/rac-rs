//! Nine-way fan-out over a grid of sub-cells (retail 0x1ed808): calls the retail subdivision routine 0x1ec948 nine
//! times with the offsets of the 3x3 neighbourhood, passing the flag word (sign byte of t0, low bits 4) each time.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Retail subdivision routine called for every cell (a0..a3 = cell coordinates, t0 = flag word).
const SUBDIVIDE: u32 = 0x1ec948;
/// Mask for the top byte of the caller's flag word, and the low flag bits OR-ed into it for the first call.
const FLAG_TOP_MASK_64: u64 = 0xffff_ffff_ff00_0000;
const FLAG_BITS: u32 = 4;
/// Stack frame size.
const FRAME: u32 = 0xb0;
/// Callee-saved GPR slots: (register, offset, width). Retail stores s0..s7, fp and ra as 128-bit quadwords.
const SAVES: [(usize, u32, u8); 10] = [
    (16, 0x10, h::SQ),
    (17, 0x20, h::SQ),
    (18, 0x30, h::SQ),
    (19, 0x40, h::SQ),
    (20, 0x50, h::SQ),
    (21, 0x60, h::SQ),
    (22, 0x70, h::SQ),
    (23, 0x80, h::SQ),
    (30, 0x90, h::SQ),
    (31, 0xa0, h::SQ),
];

/// Calls the retail subdivision routine for one cell; the callee's scratch registers stay as a retail jal leaves them.
fn subdivide(c: &mut Cpu, args: [u32; 4], flags: u64) {
    set_args(c, args, flags);
    h::call_keep(c, SUBDIVIDE, &[], &[]);
}

/// Loads a0..a3 and t0 with their 64-bit values: 32-bit results are sign-extended, as the MIPS 32-bit ops leave them.
fn set_args(c: &mut Cpu, args: [u32; 4], flags: u64) {
    for (i, a) in args.iter().enumerate() {
        c.s(4 + i, *a as i32 as i64 as u64);
    }
    c.s(8, flags);
}

/// 0x1ed808 grid_fanout(x a0, y a1, z a2, w a3, flags t0): subdivides the nine cells around (a0..a3) in retail's
/// order, using the same flag word for each; the stack slots hold the neighbour coordinates between the calls.
pub fn grid_fanout(c: &mut Cpu) -> bool {
    let (s0, s5, s2, s6) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3));
    // retail: t0 = (s1 & lui(0xff00)) | 4 in 64 bits, then the same flag word (full register) for the other calls.
    let s1 = c.g(8);
    h::frame(c, FRAME, &SAVES, |c, sp| {
        let flags = (s1 & FLAG_TOP_MASK_64) | FLAG_BITS as u64;
        subdivide(c, [s0, s5, s2, s6], flags);

        let (n0, n2) = (s0.wrapping_add(1), s2.wrapping_add(3));
        let n6 = s6.wrapping_add(5);
        c.w32(sp, n0);
        c.w32(sp + 4, n2);
        c.w32(sp + 8, n6);
        let fp = s6.wrapping_sub(3);
        subdivide(c, [s0.wrapping_sub(1), n0, n2, n6], s1);

        let s7 = s5.wrapping_add(3);
        let s4 = s0.wrapping_sub(5);
        let s3 = s2.wrapping_sub(1);
        subdivide(c, [s0.wrapping_sub(3), s4, s3, fp], s1);
        let s0 = s0.wrapping_add(3);

        let n5m3 = s5.wrapping_sub(3);
        c.w32(sp + 0xc, n5m3);
        subdivide(c, [s4, n5m3, s3, s2.wrapping_add(1)], s1);
        let s3 = s5.wrapping_add(1);

        let s4 = s2.wrapping_sub(3);
        subdivide(c, [s0, s3, s4, s2.wrapping_sub(5)], s1);
        let s2 = s6.wrapping_add(1);

        subdivide(c, [s5.wrapping_sub(1), s3, s4, fp], s1);
        subdivide(c, [s7, s5.wrapping_add(5), n2, s2], s1);
        subdivide(c, [s0, s7, s6.wrapping_sub(1), s2], s1);

        // Last cell: retail leaves its scratch registers as the final jal returns them.
        let a0 = c.r32(sp);
        let a1 = c.r32(sp + 0xc);
        let a3 = c.r32(sp + 8);
        set_args(c, [a0, a1, s6.wrapping_add(3), a3], s1);
        h::call_keep(c, SUBDIVIDE, &[], &[]);
    });
    true
}
