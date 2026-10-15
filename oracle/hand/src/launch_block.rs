//! Launch and copy helpers: `launch_init` sets up a launched block from an angle, `mat3_relay` copies a 3x3 matrix
//! through a stack temporary, and `copy_hero_tint` copies the hero's 8-byte tint word into an actor.
use crate::h;
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Retail callees of `launch_init` (kept as retail code so their scratch registers match).
const FAST_COS: u32 = 0x1f1d80;
const FAST_SIN: u32 = 0x1f1d98;
const ANGLE_ADD: u32 = 0x1f2680;
const FLOAT_TO_INT_SCALED: u32 = 0x1f14b8;
const ATTACHED_VALUE: u32 = 0x24eef8;
const SPAWN_INIT: u32 = 0x24a290;
const ANIM_START: u32 = 0x24a148;
/// Retail `jal` target for pi as a single: 0x40490fdb.
const PI: F = F(0x4049_0fdb);
const ONE: F = F(0x3f80_0000);
const ZERO: F = F(0);
/// Argument of the `float_to_int_scaled` call in `launch_init` (0x12c).
const LIFE_SCALE: u32 = 0x12c;
/// Bit set in the block's flags when the attached value is exactly 0.0.
const FLAG_ATTACHED_ZERO: u32 = 0x20;
/// Bit set in the block's flags to start a curve (spawn_init) instead of an anim.
const FLAG_SPAWN_CURVE: u32 = 0x2;
/// Retail global holding the hero pointer: `lui v0, 0x14` then `lw v1, 0x13d0(v0)`.
const HERO_PTR_HI: u32 = 0x14_0000;
const HERO_PTR_OFF: u32 = 0x13d0;
/// Offset of the 8-byte tint word in an actor (and in the hero).
const ACTOR_TINT: u32 = 0x38;
/// Offset of the attached-value flag word in an actor.
const ACTOR_ATTACH_FLAG: u32 = 0x94;
/// Offsets in the launched block.
const BLK_VEL_X: u32 = 0x00;
const BLK_VEL_Y: u32 = 0x04;
const BLK_SPEED_COPY: u32 = 0x08;
const BLK_ONE: u32 = 0x0c;
const BLK_RADIUS: u32 = 0x18;
const BLK_SPEED: u32 = 0x1c;
const BLK_FLAGS: u32 = 0x24;
const BLK_LIFE: u32 = 0x2c;
const BLK_PAD: u32 = 0x3e;
const BLK_ACTIVE: u32 = 0x3c;
const BLK_HEADING: u32 = 0x40;
const BLK_TIMER: u32 = 0x44;

fn r64(c: &mut Cpu, a: u32) -> u64 {
    let lo = c.r32(a) as u64;
    let hi = c.r32(a + 4) as u64;
    lo | (hi << 32)
}

fn w64(c: &mut Cpu, a: u32, v: u64) {
    c.w32(a, v as u32);
    c.w32(a + 4, (v >> 32) as u32);
}

/// 0x24f118 launch_init(actor, blk, a2, a3, t0, angle): sets the block's velocity from `angle` (cos and sin times the
/// radius at +0x18, the speed at +0x1c), its heading, life and flags, then runs spawn_init (flag bit 1) or anim_start.
pub fn launch_init(c: &mut Cpu) -> bool {
    let (actor, blk) = (c.arg(0), c.arg(1));
    let (a2, a3, t0) = (c.arg(2), c.arg(3), c.arg(4));
    let angle = c.farg(0);
    let old_f20 = c.f[20];
    h::frame(
        c,
        0x70,
        &[
            (16, 0x00, h::SQ),
            (17, 0x10, h::SQ),
            (18, 0x20, h::SQ),
            (19, 0x30, h::SQ),
            (20, 0x40, h::SQ),
            (31, 0x50, h::SQ),
        ],
        |c, sp| {
            c.w32(sp + 0x60, old_f20);
            let (_, cos) = h::call_keep(c, FAST_COS, &[], &[angle]);
            let radius = c.rf(blk + BLK_RADIUS);
            c.wf(blk + BLK_VEL_X, cos * radius);
            let (_, sin) = h::call_keep(c, FAST_SIN, &[], &[angle]);
            let radius = c.rf(blk + BLK_RADIUS);
            let speed = c.rf(blk + BLK_SPEED);
            c.wf(blk + BLK_SPEED_COPY, speed);
            c.wf(blk + BLK_ONE, ONE);
            c.wf(blk + BLK_VEL_Y, sin * radius);
            let (_, heading) = h::call_keep(c, ANGLE_ADD, &[], &[angle, PI]);
            c.w32(blk + BLK_TIMER, 0);
            c.wf(blk + BLK_HEADING, heading);
            let (life, _) = h::call_keep(c, FLOAT_TO_INT_SCALED, &[LIFE_SCALE], &[]);
            c.w16(blk + BLK_PAD, 0);
            c.w16(blk + BLK_LIFE, life as u16);
            c.s(3, 1);
            c.w8(blk + BLK_ACTIVE, 1);
            let (attached, _) = h::call_keep(c, ATTACHED_VALUE, &[actor], &[]);
            if attached != 0 && c.rf(attached) == ZERO {
                let flags = c.r32(blk + BLK_FLAGS) | FLAG_ATTACHED_ZERO;
                c.w32(blk + BLK_FLAGS, flags);
                c.w32(actor + ACTOR_ATTACH_FLAG, 0);
            }
            let flags = c.r32(blk + BLK_FLAGS);
            if flags & FLAG_SPAWN_CURVE != 0 {
                h::call_keep(c, SPAWN_INIT, &[actor, a2, t0, a3, 1], &[]);
            } else {
                h::call_keep(c, ANIM_START, &[actor, a2, t0, a3], &[]);
            }
        },
    );
    c.f[20] = old_f20;
    true
}

/// 0x24fd28 mat3_relay(a, b): copies the matrix at `b` into a stack temporary (mat3_copy_w1), runs the retail helper
/// 0x24fab0(a, temp), then copies the temporary back to `b` (mat3_copy).
pub fn mat3_relay(c: &mut Cpu) -> bool {
    let (a, b) = (c.arg(0), c.arg(1));
    h::frame(
        c,
        0x70,
        &[(16, 0x40, h::SQ), (17, 0x50, h::SQ), (31, 0x60, h::SQ)],
        |c, sp| {
            h::run_keep(c, vecmath::mat3_copy_w1, &[sp, b], &[]);
            h::call_keep(c, 0x24fab0, &[a, sp], &[]);
            h::run_keep(c, vecmath::mat3_copy, &[b, sp], &[]);
        },
    );
    true
}

/// 0x24fd78 copy_hero_tint(actor): copies the 8 bytes at +0x38 of the hero (pointer at 0x1413d0) into actor +0x38.
pub fn copy_hero_tint(c: &mut Cpu) -> bool {
    let actor = c.arg(0);
    c.s(2, HERO_PTR_HI as u64);
    let hero = c.r32(HERO_PTR_HI + HERO_PTR_OFF) as i32 as i64 as u64;
    c.s(3, hero);
    let tint = r64(c, (hero as u32).wrapping_add(ACTOR_TINT));
    c.s(5, tint);
    w64(c, actor + ACTOR_TINT, tint);
    true
}
