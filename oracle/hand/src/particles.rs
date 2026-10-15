//! Particle pool, particle updates and the small ring-buffer / spline helpers around them.
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// gp-relative pool state: base of the 0x40-byte particle slots, first free slot hint, highest used slot, live count.
const POOL_BASE: i32 = -0x6b14;
const POOL_FIRST_FREE: i32 = -0x6b10;
const POOL_TOP: i32 = -0x6b0c;
const POOL_COUNT: i32 = -0x6b08;
/// The pool-run loop's saved ra, current slot cursor and end pointer.
const RUN_RA: i32 = -0x6af8;
const RUN_CURSOR: i32 = -0x6af0;
/// Used-slot bitmap (one bit per slot); the second bitmap 0x100 bytes on marks slots with flag 0x40.
const SLOT_BITS: u32 = 0x1b1980;
/// Per-type update function table.
const UPDATE_TABLE: u32 = 0x1b1f80;
/// Pad history block: 30 entries of u32 at +0x1e0, head index (i16) at +0x18e, valid count at +0x190.
const PAD_HISTORY: u32 = 0x13c940;
/// A constant vector added to particle positions.
const CONST_VEC: u32 = 0x13f3d0;

const SLOT_SHIFT: u32 = 6;
const FLAG_FREE: u8 = 0x80;

const ALLOC_PARTICLE: u32 = 0x259a38;
const FLOAT_TO_FIXED: u32 = 0x1f27e8;
const INT_TO_FLOAT: u32 = 0x1f27d8;
const VEC_SUB: u32 = 0x1f1840;
const VEC_LENGTH: u32 = 0x1f1970;
const VEC_ZERO_CHECK: u32 = 0x1f1530;
const VEC_ADD: u32 = 0x1f1810;
const VEC_SCALE: u32 = 0x1f1898;
const VEC_SET_LENGTH: u32 = 0x1f1a98;
const VEC_DIST3: u32 = 0x1f19e8;
const PARTICLE_FREE: u32 = 0x259b88;
const RAND_BYTE: u32 = 0x24a418;
const ERROR_PRINT: u32 = 0x1dd878;
const ERROR_TEXT: u32 = 0x1600a8;
const PAD_STATE: u32 = 0x124da0;
const PAD_READ_STATE0: u32 = 0x124cb0;
const PAD_READ_STATE1: u32 = 0x124bd8;
const PAD_READ_EXT: u32 = 0x1250d8;
const PAD_DECODE: u32 = 0x259330;
const PAD_RESET: u32 = 0x2592c8;

/// Runs `body` and puts every register back as it was on entry (a retail callee's scratch registers are garbage
/// the callers never read), keeping only v0 when the function returns a value.
fn clean(c: &mut Cpu, body: fn(&mut Cpu) -> bool, returns_v0: bool) -> bool {
    let (r, f) = (c.r, c.f);
    let ok = body(c);
    let v0 = c.r[2];
    c.r = r;
    c.f = f;
    if ok && returns_v0 {
        c.r[2] = v0;
    }
    ok
}

/// v0 = `p` (sign-extended) with the upper 64 bits of `hi_from`, as `move v0, s0` after an `lq v0` leaves it.
fn ret_with_hi(c: &mut Cpu, p: u32, hi_from: V) {
    let hi = (hi_from.0[2] as u128 | (hi_from.0[3] as u128) << 32) << 64;
    c.r[2] = hi | (p as i32 as i64 as u64) as u128;
}

fn gp(c: &Cpu, off: i32) -> u32 {
    (c.g(28) as u32).wrapping_add(off as u32)
}

fn bit_get(c: &mut Cpu, base: u32, i: u32) -> bool {
    c.r8(base + (i >> 3)) >> (i & 7) & 1 != 0
}

fn bit_clear(c: &mut Cpu, base: u32, i: u32) {
    let a = base + (i >> 3);
    let v = c.r8(a) & !(1u8 << (i & 7));
    c.w8(a, v);
}

