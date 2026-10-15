//! HUD counter and bolt widget setup: retail 0x22ac28 (scaled fields and an 8-entry table fill), 0x22c6f0 (bar init),
//! 0x22ce08 (bolts counter init) and 0x22d8a8 (counter init). The last three hand the object to the retail widget
//! setup 0x228dc0 (or to 0x22d8a8).
use crate::h;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const AT: usize = 1;
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;
const T0: usize = 8;
const T1: usize = 9;
const GP: usize = 28;
const RA: usize = 31;

/// Retail widget setup that takes the object in a0 (its scratch registers are kept as retail leaves them).
const WIDGET_SETUP: u32 = 0x22_8dc0;
/// Counter init shared by the bolts setup.
const COUNTER_INIT: u32 = 0x22_d8a8;

/// Global words and halfwords the setup writes (absolute RAM addresses).
const COUNTER_WORD: u32 = 0x15_fa48;
const TABLE_PTR_WORD: u32 = 0x15_fa78;
const BAR_FLAGS: u32 = 0x15_fa68;
const BOLTS_FLAG: u32 = 0x15_fa70;
/// Upper half of the global block address (lui at, 0x16).
const GLOBALS_HI: u32 = 0x16_0000;
/// gp-relative offsets: the two source words, the counter word cleared to zero.
const GP_SRC_V1: u32 = 0x7508;
const GP_SRC_V0: u32 = 0x7504;
const GP_ZERO_WORD: u32 = 0x74f4;
const GP_TABLE_BASE: u32 = 0x74e8;

/// Table fill: 8 words from 0x141ea0 each index a 76-byte record at 0x17b9c0 (halfword at +0x38), and the result goes to
/// the 28-byte entries at 0x17dbc0.
const SRC_WORDS: u32 = 0x14_1ea0;
/// Leftover of the scaler's float constant load (lui at, 0x3e80) that retail leaves in at.
const SCALER_AT: u32 = 0x3e80_0000;
const SRC_COUNT: u32 = 8;
const RECORD_BASE: u32 = 0x17_9bc0;
const RECORD_STRIDE: u32 = 0x4c;
const RECORD_KEY_OFF: u32 = 0x38;
const ENTRY_BASE: u32 = 0x17_dbc0;
const ENTRY_STRIDE: u32 = 0x1c;

/// Frame of the retail setup functions: s0 at +0 and ra at +0x10 are stored on the stack.
const FRAME: u32 = 0x20;

fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// Scales the integer n with the Rust twin of retail 0x1f14b8 (v0 = result).
fn scaled(c: &mut Cpu, n: u32) -> u32 {
    let (v0, _) = h::run_keep(c, mathx::float_to_int_scaled, &[n], &[]);
    v0 as u32
}

/// Retail widget setup: obj in a0 with the scratch values it leaves.
fn widget_setup(c: &mut Cpu, obj: u32) {
    h::call_keep(c, WIDGET_SETUP, &[obj], &[]);
}

