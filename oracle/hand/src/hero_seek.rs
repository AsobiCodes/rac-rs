//! Hero aim update and nearest-entry scan from overlay l4_02. Both keep the retail control flow; the retail
//! callees that have no Rust twin are called through `h::call`.
use crate::h;
use crate::ps2::*;
use crate::{collision, hero_space, mathx, trig, vecmath};
use ee_oracle::cpu::Cpu;

/// Hero state block (0x140000 - 0xcb0).
const HERO: u32 = 0x13_f350;
/// Float scale (-20.0 at run time) compared against the hero's up component.
const UP_SCALE_ADDR: u32 = 0x15_ed6c;
/// Count of records in the entry table.
const ENTRY_COUNT: u32 = 0x15_f6f0;
/// Pointer to the 0x20-byte entry table.
const ENTRY_TABLE_PTR: u32 = 0x15_f6ec;
const ENTRY_STRIDE: u32 = 0x20;
/// gp-relative pointer to the probe context (its word is the loop bound on skip paths).
const GP_PROBE_CTX: u32 = 0x7510;
/// gp-relative pointer to the parallel table (same index as the entry table).
const GP_PARALLEL_TABLE: u32 = 0x7514;
/// Aim sample block the aim update reads (0x173f80).
const AIM_DIR: u32 = 0x17_3f80;
/// Hero mode value that selects the wider aim window.
const MODE_17: u32 = 0x11;

/// Inputs of `nearest_entry`.
struct Query {
    pos: V,
    out_obj: u32,
    reference: u32,
    out_word: u32,
    out_float: u32,
    out_word2: u32,
    skip_obj: u32,
    want_obj: u32,
}

/// What one table record did to the scan.
enum Step {
    Found,
    /// Filtered out; the loop bound is then re-read from the probe context pointer (as retail does).
    Skip,
    /// The 0x250b28 probe failed; the loop bound is re-read from the entry count.
    NoHit,
}

fn gp_word(c: &mut Cpu, back: u32) -> u32 {
    let a = (c.g(28) as u32).wrapping_sub(back);
    c.r32(a)
}

/// 0x1fe660 aim_step(): per-frame hero aim update. While the hero is falling fast enough and the up component is
/// above the limit, it rebuilds a probe point from the hero's gravity sample, runs the line probe and, when clear,
/// stores the new aim/lock values into the hero block.
pub fn aim_step(c: &mut Cpu) -> bool {
    if c.rf(HERO + 0x2dc) < F(0x3ecc_cccd) {
        return true;
    }
    let up = h::run(c, hero_space::up_component, &[HERO + 0xe0], &[]).1;
    if up < c.rf(UP_SCALE_ADDR) * F(0xc1a0_0000) {
        return true;
    }
    if c.r32(HERO + 0x4e8) != 0 {
        c.w32(HERO + 0x1b4, 0);
        return true;
    }
    h::with_stack(c, 0x40, |c, sp| aim_probe(c, sp))
}

/// Second half of `aim_step`, with `sp` a 0x40-byte scratch area that stands in for the retail frame.
fn aim_probe(c: &mut Cpu, sp: u32) -> bool {
    let base = c.rv(HERO + 0x80);
    c.wv(sp, base);
    let n = h::scale_int(c, 20);
    let k = F::from_int(n);
    let fall = c.rf(HERO + 0x4a0) * k * k * F(0x3f00_0000);
    let x = base.x() + c.rf(HERO + 0x100) * k;
    let y = base.y() + c.rf(HERO + 0x104) * k;
    let z = (base.z() + c.rf(HERO + 0xe8) * k) - fall;
    c.wv(sp, base.with_xyz(x, y, z));

    if h::call(c, 0x1ff800, &[sp, HERO + 0x98], &[]).0 != 0 {
        c.w32(HERO + 0x1b4, 0);
        c.w16(HERO + 0x1f0, (n + 2) as u16);
        return true;
    }
    if h::call(c, 0x1fd2e8, &[0], &[F(0x3e99_999a), F::ONE]).0 == 0 {
        return true;
    }

    // Lean angle: 1.7 by default, 3.0 when the hero is not in mode 0x11 and either flag at +0x255 is clear or +0x254 is set.
    let mut lean = F(0x3fd9_999a);
    if c.r32(HERO + 0x2084) != MODE_17 && (c.r8(HERO + 0x255) == 0 || c.r8(HERO + 0x254) != 0) {
        lean = F(0x4040_0000);
    }
    h::run(c, hero_space::polar_to_world, &[sp], &[F::ZERO, F::ZERO, lean]);
    let radius = c.rf(HERO + 0x234) + F(0x3fb3_3333);
    h::run(c, hero_space::polar_to_world, &[sp + 0x10], &[radius, F::ZERO, lean]);
    if h::call(c, 0x1e24d8, &[sp, sp + 0x10, 2, 0, 0], &[]).0 == 0 {
        return true;
    }
    let class = h::run(c, collision::hit_surface_class, &[], &[]).0;
    if class == 0xa || class == 0xc {
        return true;
    }
    let pitch = h::run(c, hero_space::pitch_of, &[AIM_DIR], &[]).1;
    if pitch < F(0x3fa7_8d36) {
        return true;
    }

    h::run(c, vecmath::vec4_mul_mat4, &[sp + 0x20, AIM_DIR, HERO + 0x40], &[]);
    let ax = c.rf(sp + 0x20);
    let ay = c.rf(sp + 0x24);
    let heading = h::run(c, trig::fast_atan2, &[], &[ax, ay]).1;
    let turn = h::run(c, mathx::angle_dist, &[], &[heading, F(0x4049_0fdb)]).1;
    if F(0x3f5f_66f3) < turn {
        return true;
    }

    let limit = if c.r32(HERO + 0x2084) == MODE_17 { F(0x3f73_3333) } else { F(0x3f38_51ec) };
    let reach = h::run(c, vecmath::dist_xy, &[AIM_DIR - 0x20, sp], &[]).1;
    if limit < reach {
        return true;
    }

    let dir_word = c.rv(AIM_DIR);
    c.wv(HERO + 0x460, dir_word);
    let dx = c.rf(AIM_DIR);
    let dy = c.rf(AIM_DIR + 4);
    let aim_angle = h::run(c, trig::fast_atan2, &[], &[dx, dy]).1;
    c.wf(HERO + 0x46c, aim_angle);

    let mode_a = c.r32(HERO + 0x208c);
    let gated = mode_a == 4 || c.r32(HERO + 0x2094) == 4 || c.r32(HERO + 0x20a0) == 4;
    if !gated {
        return true;
    }
    if c.r32(HERO + 0x22b4) != 0 {
        let a = c.rf(HERO + 0x47c);
        let b = c.rf(HERO + 0x46c);
        let spread = h::run(c, mathx::angle_dist, &[], &[a, b]).1;
        if spread < F(0x403d_e44e) {
            return true;
        }
    }
    let mode = c.r32(HERO + 0x2084);
    let level = h::scale_int(c, if mode != MODE_17 { 6 } else { 7 });
    c.w32(HERO + 0x1b4, level as u32);
    true
}