/// Frees particle slot `p`: clears its bitmap bits, lowers the top index to the next used slot and the first-free
/// hint to this slot, marks it free.
pub fn particle_free(c: &mut Cpu, p: u32) {
    let (pool, hint, top, count) = (gp(c, POOL_BASE), gp(c, POOL_FIRST_FREE), gp(c, POOL_TOP), gp(c, POOL_COUNT));
    let idx = p.wrapping_sub(c.r32(pool)) >> SLOT_SHIFT;
    let n = c.r32(count).wrapping_sub(1);
    c.w32(count, n);
    if c.r8(p + 1) & 0x40 != 0 {
        bit_clear(c, SLOT_BITS + 0x100, idx);
    }
    bit_clear(c, SLOT_BITS, idx);
    let t = c.r32(top) as i32;
    if idx as i32 == t {
        let mut t = t - 1;
        while t >= 0 && !bit_get(c, SLOT_BITS, t as u32) {
            t -= 1;
        }
        c.w32(top, t as u32);
    }
    c.w8(p + 1, FLAG_FREE);
    let h = (c.r32(hint) as i32).min(idx as i32);
    c.w32(hint, h as u32);
}

/// 0x259b88 particle_free(p): see `particle_free`.
pub fn particle_free_fn(c: &mut Cpu) -> bool {
    particle_free(c, c.arg(0));
    true
}

/// Writes a 12-byte-stride-free 4-lane `(a - b)`-style helper result: (v.xyz * s, v.w) and (a.xyz + b.xyz, a.w).
fn scale(v: V, s: F) -> V {
    v.with(v * s, XYZ)
}

fn add(a: V, b: V) -> V {
    a.with(a + b, XYZ)
}

/// 0x268900 hermite_point(out, p0, p1, t0, t1, u): out = h00*p0 + h10*t0 + h11*t1 + h01*p1 (cubic Hermite at u=f12).
/// Retail forms each term with vec_scale into a stack temp and adds it with vec_add; the same calls are made so the
/// VU registers they leave match.
pub fn hermite_point(c: &mut Cpu) -> bool {
    let (out, p0, p1, t0, t1) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3), c.arg(4));
    let u = c.farg(0);
    let u2 = u * u;
    let u3 = u2 * u;
    let u2x3 = u2 * F(0x4040_0000);
    let u3x2 = u3 + u3;
    let u2x2 = u2 + u2;
    let h00 = (u3x2 - u2x3) + F::ONE;
    let h10 = (u3 - u2x2) + u;
    let h11 = u3 - u2;
    let h01 = u2x3 - u3x2;
    let temp = (c.g(29) as u32).wrapping_sub(0x90);
    c.f[0] = F::ONE.0;
    vec_scale_call(c, out, p0, h00);
    vec_scale_call(c, temp, t0, h10);
    vec_add_call(c, out, out, temp);
    vec_scale_call(c, temp, t1, h11);
    vec_add_call(c, out, out, temp);
    vec_scale_call(c, temp, p1, h01);
    vec_add_call(c, out, out, temp);
    true
}

/// Retail `jal vec_scale`: out = (src.xyz * s, src.w), s in f12.
fn vec_scale_call(c: &mut Cpu, out: u32, src: u32, s: F) {
    c.set_arg(0, out);
    c.set_arg(1, src);
    c.set_farg(0, s);
    c.call(VEC_SCALE);
}

/// Retail `jal vec_add`: out = a + b (xyz lanes).
fn vec_add_call(c: &mut Cpu, out: u32, a: u32, b: u32) {
    c.set_arg(0, out);
    c.set_arg(1, a);
    c.set_arg(2, b);
    c.call(VEC_ADD);
}

/// 0x259a18 input_history_scan_default(): the pad-history scan on the pad block.
pub fn pad_scan_default(c: &mut Cpu) -> bool {
    clean(c, pad_scan_default_body, false)
}

/// The body of `pad_scan_default`.
fn pad_scan_default_body(c: &mut Cpu) -> bool {
    c.set_arg(0, PAD_HISTORY);
    pad_update_body(c)
}

/// 0x259d88 run_particles(): calls each live slot's update function (type byte indexes the table) with the slot
/// pointer; the cursor and end pointer live in memory because updates may change them.
pub fn run_particles(c: &mut Cpu) -> bool {
    clean(c, run_particles_body, false)
}

