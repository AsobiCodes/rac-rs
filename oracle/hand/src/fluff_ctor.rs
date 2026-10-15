//! Fluff particle constructor (Eudora overlay 0x25da30). Allocates a 0xd-byte fluff record, copies a 16-byte template
//! into it, fills the drift/jitter fields from the RNG and seeds the sub-record at +0x20.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Retail callees, called as the retail `jal` does so their scratch registers are left as retail leaves them.
const FLOAT_TO_INT: u32 = 0x1f_27e8;
const FLOAT_TO_INT_SCALED: u32 = 0x1f_14b8;
const RAND_BELOW: u32 = 0x24_a418;
const RAND_FLOAT: u32 = 0x24_a4b0;
const RAND_FLOAT_SIGNED: u32 = 0x24_a510;
/// Retail allocator that takes the record size in a0.
const ALLOC: u32 = 0x25_9a38;
/// Record size passed to the allocator (also the fluff type byte's source, 0x0d).
const FLUFF_SIZE: u32 = 0xd;
/// Byte at +0x09 is the float-to-int of 4.0 plus this bias.
const TYPE_BIAS: u32 = 0x40;
/// Byte at +0x03 is always written with this value.
const FLUFF_KIND: u8 = 0x48;
/// Pointer slot holding the address of the RNG byte source.
const RNG_SOURCE_PTR: u32 = 0x1b_21b4;
/// Float 4.0, converted to an int for the +0x09 byte.
const FOUR: F = F(0x4080_0000);
/// Integer scale passed to float_to_int_scaled.
const JITTER_SCALE: u32 = 10;
/// Mask keeping the low 24 bits of the caller's integer seed.
const SEED_MASK: u32 = 0x00ff_ffff;
/// Offsets inside the record.
const OFF_KIND: u32 = 0x03;
const OFF_RNG: u32 = 0x02;
const OFF_TYPE: u32 = 0x09;
const OFF_FLAG1: u32 = 0x01;
const OFF_FLAG8: u32 = 0x08;
const OFF_SIZE_INT: u32 = 0x04;
const OFF_TEMPLATE: u32 = 0x10;
const OFF_JITTER_X: u32 = 0x10;
const OFF_JITTER_Y: u32 = 0x14;
const OFF_JITTER_Z: u32 = 0x18;
const OFF_SHORT: u32 = 0x0a;
const OFF_FLOAT_C: u32 = 0x0c;
const OFF_SUB: u32 = 0x20;

/// 0x25da30 fluff_ctor(tmpl, amp, seed, f12..f16) -> record or 0: builds one jittered fluff particle record.
pub fn fluff_ctor(c: &mut Cpu) -> bool {
    let tmpl = c.arg(0);
    let amp_int = c.arg(1);
    let seed = c.arg(2);
    let amp = c.farg(0);
    let spread_a = c.farg(1);
    let spread_b = c.farg(2);
    let spread_c = c.farg(3);
    let fade = c.farg(4);
    let zero = F(0);

    let (rec, _) = h::call_keep(c, ALLOC, &[FLUFF_SIZE], &[]);
    if rec == 0 {
        c.ret(0);
        return true;
    }
    // Copy the 16-byte template into the record.
    let quad = c.rv(tmpl);
    c.wv(rec + OFF_TEMPLATE, quad);
    c.w32(rec + OFF_SIZE_INT, seed);
    let (k, _) = h::call_keep(c, FLOAT_TO_INT, &[], &[FOUR]);
    c.w8(rec + OFF_TYPE, (k + TYPE_BIAS) as u8);
    c.w8(rec + OFF_KIND, FLUFF_KIND);
    c.wf(rec + OFF_FLOAT_C, fade);
    c.w8(rec + OFF_FLAG1, 0);
    c.w8(rec + OFF_FLAG8, 0);
    let rng_src = c.r32(RNG_SOURCE_PTR);
    let rng = c.r8(rng_src);
    c.w8(rec + OFF_RNG, rng);

    // Three signed random jitters added onto the template's x, y, z.
    for off in [OFF_JITTER_X, OFF_JITTER_Y] {
        let (_, r) = h::call_keep(c, RAND_FLOAT_SIGNED, &[], &[zero, amp]);
        let v = c.rf(rec + off) + r;
        c.wf(rec + off, v);
    }
    let (_, r) = h::call_keep(c, RAND_FLOAT_SIGNED, &[], &[zero, amp]);
    let z = c.rf(rec + OFF_JITTER_Z) + r;
    c.wf(rec + OFF_JITTER_Z, z);
    let (short, _) = h::call_keep(c, FLOAT_TO_INT_SCALED, &[JITTER_SCALE], &[zero]);
    c.w16(rec + OFF_SHORT, short as u16);

    // Sub-record at +0x20: a random sign on the amplitude, then the drag draw.
    let sub = rec + OFF_SUB;
    c.w32(sub + 4, 0);
    let (pick, _) = h::call_keep(c, RAND_BELOW, &[2], &[]);
    let signed = if pick == 0 { amp_int.wrapping_neg() } else { amp_int };
    c.w32(sub + 8, signed);
    c.w32(sub + 0x14, seed & SEED_MASK);
    let (_, drag) = h::call_keep(c, RAND_FLOAT, &[], &[spread_a, spread_b]);
    c.wf(sub + 0x10, spread_c);
    c.wf(sub + 0x0c, drag);
    c.ret(rec);
    true
}
