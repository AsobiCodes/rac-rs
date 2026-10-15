//! HUD counter refresh: bolts counter update, then the per-frame counter value at +0x7c.

use crate::h;
use crate::hud_bolts;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Upper half of the RAM window holding the level and difficulty globals (lui 0x14).
const GLOBALS_HI: u32 = 0x14_0000;
/// Byte at GLOBALS_HI - 0x1ad7: nonzero selects the harder counter setting.
const DIFFICULTY_BYTE: u32 = GLOBALS_HI - 0x1ad7;
/// Base of the mode table (lui 0x14 + addiu -0xcb0).
const MODE_BASE: u32 = GLOBALS_HI - 0xcb0;
/// Word at MODE_BASE + 0x10b8: the mode that enables the float-scaled path when equal to 9.
const MODE_WORD: u32 = MODE_BASE + 0x10b8;
/// Byte at MODE_BASE + 0x20a4: nonzero skips the float-scaled path.
const MODE_FLAG_BYTE: u32 = MODE_BASE + 0x20a4;
/// Mode value that enables the float-scaled path.
const MODE_ACTIVE: u32 = 9;
/// Argument passed to float_to_int_scaled.
const SCALE_ARG: u32 = 0x78;
/// Offset added to the scaled value before it is stored.
const SCALED_BIAS: u32 = 0x1e;
/// Counter cap: values at or above this are reset to SCALED_BIAS.
const COUNTER_CAP: i32 = 0x1f;

/// Object field offsets.
const OBJ_SETTING: u32 = 0x08;
const OBJ_COUNTER: u32 = 0x7c;

/// 0x22a8f8 counter_hud_tick(obj) -> void: refreshes the bolts counter and the object's counter at +0x7c
pub fn counter_hud_tick(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    h::run(c, hud_bolts::bolts_counter_update, &[obj], &[]);

    let setting = if c.r8(DIFFICULTY_BYTE) == 0 { 5 } else { 10 };
    c.w32(obj + OBJ_SETTING, setting);

    let scaled_path = c.r32(MODE_WORD) == MODE_ACTIVE && c.r8(MODE_FLAG_BYTE) == 0;
    if scaled_path {
        let (scaled, _) = h::run(c, mathx::float_to_int_scaled, &[SCALE_ARG], &[]);
        c.w32(obj + OBJ_COUNTER, scaled.wrapping_add(SCALED_BIAS));
    } else {
        let counter = c.r32(obj + OBJ_COUNTER) as i32;
        if counter >= COUNTER_CAP {
            c.w32(obj + OBJ_COUNTER, SCALED_BIAS);
        }
    }
    true
}
