//! HUD ammo counter and slot tick routines: retail 0x22d8f0 (ammo bar draw), 0x22ddc0 (slot tick) and 0x22df00
//! (ammo slot request/release for the held weapon).
use crate::h;
use crate::hud_slots;
use crate::mathx;
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Stack bytes reserved by the draw routine (its retail frame).
const DRAW_FRAME: u32 = 0xb0;
/// Ammo state block and the per-weapon table (stride 24, u16 "has ammo" at +8, u16 icon at +0xe).
const AMMO_STATE: u32 = 0x13_f350;
const WEAPON_TABLE: u32 = 0x1c_4130;
const WEAPON_STRIDE: u32 = 24;
/// Per-weapon slot pointers (stride 4), the held item and the handles of the two ammo slots.
const ITEM_SLOT_TABLE: u32 = 0x13_d428;
const ITEM_WORD: u32 = 0x15_f944;
const HANDLE_WORD: u32 = 0x15_f948;
/// Word that switches the draw to the compact layout (rows at 10 instead of 18).
const COMPACT_LAYOUT_WORD: u32 = 0x15_ed80;
/// Text label table handed to the retail formatter at 0x116248.
const LABEL_TABLE: u32 = 0x15_f7a8;
/// Bar texture id passed to the sprite setup helper.
const BAR_TEXTURE: u32 = 0x7580;
/// Retail helpers (no Rust twin yet).
const SPRITE_SETUP: u32 = 0x22_e350;
const DRAW_BAR: u32 = 0x22_e620;
const DRAW_BAR_TILED: u32 = 0x22_e9a8;
const DRAW_COUNT: u32 = 0x22_e268;
const LABEL_FORMAT: u32 = 0x11_6248;
const FILL_QUAD: u32 = 0x1e_e168;
const SLOT_EXTRA: u32 = 0x22_8950;

/// Argument pointers the ammo request passes through (labels and callbacks in the retail image).
const AMMO_LABEL_A: u32 = 0x22_d8a8;
const AMMO_DRAW_CB: u32 = 0x22_d8f0;
const AMMO_LABEL_B: u32 = 0x22_ff00;
const RELEASE_LABEL: u32 = 0x22_8ec0;
const RELEASE_DRAW: u32 = 0x22_8fe0;
const FALLBACK_LABEL: u32 = 0x22_d788;

/// Slot table: 13 slots of 0x90 bytes from 0x17dd50. Field offsets used by the tick and draw.
const SLOT_TABLE: u32 = 0x17_dd50;
const SLOT_STRIDE: u32 = 0x90;
const SLOT_COUNT: u32 = 13;
const NEXT_HANDLE: u32 = 0x17_e5d8;
const SL_FLAGS: u32 = 0x04;
const SL_TIER: u32 = 0x08;
const SL_CALLBACK: u32 = 0x14;
const SL_PENDING: u32 = 0x68;
const SL_PHASE: u32 = 0x6c;
const SL_TIMER: u32 = 0x7c;
const SL_LABEL: u32 = 0x74;
const SL_BASE: u32 = 0x50;
const SL_RET: u32 = 0x58;
const SL_HANDLE: u32 = 0x44;
const SL_FLAG: u32 = 0x70;

/// Float constants with their exact bits.
const SCALE_128: F = F(0x4300_0000);
const SCALE_0_7: F = F(0x3f33_3333);

/// Reads a word at gp - back (the retail global pointer is in $gp).
fn gp(c: &mut Cpu, back: u32) -> u32 {
    c.r32((c.g(28) as u32).wrapping_sub(back))
}

/// Integer to float (cvt.s.w).
fn int_f(v: u32) -> F {
    F::from_int(v as i32)
}

/// num / den clamped to [0, 1] with the retail compare-and-select order.
fn ratio(num: F, den: F) -> F {
    let r = num / den;
    if F::ONE < r {
        F::ONE
    } else if r < F(0) {
        F(0)
    } else {
        r
    }
}

