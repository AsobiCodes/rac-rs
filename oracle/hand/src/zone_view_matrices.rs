//! Zone-list lookup, zone shading sample and camera view-clip build (retail overlay code near 0x1e0000 and 0x1e9000).
#![allow(dead_code)]
use crate::h;
use crate::spheres;
use crate::vecmath;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Scratch bytes reserved for the local vector and the flat-zone output words.
const STACK_BYTES: u32 = 0x40;
/// One zone cell is 1/16 of a world unit.
const CELL_SCALE: F = F(0x3d80_0000);
/// Number of cells along each side of the zone grid.
const ZONE_SIDE: i32 = 0x41;
/// Pointer to the zone index table (the table holds, per cell, an offset to a zone list).
const ZONE_TABLE_PTR: u32 = 0x0015_ef80;
/// Zone list entry size and the offset of the first entry from the list header.
const ZONE_ENTRY_STRIDE: u32 = 0x30;
const ZONE_ENTRY_FIRST: u32 = 0x10;
/// Per-zone state records (stride 0x80).
const ZONE_STATE_BASE: u32 = 0x0017_efc0;
/// Retail diagnostic routine and its message string, called when a cell lies outside the grid.
const ZONE_ERROR_FN: u32 = 0x001d_d878;
const ZONE_ERROR_MSG: u32 = 0x001d_95a0;
/// The per-cell test routine that runs on each zone entry within range.
const ZONE_HIT_FN: u32 = 0x1e0b80;
/// Shading output bytes (blend weights) and floats.
const SHADE_BYTE_LO: u32 = 0x0015_f444;
const SHADE_BYTE_MID: u32 = 0x0015_f445;
const SHADE_BYTE_HI: u32 = 0x0015_f446;
const SHADE_FLOAT_A: u32 = 0x0015_f448;
const SHADE_FLOAT_B: u32 = 0x0015_f44c;
const SHADE_FLOAT_C: u32 = 0x0015_f450;
const SHADE_FLOAT_D: u32 = 0x0015_f454;
/// 255.0 and 1024.0.
const K255: F = F(0x437f_0000);
const K1024: F = F(0x4480_0000);

/// Camera view build: one-time init flag, initial 3-quad source block, and the matrices.
const VIEW_INIT_FLAG: u32 = 0x0016_c16c;
const VIEW_INIT_SRC: u32 = 0x0016_70d0;
const VIEW_BASE: u32 = 0x0016_6d80;
const VIEW_TRANSFORMS: [(u32, u32); 3] = [(0x150, 0x0012_5360), (0x154, 0x0012_5408), (0x158, 0x0012_52b8)];
const VIEW_INIT_FN: u32 = 0x0012_5218;
const VIEW_MAT_A: u32 = 0x0016_cc00;
const VIEW_OUT: u32 = 0x0016_cdc0;

/// 0x1e0a18 zone_hit_entry(pos, radius) -> entry or 0: first zone-list entry whose cell is near `pos` within `radius`.
pub fn zone_hit_entry(c: &mut Cpu) -> bool {
    let pos = c.arg(0);
    let radius = c.farg(0);
    let hit = h::with_stack(c, STACK_BYTES, |c, buf| find_hit(c, pos, radius, buf));
    c.ret(hit);
    true
}

fn find_hit(c: &mut Cpu, pos: u32, radius: F, buf: u32) -> u32 {
    let v = c.rv(pos);
    c.wv(buf, v);
    let cx = (c.rf(buf) * CELL_SCALE).to_int();
    let cy = (c.rf(buf + 4) * CELL_SCALE).to_int();
    if !(0..ZONE_SIDE).contains(&cx) || !(0..ZONE_SIDE).contains(&cy) {
        h::call(c, ZONE_ERROR_FN, &[ZONE_ERROR_MSG], &[]);
        return 0;
    }
    let table = c.r32(ZONE_TABLE_PTR);
    let slot = table + 4 * ((cy * 64 + cx) as u32);
    let raw = c.r32(slot);
    if raw == 0 {
        return 0;
    }
    let list = raw.wrapping_add(table);
    let count = c.r32(list) as i32;
    c.wf(buf + 0x1c, F::ONE);
    for k in 0..count.max(0) as u32 {
        let entry = list + ZONE_ENTRY_FIRST + ZONE_ENTRY_STRIDE * k;
        h::run(c, vecmath::vec_sub, &[buf + 0x10, entry, buf], &[]);
        let dist = h::run(c, vecmath::length_xy, &[buf + 0x10], &[]).1;
        let limit = c.rf(entry + 0xc) + radius;
        if dist < limit {
            let r = h::call(c, ZONE_HIT_FN, &[buf, entry], &[radius]).0;
            if r != 0 {
                // Retail sets v0 = entry in the delay slot of the `bnez`, so the entry is returned, not r.
                return entry;
            }
        }
    }
    0
}

