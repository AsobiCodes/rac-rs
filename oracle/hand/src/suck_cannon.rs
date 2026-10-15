//! Suck Cannon tube fade and per-object jitter setup.

use crate::draw_queue;
use crate::h;
use crate::mathx;
use crate::ps2::*;
use crate::rand;
use ee_oracle::cpu::Cpu;

/// Overlay halfword table: ten alpha values, the first one also gates the fade (lh at 0x1e0000 - 0x79e8).
const TUBE_ALPHA: u32 = 0x001d_8618;
const TUBE_STEPS: u32 = 10;
/// A value at or above this is clamped to `TUBE_RESET` before the fade step.
const TUBE_CAP: i32 = 0x41;
const TUBE_RESET: u16 = 0x40;
const TUBE_FADE: u16 = 4;
/// Draw-queue descriptor passed to draw_queue_add (lui 0x2e, addiu -0xa60).
const TUBE_QUEUE: u32 = 0x002d_f5a0;
const TUBE_DRAW_LAYER: u32 = 0x40;

/// Frame size of the seed function's retail prologue, and its object fields.
const SEED_FRAME: u32 = 0x50;
const JITTER_RANGE: u32 = 0xff;
const JITTER_COUNT: u32 = 4;
const OBJ_PTR: u32 = 0x78;
const JITTER_OFF: u32 = 0x10;
const INT_OFF: u32 = 0x30;
/// Scale argument of float_to_int_scaled for the first of the four entries.
const SCALE_FIRST: u32 = 0x3f;
const SCALE_STEP: u32 = 0x40;

/// Object field selecting the kind scale factor.
const KIND_FIELD: u32 = 0xa6;
const KIND_A: i32 = 0x5fb;
const KIND_B: i32 = 0x5fc;
const KIND_C: i32 = 0x44d;

const F_ONE: u32 = 0x3f80_0000;
const F_QUARTER: u32 = 0x3e80_0000;
const F_NEG_ONE: u32 = 0xbf80_0000;
const F_NEG_2_25: u32 = 0xc010_0000;
const F_1_25: u32 = 0x3fa0_0000;
const F_2_5: u32 = 0x4020_0000;
const F_3_0: u32 = 0x4040_0000;
const F_1_3: u32 = 0x3fa6_6666;
const F_NEG_2_55: u32 = 0xc023_3333;
const F_NEG_2_05: u32 = 0xc003_3333;

/// 0x2e0c98 suck_tube_fade_step(arg0) -> (): fades the ten tube alpha halfwords by 4 (clamped to 0x40 first),
/// then queues the tube draw. Does nothing but the frame when the first alpha is <= 0.
pub fn suck_tube_fade_step(c: &mut Cpu) -> bool {
    let arg0 = c.arg(0);
    h::frame(c, 16, &[(31, 0, h::SQ)], |c, _sp| {
        let first = c.r16(TUBE_ALPHA) as i16;
        if first <= 0 {
            c.ret(first as i32 as u32);
            return;
        }
        let mut last_v0 = 0u32;
        for i in 0..TUBE_STEPS {
            let addr = TUBE_ALPHA + 2 * i;
            let mut val = c.r16(addr);
            if i32::from(val as i16) >= TUBE_CAP {
                val = TUBE_RESET;
                c.w16(addr, val);
            }
            last_v0 = (val as i32 - TUBE_FADE as i32) as u32;
            c.w16(addr, val.wrapping_sub(TUBE_FADE));
        }
        c.ret(last_v0);
        // Retail never writes a3 here: its callee stores a3 (sw $a3 at 0x1ebc80), so keep the entry value.
        let a3 = c.r[7];
        h::run_keep(c, draw_queue::draw_queue_add, &[TUBE_QUEUE, arg0, TUBE_DRAW_LAYER], &[]);
        c.r[7] = a3;
    });
    true
}

/// 0x2e1090 kind_scale_factor(obj) -> f0: float scale picked by the s16 kind at obj+0xa6 (-2.55, 1.3 or -2.05).
pub fn kind_scale_factor(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let kind = c.r16(obj.wrapping_add(KIND_FIELD)) as i16 as i32;
    // The delay slot of the first beq (slti v0, v1, 0x5fc) leaves v0 = 1 on the KIND_A path.
    let (v0, bits) = if kind == KIND_A {
        (1, F_NEG_2_55)
    } else if kind >= KIND_B {
        (KIND_B, if kind == KIND_B { F_1_3 } else { F_NEG_2_05 })
    } else {
        (KIND_C, if kind == KIND_C { F_1_3 } else { F_NEG_2_05 })
    };
    // Retail sets $at (r1) = the float bits (lui $at), leaves a3 alone.
    c.s(1, bits as i32 as i64 as u64);
    c.ret(v0 as u32);
    c.fret(F(bits));
    true
}

/// 0x2e1548 seed_jitter_block(obj) -> (): fills obj+0x10..0x1c with four random floats (0..255), writes four fixed
/// floats at obj+0x20..0x2c, four float_to_int_scaled words at obj+0x30..0x3c and fixed floats at obj+0x40..0x4c.
pub fn seed_jitter_block(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    // Retail prologue: sq s2,0x20; s0,0; ra,0x40; s3,0x30; s1,0x10 (register numbers 18, 16, 31, 19, 17).
    let saves = [(18, 0x20, h::SQ), (16, 0x00, h::SQ), (31, 0x40, h::SQ), (19, 0x30, h::SQ), (17, 0x10, h::SQ)];
    h::frame(c, SEED_FRAME, &saves, |c, _sp| {
        let s1 = c.r32(obj.wrapping_add(OBJ_PTR));
        let mut dst = s1.wrapping_add(JITTER_OFF);
        for _ in 0..JITTER_COUNT {
            let (v0, _) = h::run_keep(c, rand::rand_below, &[JITTER_RANGE], &[]);
            let f = F::from_int(v0 as i32);
            c.f[0] = f.0;
            c.wf(dst, f);
            dst = dst.wrapping_add(4);
        }
        c.f[3] = F_NEG_ONE;
        c.f[2] = F_NEG_2_25;
        c.f[1] = F_1_25;
        c.f[0] = F_2_5;
        c.wf(s1.wrapping_add(0x20), F(F_NEG_ONE));
        c.wf(s1.wrapping_add(0x24), F(F_NEG_2_25));
        c.wf(s1.wrapping_add(0x28), F(F_1_25));
        c.wf(s1.wrapping_add(0x2c), F(F_2_5));
        let mut scale = SCALE_FIRST;
        let mut dst = s1.wrapping_add(INT_OFF);
        for _ in 0..JITTER_COUNT {
            let (v0, _) = h::run_keep(c, mathx::float_to_int_scaled, &[scale], &[]);
            c.w32(dst, v0);
            scale = scale.wrapping_add(SCALE_STEP);
            dst = dst.wrapping_add(4);
        }
        // float_to_int_scaled leaves f2 = 1.0 and f3 = 0.25 (its constants) in retail.
        c.f[2] = F_ONE;
        c.f[3] = F_QUARTER;
        c.f[0] = F_2_5;
        c.f[1] = F_3_0;
        c.wf(s1.wrapping_add(0x4c), F(F_2_5));
        c.wf(s1.wrapping_add(0x48), F(F_3_0));
        c.wf(s1.wrapping_add(0x40), F(F_2_5));
        c.wf(s1.wrapping_add(0x44), F(F_3_0));
    });
    true
}
