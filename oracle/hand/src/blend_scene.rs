//! Blend helper and scene flag flush (retail 0x1ef528 and 0x1eefe0).
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Scene flag word: when nonzero, the flush sequence runs.
const SCENE_FLAG: u32 = 0x15_f438;
/// gp-relative float (-0.04) written before the timer tick and cleared after.
const GP_TIMER: i32 = -0x78f8;
/// Timer value written before and after the tick, -0.04 as f32 bits.
const TIMER_STEP: u32 = 0xbd23_d70a;
/// Debug/log print routine.
const LOG_PRINT: u32 = 0x2913_c8;
/// Retail helper called before the tick.
const SCENE_PREP: u32 = 0x1f_0c70;
/// Retail helper called after the tick.
const SCENE_DONE: u32 = 0x1f_0624;
/// Retail per-frame tick (the 0x1eecd8 function).
const SCENE_TICK: u32 = 0x1e_ecd8;

/// 0x1ef528 blend_pack(out, a, b, flag): lerps two 0x1c-byte records by a ratio t picked from
/// a and b (word at flag>>1, sign-flipped when flag is odd); writes lerped quad and two floats,
/// and packs the lerped bytes at offset 0x18 into a word.
pub fn blend_pack(c: &mut Cpu) -> bool {
    // Not verified: every call differs in scratchpad (quad/packed-word lerp unresolved). Retail runs it.
    if true {
        let _ = c;
        return false;
    }
    let out = c.arg(0);
    let a = c.arg(1);
    let b = c.arg(2);
    let flag = c.arg(3);
    let idx = (flag >> 1) << 2;
    let sign = flag << 31;
    let ta = c.r32(a.wrapping_add(idx)) ^ sign;
    let tb = c.r32(b.wrapping_add(idx)) ^ sign;
    let a_c = c.rf(a.wrapping_add(0xc));
    let b_c = c.rf(b.wrapping_add(0xc));
    let fa = F(ta);
    let fb = F(tb);
    let diff = fa - fb;
    let denom = (b_c - a_c) + diff;
    let t = (fa - a_c) / denom;
    let ts = V::splat(t);

    // Quad of floats: out = a + (b - a) * t, lane-wise (VU multiply-add pairs).
    let va = c.rv(a);
    let vb = c.rv(b);
    let quad = V::msub(V::madd(va, vb, ts), va, ts);
    c.wv(out, quad);

    // Two scalar floats lerped with the same ratio.
    let a10 = c.rf(a.wrapping_add(0x10));
    let b10 = c.rf(b.wrapping_add(0x10));
    c.wf(out.wrapping_add(0x10), (b10 - a10) * t + a10);
    let a14 = c.rf(a.wrapping_add(0x14));
    let b14 = c.rf(b.wrapping_add(0x14));
    c.wf(out.wrapping_add(0x14), (b14 - a14) * t + a14);

    // Bytes 0 and 1 of the words at 0x18 are lerped as ints, then packed back to bytes.
    let wa = c.r32(a.wrapping_add(0x18));
    let wb = c.r32(b.wrapping_add(0x18));
    let ia = V(std::array::from_fn(|i| (wa >> (8 * i)) & 0xff)).itof(0);
    let ib = V(std::array::from_fn(|i| (wb >> (8 * i)) & 0xff)).itof(0);
    let lerp = V::msub(V::madd(ia, ib, ts), ia, ts).ftoi(4);
    // Round by +2 (1/16 units), shift down by 4, keep the low byte; the packed
    // halfwords land in bytes 2 and 3 of the word (bytes 0 and 1 are zero).
    let byte = |lane: u32| ((lane as u16).wrapping_add(2) >> 4) as u8;
    let packed = ((byte(lerp.0[0]) as u32) << 16) | ((byte(lerp.0[1]) as u32) << 24);
    c.w32(out.wrapping_add(0x18), packed);

    c.set_arg(3, sign);
    c.fret(t);
    true
}

/// 0x1eefe0 scene_flush(): when the scene flag is set, runs the tick sequence with the
/// timer set to -0.04 around it, then clears the timer.
pub fn scene_flush(c: &mut Cpu) -> bool {
    if c.r32(SCENE_FLAG) == 0 {
        c.ret(0);
        return true;
    }
    let gp = c.g(28) as u32;
    let timer = gp.wrapping_add(GP_TIMER as u32);
    h::call(c, LOG_PRINT, &[8, 5], &[]);
    h::call(c, LOG_PRINT, &[0x14, 0x61], &[]);
    h::call(c, LOG_PRINT, &[0x47, 0x0005_13f1], &[]);
    h::call(c, LOG_PRINT, &[0x4a, 1], &[]);
    h::call(c, SCENE_PREP, &[], &[]);
    c.fret(F(TIMER_STEP));
    c.w32(timer, TIMER_STEP);
    h::call(c, SCENE_TICK, &[], &[]);
    h::call(c, SCENE_DONE, &[], &[]);
    c.w32(timer, 0);
    c.set_arg(1, 0);
    c.set_arg(0, 0x4a);
    let (v0, f0) = h::call(c, LOG_PRINT, &[0x4a, 0], &[]);
    c.ret(v0);
    c.fret(f0);
    true
}