/// The body of `run_particles`.
fn run_particles_body(c: &mut Cpu) -> bool {
    let ra = c.g(31);
    let slot = gp(c, RUN_RA);
    c.w32(slot, ra as u32);
    c.w32(slot + 4, (ra >> 32) as u32);
    let cursor = gp(c, RUN_CURSOR);
    let base = c.r32(gp(c, POOL_BASE));
    let top = c.r32(gp(c, POOL_TOP));
    let mut end = (top.wrapping_add(1) << SLOT_SHIFT).wrapping_add(base);
    c.w32(cursor + 4, end);
    let mut p = base;
    while p != end {
        let flags = c.r8(p + 1) as i8;
        let kind = c.r8(p) as i8;
        p = p.wrapping_add(0x40);
        if flags < 0 {
            continue;
        }
        c.w32(cursor, p);
        let f = c.r32(UPDATE_TABLE.wrapping_add((kind as i32 as u32) << 2));
        c.set_arg(0, p.wrapping_sub(0x40));
        c.call(f);
        p = c.r32(cursor);
        end = c.r32(cursor + 4);
    }
    let ra = c.r32(slot) as u64 | (c.r32(slot + 4) as u64) << 32;
    c.s(31, ra);
    true
}

/// 0x25aa0c pool_tex_upload_tag (register call: t0 packed texture word, s0 address, s1 count, s2 tag cursor, t1
/// table base, vf20 the TEX0 template): writes one 16-byte tag and the TEX0 table entry at t1+0x3000.
pub fn pool_tex_upload_tag(c: &mut Cpu) -> bool {
    let t0 = c.g(8);
    let (s0, s1, s2, t1) = (c.g(16) as u32, c.g(17) as u32, c.g(18) as u32, c.g(9) as u32);
    let size_lo = t0 as u32 & 0xf;
    c.w32(s2, t0 as u32 >> 4);
    c.w32(s2 + 8, (t0 >> 36) as u32);
    c.w16(s2 + 0xc, 0x505);
    c.w16(s2 + 4, size_lo as u16);
    let shifted = size_lo << 8;
    let step = 0x400u32.wrapping_sub(shifted);
    let addr_hi = s0 >> 8;
    c.w16(s2 + 6, addr_hi as u16);
    let tex_a = (shifted as u64) << 43;
    let tex_b = (addr_hi as u64) << 37;
    let s0 = s0.wrapping_add(step);
    let addr_hi2 = s0 >> 8;
    let tex0 = c.vf[20];
    let mut lo = (tex0[0] as u64 | (tex0[1] as u64) << 32) | tex_b;
    lo |= addr_hi2 as u64;
    c.w16(s2 + 0xe, addr_hi2 as u16);
    lo |= tex_a;
    c.s(16, s0.wrapping_add(0x400) as i32 as i64 as u64);
    c.s(17, s1.wrapping_add(1) as i32 as i64 as u64);
    c.s(18, s2.wrapping_add(0x10) as i32 as i64 as u64);
    c.r[8] = (c.vf[20][2] as u128 | (c.vf[20][3] as u128) << 32) << 64 | lo as u128;
    c.w32(t1 + 0x3000, lo as u32);
    c.w32(t1 + 0x3004, (lo >> 32) as u32);
    true
}

/// 0x258500 set_once(value, ptr): `*ptr` holds 0xffffffff while unset; sets it to `value` (and field 0xa to 7 when
/// the value is 0); a set slot reports an error. A null ptr does nothing.
pub fn set_once(c: &mut Cpu) -> bool {
    clean(c, set_once_body, false)
}

/// The body of `set_once`.
fn set_once_body(c: &mut Cpu) -> bool {
    let (value, ptr) = (c.arg(0), c.arg(1));
    if ptr == 0 {
        return true;
    }
    if c.r32(ptr) != 0xffff_ffff {
        c.set_arg(0, ERROR_TEXT);
        c.call(ERROR_PRINT);
        return true;
    }
    c.w32(ptr, value);
    if value == 0 {
        c.w16(ptr + 0xa, 7);
    }
    true
}

/// 0x258ee0 input_history_scan(mask, frames, out_age) -> hit bits: scans the pad history (30 entries, newest at the
/// head index) from 1 frame back to `frames`; returns the first entry's `& mask` that is non-zero and stores its
/// age (frames back minus 1) through out_age when non-null; 0 when none.
pub fn input_history_scan(c: &mut Cpu) -> bool {
    let (mask, frames, out) = (c.arg(0), c.arg(1) as i32, c.arg(2));
    let n = (c.r32(PAD_HISTORY + 0x190) as i32).min(frames) + 1;
    let mut age = 1;
    while age < n {
        let head = c.r16(PAD_HISTORY + 0x18e) as i16 as i32;
        let idx = head.wrapping_sub(age).wrapping_add(30) % 30;
        let entry = c.r32(PAD_HISTORY + 0x1e0 + (idx as u32) * 4);
        if entry & mask != 0 {
            if out != 0 {
                c.w32(out, (age - 1) as u32);
            }
            c.ret(entry & mask);
            return true;
        }
        age += 1;
    }
    c.ret(0);
    true
}