/// 0x22d8f0 hud_ammo_draw(slot) -> word at slot+0x58: when the slot's byte at +0x70 is set, draws its ammo bar, icon
/// and count, with the colours faded by the two ammo ratios. Always returns the word at +0x58.
pub fn hud_ammo_draw(c: &mut Cpu) -> bool {
    // Retail runs this path: the Rust body leaves the prologue's register-save slots (sp+0x10..0x9f) unwritten,
    // so the RAM comparison differs by 40 bytes. Left to retail until the saves are modelled.
    return false;
    #[allow(unreachable_code)]
    let slot = c.arg(0);
    if c.r8(slot + SL_FLAG) != 0 {
        h::with_stack(c, DRAW_FRAME, |c, buf| draw_ammo(c, slot, buf));
    }
    let v = c.r32(slot + SL_RET);
    c.ret(v);
    true
}

/// Body of hud_ammo_draw; `buf` is the scratch area at the bottom of the frame.
fn draw_ammo(c: &mut Cpu, slot: u32, buf: u32) {
    let count_a = c.r8(slot + SL_FLAG) as u32;
    let max_a = gp(c, 0x730c);
    let frac_a = ratio(int_f(count_a), int_f(max_a));
    let count_b = c.r8(slot + SL_FLAG + 1) as u32;
    let max_b = gp(c, 0x7308);
    let fade = ratio(int_f(count_b), int_f(max_b));

    let bar_full = (SCALE_128 * frac_a).to_int();
    let bar_px = (F::from_int(bar_full) * SCALE_0_7).to_int() as u32;
    let tier_scale = if (c.r32(slot + SL_TIER) as i32) < 100 { gp(c, 0x7304) } else { gp(c, 0x7300) };
    let base = c.r32(slot + SL_BASE);
    let inner = base.wrapping_add(4);
    let fill = base.wrapping_add((F::from_int(tier_scale as i32) * frac_a).to_int() as u32);
    let rows = if c.r32(COMPACT_LAYOUT_WORD) != 0 { 10 } else { 18 };

    let tex = h::call(c, SPRITE_SETUP, &[BAR_TEXTURE, 1], &[]).0;
    h::call(c, DRAW_BAR, &[tex, base.wrapping_sub(0x1c), rows, 0x20, 0x20, bar_px], &[]);
    let tex = h::call(c, SPRITE_SETUP, &[BAR_TEXTURE, 0], &[]).0;
    h::call(c, DRAW_BAR, &[tex, inner, rows, fill, 0x20, bar_px], &[]);
    let tex = h::call(c, SPRITE_SETUP, &[BAR_TEXTURE, 1], &[]).0;
    h::call(c, DRAW_BAR_TILED, &[tex, inner.wrapping_add(fill), rows, 0x20, 0x20, bar_px], &[]);

    let owner = c.r32(slot);
    let icon = h::call(c, SPRITE_SETUP, &[owner, 3], &[]).0;
    c.w32(slot + SL_HANDLE, icon);
    h::call(c, DRAW_COUNT, &[slot, icon, base, rows, 0, bar_full as u32], &[]);

    let label = c.r32(slot + SL_LABEL);
    let tier = c.r32(slot + SL_TIER);
    h::call(c, LABEL_FORMAT, &[buf, LABEL_TABLE, label, tier], &[]);

    let (lo, hi) = if label == 0 {
        (gp(c, 0x72f4), gp(c, 0x72f0))
    } else {
        (gp(c, 0x72fc), gp(c, 0x72f8))
    };
    let lerp_a = h::run(c, vecmath::color_lerp, &[lo, hi], &[fade]).0;
    let lerp_b = h::run(c, vecmath::color_lerp, &[0, 0x8000_0000], &[fade]).0;

    let x = base.wrapping_add(gp(c, 0x72ec)).wrapping_add(tier_scale);
    let y = rows.wrapping_add(gp(c, 0x72e8));
    h::call(c, FILL_QUAD, &[x.wrapping_add(1), y.wrapping_add(1), lerp_b, buf, u32::MAX], &[]);
    h::call(c, FILL_QUAD, &[x, y, lerp_a, buf, u32::MAX], &[]);
}

