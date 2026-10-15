//! Ember spawn: allocates an ember particle (kind 0xf), fills its header, timing and colour fields from the arguments
//! and returns it. Returns 0 when the gate argument is zero or the allocation fails.
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Particle-allocator kind for embers (retail `li a0, 0xf`).
const EMBER_KIND: u32 = 0xf;
/// Particle allocator (0x259a38): returns the new particle or 0.
const ALLOC_PARTICLE: u32 = 0x259a38;
/// Generator draw (0x1160d8): its result becomes the ember's random byte at +8.
const RAND_DRAW: u32 = 0x1160d8;
/// Float 4.0 (bits 0x40800000), converted to int by `mathx::float_to_int`.
const FOUR: F = F(0x4080_0000);
/// Subtracted from the int conversion of 4.0 for the byte at +9 (retail `addiu v0, v0, -0x60`).
const BYTE9_BIAS: u32 = 0x60;
/// Type byte used when the type argument is `USE_DEFAULT` (0x48).
const DEFAULT_TYPE: u8 = 0x48;
/// Address of the pointer to the default life byte (retail `lui 0x1b` then `lw 0x21bc`).
const DEFAULT_LIFE_PTR: u32 = 0x001b_21bc;
/// Marker argument (-1) meaning "use the default" for the type and life arguments.
const USE_DEFAULT: u32 = u32::MAX;
/// Retail stack frame size (`addiu sp, sp, -0xb0`).
const FRAME: u32 = 0xb0;
/// Frame slots: s0..s7, fp, ra as retail saves them with `sq`.
const SAVES: [(usize, u32, u8); 10] = [
    (16, 0x00, crate::h::SQ),
    (17, 0x10, crate::h::SQ),
    (18, 0x20, crate::h::SQ),
    (19, 0x30, crate::h::SQ),
    (20, 0x40, crate::h::SQ),
    (21, 0x50, crate::h::SQ),
    (22, 0x60, crate::h::SQ),
    (23, 0x70, crate::h::SQ),
    (30, 0x80, crate::h::SQ),
    (31, 0x90, crate::h::SQ),
];
/// Frame slot where retail saves f20 (`swc1 $f20, 0xa0($sp)`).
const F20_SLOT: u32 = 0xa0;
/// Offset of the second header block inside the ember (retail `addiu v1, s0, 0x20`).
const BLOCK2: u32 = 0x20;

struct Args {
    pos: u32,
    rec: u32,
    a2: u32,
    a3: u32,
    gate: u32,
    tag: u32,
    life: u32,
    kind: u32,
    f20: F,
}

/// 0x25de10 spawn_ember15(pos, rec, a2, a3, gate, tag, life, kind, scale) -> ember or 0: allocates an ember, copies
/// the 16-byte position `pos` and the block `rec`, and sets the type, life, timing and colour fields.
pub fn spawn_ember15(c: &mut Cpu) -> bool {
    let a = Args {
        pos: c.arg(0),
        rec: c.arg(1),
        a2: c.arg(2),
        a3: c.arg(3),
        gate: c.arg(4),
        tag: c.arg(5),
        life: c.arg(6),
        kind: c.arg(7),
        f20: c.farg(0),
    };
    let ember = crate::h::frame(c, FRAME, &SAVES, |c, sp| spawn_body(c, &a, sp));
    c.ret(ember);
    true
}

fn spawn_body(c: &mut Cpu, a: &Args, sp: u32) -> u32 {
    c.wf(sp + F20_SLOT, a.f20);
    if a.gate == 0 {
        return 0;
    }
    let (ember, _) = crate::h::call_keep(c, ALLOC_PARTICLE, &[EMBER_KIND], &[]);
    if ember == 0 {
        return 0;
    }
    let pos = c.rv(a.pos);
    c.wv(ember + 0x10, pos);
    // mtc1 of 4.0 into f12 with `lui at, 0x4080` before the conversion call.
    c.s(1, FOUR.0 as u64);
    let (int4, _) = crate::h::run_keep(c, crate::mathx::float_to_int, &[], &[FOUR]);
    // retail converts in place (cvt.w.s f12, f12), so f12 keeps the integer bits.
    c.set_farg(0, F(int4));
    c.w8(ember + 9, int4.wrapping_sub(BYTE9_BIAS) as u8);
    c.w32(ember + 4, a.a2);
    let kind = if a.kind == USE_DEFAULT { DEFAULT_TYPE } else { a.kind as u8 };
    c.w8(ember + 3, kind);
    c.w8(ember + 1, 0);
    let (draw, _) = crate::h::call_keep(c, RAND_DRAW, &[], &[]);
    c.w8(ember + 8, draw as u8);
    c.wf(ember + 0xc, a.f20);
    if a.life == USE_DEFAULT {
        let ptr = c.r32(DEFAULT_LIFE_PTR);
        let life = c.r8(ptr);
        c.w8(ember + 2, life);
        c.set_arg(0, life as u32);
    } else {
        c.w8(ember + 2, a.life as u8);
    }
    let blk = ember + BLOCK2;
    c.w16(ember + 0xa, a.gate as u16);
    c.w16(blk + 0x18, a.gate as u16);
    c.wf(blk + 0x1c, a.f20);
    c.w32(blk + 0x10, a.a2);
    c.w32(blk + 0x14, a.a3);
    c.w16(blk + 0x1a, a.tag as u16);
    let rec = c.rv(a.rec);
    c.wv(blk, rec);
    c.s(3, blk as u64);
    ember
}
