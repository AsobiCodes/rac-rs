//! Colour channel swap by mask and the flag-0x20 pvar lookup (retail 0x24eca8..0x24ed2c and 0x24ef28..0x24ef58).
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const V1: usize = 3;
const A0: usize = 4;
const A3: usize = 7;
const S0: usize = 16;
const RA: usize = 31;

/// Retail channel swap helper: swaps channels of the packed colour by `mask` (0x24ec48).
const SWAP_CHANNELS: u32 = 0x24ec48;
/// Frame size of the retail prologue (`addiu sp,sp,-0x30`).
const FRAME: u32 = 0x30;
/// Offsets of the saved ra and s0 in the retail frame (`sq ra,0x20(sp)`, `sq s0,0x10(sp)`).
const SAVE_RA: u32 = 0x20;
const SAVE_S0: u32 = 0x10;
/// Offset of the moby flag halfword and of its component pointer.
const MOBY_FLAGS: u32 = 0x34;
const MOBY_COMPONENT: u32 = 0x78;
/// Flag bit that makes the first pvar word valid.
const FLAG20: u32 = 0x20;
/// Offset of the first pvar word inside the component.
const PVAR_WORD0: u32 = 0x10;

fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// 0x24eca8 colour_swap_channels(packed, mask) -> packed: swaps the R/G/B bytes of `packed` (alpha kept) by `mask`
/// through retail 0x24ec48; mask 0 returns `packed` unchanged.
pub fn colour_swap_channels(c: &mut Cpu) -> bool {
    let packed = c.arg(0);
    let mask = c.arg(1);
    h::frame(c, FRAME, &[(RA, SAVE_RA, h::SQ), (S0, SAVE_S0, h::SQ)], |c, sp| {
        if mask == 0 {
            set_gpr(c, A3, mask);
            c.ret(packed);
            return;
        }
        let red = sp;
        let green = sp + 4;
        let blue = sp + 8;
        c.w32(red, packed & 0xff);
        c.w32(green, (packed >> 8) & 0xff);
        c.w32(blue, (packed >> 16) & 0xff);
        h::call_keep(c, SWAP_CHANNELS, &[red, green, blue, mask], &[]);
        let r = c.r32(red);
        let g = c.r32(green);
        let b = c.r32(blue);
        let alpha = packed >> 24;
        // Retail leaves the shifted blue and green in v1 and a0 (its scratch registers).
        set_gpr(c, V1, b << 16);
        set_gpr(c, A0, g << 8);
        c.ret((alpha << 24) | (b << 16) | (g << 8) | r);
    });
    true
}

/// 0x24ef28 flag20_first_pvar(moby) -> word: when the moby's flags halfword has 0x20 set, the first pvar word
/// (through the component pointer at +0x78) is returned; otherwise (or for a null moby) 0.
pub fn flag20_first_pvar(c: &mut Cpu) -> bool {
    let moby = c.arg(0);
    if moby == 0 || (c.r16(moby + MOBY_FLAGS) as u32 & FLAG20) == 0 {
        c.ret(0);
        return true;
    }
    let component = c.r32(moby + MOBY_COMPONENT);
    set_gpr(c, V1, component);
    let word = c.r32(component + PVAR_WORD0);
    c.ret(word);
    true
}