/// 0x1e0f20 zone_shade_sample(src, t) -> k: finds the flat zone for `src` and writes its blended shade bytes and
/// floats; returns k = (int)(255 * frac), or 0 when there is no zone or its flag bit 2 is clear.
pub fn zone_shade_sample(_c: &mut Cpu) -> bool {
    // Not merged: the Rust port leaves one RAM byte different on 3/300 checked calls (t near 1.0; the
    // difference is in the frac that flat_zone_hit stores, outside this batch). Retail runs this function.
    false
}

#[allow(dead_code)]
pub fn zone_shade_sample_rust(c: &mut Cpu) -> bool {
    let src = c.arg(0);
    let t = c.farg(0);
    let k = h::with_stack(c, STACK_BYTES, |c, buf| shade_sample(c, src, t, buf));
    c.ret(k as u32);
    true
}

fn shade_sample(c: &mut Cpu, src: u32, t: F, buf: u32) -> i32 {
    // Retail frame saves: s0 quadword at sp+0x10, ra quadword at sp+0x20, f20 word at sp+0x30 (buf == retail sp).
    let (s0, ra) = (c.r[16], c.r[31]);
    for i in 0..4 {
        c.w32(buf + 0x10 + 4 * i, (s0 >> (32 * i)) as u32);
        c.w32(buf + 0x20 + 4 * i, (ra >> (32 * i)) as u32);
    }
    c.w32(buf + 0x30, c.f[20]);
    if h::run(c, spheres::flat_zone_hit, &[src, buf, buf + 4], &[t]).0 == 0 {
        return 0;
    }
    let zone = c.r32(buf + 4);
    let state = ZONE_STATE_BASE.wrapping_add(zone << 7);
    if c.r32(state + 0x50) & 2 == 0 {
        return 0;
    }
    let frac = c.rf(buf);
    let k = (frac * K255).to_int();
    let k_inv = 255 - k;
    let w_inv = F::ONE - frac;
    let f2 = c.rf(state + 0x78) * frac + c.rf(state + 0x68) * w_inv;
    let f5 = c.rf(state + 0x70) * frac + c.rf(state + 0x60) * w_inv;
    let f6 = c.rf(state + 0x6c) * frac + c.rf(state + 0x5c) * w_inv;
    let f3 = c.rf(state + 0x74) * frac + c.rf(state + 0x64) * w_inv;
    c.wf(SHADE_FLOAT_A, f6 * K1024);
    c.wf(SHADE_FLOAT_B, f3 * K1024);
    c.wf(SHADE_FLOAT_C, K255 - f5 * K255);
    c.wf(SHADE_FLOAT_D, K255 - f2 * K255);

    let a = c.r32(state + 0x58);
    let b = c.r32(state + 0x54);
    let channel = |w: u32, shift: u32| ((w >> shift) & 0xff) as i32;
    let blend = |ca: i32, cb: i32| ((ca * k + cb * k_inv) >> 8) as u8;
    c.w8(SHADE_BYTE_HI, blend(channel(a, 16), channel(b, 16)));
    c.w8(SHADE_BYTE_LO, blend(channel(a, 0), channel(b, 0)));
    c.w8(SHADE_BYTE_MID, blend(channel(a, 8), channel(b, 8)));
    k
}

