//! Spawns a type-0x16 smoke object from the loft emitter: allocation, header bytes, timing fields and two copied quads.

use crate::h;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// 0x259a38 alloc_smoke(type): the new object's address, or 0 when no slot is free.
const ALLOC_SMOKE: u32 = 0x0025_9a38;
/// 0x1160d8 rand_rot(): returns a random byte that the caller stores as the object's rotation.
const RAND_ROT: u32 = 0x0011_60d8;
/// Object type passed to `alloc_smoke`.
const SMOKE_TYPE: u32 = 0x16;
/// Pointer to the random-table byte source, stored at 0x1b21d8.
const RAND_PTR_SLOT: u32 = 0x001b_21d8;
/// Float at 0x15ed70 (lui 0x16, offset -0x1290) that scales the lifetime.
const LIFE_SCALE_SRC: u32 = 0x0015_ed70;
/// Stack frame size of the retail function.
const FRAME: u32 = 0x90;
/// 4.0f: the argument of `float_to_int` that yields the object's random-byte table index.
const F_FOUR: u32 = 0x4080_0000;
/// 0.2f: multiplies the scale into the object's first timing field.
const F_0P2: u32 = 0x3e4c_cccd;
/// 1.24f: the lifetime multiplier applied to the scale source.
const F_1P24: u32 = 0x3f9e_b852;

/// Packs a VU quadword into the 128-bit EE register image (lane 0 in the low bits).
fn quad_bits(v: V) -> u128 {
    (v.0[0] as u128) | ((v.0[1] as u128) << 32) | ((v.0[2] as u128) << 64) | ((v.0[3] as u128) << 96)
}

/// 0x25f170 spawn_smoke_loft(quad_a, quad_b, a2, a3, t0, scale): allocates a smoke object (type 0x16) and fills it
/// from the arguments: quad_a to +0x10, quad_b to +0x20, a2/a3 and t0 as timing words, scale into the lifetime.
pub fn spawn_smoke_loft(c: &mut Cpu) -> bool {
    let quad_a = c.arg(0);
    let quad_b = c.arg(1);
    let a2 = c.arg(2);
    let a3 = c.arg(3);
    let t0 = c.arg(4);
    let scale = c.farg(0);
    let saves = [
        (22, 0x60, h::SQ),
        (21, 0x50, h::SQ),
        (20, 0x40, h::SQ),
        (19, 0x30, h::SQ),
        (18, 0x20, h::SQ),
        (17, 0x10, h::SQ),
        (16, 0x00, h::SQ),
        (31, 0x70, h::SQ),
    ];
    h::frame(c, FRAME, &saves, |c, sp| {
        c.w32(sp + 0x80, c.f[20]);
        let _ = h::call_keep(c, ALLOC_SMOKE, &[SMOKE_TYPE], &[]);
        let obj = c.result();
        if obj == 0 {
            return;
        }
        let s1 = obj;
        let s0 = obj + 0x20;

        c.w32(s1 + 4, a2);
        let _ = h::run_keep(c, mathx::float_to_int, &[], &[F(F_FOUR)]);
        // retail cvt.w.s f12, f12 leaves the integer 4 in f12
        c.f[12] = 4;
        let n = c.result().wrapping_add(0x70);
        let prod = scale * F(F_0P2);
        c.s(3, 0x44);
        c.w8(s1 + 1, 0);
        c.w8(s1 + 3, 0x44);
        c.w8(s1 + 9, n as u8);
        c.wf(s1 + 0xc, prod);
        c.ret(n);
        c.f[0] = prod.0;
        let _ = h::call_keep(c, RAND_ROT, &[], &[]);
        c.w8(s1 + 8, c.result() as u8);

        let ptr = c.r32(RAND_PTR_SLOT);
        c.ret(ptr);
        let byte = c.r8(ptr);
        c.w8(s1 + 2, byte);
        c.w16(s1 + 0xa, t0 as u16);
        c.w16(s0 + 0x18, t0 as u16);
        let _ = h::run_keep(c, mathx::int_to_float, &[t0], &[]);
        let inv = c.fresult();
        let span = (scale - c.rf(s1 + 0xc)) / inv;
        c.w32(s0 + 0x10, a2);
        c.w32(s0 + 0x14, a3);
        c.wf(s0 + 0x1c, span);

        let first = c.rv(quad_b);
        c.wv(s0, first);
        let second = c.rv(quad_a);
        c.wv(s1 + 0x10, second);
        c.r[2] = quad_bits(second);

        let life = c.rf(LIFE_SCALE_SRC) * F(F_1P24);
        c.wf(s0 + 0xc, life);
        c.f[0] = life.0;
        c.f[1] = F_1P24;
        c.s(1, F_1P24 as u64);
        c.s(3, (s1 + 0x10) as u64);
        c.s(5, byte as u64);
    });
    true
}
