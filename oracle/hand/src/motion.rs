//! Scalar motion helpers.
use crate::ps2::*;
use crate::vecmath::wrap_once;
use ee_oracle::cpu::Cpu;

/// 0x1f17b0 fabs(x) -> |x|.
pub fn fabs(c: &mut Cpu) -> bool {
    let x = c.farg(0);
    c.fret(x.abs());
    true
}

/// Spring velocity step: v += k * err - d * v, clamped to [-clamp, clamp] when clamp > 0, then to [-|err|, |err|].
/// Returns the last bound retail leaves in f0.
pub fn spring_vel(c: &mut Cpu, v: u32, err: F, k: F, d: F, clamp: F) -> F {
    let mut x = c.rf(v) + (k * err - d * c.rf(v));
    if F::ZERO < clamp {
        if clamp < x {
            x = clamp;
        } else if x < -clamp {
            x = -clamp;
        }
    }
    c.wf(v, x);
    let limit = err.abs();
    if limit < x {
        c.wf(v, limit);
        return limit;
    }
    if x < -limit {
        c.wf(v, -limit);
    }
    -limit
}

/// 0x24e358 spring_vel_step(vel*, err, k, d, clamp): see spring_vel.
pub fn spring_vel_step(c: &mut Cpu) -> bool {
    let v = c.arg(0);
    let (err, k, d, clamp) = (c.farg(0), c.farg(1), c.farg(2), c.farg(3));
    let f0 = spring_vel(c, v, err, k, d, clamp);
    c.fret(f0);
    true
}

/// v += clamp(target - v, -step, step); returns |target - v|.
pub fn slew(c: &mut Cpu, v: u32, target: F, step: F) -> F {
    let d = target - c.rf(v);
    let d = if step < d {
        step
    } else if d < -step {
        -step
    } else {
        d
    };
    let x = c.rf(v) + d;
    c.wf(v, x);
    (target - x).abs()
}

/// 0x24e428 slew_toward(v*, target, step) -> |target - v|: see slew.
pub fn slew_toward(c: &mut Cpu) -> bool {
    let v = c.arg(0);
    let r = slew(c, v, c.farg(0), c.farg(1));
    c.fret(r);
    true
}

/// Spring of value v (velocity vel) toward target; settles exactly on the target (velocity 0) once within 1 % of
/// max. Returns the remaining target - v (0 when settled).
pub fn spring_settle(c: &mut Cpu, v: u32, vel: u32, target: F, k: F, d: F, max: F) -> F {
    let err = target - c.rf(v);
    spring_vel(c, vel, err, k, d, max);
    let x = c.rf(v) + c.rf(vel);
    let rest = target - x;
    c.wf(v, x);
    if rest.abs() < max * F(0x3c23_d70a) {
        c.wf(v, target);
        c.w32(vel, 0);
        return c.rf(vel);
    }
    rest
}

/// 0x24e480 spring_settle(v*, vel*, target, k, d, max) -> remaining distance: see spring_settle.
pub fn spring_settle_hand(c: &mut Cpu) -> bool {
    let (v, vel) = (c.arg(0), c.arg(1));
    let r = spring_settle(c, v, vel, c.farg(0), c.farg(1), c.farg(2), c.farg(3));
    c.fret(r);
    true
}

/// Turn of a - b (wrapped) in direction `dir` (sign; 0 = shortest): a turn against the direction goes the long way
/// round (2 pi off), except a near-zero one which becomes 0.
pub fn angle_diff_dir(dir: i32, a: F, b: F) -> F {
    const TWO_PI: F = F(0x40c9_0fdb);
    let diff = wrap_once(a - b);
    if dir == 0 || F::ZERO < diff * F::from_int(dir) {
        diff
    } else if diff.abs() <= F(0x3ae4_c388) {
        F::ZERO
    } else if F::ZERO < diff {
        diff - TWO_PI
    } else {
        diff + TWO_PI
    }
}

/// 0x24e6f8 angle_diff_dir(dir, a, b) -> turn: see angle_diff_dir.
pub fn angle_diff_dir_hand(c: &mut Cpu) -> bool {
    let r = angle_diff_dir(c.arg(0) as i32, c.farg(0), c.farg(1));
    c.fret(r);
    true
}

/// Steps angle a toward target by at most step (turning in direction dir, see angle_diff_dir); returns the turn
/// left.
pub fn angle_step_toward(c: &mut Cpu, a: u32, dir: i32, target: F, step: F) -> F {
    let d = angle_diff_dir(dir, target, c.rf(a));
    let d = if step < d {
        step
    } else if d < -step {
        -step
    } else {
        d
    };
    let x = wrap_once(c.rf(a) + d);
    c.wf(a, x);
    angle_diff_dir(dir, target, x)
}

/// 0x24e7c0 angle_step_toward(a*, dir, target, step) -> turn left: see angle_step_toward.
pub fn angle_step_toward_hand(c: &mut Cpu) -> bool {
    let (a, dir) = (c.arg(0), c.arg(1) as i32);
    let r = angle_step_toward(c, a, dir, c.farg(0), c.farg(1));
    c.fret(r);
    true
}