/// 0x1e96b0 build_view_clip(): rebuilds the camera view matrix at 0x16cdc0 from the view block at 0x166d80 and the
/// per-frame transform matrices (first call copies the initial block and runs the one-time setup).
pub fn build_view_clip(c: &mut Cpu) -> bool {
    h::with_stack(c, STACK_BYTES, |c, buf| build_view(c, buf));
    true
}

fn build_view(c: &mut Cpu, buf: u32) {
    if c.r32(VIEW_INIT_FLAG) == 0 {
        for i in 0..3 {
            let v = c.rv(VIEW_INIT_SRC + 0x10 * i);
            c.wv(buf + 0x10 * i, v);
        }
    } else {
        let arg = c.farg(0);
        h::call(c, VIEW_INIT_FN, &[buf], &[arg]);
        for (off, func) in VIEW_TRANSFORMS {
            let f = c.rf(VIEW_BASE + off);
            h::call(c, func, &[buf, buf], &[f]);
        }
    }

    let base = VIEW_BASE;
    let mat_a = VIEW_MAT_A;
    let tables = mat_a - 0xc0;
    let mat_c = mat_a + 0x80;
    let vec_out = base + 0x100;

    let neg_y = -c.rf(buf + 0x10);
    c.wf(base + 0x3c, F::ONE);
    c.wf(base, neg_y);
    let stores = [
        (base + 0x10, -c.rf(buf + 0x14)),
        (base + 0x20, -c.rf(buf + 0x18)),
        (base + 4, -c.rf(buf + 0x20)),
        (base + 0x14, -c.rf(buf + 0x24)),
        (base + 0x24, -c.rf(buf + 0x28)),
        (base + 8, c.rf(buf)),
        (base + 0x18, c.rf(buf + 4)),
        (base + 0x28, c.rf(buf + 8)),
    ];
    for (addr, v) in stores {
        c.wf(addr, v);
    }
    for off in [0x30, 0x34, 0x38, 0xc, 0x1c, 0x2c] {
        c.w32(base + off, 0);
    }
    h::run(c, vecmath::mat4_mul, &[base + 0x40, mat_a, base], &[]);
    h::run(c, vecmath::mat4_mul, &[base + 0x80, mat_a + 0x40, base], &[]);

    // Scale the view-matrix columns by the per-frame factors at 0x1a0/0x1a4/0x1a8 and accumulate.
    let ka = c.rf(tables + 0x1a0);
    let kb = c.rf(tables + 0x1a4);
    let kc = c.rf(tables + 0x1a8);
    let m8c = c.rf(base + 0x8c);
    let m9c = c.rf(base + 0x9c);
    let mac = c.rf(base + 0xac);
    let mbc = c.rf(base + 0xbc);
    let adds = [
        (0x80, m8c * ka),
        (0x84, m8c * kb),
        (0x88, m8c * kc),
        (0x90, m9c * ka),
        (0x94, m9c * kb),
        (0x98, m9c * kc),
        (0xa0, mac * ka),
        (0xa4, mac * kb),
        (0xa8, mac * kc),
        (0xb0, mbc * ka),
        (0xb4, mbc * kb),
        (0xb8, mbc * kc),
    ];
    for (off, p) in adds {
        let sum = c.rf(base + off) + p;
        c.wf(base + off, sum);
    }
    h::run(c, vecmath::mat4_mul, &[base + 0xc0, mat_c, base], &[]);

    let kd = c.rf(tables + 0x1c0);
    h::run(c, vecmath::vec4_scale, &[vec_out, mat_c], &[kd]);
    h::run(c, vecmath::vec4_scale, &[base + 0x110, mat_a + 0x90], &[kd]);
    let row = c.rv(mat_a + 0xa0);
    c.wv(base + 0x120, row);
    let row = c.rv(mat_a + 0xb0);
    c.wv(base + 0x130, row);
    h::run(c, vecmath::mat4_mul, &[vec_out, vec_out, base], &[]);

    for off in [0, 0x10, 0x20] {
        let v = c.rv(base + off);
        c.wv(VIEW_OUT + off, v);
    }
    h::run(c, vecmath::vec_scale, &[VIEW_OUT + 0x30, base + 0x140], &[K1024]);
    c.wf(VIEW_OUT + 0x3c, K1024);
}