/// Last valid index of a ring object (flag byte at +4 negative = single slot): count-1 from the table at +0x10.
fn ring_last(c: &mut Cpu, obj: u32) -> i32 {
    if (c.r8(obj + 4) as i8) < 0 {
        0
    } else {
        let t = c.r32(obj + 0x10);
        (c.r32(t) as i32).wrapping_sub(1)
    }
}

/// Wraps index `i` into a ring whose last index is `last`: negative indexes count back from the end, an index above
/// `last` wraps modulo `last + 1`. None when the modulus is zero.
fn ring_wrap(i: i32, last: i32) -> Option<i32> {
    if i < 0 {
        Some(last.wrapping_add(i))
    } else if last < i {
        let m = last.wrapping_add(1);
        (m != 0).then(|| i.wrapping_rem(m))
    } else {
        Some(i)
    }
}

/// 0x268630 ring_point(obj, i) -> pointer to point i (16 bytes each, +0x10 in the object's table, +0x20 for a
/// negative index).
pub fn ring_point(c: &mut Cpu) -> bool {
    let (mut obj, i) = (c.arg(0), c.arg(1) as i32);
    let last;
    if (c.r8(obj + 4) as i8) < 0 {
        obj = c.r32(obj + 0x10);
        last = 0;
    } else {
        let t = c.r32(obj + 0x10);
        last = (c.r32(t) as i32).wrapping_sub(1);
        obj = t;
    }
    if i < 0 {
        c.ret(obj.wrapping_add(((last.wrapping_add(i)) << 4) as u32).wrapping_add(0x20));
        return true;
    }
    let idx = if last < i {
        let m = last.wrapping_add(1);
        if m == 0 {
            return false;
        }
        i.wrapping_rem(m)
    } else {
        i
    };
    c.ret(obj.wrapping_add((idx << 4) as u32).wrapping_add(0x10));
    true
}

/// 0x2686a8 ring_index(obj, i) -> i wrapped into the object's ring (see `ring_wrap`).
pub fn ring_index(c: &mut Cpu) -> bool {
    let (obj, i) = (c.arg(0), c.arg(1) as i32);
    let last = ring_last(c, obj);
    match ring_wrap(i, last) {
        Some(v) => {
            c.ret(v as u32);
            true
        }
        None => false,
    }
}

/// 0x2686f8 ring_advance(counter) -> *counter = (*counter + 1) wrapped into the object's ring; returns it.
pub fn ring_advance(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let last = if (c.r8(obj + 4) as i8) < 0 {
        0
    } else {
        let t = c.r32(obj + 0x10);
        (c.r32(t) as i32).wrapping_sub(1)
    };
    let mut next = (c.r32(obj) as i32).wrapping_add(1);
    if last < next {
        let m = last.wrapping_add(1);
        if m == 0 {
            return false;
        }
        next = next.wrapping_rem(m);
    }
    c.w32(obj, next as u32);
    c.ret(next as u32);
    true
}

/// 0x265460 particle_fall_update(p): optionally adds a constant vector to the position; frees the particle when
/// the zero check on its velocity fields says so. Retail's scratch registers after its calls are kept (no restore).
pub fn particle_fall_update(c: &mut Cpu) -> bool {
    particle_fall_update_body(c)
}

/// The body of `particle_fall_update`.
fn particle_fall_update_body(c: &mut Cpu) -> bool {
    let p = c.arg(0);
    c.set_arg(2, p + 0x20);
    if c.r32(p + 0x30) == 1 {
        vec_add_call(c, p + 0x10, CONST_VEC, p + 0x20);
    }
    c.set_arg(0, p + 0xa);
    c.call(VEC_ZERO_CHECK);
    if c.result() != 0 {
        c.set_arg(0, p);
        c.call(PARTICLE_FREE);
    }
    true
}

/// 0x265340 spawn_particle_3b(pos, w4, byte8, table_idx, flag, u16_a, mode, scale) -> particle or 0.
pub fn spawn_particle_3b(c: &mut Cpu) -> bool {
    clean(c, spawn_particle_3b_body, true)
}

