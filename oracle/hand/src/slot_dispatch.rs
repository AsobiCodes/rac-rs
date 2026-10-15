//! Texture-slot upload and fill helpers (EE, retail overlay rec/l4_01).

use crate::h;
use crate::ps2::{Ee, F};
use ee_oracle::cpu::Cpu;

/// Retail `fx_tex_upload`, called raw so its scratch registers match retail.
const FX_TEX_UPLOAD: u32 = 0x1ebb00;
/// Retail helper taking the upload result in t0 (table at 0x1c3250 for slot 1).
const UPLOAD_TAIL: u32 = 0x1ee4e0;
/// Table pointers loaded into t1 for the two slot variants.
const SLOT1_TABLE: u64 = 0x1c_3250;
const SLOT2_TABLE: u64 = 0x1c_35f0;
/// Retail frame size of the three helpers.
const FRAME: u32 = 0x50;
/// Retail callees used by `fill_pair_slots`.
const PAIR_SETUP: u32 = 0x1f_41c0;
const PAIR_BITS: u32 = 0x1e_a808;
const PAIR_FILL: u32 = 0x29_13c8;
/// Word and halfword the pair-slot size is read from.
const SHIFT_WORD: u32 = 0x15_EE8C;
const SHIFT_HALF: u32 = 0x15_18EE;
/// Float bit patterns passed to `PAIR_BITS` in f14 and f15.
const F14_BITS: u32 = 0x4900_0000;
const F15_BITS: u32 = 0x437f_0000;
/// Fill command words sent to `PAIR_FILL`.
const FILL_CMD_A: u32 = 0x47;
const FILL_CMD_B: u32 = 0x42;
const FILL_ARG_HI: u64 = 0x8000_0000_44;

fn set32(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// Shared body of the two upload variants: `fx_tex_upload(slot)` returns the 64-bit TEX0 word in v0, which retail copies
/// whole into t0 for the tail. Argument registers are restored from the entry values before the tail runs.
fn upload_then_tail(c: &mut Cpu, slot: u64, table: u64) {
    let args = [c.r[4], c.r[5], c.r[6], c.r[7]];
    for (i, a) in args.iter().enumerate() {
        c.r[16 + i] = *a;
    }
    c.s(4, slot);
    h::call_keep(c, FX_TEX_UPLOAD, &[], &[]);
    c.s(8, c.r[2] as u64);
    c.s(9, table);
    for (i, a) in args.iter().enumerate() {
        c.r[4 + i] = *a;
    }
    h::call_keep(c, UPLOAD_TAIL, &[], &[]);
}

/// 0x1ee9d0 upload_then_fill_1(a0..a3) -> v0: uploads texture slot 1, then runs the fill tail with table 0x1c3250.
pub fn upload_then_fill_1(c: &mut Cpu) -> bool {
    h::frame(c, FRAME, &saves(), |c, _sp| upload_then_tail(c, 1, SLOT1_TABLE));
    true
}

/// 0x1eea40 upload_then_fill_2(a0..a3) -> v0: uploads texture slot 2, then runs the fill tail with table 0x1c35f0.
pub fn upload_then_fill_2(c: &mut Cpu) -> bool {
    h::frame(c, FRAME, &saves(), |c, _sp| upload_then_tail(c, 2, SLOT2_TABLE));
    true
}

fn saves() -> [(usize, u32, u8); 5] {
    [(16, 0x0, h::SQ), (17, 0x10, h::SQ), (18, 0x20, h::SQ), (19, 0x30, h::SQ), (31, 0x40, h::SQ)]
}

/// 0x1eeec0 fill_pair_slots(a0 slot, a1 level, a2 flag, f12 scale): sets up the slot bits and sends the fill commands.
pub fn fill_pair_slots(c: &mut Cpu) -> bool {
    let fscale = c.farg(0);
    let saves = [(16, 0x0, h::SQ), (17, 0x10, h::SQ), (18, 0x20, h::SQ), (31, 0x30, h::SQ)];
    h::frame(c, FRAME, &saves, |c, sp| {
        c.w32(sp.wrapping_add(0x40), fscale.0);
        let (s1, s0, s2) = (c.arg(0), c.arg(1), c.arg(2));
        let shift = if s2 == 0 {
            let sum = s0.wrapping_add(s1) as i32;
            let idx = if sum < 0x11 { sum as u32 } else { 0x10 };
            let base = c.r32(SHIFT_WORD) as i32 as u32;
            let rest = base.wrapping_sub(4u32 << (idx & 31));
            ((rest as i32) >> 13) as u32
        } else {
            c.r16(SHIFT_HALF) as i16 as i32 as u32
        };
        let bits_s0 = 1u32 << (s0 & 31);
        let bits_s1 = 1u32 << (s1 & 31);
        set32(c, 4, s1);
        set32(c, 5, s0);
        set32(c, 6, shift << 13);
        c.call(PAIR_SETUP);
        set32(c, 4, bits_s1);
        set32(c, 5, bits_s0);
        c.set_farg(0, fscale);
        c.set_farg(1, F::ZERO);
        c.set_farg(2, F(F14_BITS));
        c.set_farg(3, F(F15_BITS));
        c.f[16] = F::ZERO.0;
        c.call(PAIR_BITS);
        set32(c, 4, FILL_CMD_A);
        set32(c, 5, if s2 == 0 { 0x3_0000 } else { 0 });
        c.call(PAIR_FILL);
        set32(c, 4, FILL_CMD_B);
        c.s(5, FILL_ARG_HI);
        c.call(PAIR_FILL);
    });
    true
}
