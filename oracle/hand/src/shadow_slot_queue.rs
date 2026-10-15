//! Ground-shadow slot queue writer and the angle fold helper used with it.
use crate::h;
use crate::motion;
use crate::sweep;
use crate::collision;
use crate::vecmath;
use ee_oracle::cpu::Cpu;
use crate::ps2::*;

/// Retail address of the shadow queue count (a 32-bit word in RAM).
const QUEUE_COUNT: u32 = 0x15_F434;
/// Base of the shadow slot table; each slot is 0x20 bytes.
const SLOT_TABLE: u32 = 0x16_E180;
/// Slots are kept below this count.
const SLOT_LIMIT: u32 = 0x20;
/// Stride of one shadow slot.
const SLOT_SIZE: u32 = 0x20;
/// Quadword template copied into the slot's second half.
const SLOT_TEMPLATE: u32 = 0x17_3F80;
/// Offset of the owner's position block inside the actor.
const ACTOR_POS: u32 = 0x10;
/// Offset of the owner's height float inside the actor.
const ACTOR_HEIGHT: u32 = 0x18;
/// Offset of the owner's shadow-pending flag byte.
const ACTOR_SHADOW_FLAG: u32 = 0x31;
/// Ground probe origin (0.5f).
const PROBE_HEIGHT: F = F(0x3f00_0000);
/// Lift added above the ground (0.1f).
const LIFT: F = F(0x3ccc_cccd);
/// Shadow scale bounds: 8.0, 0.125, 0.25.
const SCALE_TOP: F = F(0x4100_0000);
const SCALE_RATE: F = F(0x3e00_0000);
const SCALE_FLOOR: F = F(0x3e80_0000);

/// Sets the low 64 bits of GPR `n` (a 32-bit result sign-extends; the upper half is left alone).
fn set_lo(c: &mut Cpu, n: usize, v: u32) {
    c.r[n] = (c.r[n] >> 64 << 64) | v as u128;
}

/// 0x24cba8 shadow_queue_push(actor, scale) -> (): when the actor wants a shadow and the queue has room, copies the
/// ground position into the next shadow slot and sets its alpha from the height above the ground.
pub fn shadow_queue_push(c: &mut Cpu) -> bool {
    let actor = c.arg(0);
    let scale = c.farg(0);
    if c.r8(actor + ACTOR_SHADOW_FLAG) == 0 {
        set_lo(c, 2, 0);
        return true;
    }
    let count = c.r32(QUEUE_COUNT);
    if count as i32 >= SLOT_LIMIT as i32 {
        set_lo(c, 2, 0);
        set_lo(c, 3, count);
        return true;
    }
    let slot = SLOT_TABLE + count * SLOT_SIZE;
    let pos = c.rv(actor + ACTOR_POS);
    c.wv(slot, pos);

    // Probe the ground under the actor; retail's scratch registers after each callee stay as the callee left them.
    let (_, ground_y) = h::run_keep(c, sweep::ground_height, &[actor + ACTOR_POS, 0], &[PROBE_HEIGHT]);
    let (a0, a1) = (c.arg(0), c.arg(1));
    let (hit, _) = h::run_keep(c, collision::hit_surface_class, &[a0, a1], &[]);
    if hit == 0 {
        return true;
    }
    c.wf(slot + 8, ground_y + LIFT);
    let tmpl = c.rv(SLOT_TEMPLATE);
    c.wv(slot + 0x10, tmpl);

    // Scale falls off with the height of the actor above the ground.
    let height = c.rf(actor + ACTOR_HEIGHT) - ground_y;
    let (_, abs_h) = h::run(c, motion::fabs, &[], &[height]);
    let top_gap = SCALE_TOP - abs_h;
    let mut alpha = top_gap * SCALE_RATE;
    if alpha < SCALE_FLOOR {
        alpha = SCALE_FLOOR;
    }
    c.fret(scale * alpha);
    c.f[1] = top_gap.0;
    c.f[2] = SCALE_RATE.0;
    c.f[3] = SCALE_FLOOR.0;
    c.f[12] = height.0;
    set_lo(c, 1, 0x16_0000);
    set_lo(c, 2, count + 1);
    set_lo(c, 3, slot);
    set_lo(c, 4, SLOT_TEMPLATE);
    set_lo(c, 5, slot);
    let idx = c.r32(QUEUE_COUNT);
    c.w32(QUEUE_COUNT, idx.wrapping_add(1));
    c.wf(SLOT_TABLE + idx * SLOT_SIZE + 0xc, scale * alpha);
    true
}

/// 0x24e310 angle_fold(a, b, c) -> f: subtracts angle b from a, scales the difference by c and adds it back to a.
pub fn angle_fold(c: &mut Cpu) -> bool {
    let a = c.farg(0);
    let b = c.farg(1);
    let k = c.farg(2);
    let (_, diff) = h::run(c, vecmath::angle_sub, &[], &[b, a]);
    let (_, sum) = h::run(c, vecmath::angle_add, &[], &[a, diff * k]);
    c.fret(sum);
    true
}
