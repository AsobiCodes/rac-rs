//! Hero held-item animation layers: slot allocation on idle exit and the per-frame layer update.
use crate::actor;
use crate::glove_slot;
use crate::h;
use crate::motion;
use crate::ps2::*;
use crate::trig;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Hero state block: 0x140000 - 0xcb0.
const STATE: u32 = 0x0014_0000 - 0x0cb0;
/// Two held-item layer slot pointers at STATE + 0xd08 + 4 * i.
const SLOT_PTRS: u32 = 0x0d08;
/// Actor pointer handed to the fx slot functions.
const ACTOR: u32 = 0x2080;
/// Byte: layers locked (the allocation path is skipped while set).
const LOCKED: u32 = 0x20a4;
/// Byte: layers enabled (cleared when a layer finishes its fade-out).
const ENABLED: u32 = 0x20a8;
/// Byte: a held item is valid.
const HELD_VALID: u32 = 0x20aa;
/// Byte: second-layer mode; the second layer is only allocated when it equals 2.
const SECOND_MODE: u32 = 0x20ab;
/// Word: last held-item id seen by the update.
const LAST_ITEM: u32 = 0x2298;
/// Word: animation kind last passed to fx_slot_start by the update.
const LAST_KIND: u32 = 0x2294;
/// Word: hero kind; 0xc selects the alternate item animation field.
const KIND: u32 = 0x208c;
/// Hero kind that selects the alternate item animation field.
const KIND_ALT: u32 = 0x0c;
/// Hero kind that selects the transform path at the end of the update.
const KIND_XFORM: u32 = 0x0f;
/// Word: layer mode (0 = fading in, 1 = active, 2 = fast slew).
const MODE: u32 = 0x0d14;
/// Pointer to the aim target record.
const TARGET: u32 = 0x05d8;
/// Item record table: 0x180000 - 0x6440, one 0x4c-byte record per item id.
const ITEM_TABLE: u32 = 0x0018_0000 - 0x6440;
/// Size of one item record.
const ITEM_STRIDE: u32 = 0x4c;
/// Item record field: animation kind.
const ITEM_ANIM: u32 = 0x28;
/// Item record field: animation kind used when the hero kind is KIND_ALT.
const ITEM_ANIM_ALT: u32 = 0x2c;
/// Returned by the item lookup when the item has no animation.
const NO_ANIM: u32 = 0xffff_ffff;
/// Slot record byte: flags (bit 1 keeps the layer alive).
const SLOT_FLAGS: u32 = 0x05;
/// Slot record byte: animation kind the layer was started with.
const SLOT_KIND: u32 = 0x23;
/// Slot record float: layer weight (0.0 = finished).
const SLOT_WEIGHT: u32 = 0x08;
/// Slot record word: cleared on the fade-in path.
const SLOT_TIMER: u32 = 0x28;
/// Global kind table at gp - 0x7580: animation kind per layer index.
const KIND_TABLE_GP: i32 = -0x7580;
/// Global float at gp - 0x7ea0: base step for the weight slew.
const SLEW_BASE_GP: i32 = -0x7ea0;
/// Float at 0x15ed60: scale source for the fade-in slew.
const FADE_BASE_ADDR: u32 = 0x0015_ed60;
/// Transform block (3 rows of quadwords) at STATE + 0x640.
const XFORM: u32 = 0x0640;
/// Quadword at STATE + 0x90: copied to STATE + 0x680.
const BASIS_SRC: u32 = 0x90;
/// Quadword destination (STATE + 0x680).
const BASIS: u32 = 0x0680;
/// Vector written by the transform at STATE + 0x670.
const OFFSET: u32 = 0x0670;
/// Float fields of the angle output: angle at 0x688, negated Y at 0x684.
const ANGLE_OUT: u32 = 0x0688;
const ANGLE_NEG_OUT: u32 = 0x0684;
/// Float inputs of the angle path.
const ANGLE_IN_X: u32 = 0x057c;
const ANGLE_IN_Y: u32 = 0x0580;
const ANGLE_IN_D: u32 = 0x05d0;
/// Float positions compared against the target record.
const POS_X: u32 = 0x80;
const POS_Y: u32 = 0x84;