/// The body of `spawn_particle_3b`.
fn spawn_particle_3b_body(c: &mut Cpu) -> bool {
    let (pos, w4, b8) = (c.arg(0), c.arg(1), c.arg(2) as u8);
    let (idx, flag, ha, mode) = (c.arg(3), c.arg(4), c.arg(5) as u16, c.arg(6));
    let scale = c.farg(0);
    c.set_arg(0, 0x3b);
    c.call(ALLOC_PARTICLE);
    let p = c.result();
    if p == 0 {
        c.ret(0);
        return true;
    }
    c.w32(p + 0x30, mode);
    if mode == 1 {
        c.set_arg(0, p + 0x20);
        c.set_arg(1, pos);
        c.set_arg(2, CONST_VEC);
        c.call(VEC_SUB);
    }
    let v = c.rv(pos);
    c.wv(p + 0x10, v);
    c.w32(p + 4, w4);
    c.set_farg(0, F::ONE);
    c.call(FLOAT_TO_FIXED);
    let r = c.result();
    c.w8(p + 9, r.wrapping_add(0x20) as u8);
    c.w8(p + 3, if flag != 0 { 0x44 } else { 0x48 });
    c.w8(p + 8, b8);
    c.w8(p + 1, 0);
    c.wf(p + 0xc, scale * F(0x484d_1400));
    let row = c.r32(0x1b2180 + idx.wrapping_mul(4));
    let t = c.r8(row);
    c.w16(p + 0xa, ha);
    c.w8(p + 2, t);
    ret_with_hi(c, p, v);
    true
}

/// 0x265ae0 spawn_orb_sparkle(pos, mode_f, kind) -> particle or 0: type 0x3e orb sparkle with random alpha.
pub fn spawn_orb_sparkle(c: &mut Cpu) -> bool {
    clean(c, spawn_orb_sparkle_body, true)
}

/// The body of `spawn_orb_sparkle`.
fn spawn_orb_sparkle_body(c: &mut Cpu) -> bool {
    let (pos, seed, kind) = (c.arg(0), c.arg(1), c.arg(2) as i32);
    c.set_arg(0, 0x3e);
    c.call(ALLOC_PARTICLE);
    let p = c.result();
    if p == 0 {
        c.ret(0);
        return true;
    }
    let v = c.rv(pos);
    c.wv(p + 0x10, v);
    c.w32(p + 4, 0x7f7f_4040);
    c.set_farg(0, F(0x4080_0000));
    c.call(FLOAT_TO_FIXED);
    let r = c.result();
    c.w8(p + 1, 0);
    c.w8(p + 3, 0x48);
    c.wf(p + 0xc, F(0x46ea_6000));
    c.w8(p + 9, r.wrapping_add(0x40) as u8);
    c.set_arg(0, 0xff);
    c.call(RAND_BYTE);
    let rb = c.result();
    c.w8(p + 8, rb as u8);
    c.w16(p + 0x22, kind as u16);
    let kind = if kind < 2 { kind } else { 0 };
    let tab = c.r32(0x1b2278);
    let t = c.r8(tab.wrapping_add(kind as u32));
    c.w8(p + 2, t);
    c.set_arg(0, seed);
    c.call(INT_TO_FLOAT);
    let f = c.fresult();
    c.set_farg(0, F(0x42fe_0000) / f);
    c.call(FLOAT_TO_FIXED);
    let r = c.result();
    c.w16(p + 0x20, r as u16);
    c.w16(p + 0xa, seed as u16);
    ret_with_hi(c, p, v);
    true
}

/// 0x265bd8 orb_sparkle_update(p): mode 2 follows the target position + offset; the alpha byte fades by the step
/// at +0x20 and the particle is freed when it runs out.
pub fn orb_sparkle_update(c: &mut Cpu) -> bool {
    let p = c.arg(0);
    let state = p + 0x20;
    let mode = c.r16(state + 2) as i16;
    if mode == 1 {
        return true;
    }
    if mode == 2 {
        for k in 0..3 {
            let target = c.r32(state + 4);
            let v = c.rf(target + 0x10 + 4 * k) + c.rf(state + 0x10 + 4 * k);
            c.wf(p + 0x10 + 4 * k, v);
        }
    }
    let word = c.r32(p + 4);
    let step = c.r16(p + 0x20) as i16 as i32;
    let alpha = (word as i32 >> 24).wrapping_sub(step);
    if alpha > 0 {
        c.w32(p + 4, (word & 0x00ff_ffff) | ((alpha as u32) << 24));
    } else {
        particle_free(c, p);
    }
    true
}

