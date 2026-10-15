//! Frustum sphere test (VU0 plane tests on a sphere centre) and the probe-advance loop that walks an object along its
//! depth axis until the sphere test clears.
use crate::h;
use crate::ps2::*;
use crate::trig;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Frustum plane constants in retail RAM (`lui 0x17` + `addiu -0x3240`): eight 16-byte rows at +0x00..+0x70.
const FRUSTUM_TABLE: u32 = 0x16_cdc0;
/// Reset position of the probe-advance loop (`lui 0x17` + `addiu 0x3f70`).
const PROBE_RESET: u32 = 0x17_3f70;
/// Sphere test helper of retail (the `jal 0x1e35c8` in the probe loop).
const SPHERE_HELPER: u32 = 0x1e_35c8;
/// Maximum probe steps along the depth axis.
const PROBE_STEPS: u32 = 6;
/// 0.25 (the VU scale of the probe radius).
const QUARTER: F = F(0x3e80_0000);
/// pi as used by the angle wrap.
const PI: F = F(0x4049_0fdb);
/// Scratch frame size of the probe loop.
const PROBE_FRAME: u32 = 0x90;

/// Packs a low and a high 32-bit lane into the 64-bit view that QMFC2 and the MIPS sign tests see.
fn lo_hi(lo: F, hi: F) -> u64 {
    lo.0 as u64 | (hi.0 as u64) << 32
}

/// Leaves the registers the retail exit path leaves behind and returns `ret` in v0.
fn finish(c: &mut Cpu, ret: i32, at: u64, v1: u64, a0: u64, a1: u64) -> bool {
    c.s(1, at);
    c.s(3, v1);
    c.s(4, a0);
    c.s(5, a1);
    c.ret(ret as u32);
    true
}

/// 0x1f2878 sphere_visible(sphere, bias) -> 1 inside, 0 straddling, -1 outside: VU0 test of a sphere centre against
/// the frustum planes (centre scaled by the w constant, depth from the f12 bias), decided by the sign bits.
pub fn sphere_visible(c: &mut Cpu) -> bool {
    let sphere = c.arg(0);
    let bias = c.farg(0);
    let mut v1 = c.g(3);
    let pos = c.rv(sphere);
    let m20 = c.rv(FRUSTUM_TABLE + 0x40);
    let m21 = c.rv(FRUSTUM_TABLE + 0x50);
    let m22 = c.rv(FRUSTUM_TABLE + 0x60);
    let m23 = c.rv(FRUSTUM_TABLE + 0x70);
    let m24 = c.rv(FRUSTUM_TABLE + 0x30);
    let m25 = c.rv(FRUSTUM_TABLE);
    let m26 = c.rv(FRUSTUM_TABLE + 0x10);
    let m27 = c.rv(FRUSTUM_TABLE + 0x20);

    let p = pos * m24.w();
    let vf2 = V::ZERO_W1.with(p - m24, X | Y | Z);
    let w = p.w();
    let vf3 = V::ZERO_W1
        .with(V::ZERO_W1 + V::splat(w), X)
        .with(V::ZERO_W1 - V::splat(w), Y);
    let vf4 = V([0, bias.0, 0, F::ONE.0]);
    let vf19 = V::ZERO_W1.with(V::ZERO_W1 + V::ZERO_W1, X).with(vf4 * m24.w(), Y);
    let vf8 = V::ZERO_W1.with(m22 * p.w(), X | Y);

    let acc = V::madd(
        V::madd(m25 * V::splat(vf2.x()), m26, V::splat(vf2.y())),
        m27,
        V::splat(vf2.z()),
    );
    let vf2 = vf2.with(acc, X | Y | Z);
    let vf1 = p.with(vf3 + V::splat(vf2.z()), X | Y);
    let vf5 = V::ZERO_W1.with(m20 * V::splat(vf2.z()), X | Y);
    let vf2 = vf2.with(V([vf2.x().abs().0, vf2.y().abs().0, 0, 0]), X | Y);
    let vf7 = V::ZERO_W1.with(vf8 * V::splat(m22.z()), X | Y);
    let vf3 = vf3.with(vf19 - vf1, X | Y);
    let vf9 = V::ZERO_W1.with(vf5 * m21, X | Y);
    let vf6 = V::ZERO_W1.with(vf2 - vf8, X | Y);
    let vf8 = vf8.with(vf2 + vf7, X | Y);
    let vf4 = vf4.with(vf1 - m23, X | Y);
    let vf7 = vf7.with(vf5 - vf6, X | Y);
    let vf8 = vf8.with(vf9 - vf8, X | Y);

    let at = lo_hi(vf3.x(), vf3.y());
    let a0 = lo_hi(vf4.x(), vf4.y());
    let a1: u64 = 0x4000_0000;
    let v0 = at << 32;
    if (at as i64) < 0 || (v0 as i64) >= 0 {
        return finish(c, -1, at, v1, a0, a1);
    }
    let a1 = a1.wrapping_add(a1);
    let v0 = lo_hi(vf7.x(), vf7.y());
    let mut v1 = lo_hi(vf8.x(), vf8.y());
    let a0 = a0 ^ a1;
    if (v0 as i64) < 0 {
        return finish(c, -1, at, v1, a0, a1);
    }
    let v0 = v0 << 32;
    v1 |= a0;
    if (v0 as i64) < 0 {
        return finish(c, -1, at, v1, a0, a1);
    }
    if (v1 as i64) < 0 {
        return finish(c, 0, at, v1 << 32, a0, a1);
    }
    let v1s = v1 << 32;
    if (v1s as i64) < 0 {
        return finish(c, 0, at, v1s, a0, a1);
    }
    finish(c, 1, at, v1s, a0, a1)
}

