//! Warp-effect quad builders: the big warp quad and the banner quad drawn during the warp transition.
use crate::h;
use crate::mathx;
use crate::ps2::*;
use crate::sprite_text;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Retail helper at 0x2913c8: `(tag, value)` packet/command call made before and after the quad work.
const HELPER: u32 = 0x2913_c8;
/// First helper call tag (both quads).
const TAG_OPEN: u32 = 0x47;
/// Second helper tag used by the banner quad.
const TAG_BANNER: u32 = 0x42;
/// Second helper value (`lui 5; ori 0x360b`) used to close both quads.
const VALUE_CLOSE: u32 = 0x5_360b;
/// First helper value (`lui 3; ori 0x1801`) used to open both quads.
const VALUE_OPEN: u32 = 0x3_1801;
/// Helper value for the banner tag: `0x8000 << 24 | 0x44` (a full 64-bit argument).
const VALUE_BANNER: u64 = 0x80_0000_0044;
/// Quadword source copied to the stack vertex area (0x1bd9b0).
const COPY_SRC: u32 = 0x1c_0000 - 0x2650;
/// Byte count for the quadword copy.
const COPY_BYTES: u32 = 0x20;
/// Warp scale table: entry `i` is the float at `SCALE_TABLE + 4 * i`, valid for `i < SCALE_COUNT`.
const SCALE_TABLE: u32 = 0x1c_0000 - 0x2370;
/// Index of the warp stage (signed word at `0x160000 - 0x127c`).
const SCALE_INDEX: u32 = 0x16_0000 - 0x127c;
/// Number of valid warp scale entries.
const SCALE_COUNT: u64 = 0x13;
/// Vertex source vectors (four 16-byte entries at 0x1bdc50).
const VERTEX_SRC: u32 = 0x1c_0000 - 0x23b0;
/// Number of quad vertices processed in the big quad.
const VERTEX_COUNT: u32 = 4;
/// Packed white colour word stored per vertex.
const WHITE: u32 = 0x8080_8080;
/// Constant GP-relative pointer offset passed to the transform helper.
const GP_XFORM: u32 = 0x66d0;
/// Transform helper at 0x1f1810 (in place on a 16-byte vector).
const XFORM: u32 = 0x1f_1810;
/// Stack clear helper at 0x1ef7c0.
const CLEAR: u32 = 0x1e_f7c0;
/// Banner blit source word (`0x140000 - 0x1afc`).
const BANNER_SRC: u32 = 0x14_0000 - 0x1afc;
/// Banner blit 64-bit parameter (`0x160000 + 0x5c8`).
const BANNER_PARAM: u32 = 0x16_0000 + 0x5c8;
/// Big-quad 64-bit parameter (`0x160000 + 0x5c0`).
const BIG_PARAM: u32 = 0x16_0000 + 0x5c0;
/// Starting value of the scale when the stage index is out of range (1.0).
const ONE: u32 = 0x3f80_0000;

fn get64(c: &mut Cpu, a: u32) -> u64 {
    let lo = c.r32(a) as u64;
    let hi = c.r32(a.wrapping_add(4)) as u64;
    lo | (hi << 32)
}

fn put64(c: &mut Cpu, a: u32, v: u64) {
    c.w32(a, v as u32);
    c.w32(a.wrapping_add(4), (v >> 32) as u32);
}

/// Calls retail `addr` with `a0` and a full 64-bit `a1` (as `h::call` does, with the wide argument), then restores the
/// registers.
fn call_wide(c: &mut Cpu, addr: u32, a0: u32, a1: u64) -> (u32, F) {
    let (r, f) = (c.r, c.f);
    c.set_arg(0, a0);
    c.s(5, a1);
    c.call(addr);
    let ret = (c.result(), c.fresult());
    c.r = r;
    c.f = f;
    ret
}

/// 0x27ff00 warp_big_quad(): builds four scaled vertices from the warp table on the stack, runs the transform on each
/// and closes the packet.
pub fn warp_big_quad(c: &mut Cpu) -> bool {
    let saves = [(31, 0xe0, h::SQ), (20, 0xd0, h::SQ), (19, 0xc0, h::SQ), (18, 0xb0, h::SQ), (17, 0xa0, h::SQ),
        (16, 0x90, h::SQ)];
    h::frame(c, 0x100, &saves, |c, sp| {
        // swc1 $f20 keeps the caller's value for the epilogue reload.
        c.wf(sp + 0xf0, F(c.f[20]));
        h::call(c, HELPER, &[TAG_OPEN, VALUE_OPEN], &[]);
        let big = get64(c, BIG_PARAM);
        put64(c, sp + 0x78, big);
        put64(c, sp + 0x80, 0xff90_0000_0260);
        put64(c, sp + 0x88, 0x0080_0000_0044);
        put64(c, sp + 0x70, 0);
        h::run(c, mathx::copy_qwords, &[sp + 0x50, COPY_SRC, COPY_BYTES], &[]);
        let index = c.r32(SCALE_INDEX) as i32 as i64 as u64;
        let scale = if index < SCALE_COUNT {
            c.rf(SCALE_TABLE + 4 * index as u32)
        } else {
            F(ONE)
        };
        let gp = c.g(28) as u32;
        for i in 0..VERTEX_COUNT {
            let out = sp + 16 * i;
            c.w32(sp + 0x40 + 4 * i, WHITE);
            h::run(c, vecmath::vec_scale, &[out, VERTEX_SRC + 16 * i], &[scale]);
            h::call(c, XFORM, &[out, out, gp.wrapping_sub(GP_XFORM)], &[]);
        }
        h::call(c, CLEAR, &[sp, 0, 0], &[]);
        let (v0, f0) = h::call(c, HELPER, &[TAG_OPEN, VALUE_CLOSE], &[]);
        c.ret(v0);
        c.fret(f0);
    });
    true
}

/// 0x280040 warp_banner_quad(fade): draws the warp banner sprite with its alpha word built from `fade`, then closes
/// the packet.
pub fn warp_banner_quad(c: &mut Cpu) -> bool {
    let fade = c.arg(0);
    let saves = [(16, 0x10, h::SQ), (31, 0x20, h::SQ)];
    h::frame(c, 0x30, &saves, |c, sp| {
        h::call(c, HELPER, &[TAG_OPEN, VALUE_OPEN], &[]);
        call_wide(c, HELPER, TAG_BANNER, VALUE_BANNER);
        let src = c.r32(BANNER_SRC);
        let param = get64(c, BANNER_PARAM);
        let s0 = ((fade << 24) as i32) as i64 as u64;
        put64(c, sp, s0 | 0x0080_8080);
        put64(c, sp + 8, param);
        h::run(c, sprite_text::sprite_blit_int, &[0x20, src.wrapping_sub(0x58), 0x100, 0x20, 0, 0, 0x100, 0x20], &[]);
        let (v0, f0) = h::call(c, HELPER, &[TAG_OPEN, VALUE_CLOSE], &[]);
        c.ret(v0);
        c.fret(f0);
    });
    true
}
