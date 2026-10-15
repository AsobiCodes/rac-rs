//! Hero tilt helpers: magnet-boot tilt blocks, the tilt gain block, and the local normal angles.
use crate::h;
use crate::pose_spring;
use crate::ps2::*;
use crate::trig;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Hero object (0x140000 - 0xcb0): enable halfword, matrix at +0x40, mode word at +0x2084.
const HERO: u32 = 0x0013_F350;
/// Tilt block base (0x180000 - 0x5880). Field offsets below are relative to it.
const TILT: u32 = 0x0017_A780;
/// Size of the scratch frame the retail function reserves.
const FRAME: u32 = 0x40;

/// Clamps `t` to `[-limit, limit]`, testing the upper bound first as the retail branches do.
fn clamp_sym(t: F, limit: F) -> F {
    if limit < t {
        limit
    } else if t < -limit {
        -limit
    } else {
        t
    }
}

/// 0x208128 hero_magnet_tilt_blocks(): when the hero's enable halfword (+0x308) is set, transforms the up vector
/// (0,0,-1) by the hero matrix and writes clamped tilt gains for the magnet boots into the tilt block.
pub fn hero_magnet_tilt_blocks(c: &mut Cpu) -> bool {
    if c.r16(HERO + 0x308) == 0 {
        return true;
    }
    h::with_stack(c, FRAME, |c, fr| {
        c.w32(fr, 0);
        c.w32(fr + 4, 0);
        c.wf(fr + 8, F(0xbf80_0000));
        c.w32(fr + 12, 0);
        h::run(c, vecmath::vec4_mul_mat4, &[fr, fr, HERO + 0x40], &[]);

        let k = c.rf(0x0015_ED64);
        h::run(c, pose_spring::set_spring_pair_b, &[], &[k * F(0x3c54_fdf4), k * F(0x3e99_999a)]);

        let x = c.rf(fr);
        let y = c.rf(fr + 4);
        let z = c.rf(fr + 8);

        // Limit depends on the hero mode; a nonzero flag at +0x2fc caps it at 0.156.
        let mut limit = match c.r32(HERO + 0x2084) {
            0x3f => F(0x3e97_e9d8),
            0x70 => F(0x3db2_b8c2),
            0x71 | 4 => F(0x3e44_9809),
            _ => F(0x3edf_66f3),
        };
        if c.r32(HERO + 0x2fc) != 0 && F(0x3e20_d97c) < limit {
            limit = F(0x3e20_d97c);
        }

        let gain_x = clamp_sym(x * F(0x3f32_b8c2), limit);
        c.wf(TILT + 0x114, gain_x);
        c.wf(TILT + 0x274, -gain_x * F(0x3f00_0000));

        let gain_y = clamp_sym(-y * F(0x3f32_b8c2), limit);
        c.wf(TILT + 0x110, gain_y);
        c.wf(TILT + 0x270, -gain_y * F(0x3e80_0000));

        let mut side = -x * F(0x3f9c_61aa);
        if F(0x3f7e_adae) < side {
            side = F(0x3f7e_adae);
        }
        if side < F(0xbdfa_35dd) {
            side = F(0xbdfa_35dd);
        }
        if F(0x3f59_999a) < z {
            if side < F(0x3fb2_b8c2) {
                side = F(0x3fb2_b8c2);
            }
        } else if F(0x3f00_0000) < z && side < F(0x3f9c_61aa) {
            side = F(0x3f9c_61aa);
        }
        c.wf(TILT + 0x954, side * F(0x3f11_eb85));
        c.wf(TILT + 0xab4, side * F(0x3f00_0000));
        c.wf(TILT + 0xa04, side * F(0x3f00_0000));
        c.wf(TILT + 0xb64, side * F(0x3eb3_3333));

        let pitch = clamp_sym(y * F(0x3f9c_61aa), F(0x3f9c_61aa));
        c.wf(TILT + 0xb60, pitch);
        let pitch_scaled = pitch * F(0x3f33_3333);
        c.wf(TILT + 0xab0, pitch_scaled);
        c.wf(TILT + 0x950, pitch_scaled);
        c.wf(TILT + 0xa00, pitch_scaled);
    });
    true
}

/// 0x209038 hero_tilt_gain_blocks(): writes the tilt-gain blocks from the hero's speed-like field (+0x2c of the
/// object at +0x1184), with a 1024/x gain scale and clamped magnitude; only runs in hero mode 2.
pub fn hero_tilt_gain_blocks(c: &mut Cpu) -> bool {
    let obj = c.r32(HERO + 0x1184);
    if obj == 0 {
        return true;
    }
    if c.r8(0x0015_EDB3) != 0 {
        let gp = c.g(28) as u32;
        let v = c.rf(gp.wrapping_sub(0x7de8));
        c.wf(0x0017_B48C, v);
    }
    if c.r32(HERO + 0x2084) != 2 {
        return true;
    }
    let gain = F(0x4480_0000) / c.rf(obj + 0x2c);

    let mut t = c.rf(HERO + 0x188);
    c.wf(TILT + 0xd04, F(0x3ca3_d70a));
    c.wf(TILT + 0xd08, F(0x3eb3_3333));
    t = clamp_sym(t, F(0x3fb3_3333));

    let m = clamp_sym(-t * F(0x3f99_999a), F(0x3fa7_8d36));
    c.wf(TILT + 0xcc8, m);

    // Retail fabs takes f12, which still holds the clamped t here (m is only stored), so |t| is used.
    let mut k = t.abs() * F(0x3dcc_cccd) + F(0x3dcc_cccd);
    if F(0x3d8f_5c29) < k {
        k = F(0x3d8f_5c29);
    }
    c.wf(TILT + 0xcf8, k * gain);
    true
}

/// 0x2091b8 normal_local_angles(normal, out): rotates the vec4 at `normal` into the hero frame with the matrix
/// at 0x1413d0 and writes two angles to `out`: atan2 of the horizontal components, and its negation.
pub fn normal_local_angles(c: &mut Cpu) -> bool {
    let normal = c.arg(0);
    let out = c.arg(1);
    h::with_stack(c, 0xd0, |c, fr| {
        let v = c.rv(normal);
        c.wv(fr + 0x80, v);
        let mat = c.r32(0x0014_13D0) + 0x40;
        h::call(c, 0x001f_2008, &[fr, mat], &[]);
        h::run(c, vecmath::mat3_transpose, &[fr + 0x40, fr], &[]);
        h::run(c, vecmath::vec4_mul_mat4, &[fr + 0x80, fr + 0x80, fr + 0x40], &[]);
        let x = c.rf(fr + 0x80);
        let z = c.rf(fr + 0x88);
        // The retail code also computes atan2(y, hypot(x, z)) here, but its result is overwritten before use.
        let (_, angle) = h::run(c, trig::fast_atan2, &[], &[z, x]);
        c.wf(out, angle);
        c.wf(out + 4, -angle);
    });
    true
}
