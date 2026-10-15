//! Ground and wall probes built on the retail collision queries (0x1e24d8 line test, 0x1e35c8 sphere test): height
//! under a point, line-of-sight sweeps, and the ground-follow step that keeps a mover on the floor.
use crate::ps2::*;
use crate::rcall::*;
use crate::motion;
use crate::vu_geom;
use ee_oracle::cpu::Cpu;

/// Result block of the last collision query (hit count at +0x1c, hit normal at +0x40, hit point at +0x60 ...).
const HIT: u32 = 0x17_3f40;
const HIT_FLAG: u32 = HIT + 0x18;
const HIT_COUNT: u32 = HIT + 0x1c;
const HIT_NORMAL: u32 = HIT + 0x40;
const HIT_NORMAL_Z: u32 = HIT + 0x48;
const HIT_Z: u32 = HIT + 0x28;
const HIT_POINT: u32 = 0x17_3f60;
const HIT_SAFE_POS: u32 = 0x17_3f70;
/// Retail vector helpers and int_to_float used by sight_sweep, and the 1.0f bits they are fed with.
const VEC_ADD: u32 = 0x1f_1810;
const VEC_SUB: u32 = 0x1f_1840;
const VEC_SCALE: u32 = 0x1f_1898;
const INT_TO_FLOAT: u32 = 0x1f_27d8;
const ONE_BITS: u32 = 0x3f80_0000;
/// Retail sphere-vs-world test and the normal-slide helper.
const SPHERE_PROBE_FN: u32 = 0x1e_35c8;
const NORMALIZE_FN: u32 = 0x12_5180;

/// Snapshot of every register. `restore_kept` puts back only the callee-saved ones (s0-s7, gp, sp, fp, ra, f20-f31),
/// so the scratch registers keep what the retail calls and the Rust steps left, as retail does.
struct Kept {
    r: [u128; 32],
    f: [u32; 32],
}

fn keep(c: &Cpu) -> Kept {
    Kept { r: c.r, f: c.f }
}

fn restore_kept(c: &mut Cpu, k: &Kept) {
    for i in (16..24).chain([28, 29, 30, 31]) {
        c.r[i] = k.r[i];
    }
    for i in 20..32 {
        c.f[i] = k.f[i];
    }
}

/// The Rust hand of retail jal 0x1f1e48 (vu_geom::fast_atan2) with its VF and scratch leftovers.
fn atan2_hand(c: &mut Cpu, a: F, b: F) -> F {
    c.set_farg(0, a);
    c.set_farg(1, b);
    vu_geom::fast_atan2(c);
    c.fresult()
}

/// The Rust hand of retail jal 0x1f17b0 (motion::fabs) with its leftovers.
fn fabs_hand(c: &mut Cpu, x: F) -> F {
    c.set_farg(0, x);
    motion::fabs(c);
    c.fresult()
}

/// Height of the ground under `pos` found by a short vertical segment (z 0.01 .. pos.z + dz) with query flags
/// `flags | 2`; 0 when nothing is hit. `base` is the 0x30-byte local area of the retail frame.
fn ground_probe(c: &mut Cpu, base: u32, pos: u32, flags: u32, dz: F, retail_regs: bool) -> F {
    let p = c.rv(pos);
    c.wv(base, p);
    c.wf(base + 8, F(0x3c23_d70a));
    c.wv(base + 0x10, p);
    let z = c.rf(base + 0x18) + dz;
    c.wf(base + 0x18, z);
    // Retail leaves v1 = pos (move in the prologue) and v0 = 0x170000 (lui in the branch delay slot) around the query.
    if retail_regs {
        c.s(3, pos as u64);
    }
    let hit = line_probe(c, base + 0x10, base, flags | 2, 0, 0);
    if retail_regs {
        c.s(2, 0x17_0000);
    }
    if hit != 0 {
        c.rf(HIT_Z)
    } else {
        F::ZERO
    }
}

/// 0x24bfd8 ground_probe(pos, flags, dz) -> ground height or 0: see ground_probe.
pub fn ground_height(c: &mut Cpu) -> bool {
    let (pos, flags, dz) = (c.arg(0), c.arg(1), c.farg(0));
    let sp = c.g(29) as u32;
    let r = ground_probe(c, sp.wrapping_sub(0x30), pos, flags, dz, true);
    // (no register restore: the query leftovers stay as retail leaves them)
    c.fret(r);
    true
}

