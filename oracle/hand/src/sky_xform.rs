//! Sky vertex transform: the VU0 pipeline of retail 0x27c9d4.

use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

const AT: usize = 1;
const V0: usize = 2;
const V1: usize = 3;
const T6: usize = 14;

/// `lui $at, 0x16; addiu $at, 0x6d80`: the rows VF11..VF14 are read at +0x100, +0x110, +0x120, +0x130.
const ROWS_BASE: u32 = 0x0016_6d80;
/// `lui $at, 0x1c; addiu $at, -0x2720`: the matrix columns VF1..VF4 at +0, +0x10, +0x20, +0x30.
const COLS_BASE: u32 = 0x001b_d8e0;
/// `lui $v0, 0x17; addiu $v0, -0x34c0`: VF26 at +0x190 (screen scale) and VF27 at +0x1a0 (screen offset).
const SCREEN_BASE: u32 = 0x0016_cb40;
/// `lui $at, 0x3e80`: 0.25 as f32 bits; VF5 is loaded from it, and its x lane scales VF26.x and VF26.y.
const QUARTER: u64 = 0x3e80_0000;
/// VU0 mask for the xy lanes.
const XY: u32 = X | Y;

fn lh_to(c: &mut Cpu, r: usize, a: u32) {
    let v = c.r16(a) as i16 as i64 as u64;
    c.s(r, v);
}

/// PPACW rd, rs, rt: rd = [rt.w0, rt.w2, rs.w0, rs.w2] (32-bit words, low first).
fn ppacw(rs: u128, rt: u128) -> u128 {
    let w = |x: u128, i: u32| (x >> (32 * i)) & 0xffff_ffff;
    w(rt, 0) | (w(rt, 2) << 32) | (w(rs, 0) << 64) | (w(rs, 2) << 96)
}

/// The 128 bits of a GPR as a VU register (qmtc2).
fn quad(r: u128) -> V {
    V([r as u32, (r >> 32) as u32, (r >> 64) as u32, (r >> 96) as u32])
}

/// Unpacks the three shorts at `a` the way retail does (lh, ppacw, lh, ppacw, qmtc2, vitof0.xyz) and returns VF11.
fn unpack_vertex(c: &mut Cpu, a: u32) -> V {
    lh_to(c, AT, a);
    lh_to(c, V0, a + 2);
    c.r[AT] = ppacw(c.r[V0], c.r[AT]);
    lh_to(c, V0, a + 4);
    c.r[AT] = ppacw(c.r[V0], c.r[AT]);
    let raw = quad(c.r[AT]);
    raw.with(raw.itof(0), XYZ)
}

/// VU0 product `m * v` (the four VMUL/VMADD steps): returns (the result, ACC before the last add).
fn transform(m: &[V; 4], v: V, w: F) -> (V, V) {
    let acc = m[0] * v.x();
    let acc = V::madd(acc, m[1], V::splat(v.y()));
    let acc = V::madd(acc, m[2], V::splat(v.z()));
    (V::madd(acc, m[3], V::splat(w)), acc)
}

/// 0x27c9d4 sky_vertex_xform(src, dst, count) -> flags: transforms count packed xyz shorts (8 bytes each) by the
/// VU0 matrix, divides by w, writes a 16-byte screen record per vertex (flags byte at +15), returns the AND of the flags.
pub fn sky_vertex_xform(c: &mut Cpu) -> bool {
    let src = c.arg(0);
    let dst = c.arg(1);
    let count = c.arg(2);

    let rows = [
        c.rv(ROWS_BASE + 0x100),
        c.rv(ROWS_BASE + 0x110),
        c.rv(ROWS_BASE + 0x120),
        c.rv(ROWS_BASE + 0x130),
    ];
    let mut m = [V::default(); 4];
    for (i, col) in m.iter_mut().enumerate() {
        let v = c.rv(COLS_BASE + 0x10 * i as u32);
        *col = transform(&rows, v, v.w()).0;
    }

    // VF5 = 0.25 (all of $at goes to VF5); VF26.xy are scaled by it, VF27 is loop-invariant.
    c.s(AT, QUARTER);
    let vf5 = quad(c.r[AT]);
    let vf26 = {
        let a = c.rv(SCREEN_BASE + 0x190);
        a.with(a * vf5.x(), XY)
    };
    let vf27 = c.rv(SCREEN_BASE + 0x1a0);

    let mut vf15 = V::ZERO_W1 * F::ZERO;
    let mut vf11 = unpack_vertex(c, src);
    let (mut vf10, mut acc) = transform(&m, vf11, F::ONE);
    let mut vf14 = V::default();
    let mut t6: u64 = 0x3f;
    c.s(T6, t6);

    let (mut a0, mut a1, mut a2) = (src, dst, count);
    let mut v1: u64 = 0;
    loop {
        // vclipw on the current vertex; the clip flags are read back (cfc2, andi 0x2f) further down.
        let clip = (vecmath::clip_bits(vf10) & 0x2f) as u64;
        let q = F::ONE.q_div(vf10.w());
        a0 = a0.wrapping_add(8);
        c.set_arg(0, a0);
        a2 = a2.wrapping_sub(1);
        c.set_arg(2, a2);
        v1 = c.r8(a0.wrapping_sub(2)) as u64;
        c.s(V1, v1);
        vf11 = unpack_vertex(c, a0);

        // vmulq.xyz (divide by w), vmula ACC = VF10 * VF26, vmaddw VF14 = ACC + VF27.
        let scaled = vf10.with(vf10 * q, XYZ);
        let screen = V::madd(scaled * vf26, vf27, V::splat(F::ONE));
        vf14 = screen;
        c.s(AT, clip);
        vf15 = vf15.with(vf14.ftoi(4), XYZ);

        let next = transform(&m, vf11, F::ONE);
        vf10 = next.0;
        acc = next.1;

        c.wv(a1, vf15);
        c.w8(a1.wrapping_add(12), v1 as u8);
        t6 &= clip;
        c.w8(a1.wrapping_add(15), clip as u8);
        a1 = a1.wrapping_add(16);
        c.set_arg(1, a1);
        if a2 == 0 {
            break;
        }
    }

    c.s(T6, t6);
    c.ret(t6 as u32);
    for (i, col) in m.iter().enumerate() {
        c.set_vf(1 + i, *col);
    }
    c.set_vf(5, vf5);
    c.set_vf(10, vf10);
    c.set_vf(11, vf11);
    c.set_vf(12, rows[1]);
    c.set_vf(13, rows[2]);
    c.set_vf(14, vf14);
    c.set_vf(15, vf15);
    c.set_vf(26, vf26);
    c.set_vf(27, vf27);
    c.set_acc(acc);
    true
}