/// 0.25 (0x3e800000).
const QUARTER: F = F(0x3e80_0000);
/// 0.07 (0x3d8f5c29).
const SEVEN_PCT: F = F(0x3d8f_5c29);
/// 0.2 (0x3e4ccccd).
const FIFTH: F = F(0x3e4c_cccd);

/// Address of the item record for item id `item`.
fn item_record(item: u32) -> u32 {
    ITEM_TABLE.wrapping_add(item.wrapping_mul(ITEM_STRIDE))
}

/// Address of the second-level transform row at `base + off`.
fn at(base: u32, off: u32) -> u32 {
    base.wrapping_add(off)
}

/// 0x201408 hero_item_layers_on_idle_exit(a0, a1): when leaving idle (a0 == 0) with a held item (a1 != 0), allocate
/// the held-item animation layers (up to two) and start each one.
pub fn hero_item_layers_on_idle_exit(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let (a0, a1) = (c.arg(0), c.arg(1));
    if c.r8(STATE + LOCKED) != 0 || c.r8(STATE + ENABLED) == 0 || c.r8(STATE + HELD_VALID) == 0 {
        return true;
    }
    if a0 != 0 || a1 == 0 {
        return true;
    }
    let item = h::run(c, glove_slot::slot_value_b_fn, &[0], &[]).0;
    let anim = c.r32(item_record(item) + ITEM_ANIM);
    if anim == NO_ANIM {
        return true;
    }
    let gp = c.g(28) as u32;
    for i in 0..2u32 {
        let slot_addr = STATE + SLOT_PTRS + 4 * i;
        if c.r32(slot_addr) != 0 {
            continue;
        }
        if i == 1 && c.r8(STATE + SECOND_MODE) != 2 {
            break;
        }
        let kind = c.r32(gp.wrapping_add_signed(KIND_TABLE_GP) + 4 * i);
        let actor = c.r32(STATE + ACTOR);
        let slot = h::run(c, actor::fx_slot_alloc, &[actor, kind], &[]).0;
        c.w32(slot_addr, slot);
        if slot == 0 {
            continue;
        }
        c.w32(STATE + MODE, 0);
        c.w32(slot + SLOT_WEIGHT, 0);
        let t0 = h::scale_int(c, 15);
        let actor = c.r32(STATE + ACTOR);
        h::run(c, actor::fx_slot_start, &[actor, slot, anim, 0, t0 as u32, 1], &[]);
        c.wf(slot + 0x28, F::ONE);
    }
    true
}