/// 0x24c050 ground_height_here(pos) -> ground height 0.5 above pos (flags 0x20): see ground_probe.
pub fn ground_height_here(c: &mut Cpu) -> bool {
    let pos = c.arg(0);
    let saved = save(c);
    let sp = c.g(29) as u32;
    let r = ground_probe(c, sp.wrapping_sub(0x40), pos, 0x20, F(0x3f00_0000), false);
    restore(c, &saved);
    c.fret(r);
    true
}

/// 0x24c8c8 sight_sweep(a0, a1, c0, d0, flags, t0, n) -> 0/1: steps n times along the lerp of segment (a0 -> c0)
/// and (a1 -> d0) and returns 1 at the first step whose segment hits the world.
pub fn sight_sweep(c: &mut Cpu) -> bool {
    let (a, b, c0, d0) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3));
    let (flags, t0, n) = (c.arg(4), c.arg(5), c.arg(6));
    // Retail prologue: sp -= 0xf0 (the frame base f); the epilogue adds it back. Only sp moves here: pop_frame would
    // also roll back the scratch registers the callees leave.
    let sp0 = c.g(29);
    let f = (sp0 as u32).wrapping_sub(0xf0);
    c.s(29, f as i32 as i64 as u64);
    // Retail calls for these helpers: their scratch leftovers (at, f0, f1, a0-a3, t0-t9, VF lanes) reach the caller.
    let (_, n_float) = crate::h::call_keep(c, INT_TO_FLOAT, &[n], &[]);
    c.s(1, ONE_BITS as u64);
    c.f[1] = ONE_BITS;
    let step = F::ONE / n_float;
    crate::h::call_keep(c, VEC_SUB, &[f, a, c0], &[]);
    crate::h::call_keep(c, VEC_SUB, &[f + 0x10, b, d0], &[]);
    let mut hit = 0;
    let mut t = step;
    for _ in 0..(n as i32).max(0) {
        crate::h::call_keep(c, VEC_SCALE, &[f + 0x20, f], &[t]);
        crate::h::call_keep(c, VEC_SCALE, &[f + 0x30, f + 0x10], &[t]);
        crate::h::call_keep(c, VEC_ADD, &[f + 0x20, f + 0x20, c0], &[]);
        crate::h::call_keep(c, VEC_ADD, &[f + 0x30, f + 0x30, d0], &[]);
        if line_probe(c, f + 0x20, f + 0x30, 0, flags, t0) != 0 {
            hit = 1;
            break;
        }
        t = t + step;
    }
    c.s(29, sp0);
    c.ret(hit);
    true
}

/// 0x24cce0 ledge_probe(obj): casts down from obj.pos (z lowered by 16, at least 0.5, to z + 0.5) and stores the
/// hit height -+ 0.2 in obj+0x84 / +0x88 (both 0 when nothing is hit).
pub fn ledge_probe(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let kept = keep(c);
    let f = (c.g(29) as u32).wrapping_sub(0x40);
    c.s(29, f as i32 as i64 as u64);
    let pos = c.rv(obj + 0x10);
    c.wv(f, pos);
    let half = F(0x3f00_0000);
    let sixteen = F(0x4180_0000);
    let z = c.rf(f + 8) - sixteen;
    c.wf(f + 8, if z < half { half } else { z });
    c.wv(f + 0x10, pos);
    let z = c.rf(f + 0x18) + half;
    c.wf(f + 0x18, z);
    // Registers as retail leaves them at the probe jal: f0 = z, f1 = 16.0, f2 = 0.5, at = 0.5, v1 = obj + 0x10.
    c.f[0] = z.0;
    c.f[1] = sixteen.0;
    c.f[2] = half.0;
    c.s(1, half.0 as u64);
    c.s(3, (obj + 0x10) as u64);
    let hit = line_probe(c, f + 0x10, f, 0x22, 0, 0);
    // The delay slot lui v0, 0x17 runs on both paths.
    c.s(2, 0x17_0000);
    if hit != 0 {
        let margin = F(0x3e4c_cccd);
        c.s(1, margin.0 as u64);
        c.f[2] = margin.0;
        c.s(2, HIT as u64);
        let h = c.rf(HIT_Z);
        c.wf(obj + 0x84, h - margin);
        c.f[0] = (h - margin).0;
        let h = c.rf(HIT_Z);
        c.wf(obj + 0x88, h + margin);
        c.f[1] = (h + margin).0;
    } else {
        c.w32(obj + 0x88, 0);
        c.w32(obj + 0x84, 0);
    }
    restore_kept(c, &kept);
    true
}