/// 0x268748 spline_point(out, curve, index, u, v): Catmull-Rom style blend of three consecutive curve points,
/// limited to 1.1x the shorter segment.
pub fn spline_point(c: &mut Cpu) -> bool {
    clean(c, spline_point_body, false)
}

/// The body of `spline_point`.
fn spline_point_body(c: &mut Cpu) -> bool {
    let (out, curve, idx) = (c.arg(0), c.arg(1), c.arg(2));
    let (u, v) = (c.farg(0), c.farg(1));
    let sp = c.g(29) as u32;
    let fr = sp.wrapping_sub(0xb0);
    c.s(29, fr as i32 as i64 as u64);
    let mut pts = [0u32; 3];
    for (k, off) in [(0usize, -1i32), (1, 0), (2, 1)] {
        let n = (c.r32(curve) as i32).wrapping_add(idx as i32).wrapping_add(off);
        c.set_arg(0, curve);
        c.set_arg(1, n as u32);
        if !ring_point(c) {
            c.s(29, sp as i32 as i64 as u64);
            return false;
        }
        pts[k] = c.result();
        c.w32(fr + 4 * k as u32, pts[k]);
    }
    let one = F::ONE;
    let half = F(0x3f00_0000);
    let a = one - u;
    let w_back = (a * (one - v)) * half;
    let w_fwd = (a * (v + one)) * half;
    let d01 = dist3(c, pts[0], pts[1]);
    let d12 = dist3(c, pts[1], pts[2]);
    let shorter = if d01 < d12 { d01 } else { d12 };
    let s1 = fr + 0x10;
    c.set_arg(0, s1);
    c.set_arg(1, pts[1]);
    c.set_arg(2, pts[0]);
    c.call(VEC_SUB);
    vec_scale_call(c, s1, s1, w_fwd);
    let s0 = fr + 0x20;
    c.set_arg(0, s0);
    c.set_arg(1, pts[2]);
    c.set_arg(2, pts[1]);
    c.call(VEC_SUB);
    vec_scale_call(c, s0, s0, w_back);
    let s2 = fr + 0x30;
    vec_add_call(c, s2, s1, s0);
    let limit = shorter * F(0x3f8c_cccd);
    c.set_arg(0, s2);
    c.call(VEC_LENGTH);
    let len = c.fresult();
    if limit < len {
        c.set_arg(0, out);
        c.set_arg(1, s2);
        c.set_farg(0, limit);
        c.call(VEC_SET_LENGTH);
    } else {
        let v = c.rv(s2);
        c.wv(out, v);
    }
    c.s(29, sp as i32 as i64 as u64);
    true
}

fn dist3(c: &mut Cpu, a: u32, b: u32) -> F {
    c.set_arg(0, a);
    c.set_arg(1, b);
    c.call(VEC_DIST3);
    c.fresult()
}

/// 0x2590d0 pad_update(pad): polls the pad port once (state at +0x198, port at +0x194), copying the 4+4 pad bytes
/// into +0x180 / +0x184 or resetting the pad block.
pub fn pad_update(c: &mut Cpu) -> bool {
    clean(c, pad_update_body, false)
}