/// 0x1f35b0 probe_advance(obj, rec, params, step) -> void: adds the normalised rec offset to obj.pos, then steps
/// obj.pos back along z by `step` (at most six times) while the sphere helper 0x1e35c8 still reports a hit, and
/// stores obj.0x48 = atan2(rec.x, rec.y) + pi (wrapped).
pub fn probe_advance(c: &mut Cpu) -> bool {
    let (obj, rec, params) = (c.arg(0), c.arg(1), c.arg(2));
    let step = c.farg(0);
    let rec_pos = rec + 0x10;
    let obj_pos = obj + 0x10;
    h::vec_add(c, rec_pos, rec_pos, rec);
    let len = c.rf(params + 0x1c);
    h::vec_set_length(c, rec_pos, rec_pos, len);
    let ratio = c.rf(rec + 0x18) / c.rf(rec + 0x48);
    c.wf(rec + 0x18, ratio);
    h::vec_add(c, obj_pos, obj_pos, rec_pos);

    let buf = (c.g(29) as u32).wrapping_sub(PROBE_FRAME);
    let radius = c.rf(params + 0x10) * QUARTER;
    h::with_stack(c, PROBE_FRAME, |c, _| {
        for _ in 0..PROBE_STEPS {
            let pos = c.rv(obj_pos);
            c.wv(buf, pos);
            let z = pos.z() + step;
            c.wf(buf + 8, z);
            let (hit, _) = h::call(c, SPHERE_HELPER, &[buf, 4, obj], &[radius]);
            if hit == 0 {
                break;
            }
            let reset = c.rv(PROBE_RESET);
            c.wv(obj_pos, reset);
            let z = c.rf(obj + 0x18) - step;
            c.wf(obj + 0x18, z);
        }
    });

    let (x, y) = (c.rf(rec + 0x10), c.rf(rec + 0x14));
    let (_, angle) = h::run(c, trig::fast_atan2, &[], &[x, y]);
    let (_, sum) = h::run(c, vecmath::angle_add, &[], &[angle, PI]);
    c.wf(obj + 0x48, sum);
    c.s(1, PI.0 as u64);
    c.s(4, buf as i32 as i64 as u64);
    c.s(5, 4);
    c.s(6, obj as i32 as i64 as u64);
    c.set_farg(0, angle);
    c.set_farg(1, PI);
    c.fret(sum);
    c.ret(0);
    true
}
