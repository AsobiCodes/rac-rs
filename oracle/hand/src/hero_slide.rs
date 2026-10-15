//! Hero movement helpers from the L4 overlay: the heading vector, the ground probe and slide, and the push/pull of
//! the hero position against a limit vector.
use crate::collision;
use crate::geom;
use crate::h;
use crate::hero_space;
use crate::ps2::*;
use crate::scene;
use crate::trig;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Hero block (0x140000 - 0xcb0).
const HERO: u32 = 0x0013_f350;
/// Table block (0x140000 - 0xc30): the up vector and the limit vectors.
const TABLE: u32 = 0x0013_f3d0;
/// Heading vector output (0x140000 - 0xbd0).
const DIR_OUT: u32 = 0x0013_f430;
/// Reference direction vectors (0x140000 - 0xa20).
const REF: u32 = 0x0013_f5e0;
/// Zone record (0x170000 + 0x3f40).
const ZONE: u32 = 0x0017_3f40;
/// Zone record count word (ZONE + 0x1c).
const ZONE_COUNT: u32 = 0x0017_3f5c;
/// Float constant at 0x15ed6c: scale of the minimum probe length.
const MIN_LEN_ADDR: u32 = 0x0015_ed6c;
/// Retail matrix builder (0x1f2008).
const MAT_BUILD: u32 = 0x001f_2008;
/// Retail surface basis helper (0x125180).
const SURFACE_BASIS: u32 = 0x0012_5180;
/// Retail segment probe (0x1e24d8).
const PROBE: u32 = 0x001e_24d8;
/// Pi as a float, the wrap limit of the heading.
const PI: F = F(0x4049_0fdb);

fn copy16(c: &mut Cpu, dst: u32, src: u32) {
    let v = c.rv(src);
    c.wv(dst, v);
}

fn len_xy(c: &mut Cpu, v: u32) -> F {
    h::run(c, vecmath::length_xy, &[v], &[]).1
}

fn cos_of(c: &mut Cpu, x: F) -> F {
    h::run(c, trig::fast_cos, &[], &[x]).1
}

fn sin_of(c: &mut Cpu, x: F) -> F {
    h::run(c, trig::fast_sin, &[], &[x]).1
}

/// 0x205180 hero_move_dir_vec(heading) -> writes the heading vector to 0x13f430: (cos, sin) scaled by the speed
/// (or a spherical vector when the hero's flag at 0x20b3 is clear). The heading is clamped to pi.
pub fn hero_move_dir_vec(c: &mut Cpu) -> bool {
    let heading = c.farg(0);
    let yaw = if heading <= PI { heading } else { c.rf(HERO + 0x98) };
    c.set_farg(1, yaw);
    let speed = c.rf(HERO + 0x194);
    let flagged = c.r8(HERO + 0x20b3) != 0;
    let mat_src = c.r32(HERO + 0x2080).wrapping_add(0x40);
    h::with_stack(c, 0xa0, |c, sp| {
        if flagged {
            let x = cos_of(c, F::ZERO) * speed;
            c.wf(sp + 0x50, x);
            let y = sin_of(c, F::ZERO) * speed;
            c.wf(sp + 0x54, y);
            c.wf(sp + 0x58, F::ZERO);
            h::call(c, MAT_BUILD, &[sp + 0x10, mat_src], &[]);
            h::run(c, vecmath::vec4_mul_mat4, &[sp + 0x50, sp + 0x50, sp + 0x10], &[]);
            copy16(c, sp, sp + 0x50);
        } else {
            let pitch = c.rf(HERO + 0x2e4);
            h::run(c, scene::spherical_to_xyz, &[sp], &[speed, yaw, pitch]);
        }
        copy16(c, DIR_OUT, sp);
    });
    true
}

