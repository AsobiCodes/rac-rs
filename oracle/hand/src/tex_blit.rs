//! Texture upload and blit (retail 0x1f7770..0x1f7854) and the texture slot lookup (retail 0x1f7ba0..0x1f7c04).

use crate::fxtex;
use crate::h;
use crate::ps2::*;
use crate::sprite_text;
use ee_oracle::cpu::Cpu;

const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;
const T0: usize = 8;
const T1: usize = 9;
const T2: usize = 10;
const S0: usize = 16;
const S1: usize = 17;
const S2: usize = 18;
const RA: usize = 31;

/// Base of the upper half of the lui 0x18 address pair: the state block is at STATE (0x180000 - 0x6af0).
const LUI_BASE: u32 = 0x0018_0000;
const STATE: u32 = LUI_BASE - 0x6af0;
/// Retail setup routine called first with four rect corners.
const RECT_SETUP: u32 = 0x001e_d6c0;
/// Texture table of 150 halfwords (0x180000 - 0x6ab0), searched by `find_tex_slot`.
const SLOT_TABLE: u32 = LUI_BASE - 0x6ab0;
const SLOT_COUNT: u32 = 0x96;

fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// 0x1f7770 tex_blit_slot() -> (): resets the two state rect heights to 0x20, sets up the rects, uploads the texture
/// at the size picked by the state mode, and blits the 64x64 tile with the packed colour word.
pub fn tex_blit_slot(c: &mut Cpu) -> bool {
    let saves = [(S0, 0x10, h::SQ), (S1, 0x20, h::SQ), (S2, 0x30, h::SQ), (RA, 0x40, h::SQ)];
    h::frame(c, 0x50, &saves, |c, sp| {
        c.w32(STATE + 0x1c, 0x20);
        c.w32(STATE + 0x18, 0x20);
        set_gpr(c, V0, 0x20);
        let x = c.r32(STATE + 0x14);
        let y = c.r32(STATE + 0x10);
        let rect = [x.wrapping_sub(0x20), x.wrapping_add(0x20), y.wrapping_sub(0x20), y.wrapping_add(0x20)];
        h::call_keep(c, RECT_SETUP, &[rect[0], rect[1], rect[2], rect[3], 0x60], &[]);

        // Size: modes 1 and 7 scale the stored factor by 0x15, any other mode gives 0x7e; capped at 0x80.
        let mode = c.r32(STATE);
        let mut size = 0x7e;
        if mode == 1 || mode == 7 {
            size = (c.r32(STATE + 4) as i32).wrapping_mul(0x15) as u32;
        }
        let size = if (size as i32) < 0x81 { size } else { 0x80 };

        set_gpr(c, V0, 0x80);
        h::run_keep(c, fxtex::fx_tex_upload, &[4], &[]);
        c.mem.wr64(sp + 8, c.r[V0] as u64);
        let packed = ((size << 24) as i32 as i64 as u64) | 0x0080_8080;
        c.mem.wr64(sp, packed);

        let x = c.r32(STATE + 0x14).wrapping_sub(0x20);
        let y = c.r32(STATE + 0x10).wrapping_sub(0x20);
        h::run_keep(c, sprite_text::sprite_blit_int, &[y, x, 0x40, 0x40, 0, 0, 0x40, 0x40], &[]);
    });
    true
}

/// 0x1f7ba0 find_tex_slot(key, sel, out) -> index or -1: scans up to 150 halfword slots from SLOT_TABLE + 2*sel (stride 4)
/// for the signed key; on a hit with out != 0 stores the halfword at the matching column to *out.
pub fn find_tex_slot(c: &mut Cpu) -> bool {
    let key = c.g(A0) as u32 as i16 as i32;
    let sel = c.g(A1) as u32;
    let out = c.g(A2) as u32;
    let start = SLOT_TABLE.wrapping_add(sel << 1);

    let mut hit = None;
    for t1 in 0..SLOT_COUNT {
        let t0 = start.wrapping_add(4 * t1);
        if (c.r16(t0) as i16 as i32) == key {
            hit = Some(t1);
            break;
        }
    }

    match hit {
        Some(index) => {
            let shifted = index << 2;
            let column = if sel != 0 { shifted } else { 2 + shifted };
            let mut v1 = shifted;
            if out != 0 {
                v1 = c.r16(SLOT_TABLE.wrapping_add(column)) as u32;
                c.w16(out, v1 as u16);
            }
            set_gpr(c, V0, index);
            set_gpr(c, V1, v1);
            set_gpr(c, A3, column);
            set_gpr(c, T0, start.wrapping_add(4 * index));
            set_gpr(c, T1, index);
        }
        None => {
            set_gpr(c, V0, u32::MAX);
            set_gpr(c, V1, (SLOT_COUNT - 1) << 2);
            set_gpr(c, A3, 2 + 4 * SLOT_COUNT);
            set_gpr(c, T0, start.wrapping_add(4 * SLOT_COUNT));
            set_gpr(c, T1, SLOT_COUNT);
        }
    }
    set_gpr(c, A0, key as u32);
    set_gpr(c, T2, SLOT_TABLE);
    true
}