fn pad_update_body(c: &mut Cpu) -> bool {
    let pad = c.arg(0);
    let sp = c.g(29) as u32;
    let buf = sp.wrapping_sub(0x60);
    c.s(29, buf as i32 as i64 as u64);
    c.w32(pad + 0x1a0, 0);
    let (a, b) = (c.r32(pad + 0x1b0), c.r32(pad + 0x348));
    let port = c.r32(pad + 0x194);
    c.w32(pad + 0x1b0, 0);
    c.w32(pad + 0x1ac, b);
    c.w32(pad + 0x1bc, a);
    c.set_arg(0, port);
    c.call(PAD_STATE);
    let state = c.result();
    c.w32(pad + 0x19c, state);
    let reset = |c: &mut Cpu| {
        c.set_arg(0, pad);
        c.call(PAD_RESET);
    };
    if state != 1 {
        reset(c);
        c.w32(pad + 0x198, 0);
        c.w8(pad + 0x189, 0);
    } else {
        let mode = c.r32(pad + 0x198) as i32;
        if mode == 1 {
            let port = c.r32(pad + 0x194);
            c.set_arg(0, port);
            c.set_arg(1, buf);
            c.call(PAD_READ_STATE1);
            let n = c.result();
            c.set_arg(0, pad);
            c.set_arg(1, buf);
            c.set_arg(2, n);
            c.call(PAD_DECODE);
            c.w8(pad + 0x189, 0);
        } else if mode == 0 {
            let port = c.r32(pad + 0x194);
            c.set_arg(0, port);
            c.set_arg(1, buf);
            c.call(PAD_READ_STATE0);
            let n = c.result() as i32;
            let first = c.r32(buf);
            c.w32(pad + 0x1dc, if first == 0xffff_ffff { 0x79 } else { 0 });
            let (n, next_mode) = if n < 5 { (n, 1) } else { (0, 2) };
            copy_pad_bytes(c, buf, pad + 0x180, 0, n.max(0));
            c.w32(pad + 0x198, next_mode);
            copy_pad_bytes(c, buf, pad + 0x180, n, 4);
            let port = c.r32(pad + 0x194);
            c.set_arg(0, port);
            c.set_arg(1, buf);
            c.call(PAD_READ_EXT);
            let m = c.result() as i32;
            let m = if m < 5 { m } else { 0 };
            copy_pad_bytes(c, buf, pad + 0x184, 0, m.max(0));
            copy_pad_bytes(c, buf, pad + 0x184, m, 4);
            reset(c);
            c.w8(pad + 0x189, 0);
        } else if mode == 2 {
            reset(c);
            c.w8(pad + 0x189, 0);
        } else {
            c.w8(pad + 0x189, 0);
        }
    }
    c.w8(pad + 0x188, 0);
    c.s(29, sp as i32 as i64 as u64);
    true
}

/// Copies `buf[i]` to `dst[i]` for i in from..to.
fn copy_pad_bytes(c: &mut Cpu, buf: u32, dst: u32, from: i32, to: i32) {
    for i in from..to {
        let v = c.r8(buf.wrapping_add(i as u32));
        c.w8(dst.wrapping_add(i as u32), v);
    }
}

const WORLD: u32 = 0x13f350;
const TALK_STATE: u32 = 0x179200;
const TALK_TABLE: u32 = 0x13d5bc;
const GP_TALK_HANDLE: i32 = -0x6b50;
const GP_TALK_COOLDOWN: i32 = -0x765c;
const GP_TALK_VALUE: i32 = -0x7e68;
const TALK_LIMIT_A: u32 = 0x15f5a4;
const TALK_LIMIT_B: u32 = 0x15f5ac;
const TIMER: u32 = 0x15ed98;
const PAD_BITS: u32 = 0x13cae4;
const ANGLE_LIMIT: F = F(0x3fc8_f5c3);

/// 0x2585c8 npc_talk(player, npc) -> 1 when a dialogue was started or queued, else 0: range and facing checks,
/// then either the queued dialogue entry (npc table +0x3c, 0x1c bytes each) or the talk-prompt popup.
pub fn npc_talk(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    clean(c, npc_talk_body, true)
}

fn call_f(c: &mut Cpu, t: u32, a: F, b: F) -> F {
    c.set_farg(0, a);
    c.set_farg(1, b);
    c.call(t);
    c.fresult()
}

fn call_i(c: &mut Cpu, t: u32, args: &[u32]) -> u32 {
    for (i, &a) in args.iter().enumerate() {
        c.set_arg(i, a);
    }
    c.call(t);
    c.result()
}