/// 0x2017d0 weapon_layer_update(): per-frame held-item layer update: restart a layer when the item changed, slew
/// each live layer's weight (fade in or out), free a finished layer, and refresh the transform block.
pub fn weapon_layer_update(c: &mut Cpu) -> bool {
    if c.r8(STATE + ENABLED) != 0 && c.r8(STATE + HELD_VALID) != 0 && h::call(c, 0x226fd8, &[u32::MAX], &[]).0 == 0 {
        h::call(c, 0x201740, &[u32::MAX], &[]);
    }
    let gp = c.g(28) as u32;
    for j in 0..2u32 {
        let slot_addr = STATE + SLOT_PTRS + 4 * j;
        if c.r32(slot_addr) == 0 {
            continue;
        }
        let item = h::run(c, glove_slot::slot_value_b_fn, &[0], &[]).0;
        if item == c.r32(STATE + LAST_ITEM) {
            let rec = item_record(item);
            let kind = if c.r32(STATE + KIND) == KIND_ALT {
                c.r32(rec + ITEM_ANIM_ALT)
            } else {
                c.r32(rec + ITEM_ANIM)
            };
            let slot = c.r32(slot_addr);
            if kind != NO_ANIM && c.r8(slot + SLOT_KIND) as u32 != kind {
                c.w32(STATE + LAST_KIND, kind);
                let t0 = h::scale_int(c, 8);
                let actor = c.r32(STATE + ACTOR);
                let slot = c.r32(slot_addr);
                let kind = c.r32(STATE + LAST_KIND);
                h::run(c, actor::fx_slot_start, &[actor, slot, kind, 0, t0 as u32, 1], &[]);
            }
        }
        let actor = c.r32(STATE + ACTOR);
        let slot = c.r32(slot_addr);
        h::call(c, 0x241a58, &[actor, slot], &[]);
        let mode = c.r32(STATE + MODE);
        let slot = c.r32(slot_addr);
        if mode == 0 {
            let step = c.rf(FADE_BASE_ADDR) * FIFTH;
            h::run(c, motion::slew_toward, &[at(slot, SLOT_WEIGHT)], &[F::ONE, step]);
            let slot = c.r32(slot_addr);
            if c.r8(slot + SLOT_FLAGS) & 2 == 0 {
                continue;
            }
            if c.r8(STATE + HELD_VALID) == 0 {
                c.w32(STATE + MODE, 1);
                c.w32(slot + SLOT_TIMER, 0);
                c.w8(STATE + ENABLED, 0);
                return true;
            }
            continue;
        }
        let scale = if mode == 2 { QUARTER } else { SEVEN_PCT };
        let step = c.rf(gp.wrapping_add_signed(SLEW_BASE_GP)) * scale;
        h::run(c, motion::slew_toward, &[at(slot, SLOT_WEIGHT)], &[F::ZERO, step]);
        let slot = c.r32(slot_addr);
        if c.rf(at(slot, SLOT_WEIGHT)) == F::ZERO {
            let actor = c.r32(STATE + ACTOR);
            h::run(c, actor::fx_slot_free, &[actor, slot_addr], &[]);
            return true;
        }
    }

    if c.r32(STATE + KIND) != KIND_XFORM {
        let actor = c.r32(STATE + ACTOR);
        let row0 = c.rv(actor + 0xc0);
        let row1 = c.rv(actor + 0xd0);
        let row2 = c.rv(actor + 0xe0);
        c.wv(STATE + XFORM, row0);
        c.wv(STATE + XFORM + 0x10, row1);
        c.wv(STATE + XFORM + 0x20, row2);
        h::call(c, 0x1f17f8, &[STATE + OFFSET], &[]);
        c.wf(STATE + OFFSET, F::ONE);
        h::run(c, vecmath::vec3_mul_mat3, &[STATE + OFFSET, STATE + OFFSET, STATE + XFORM], &[]);
        let basis = c.rv(STATE + BASIS_SRC);
        c.wv(STATE + BASIS, basis);
        return true;
    }

    let basis = c.rv(STATE + BASIS_SRC);
    c.wv(STATE + BASIS, basis);
    let target = c.r32(STATE + TARGET);
    let angle = if target != 0 {
        let x = c.rf(at(target, 0x10)) - c.rf(STATE + POS_X);
        let y = c.rf(at(target, 0x14)) - c.rf(STATE + POS_Y);
        h::run(c, trig::fast_atan2, &[], &[x, y]).1
    } else {
        let (x, d) = (c.rf(STATE + ANGLE_IN_X), c.rf(STATE + ANGLE_IN_D));
        h::run(c, vecmath::angle_add, &[], &[x, d]).1
    };
    let neg_y = -c.rf(STATE + ANGLE_IN_Y);
    c.wf(STATE + ANGLE_OUT, angle);
    c.wf(STATE + ANGLE_NEG_OUT, neg_y);

    let xform = STATE + XFORM;
    h::call(c, 0x1f1fe8, &[xform, STATE + BASIS], &[]);
    h::call(c, 0x1f17f8, &[STATE + OFFSET], &[]);
    c.wf(STATE + OFFSET, F::ONE);
    h::run(c, vecmath::vec3_mul_mat3, &[STATE + OFFSET, STATE + OFFSET, xform], &[]);
    true
}

/// 0x201af8 held_layers_frame(): per-frame wrapper: 0x200dc8, then weapon_layer_update, then 0x200b10.
pub fn held_layers_frame(c: &mut Cpu) -> bool {
    h::call(c, 0x200dc8, &[], &[]);
    h::run(c, weapon_layer_update, &[], &[]);
    // Plain call, not h::call: h::call restores v0/v1, but retail returns 0x200b10's v0/v1 to the caller.
    c.call(0x200b10);
    true
}