/// Resets `cur` to `prev` when the ground height under it (probe at z + h) differs from prev.z by more than
/// `max_dz` (flags bit 0 set: only when the ground is higher than prev.z). Returns true on a reset.
fn snap_to_prev(c: &mut Cpu, f: u32, cur: u32, prev: u32, flags: u32, max_dz: F, h: F) -> bool {
    let z = c.rf(cur + 8) + h;
    c.wf(cur + 8, z);
    let ground = ground_probe(c, f.wrapping_sub(0x40), cur, 0x20, F(0x3f00_0000), false);
    let z = c.rf(cur + 8) - h;
    c.wf(cur + 8, z);
    let prev_z = c.rf(prev + 8);
    if max_dz < fabs_hand(c, ground - prev_z) {
        let reset = if flags & 1 == 0 { ground < prev_z || prev_z < ground } else { prev_z < ground };
        if reset {
            let p = c.rv(prev);
            c.wv(cur, p);
            return true;
        }
    }
    false
}

/// 0x24ad58 ground_follow(world, prev*, cur*, flags, h, radius, max_dz, slope) -> 1 when cur moved freely, 0 when it
/// was snapped back / slid: keeps `cur` on the floor (probe heights), slides it along walls hit by the move
/// prev -> cur (normal at HIT_NORMAL, pitch limit `slope`) and pushes it out of ceilings (unless flags bit 1).
/// prev's x, y are updated to cur's.
pub fn ground_follow(c: &mut Cpu) -> bool {
    let (world, prev, cur, flags) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3));
    let (h, radius, max_dz, slope) = (c.farg(0), c.farg(1), c.farg(2), c.farg(3));
    let (sp, f) = push_frame(c, 0x130);
    let mut ok = true;
    if snap_to_prev(c, f, cur, prev, flags, max_dz, h) {
        ok = false;
    }
    // sp+0 = delta (cur - prev), sp+0x10 = delta set to 1.2 radius, sp+0x20 / 0x30 = probe segment.
    vec_sub(c, f, cur, prev);
    vec_set_length(c, f + 0x10, f, radius * F(0x3f99_999a));
    c.wv(f + 0x40, V([0; 4]));
    c.wf(f + 0x48, h);
    vec_add(c, f + 0x20, prev, f + 0x40);
    vec_add(c, f + 0x30, f + 0x20, f + 0x10);
    if ok && line_probe(c, f + 0x20, f + 0x30, (flags & 2) | 0x24, world, 0) != 0 && c.r32(HIT_COUNT) as i32 > 0 {
        let n = HIT + 0x40;
        let len = length_xy(c, n);
        let nz = c.rf(HIT_NORMAL_Z);
        let pitch = atan2(c, nz, len);
        if slope <= pitch {
            c.w32(HIT_NORMAL_Z, 0);
            c.set_arg(0, n);
            c.set_arg(1, n);
            c.call(NORMALIZE_FN);
            let dot = (-c.rf(f)) * c.rf(HIT_NORMAL) - c.rf(f + 4) * c.rf(HIT_NORMAL + 4);
            if F::ZERO < dot {
                vec_scale(c, n, n, dot);
                ok = false;
                vec_add(c, f, f, n);
            }
        }
        vec_add(c, cur, prev, f);
    }
    if flags & 2 == 0 {
        let lift = h + radius;
        for _ in 0..6 {
            let p = c.rv(cur);
            c.wv(f + 0x50, p);
            let z = c.rf(f + 0x58) + lift;
            c.wf(f + 0x58, z);
            c.set_farg(0, radius);
            c.set_arg(0, f + 0x50);
            c.set_arg(1, 0x24);
            c.set_arg(2, world);
            c.call(SPHERE_PROBE_FN);
            if c.result() == 0 {
                break;
            }
            let d = dist_xy(c, cur, HIT_POINT);
            let dz = c.rf(HIT_Z) - c.rf(cur + 8);
            let pitch = atan2(c, d, dz);
            if c.r32(HIT_FLAG) != 0 || slope < pitch {
                let p = c.rv(HIT_SAFE_POS);
                c.wv(cur, p);
                let z = c.rf(cur + 8) - lift;
                c.wf(cur + 8, z);
                ok = false;
            }
        }
    }
    if snap_to_prev(c, f, cur, prev, flags, max_dz, h) {
        ok = false;
    }
    let (x, y) = (c.rf(cur), c.rf(cur + 4));
    c.wf(prev, x);
    c.wf(prev + 4, y);
    pop_frame(c, &sp);
    c.ret(ok as u32);
    true
}