fn npc_talk_body(c: &mut Cpu) -> bool {
    let (player, npc) = (c.arg(0), c.arg(1));
    let sp = c.g(29) as u32;
    let result = |c: &mut Cpu, v: u32| {
        c.ret(v);
        c.s(29, sp as i32 as i64 as u64);
        true
    };
    if c.r32(WORLD + 0x2084) == 0x1d || c.r32(WORLD + 0x22a8) == 0 {
        return result(c, 0);
    }
    let gate = if c.r8(npc + 9) == 0 {
        call_i(c, 0x258a20, &[player, npc]);
        c.r32(TALK_LIMIT_A)
    } else {
        c.r32(gp(c, GP_TALK_COOLDOWN))
    };
    if gate != 0 || npc == 0 || c.r16(npc + 0x36) as i16 == -1 || c.r8(npc + 8) == 0xff {
        return result(c, 0);
    }
    let d = dist3(c, player + 0x10, WORLD + 0x80);
    let range = c.rf(npc + 0xc);
    if range + range < d {
        return result(c, 0);
    }
    call_i(c, 0x258af0, &[player, npc, 0]);
    let kind = c.r8(npc + 8);
    if kind == 0 {
        let (hx, hy) = (c.rf(WORLD + 0x80), c.rf(WORLD + 0x84));
        let (px, py) = (c.rf(player + 0x10), c.rf(player + 0x14));
        let a = call_f(c, 0x1f1e48, hx - px, hy - py);
        let facing = c.rf(player + 0x48);
        let diff = call_f(c, 0x1f2788, a, facing);
        if c.rf(npc + 0xc) < diff {
            return result(c, 0);
        }
        let a = call_f(c, 0x1f1e48, px - hx, py - hy);
        let hero_facing = c.rf(WORLD + 0x98);
        let diff = call_f(c, 0x1f2788, a, hero_facing);
        if ANGLE_LIMIT < diff {
            return result(c, 0);
        }
    }
    if (c.r32(TALK_LIMIT_B) as i32) < c.r32(TALK_STATE + 0x10) as i32 {
        return result(c, 0);
    }
    let handle = gp(c, GP_TALK_HANDLE);
    let queued = kind != 0 || c.r32(PAD_BITS) & 0x10 != 0;
    if queued {
        let idx = call_i(c, 0x258950, &[player]);
        let slot = TALK_TABLE.wrapping_add(idx << 4);
        if c.r32(slot) == 0 {
            c.w32(slot, 1);
        }
        c.w32(TALK_STATE + 0xc, npc);
        c.w32(TALK_STATE + 8, player);
        let entry = c.r32(npc + 0x3c).wrapping_add((c.r16(npc + 0x36) as i16 as i32 as u32).wrapping_mul(0x1c));
        if c.r16(entry + 0x10) & 8 != 0 {
            let ty = c.r16(entry + 8) as i16;
            if ty == 1 {
                let i = c.r16(entry + 0xa) as i16 as i32 as u32;
                let t = c.r32(0x1c4130u32.wrapping_add(i.wrapping_mul(0x18)));
                let v = c.r32(TIMER).wrapping_sub(t);
                let g = gp(c, GP_TALK_VALUE);
                c.w32(g, v);
            } else if ty == 4 {
                let i = c.r16(entry + 0xa) as i16 as i32 as u32;
                c.w8(0x13d388u32.wrapping_add(i), 0);
            }
        }
        let line = c.r16(entry + 4) as i16;
        if line == -1 {
            call_i(c, 0x258af0, &[player, npc, 1]);
            return result(c, 1);
        }
        c.w32(TALK_STATE + 0xc, npc);
        c.w32(TALK_STATE + 8, player);
        let word = c.r16(entry + 4) as u32;
        if word & 0x4000 != 0 {
            call_i(c, 0x289c80, &[(((word ^ 0x4000) << 16) as i32 >> 16) as u32]);
            return result(c, 1);
        }
        let h = c.r32(handle);
        call_i(c, 0x228a48, &[h]);
        c.w32(handle, 0xffff_ffff);
        let l = c.r16(entry + 4) as i16 as i32 as u32;
        call_i(c, 0x289060, &[l]);
        return result(c, 1);
    }
    let h = c.r32(handle);
    if call_i(c, 0x228f58, &[h, 0xa]) != 0 {
        let v = call_i(c, 0x228840, &[0xc, 0, 0x22a2d0, 0x22a320, 0x22a340, 0, 0]);
        c.w32(handle, v);
    }
    let entry = c.r32(npc + 0x3c).wrapping_add((c.r16(npc + 0x36) as i16 as i32 as u32).wrapping_mul(0x1c));
    let ty = c.r16(entry + 8) as i16;
    if ty == 1 || ty == 6 {
        let v = call_i(c, 0x228840, &[2, 0x754e, 0x22ce08, 0x22ce40, 0x22cf38, TIMER, 0x98_967f]);
        let t = call_i(c, 0x1f14b8, &[0x3c]);
        call_i(c, 0x228f58, &[v, t]);
    }
    result(c, 0)
}
