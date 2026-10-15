//! Shop price text and offscreen/DMA frame setup (Eudora overlay addresses).
use crate::h;
use crate::dma_setup;
use crate::proj_init;
use crate::proj_setup;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const RA: usize = 31;
const S0: usize = 16;
const S1: usize = 17;
const A1: usize = 5;

/// printf-style formatter the price text calls (retail).
const FORMAT: u32 = 0x0011_6248;
/// Format strings in the overlay data (the code loads their addresses as immediates).
const FMT_PLAIN: u32 = 0x0016_10e0;
const FMT_GROUPED: u32 = 0x0016_10d8;
const THOUSAND: i32 = 1000;

/// Retail 0x1f41c0: called with the two shifted args and an aligned buffer base (retail).
const BUFFER_SETUP: u32 = 0x001f_41c0;
/// Retail 0x2913c8: one-register-pair write (retail), called twice.
const REG_WRITE: u32 = 0x0029_13c8;
/// Retail 0x1ea1e8: last call of the DMA/projection setup (retail).
const PROJ_TAIL: u32 = 0x001e_a1e8;

/// Top of the aligned buffer range: lui 0x3f, ori 0xf000.
const BUFFER_TOP: u32 = 0x003f_f000;
/// Buffer sizes are 4 << min(a0 + a1, 16) bytes, aligned down to 8 KB (shift 13).
const ALIGN_SHIFT: u32 = 13;
const SIZE_BASE: u32 = 4;
const SIZE_CLAMP: i32 = 17;

/// Float constants 524288.0 and 255.0 (bits from the retail lui pairs).
const F_524288: u32 = 0x4900_0000;
const F_255: u32 = 0x437f_0000;

/// Arguments of the two REG_WRITE calls.
const WRITE_A_ID: u32 = 0x47;
const WRITE_A_VAL: u32 = 0x0003_0000;
const WRITE_B_ID: u32 = 0x42;
const WRITE_B_VAL: u128 = 0x80_0000_0044;

/// Frame slot where retail saves f20 (swc1 f20, 0x30(sp)).
const F20_SLOT: u32 = 0x30;

/// 0x28e660 shop_price_format(buf, price) -> v0: formats a shop price into buf: "%d" below 1000, else "%d,%03d"
pub fn shop_price_format(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let buf = c.arg(0);
    let price = c.arg(1) as i32;
    h::frame(c, 0x10, &[(RA, 0x0, h::SQ)], |c, _sp| {
        if price < THOUSAND {
            h::call_keep(c, FORMAT, &[buf, FMT_PLAIN, price as u32], &[]);
        } else {
            let (thousands, rest) = (price / THOUSAND, price % THOUSAND);
            h::call_keep(c, FORMAT, &[buf, FMT_GROUPED, thousands as u32, rest as u32], &[]);
        }
    });
    true
}

/// 0x28faf0 offscreen_target_init(a0, a1, f12) -> none: aligned buffer base from a0 + a1, then offscreen setup
/// with 1 << a0 and 1 << a1, and two register writes.
pub fn offscreen_target_init(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let (a0, a1) = (c.arg(0), c.arg(1));
    let scale = c.farg(0);
    let f20_bits = c.f[20];
    h::frame(c, 0x40, &[(S0, 0x00, h::SQ), (S1, 0x10, h::SQ), (RA, 0x20, h::SQ)], |c, sp| {
        c.w32(sp.wrapping_add(F20_SLOT), f20_bits);
        let sum = a0.wrapping_add(a1) as i32;
        let shift = if sum < SIZE_CLAMP { sum as u32 } else { 16 };
        let size = SIZE_BASE.wrapping_shl(shift & 31);
        let base = ((BUFFER_TOP.wrapping_sub(size) as i32 >> ALIGN_SHIFT).wrapping_shl(ALIGN_SHIFT)) as u32;
        h::call_keep(c, BUFFER_SETUP, &[a0, a1, base], &[]);
        let (w, h_) = (1u32.wrapping_shl(a0 & 31), 1u32.wrapping_shl(a1 & 31));
        h::run_keep(c, proj_init::offscreen_setup, &[w, h_], &[scale, F::ZERO, F(F_524288), F(F_255), F::ZERO]);
        h::call_keep(c, REG_WRITE, &[WRITE_A_ID, WRITE_A_VAL], &[]);
        c.r[A1] = WRITE_B_VAL;
        h::call_keep(c, REG_WRITE, &[WRITE_B_ID], &[]);
    });
    true
}

/// 0x28fbb0 proj_dma_setup() -> none: DMA tag emit/flush, projection setup, then one retail setup call.
pub fn proj_dma_setup(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    h::frame(c, 0x10, &[(RA, 0x0, h::SQ)], |c, _sp| {
        h::run_keep(c, dma_setup::tag_emit_or_flush, &[], &[]);
        h::run_keep(c, proj_setup::proj_setup, &[], &[]);
        h::call_keep(c, PROJ_TAIL, &[], &[]);
    });
    true
}