/// 0x22ac28 counter_table_init(obj) -> void: sets the two counters, the scaled field (30) and the 8-entry table fill
/// from the words at 0x141ea0 through the 0x17b9c0 records.
pub fn table_init(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    h::frame(c, FRAME, &[(16, 0x00, h::SQ), (RA, 0x10, h::SQ)], |c, _sp| {
        let gp = c.g(GP) as u32;
        c.w32(COUNTER_WORD, 8);
        c.w32(TABLE_PTR_WORD, gp.wrapping_sub(GP_TABLE_BASE));
        let v1_src = c.r32(gp.wrapping_sub(GP_SRC_V1));
        let v0_src = c.r32(gp.wrapping_sub(GP_SRC_V0));
        c.w16(obj + 0x48, 0);
        c.w16(obj + 0x4a, 0);
        c.w32(obj + 0x58, v1_src);
        c.w32(obj + 0x74, -2i32 as u32);
        c.w32(obj + 0x5c, v0_src);
        let s = scaled(c, 0x1e);
        c.w32(obj + 0x78, s);
        c.w32(obj + 0x70, 0);
        c.w32(gp.wrapping_sub(GP_ZERO_WORD), 0);

        let mut dst = ENTRY_BASE;
        let mut src = SRC_WORDS;
        let (mut word, mut v0, mut v1, mut prod) = (0u32, 0u32, 0u32, 0i64);
        for _ in 0..SRC_COUNT {
            word = c.r32(src);
            src += 4;
            prod = word as i32 as i64 * RECORD_STRIDE as i64;
            v0 = prod as i32 as u32;
            v1 = v0.wrapping_add(RECORD_BASE);
            c.w32(dst + 0x18, word);
            v0 = c.r16(v1.wrapping_add(RECORD_KEY_OFF)) as u32;
            c.w32(dst, v0);
            dst += ENTRY_STRIDE;
        }
        // Registers the loop leaves behind (a2 counts down past zero; HI/LO hold the last product).
        c.lo = c.lo & !(u64::MAX as u128) | (prod as i32 as i64 as u64) as u128;
        c.hi = c.hi & !(u64::MAX as u128) | ((prod >> 32) as i32 as i64 as u64) as u128;
        set_gpr(c, A0, word);
        set_gpr(c, A1, dst);
        set_gpr(c, A2, -1i32 as u32);
        set_gpr(c, A3, src);
        set_gpr(c, T0, RECORD_STRIDE);
        set_gpr(c, T1, RECORD_BASE);
        set_gpr(c, V0, v0);
        set_gpr(c, V1, v1);
        set_gpr(c, AT, SCALER_AT);
    });
    true
}

/// 0x22c6f0 bar_init(obj) -> void: scaled fields (180 and +30), the widget setup, then the bar flags at 0x15fa68.
pub fn bar_init(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    h::frame(c, FRAME, &[(16, 0x00, h::SQ), (RA, 0x10, h::SQ)], |c, _sp| {
        let v = scaled(c, 0xb4);
        c.w16(obj + 0x4a, 0);
        c.w32(obj + 0x7c, v.wrapping_add(0x1e));
        c.w32(obj + 0x5c, 0x40);
        c.w16(obj + 0x48, 0x20);
        c.w32(obj + 0x58, 0x40);
        set_gpr(c, A0, obj);
        set_gpr(c, A1, 0x20);
        set_gpr(c, V1, 0x40);
        set_gpr(c, V0, v.wrapping_add(0x1e));
        h::call_keep(c, WIDGET_SETUP, &[obj], &[]);
        c.w32(obj + 0x80, BAR_FLAGS);
        c.w16(BAR_FLAGS + 6, 1);
        c.w16(BAR_FLAGS, 0);
        c.w16(BAR_FLAGS + 2, 0);
        c.w16(BAR_FLAGS + 4, 0);
        set_gpr(c, V0, BAR_FLAGS);
        set_gpr(c, V1, 1);
        set_gpr(c, AT, GLOBALS_HI);
    });
    true
}

/// 0x22ce08 bolts_init(obj) -> void: counter init (0x22d8a8) on obj, then the bolts flag at 0x15fa70 is cleared.
pub fn bolts_init(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    h::frame(c, FRAME, &[(16, 0x00, h::SQ), (RA, 0x10, h::SQ)], |c, _sp| {
        h::call_keep(c, COUNTER_INIT, &[obj], &[]);
        c.w32(obj + 0x80, BOLTS_FLAG);
        c.w16(BOLTS_FLAG, 0);
        set_gpr(c, V0, BOLTS_FLAG);
        set_gpr(c, AT, GLOBALS_HI);
    });
    true
}

/// 0x22d8a8 counter_init(obj) -> void: scaled lifetime (120) + 30 into +0x7c, size 0x20 into +0x58/+0x5c, then the
/// widget setup.
pub fn counter_init(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    h::frame(c, FRAME, &[(16, 0x00, h::SQ), (RA, 0x10, h::SQ)], |c, _sp| {
        let v = scaled(c, 0x78);
        c.w32(obj + 0x7c, v.wrapping_add(0x1e));
        c.w32(obj + 0x5c, 0x20);
        c.w32(obj + 0x58, 0x20);
        set_gpr(c, A0, obj);
        set_gpr(c, V1, 0x20);
        set_gpr(c, V0, v.wrapping_add(0x1e));
        widget_setup(c, obj);
    });
    true
}