/// Angle spring of a (velocity vel) toward target in direction mode (-1/0/1; 2 = the way that does not cross 0
/// when target and a have opposite signs); settles on the target within 1 % of max. Returns the turn left.
pub fn turn_spring(c: &mut Cpu, a: u32, vel: u32, mode: i32, target: F, k: F, d: F, max: F) -> F {
    let dir = if mode != 2 {
        mode
    } else if F::ZERO < target && c.rf(a) < F::ZERO {
        1
    } else if target < F::ZERO && F::ZERO < c.rf(a) {
        -1
    } else {
        0
    };
    let err = angle_diff_dir(dir, target, c.rf(a));
    spring_vel(c, vel, err, k, d, max);
    let x = wrap_once(c.rf(a) + c.rf(vel));
    c.wf(a, x);
    let rest = angle_diff_dir(dir, target, x);
    if rest.abs() < max * F(0x3c23_d70a) {
        c.wf(a, target);
        c.w32(vel, 0);
        return c.rf(vel);
    }
    rest
}

/// 0x24e858 turn_spring_angle(a*, vel*, mode, target, k, d, max) -> turn left: see turn_spring.
pub fn turn_spring_angle(c: &mut Cpu) -> bool {
    let (a, vel, mode) = (c.arg(0), c.arg(1), c.arg(2) as i32);
    let r = turn_spring(c, a, vel, mode, c.farg(0), c.farg(1), c.farg(2), c.farg(3));
    c.fret(r);
    true
}

/// Velocity step of an angle spring: v += k * unit(err) - d * v where unit(err) = err / 0.157 clamped to [-1, 1];
/// v then limited to [-max, max] when max != 0 and to [-|err|, |err|]. Shared by the two angle springs below.
fn angle_spring_vel(c: &mut Cpu, v: u32, err: F, k: F, d: F, max: F) {
    let ratio = err / F(0x3e20_d97c);
    let unit = if F::ONE < ratio {
        F::ONE
    } else if ratio < -F::ONE {
        -F::ONE
    } else {
        ratio
    };
    let old = c.rf(v);
    let mut x = old + (k * unit - d * old);
    c.wf(v, x);
    if max != F::ZERO {
        if max < x {
            x = max;
            c.wf(v, x);
        } else if x < -max {
            c.wf(v, -max);
        }
    }
    let limit = err.abs();
    let x = c.rf(v);
    if limit < x {
        c.wf(v, limit);
    } else if x < -limit {
        c.wf(v, -limit);
    }
}

/// 0x24a9d8 angle_spring_to(vel*, cur, target, k, d, max) -> new angle: spring step of vel toward the shortest turn
/// from cur to target (see angle_spring_vel), then cur + vel wrapped once.
pub fn angle_spring_to(c: &mut Cpu) -> bool {
    let v = c.arg(0);
    let (cur, target, k, d, max) = (c.farg(0), c.farg(1), c.farg(2), c.farg(3), c.farg(4));
    let err = wrap_once(target - cur);
    angle_spring_vel(c, v, err, k, d, max);
    let r = wrap_once(cur + c.rf(v));
    c.fret(r);
    true
}

/// 0x24ab40 angle_spring_field(obj, vel*, target, k, d, max): angle_spring_to on the angle at obj+0x48, stored
/// back.
pub fn angle_spring_field(c: &mut Cpu) -> bool {
    let (obj, v) = (c.arg(0), c.arg(1));
    let (target, k, d, max) = (c.farg(0), c.farg(1), c.farg(2), c.farg(3));
    let cur = c.rf(obj + 0x48);
    let err = wrap_once(target - cur);
    angle_spring_vel(c, v, err, k, d, max);
    let r = wrap_once(cur + c.rf(v));
    c.wf(obj + 0x48, r);
    c.fret(r);
    true
}

/// 0x24e9c0 angle_brake_to(angle*, vel*, target, step, accel, vmax) -> remaining turn: moves the angle toward
/// target with velocity vel: decelerates (slew vel to 0 at accel, 1.1 accel when stopping short) once the
/// stopping distance vel^2 / (2 accel) covers the remaining turn, otherwise accelerates toward
/// min(sqrt(2 accel dist), vmax); an opposite or zero turn just slews vel to 0 by step accel.
pub fn angle_brake_to(c: &mut Cpu) -> bool {
    let (ang, vel) = (c.arg(0), c.arg(1));
    let (target, step, accel, vmax) = (c.farg(0), c.farg(1), c.farg(2), c.farg(3));
    let saved = crate::rcall::save(c);
    let err = wrap_once(target - c.rf(ang));
    let v = c.rf(vel);
    let advance;
    if F::ZERO <= v * err && err != F::ZERO {
        let brake = v * v / accel * F(0x3f00_0000);
        if err.abs() < brake {
            let total = err.abs() + c.rf(vel).abs();
            let rate = if brake < total { accel } else { accel * F(0x3f8c_cccd) };
            slew(c, vel, F::ZERO, rate);
        } else {
            let mut s = crate::rcall::sqrt_q(c, (accel + accel) * err);
            if vmax < s {
                s = vmax;
            }
            let goal = if err < F::ZERO { -s } else { s };
            angle_step_toward(c, vel, 0, goal, step);
        }
        advance = c.rf(vel).abs() < err.abs();
        if !advance {
            c.wf(ang, target);
            crate::rcall::restore(c, &saved);
            c.fret(err);
            return true;
        }
    } else {
        angle_step_toward(c, vel, 0, F::ZERO, accel);
    }
    let r = wrap_once(c.rf(vel) + c.rf(ang));
    c.wf(ang, r);
    let out = c.rf(vel);
    crate::rcall::restore(c, &saved);
    c.fret(out);
    true
}