/// 0x2053c0 hero_ground_probe_slide(flag): when the hero is near the ground (and not in a special state), runs two
/// segment probes around the hero, looks up the surface class, and pushes the hero's contact offset (0xe0) back out
/// along the surface normal.
pub fn hero_ground_probe_slide(c: &mut Cpu) -> bool {
    let flag = c.arg(0);
    if c.r16(HERO + 0x308) != 0 {
        return true;
    }
    let len = h::run(c, hero_space::length_off_up, &[HERO + 0xe0], &[]).1;
    if len < c.rf(MIN_LEN_ADDR) * F(0x3dcc_cccd) {
        return true;
    }
    let mode = c.r32(HERO + 0x208c).wrapping_sub(0x11);
    let (lift, scale) = if mode < 2 {
        (F::ZERO, F(0x3f33_3333))
    } else {
        match c.r8(HERO + 0x20a4) {
            1 => (F(0x3eb3_3333), F(0x3e8a_3d71)),
            2 => (F(0x3fc0_0000), F(0x40a0_0000)),
            _ => (F(0x3ebd_70a4), F(0x3f00_0000)),
        }
    };
    h::with_stack(c, 0xb0, |c, sp| {
        copy16(c, sp, TABLE);
        let z0 = c.rf(sp + 8) + lift;
        c.wf(sp + 8, z0);
        copy16(c, sp + 0x20, TABLE);
        let z1 = c.rf(sp + 0x28) + lift * F(0x3e99_999a);
        c.wf(sp + 0x28, z1);

        // Direction vector C at sp+0x10 and D at sp+0x30; A = sp+0, B = sp+0x20.
        let mut use_table_dir = false;
        if flag == 0 {
            let l = len_xy(c, TABLE + 0x60);
            use_table_dir = !(l < F(0x3c23_d70a));
        }
        if use_table_dir {
            copy16(c, sp + 0x10, TABLE + 0x60);
            c.w32(sp + 0x18, 0);
            let l2 = len_xy(c, sp + 0x10);
            h::vec_scale(c, sp + 0x10, sp + 0x10, scale / l2);
            copy16(c, sp + 0x30, sp + 0x10);
            h::vec_scale(c, sp + 0x30, sp + 0x30, F(0x3fb3_3333));
        } else {
            let yaw = c.rf(HERO + 0x98);
            let x1 = cos_of(c, yaw) * scale;
            c.wf(sp + 0x10, x1);
            let y1 = sin_of(c, yaw) * scale;
            c.wf(sp + 0x14, y1);
            c.w32(sp + 0x18, 0);
            let x2 = (cos_of(c, yaw) * scale) * F(0x3fb3_3333);
            c.wf(sp + 0x30, x2);
            let y2 = (sin_of(c, yaw) * scale) * F(0x3fb3_3333);
            c.wf(sp + 0x34, y2);
            c.w32(sp + 0x38, 0);
        }
        h::vec_add(c, sp + 0x10, sp + 0x10, sp);
        h::vec_add(c, sp + 0x30, sp + 0x30, sp + 0x20);

        // Probe 1: B -> D. A hit gives the surface class (s2); -1 when there is none.
        let mut hit: i32 = -1;
        let a3 = c.r32(HERO + 0x2080);
        let probe1 = [sp + 0x20, sp + 0x30, 4, a3, 0];
        if h::call(c, PROBE, &probe1, &[]).0 != 0 && c.r32(ZONE_COUNT) as i32 > 0 {
            hit = h::run(c, collision::hit_surface_class, &probe1, &[]).0 as i32;
        }

        // Probe 2: A -> C. A hit inside the zone gives the angle used by the gate below.
        let a3b = c.r32(HERO + 0x2080);
        let probe2 = [sp, sp + 0x10, 4, a3b, 0];
        let mut ang = F::ZERO;
        if h::call(c, PROBE, &probe2, &[]).0 != 0 && c.r32(ZONE_COUNT) as i32 > 0 {
            let l = len_xy(c, ZONE + 0x40);
            let f12 = c.rf(ZONE + 0x48);
            ang = h::run(c, trig::fast_atan2, &[], &[f12, l]).1;
        }
        let gate_open = F(0x3f5f_66f3) <= ang || hit == 8 || hit == 12;
        if !gate_open {
            return;
        }

        let zone_v = c.r32(ZONE + 0x18);
        c.w32(ZONE + 0x48, 0);
        c.w32(HERO + 0x240, zone_v);
        h::call(c, SURFACE_BASIS, &[ZONE + 0x40, ZONE + 0x40], &[]);
        let f1 = -c.rf(HERO + 0xe0) * c.rf(ZONE + 0x40);
        let f0 = c.rf(HERO + 0xe4) * c.rf(ZONE + 0x44);
        let depth = f1 - f0;
        if !(F::ZERO < depth) {
            return;
        }
        h::vec_scale(c, ZONE + 0x40, ZONE + 0x40, depth);
        h::vec_add(c, HERO + 0xe0, HERO + 0xe0, ZONE + 0x40);
    });
    true
}

/// 0x205760 hero_push_pull(pos*, other*, flag, a, b): with flag set, places the hero position and the other vector
/// at the reference directions scaled by a and b, then pulls the other vector inside the hero's limit. With flag
/// clear, adds b to z of the other vector (clamped at 0) and a to z of the hero position, after copying the table.
pub fn hero_push_pull(c: &mut Cpu) -> bool {
    let (hero_pos, other, flag) = (c.arg(0), c.arg(1), c.arg(2));
    let (fa, fb) = (c.farg(0), c.farg(1));
    h::with_stack(c, 0x80, |c, sp| {
        if flag != 0 {
            h::vec_set_length(c, sp, REF, -fb);
            h::vec_add(c, other, TABLE, sp);
            h::vec_set_length(c, sp, REF, -fa);
            h::vec_add(c, hero_pos, TABLE, sp);
            copy16(c, sp + 0x10, hero_pos);
            h::run(c, geom::vec_pull_to_limit, &[sp + 0x10, other], &[]);
        } else {
            copy16(c, sp, TABLE);
            copy16(c, other, sp);
            let z = c.rf(other + 8) + fb;
            c.wf(other + 8, if z < F::ZERO { F::ZERO } else { z });
            copy16(c, hero_pos, sp);
            let z = c.rf(hero_pos + 8) + fa;
            c.wf(hero_pos + 8, z);
        }
    });
    true
}
