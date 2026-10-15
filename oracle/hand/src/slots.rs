//! 16-slot spinning-effect table at 0x16eb40 (32 bytes per slot, a slot is live while its halfword at +0x10 > 0).
use crate::mathx;
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

const SLOT_TABLE: u32 = 0x16_eb40;
const SLOT_SIZE: u32 = 0x20;
const SLOT_COUNT: u32 = 16;
/// Per-slot fields: life (i16 at +0x10), shade (i16 +0x12), param (i16 +0x14), angle (f32 +0x18), scale (f32 +0x1c).
const LIFE: u32 = 0x10;
const SHADE: u32 = 0x12;
const PARAM: u32 = 0x14;
const ANGLE: u32 = 0x18;
const SCALE: u32 = 0x1c;
/// Float multiplier applied to the angle step (rodata 0x15ed6c) and the spin rate it is scaled by.
const STEP_ADDR: u32 = 0x15_ed6c;
const SPIN_RATE: F = F(0x40a7_8d36);
/// Life at which the shade starts fading back down, and the shade step.
const FADE_LIFE: i16 = 13;
const SHADE_STEP: u16 = 5;
/// Retail's start-angle generator.
const START_ANGLE_FN: u32 = 0x24_a578;

/// 0x1f0f28 slot_spawn(pos, param, scale) -> slot index or -1: takes the first free slot, copies the position
/// qword, sets scale, life 24, param, and the start angle from the retail generator.
pub fn slot_spawn(c: &mut Cpu) -> bool {
    let (pos, param, scale) = (c.arg(0), c.arg(1), c.farg(0));
    let v = c.rv(pos);
    for i in 0..SLOT_COUNT {
        let slot = SLOT_TABLE + SLOT_SIZE * i;
        if c.r16(slot + LIFE) as i16 > 0 {
            continue;
        }
        c.wv(slot, v);
        c.wf(slot + SCALE, scale);
        c.w16(slot + LIFE, 0x18);
        c.w16(slot + PARAM, param as u16);
        c.set_arg(0, 0x18);
        c.set_arg(1, param);
        c.set_farg(0, scale);
        c.call(START_ANGLE_FN);
        let angle = c.fresult();
        c.wf(slot + ANGLE, angle);
        c.ret(i);
        return true;
    }
    c.ret(-1i32 as u32);
    true
}

/// 0x1f0fb0 slot_update(): every live slot turns its angle back by a fixed step, fades its shade (+5 while life >= 13,
/// -5 below), loses one life and is cleared when the life runs out.
pub fn slot_update(c: &mut Cpu) -> bool {
    for i in 0..SLOT_COUNT {
        let slot = SLOT_TABLE + SLOT_SIZE * i;
        if c.r16(slot + LIFE) as i16 <= 0 {
            continue;
        }
        let step = c.rf(STEP_ADDR) * SPIN_RATE;
        let old = c.rf(slot + ANGLE);
        c.set_farg(0, old);
        c.set_farg(1, step);
        vecmath::angle_sub(c);
        let angle = c.fresult();
        c.wf(slot + ANGLE, angle);
        c.set_arg(0, slot);
        c.set_arg(1, 0);
        c.set_arg(2, SLOT_SIZE);
        let life = c.r16(slot + LIFE) as i16;
        let shade = c.r16(slot + SHADE);
        let shade = if life < FADE_LIFE { shade.wrapping_sub(SHADE_STEP) } else { shade.wrapping_add(SHADE_STEP) };
        c.w16(slot + SHADE, shade);
        let life = c.r16(slot + LIFE).wrapping_sub(1);
        c.w16(slot + LIFE, life);
        if (life as i16) <= 0 {
            mathx::fill_words(c);
        }
    }
    true
}