/// 0x1ffdb0 nearest_entry(pos, out_obj, ref, out_word, out_float, out_word2, skip_obj, want_obj) -> found: walks the
/// 0x20-byte entry table and takes the first record whose object passes the filters and whose radius covers `pos`,
/// then checks the probe context and the hero's height window, and copies the hit fields out.
pub fn nearest_entry(c: &mut Cpu) -> bool {
    let pos = c.rv(c.arg(0));
    let q = Query {
        pos,
        out_obj: c.arg(1),
        reference: c.arg(2),
        out_word: c.arg(3),
        out_float: c.arg(4),
        out_word2: c.arg(5),
        skip_obj: c.arg(6),
        want_obj: c.arg(7),
    };
    // Retail returns 0 straight from the prologue when the count is not positive (no frame set up).
    if (c.r32(ENTRY_COUNT) as i32) <= 0 {
        c.ret(0);
        return true;
    }
    let found = h::with_stack(c, 0x40, |c, sp| scan(c, sp, &q));
    c.ret(found as u32);
    true
}

fn scan(c: &mut Cpu, sp: u32, q: &Query) -> bool {
    if (c.r32(ENTRY_COUNT) as i32) <= 0 {
        return false;
    }
    c.wv(sp, q.pos);
    let mut i: i32 = 0;
    let mut bound = c.r32(ENTRY_COUNT) as i32;
    loop {
        let table = c.r32(ENTRY_TABLE_PTR);
        let e = table.wrapping_add((i as u32).wrapping_mul(ENTRY_STRIDE));
        match scan_entry(c, sp, q, e, i as u32) {
            Step::Found => return true,
            Step::Skip => bound = gp_word(c, GP_PROBE_CTX) as i32,
            Step::NoHit => bound = c.r32(ENTRY_COUNT) as i32,
        }
        i += 1;
        if i >= bound {
            return false;
        }
    }
}

fn scan_entry(c: &mut Cpu, sp: u32, q: &Query, e: u32, index: u32) -> Step {
    let obj = c.r32(e + 0x10);
    if q.skip_obj != 0 && obj == q.skip_obj {
        return Step::Skip;
    }
    if q.want_obj != 0 && obj != q.want_obj {
        return Step::Skip;
    }
    if c.r32(obj) == 0 {
        return Step::Skip;
    }
    let dist = h::run(c, vecmath::dist3, &[e, sp], &[]).1;
    if c.rf(e + 0xc) < dist {
        return Step::Skip;
    }

    // Retail takes these from the entry itself (its v0 is still the entry on this path, the bc1tl delay slot is annulled).
    let probe_a = c.r32(e + 0x10);
    let probe_b = c.r32(e + 0x14);
    let gate = h::call(
        c,
        0x250b28,
        &[probe_a, sp, q.reference, sp + 0x10, sp + 0x14, probe_b],
        &[F(0x4140_0000), F(0x4120_0000), F::ZERO],
    );
    if gate.0 == 0 {
        return Step::NoHit;
    }

    let state = c.r32(HERO + 0x208c);
    let mut limit = F(0x3f66_6666);
    let mut limit_z = F(0x3fc0_0000);
    if state < 2 {
        limit = F(0x3e99_999a);
        if (c.r16(HERO + 0x1ca) as i16) != 0 {
            limit = limit + F(0x3f00_0000);
            limit_z = F(0x4120_0000);
        }
    }
    let dxy = h::run(c, vecmath::dist_xy, &[sp, q.reference], &[]).1;
    let dz = (c.rf(sp + 8) - c.rf(q.reference + 8)).abs();
    if !(dxy < limit) || !(dz < limit_z) {
        return Step::Skip;
    }

    let parallel = gp_word(c, GP_PARALLEL_TABLE);
    let mate = c.r32(parallel + index * ENTRY_STRIDE + 0x10);
    let hit_word = c.r32(sp + 0x10);
    let hit_float = c.rf(sp + 0x14);
    let extra = c.r32(e + 0x14);
    c.w32(q.out_obj, mate);
    c.w32(q.out_word, hit_word);
    c.wf(q.out_float, hit_float);
    c.w32(q.out_word2, extra);
    Step::Found
}
