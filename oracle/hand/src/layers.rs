//! Euler rotation helpers and the spring-driven layer rotation of attached mobys (0x254100..0x254f80).
use crate::actor;
use crate::h::*;
use crate::motion;
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Rounds x to n decimal places: (int)((x + 0.5 / 10^n) * 10^n) / 10^n.
pub fn round_decimals(n: i32, x: F) -> F {
    let mut pow = 1i32;
    for _ in 0..n.max(0) {
        pow = pow.wrapping_mul(10);
    }
    let scale = F::from_int(pow);
    let half = F::ONE / (scale + scale);
    let r = F::from_int(((x + half) * scale).to_int());
    r / scale
}

/// 0x254100 round_decimals(n, x) -> x rounded to n decimal places (nearest, via truncation of x + half a unit).
pub fn round_decimals_hand(c: &mut Cpu) -> bool {
    let r = round_decimals(c.arg(0) as i32, c.farg(0));
    c.fret(r);
    true
}

/// 0x254180 anim_blend_not_past(m, x) -> 1 when x <= the moby's blended animation value and the remaining
/// distance (value - x) rounded to 4 decimals is below the moby's step (+0x58 * +0x5c) rounded the same way.
pub fn anim_blend_not_past(c: &mut Cpu) -> bool {
    let (m, x) = (c.arg(0), c.farg(0));
    let value = run(c, actor::anim_blend_value, &[m], &[x]).1;
    let left = round_decimals(4, value - x);
    let step = c.rf(m + 0x58) * c.rf(m + 0x5c);
    let step = round_decimals(4, step);
    c.ret((x <= value && left < step) as u32);
    true
}

/// 0x2548a0 euler_rotate(obj, rx, ry, rz): rotates the euler angles at obj+0x40 by the rotation (rx, ry, rz).
pub fn euler_rotate(c: &mut Cpu) -> bool {
    let (obj, rx, ry, rz) = (c.arg(0), c.farg(0), c.farg(1), c.farg(2));
    let angles = obj + 0x40;
    with_stack(c, 0xd0, |c, buf| {
        let (delta, delta_m, cur_m) = (buf, buf + 0x10, buf + 0x50);
        c.wf(delta, rx);
        c.wf(delta + 4, ry);
        c.wf(delta + 8, rz);
        call(c, 0x1f2008, &[delta_m, delta], &[]);
        call(c, 0x1f2008, &[cur_m, angles], &[]);
        run(c, vecmath::mat4_mul, &[cur_m, cur_m, delta_m], &[]);
        call(c, 0x24fef0, &[cur_m, angles], &[]);
    });
    true
}

/// 0x254d58 layer_spring_update(owner, layer, parent, k, d): ensures the layer record belongs to `owner` (detaching
/// it from a previous owner), springs the layer's three euler angles (+0x40) toward the targets (+0x60) with
/// 0x24e858, attaches the layer to the owner's list when idle, rebuilds its matrix (0x24cb10) and resets the
/// targets. At rest (no target, scale 1, angles under 0.005) it detaches instead.
pub fn layer_spring_update(c: &mut Cpu) -> bool {
    let (owner, layer, parent) = (c.arg(0), c.arg(1), c.arg(2));
    let (k, d) = (c.farg(0), c.farg(1));
    let prev = c.r32(layer + 0x78);
    if prev != 0 && prev != owner {
        if c.r8(layer + 1) != 0 && c.r8(prev + 0x20) < 0x7f {
            run(c, actor::layer_detach, &[prev, layer], &[]);
        }
        c.w8(layer + 1, 0);
    }
    if prev != owner {
        c.w32(layer + 0x78, owner);
    }
    let targets_zero = (0..3).all(|i| c.rf(layer + 0x60 + 4 * i) == F::ZERO);
    let rest_angles = (0..3).all(|i| c.rf(layer + 0x40 + 4 * i).abs() < F(0x3ba3_d70a));
    if targets_zero && c.rf(layer + 0x70) == F::ONE && rest_angles {
        if c.r8(layer + 1) != 0 {
            run(c, actor::layer_detach, &[owner, layer], &[]);
        }
        return true;
    }
    let angles = layer + 0x40;
    for i in 0..3 {
        let target = c.rf(layer + 0x60 + 4 * i);
        run(
            c,
            motion::turn_spring_angle,
            &[angles + 4 * i, layer + 0x50 + 4 * i, 0],
            &[target, k, d, F::ZERO],
        );
    }
    if c.r8(layer + 1) == 0 {
        run(c, actor::layer_attach, &[owner, parent, layer], &[]);
    }
    call(c, 0x24cb10, &[layer + 0x10, angles], &[]);
    let scale = c.rf(layer + 0x70);
    c.wf(layer + 0x28, scale);
    c.wf(layer + 0x20, scale);
    c.wf(layer + 0x24, scale);
    c.wv(layer + 0x60, V([0; 4]));
    c.wf(layer + 0x70, F::ONE);
    true
}