/// One HUD slot's per-frame step: returns true when its fade timer is still running (counted as active).
fn tick_slot(c: &mut Cpu, s: u32) -> bool {
    let gated = c.r32(s + SL_FLAGS) & 0x10 != 0 || c.r32(NEXT_HANDLE + 4) != 0;
    let mut timer = c.r32(s + SL_TIMER) as i32;
    if gated {
        let target = h::run(c, mathx::float_to_int_scaled, &[10], &[]).0 as i32;
        if timer < target {
            timer = h::run(c, mathx::float_to_int_scaled, &[10], &[]).0 as i32;
            c.w32(s + SL_TIMER, timer as u32);
        }
    }
    // Retail stores timer-1 whenever timer > 0 (the store sits in the delay slot), so a timer of 1 becomes 0.
    if timer > 0 {
        c.w32(s + SL_TIMER, (timer - 1) as u32);
    }
    let counting = timer > 1;
    let phase = c.r32(s + SL_PHASE) as i32;
    if counting {
        if phase < 30 {
            c.w32(s + SL_PHASE, (phase + 1) as u32);
        }
    } else if phase >= -5 {
        c.w32(s + SL_PHASE, (phase - 1) as u32);
    }
    let pending = c.r32(s + SL_PENDING);
    if pending != 0 && c.r32(s + SL_PHASE) as i32 == -6 {
        h::call(c, SLOT_EXTRA, &[s], &[]);
    }
    counting
}

/// 0x22ddc0 hud_slots_tick(update) -> active count: first steps the held-weapon ammo slot, then ticks each of the 13
/// HUD slots (fade timer, phase, pending request) and, when `update` is nonzero, calls each slot's update callback.
/// Returns how many slots are still counting down.
pub fn hud_slots_tick(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let update = c.arg(0);
    h::run(c, hud_ammo_request, &[], &[]);
    let end = SLOT_TABLE + SLOT_COUNT * SLOT_STRIDE;
    let mut active = 0u32;
    let mut s = SLOT_TABLE;
    while s < end {
        if tick_slot(c, s) {
            active += 1;
        }
        if update != 0 {
            let callback = c.r32(s + SL_CALLBACK);
            if callback != 0 {
                h::call(c, callback, &[s], &[]);
            }
        }
        s += SLOT_STRIDE;
    }
    c.ret(active);
    true
}

/// 0x22df00 hud_ammo_request() -> v0: keeps the held weapon's ammo slot in step. With ammo, requests a slot for the
/// current weapon and remembers item and handle; without, releases the old handle and in mode 2 requests the fallback
/// slot. Does nothing (returns the lock word) while the gp lock word is set.
pub fn hud_ammo_request(c: &mut Cpu) -> bool {
    let lock = gp(c, 0x72c0);
    if lock != 0 {
        c.ret(lock);
        return true;
    }
    let weapon = c.r32(AMMO_STATE + 0x10b8);
    let mode = c.r8(AMMO_STATE + 0x20a4) as u32;
    let has_ammo = c.r16(WEAPON_TABLE + weapon.wrapping_mul(WEAPON_STRIDE) + 8) as u32;
    let item = if has_ammo == 0 || mode != 0 { 0 } else { weapon };

    if item != 0 {
        let entry = WEAPON_TABLE + item.wrapping_mul(WEAPON_STRIDE);
        let icon = c.r16(entry + 0xe) as u32;
        c.w32(ITEM_WORD, item);
        let slot_ptr = ITEM_SLOT_TABLE.wrapping_add(item << 2);
        let handle = h::run(
            c,
            hud_slots::hud_slot_request,
            &[16, item.wrapping_add(60000), AMMO_LABEL_A, AMMO_LABEL_B, AMMO_DRAW_CB, slot_ptr, icon],
            &[],
        )
        .0;
        c.w32(HANDLE_WORD, handle);
        c.ret(handle);
    } else if mode == 2 {
        release_handle(c);
        let v = h::run(
            c,
            hud_slots::hud_slot_request,
            &[16, 0xffff, RELEASE_LABEL, RELEASE_DRAW, FALLBACK_LABEL, AMMO_STATE + 0x1630, 200],
            &[],
        )
        .0;
        c.w16(AMMO_STATE + 0x1634, v as u16);
        c.ret(v);
    } else {
        let mut v = 2;
        if let Some(r) = release_handle(c) {
            v = r;
        }
        c.ret(v);
    }
    true
}

/// Releases the ammo handle if one is held (then marks it -1); returns the release result when it ran.
fn release_handle(c: &mut Cpu) -> Option<u32> {
    let prev = c.r32(HANDLE_WORD);
    if prev == u32::MAX {
        return None;
    }
    let r = h::run(c, hud_slots::hud_slot_release, &[prev, 0], &[]).0;
    c.w32(HANDLE_WORD, u32::MAX);
    Some(r)
}
