//! HUD slot lifecycle: commit a pending slot request, bind its icon, reset every slot, close one by handle.
use crate::h::{call_keep, run_keep};
use crate::hud_slots;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Slot table: 13 slots of 0x90 bytes.
const SLOT_TABLE: u32 = 0x17_dd50;
const SLOT_SIZE: u32 = 0x90;
const SLOT_COUNT: u32 = 13;
/// Pointer to the icon table (8-byte entries: sheet id u16 at +0, ..., u8 at +6, u16 at +4).
const ICON_TABLE_PTR: u32 = 0x17_e5f4;
/// Base of the HUD globals block that holds ICON_TABLE_PTR at +0x1c.
const ICON_GLOBALS: u32 = 0x17_e5d8;

/// Pending request fields (copied into the live fields by a commit).
const P_SHEET: u32 = 0x20;
const P_FLAGS: u32 = 0x24;
const P_ARG_B: u32 = 0x28;
const P_KIND: u32 = 0x2c;
const P_INIT: u32 = 0x30;
const P_ARG_D: u32 = 0x34;
const P_ARG_E: u32 = 0x38;
/// Live fields.
const L_FLAGS: u32 = 0x04;
const L_ARG_B: u32 = 0x08;
const L_KIND: u32 = 0x0c;
const L_INIT: u32 = 0x10;
const L_ARG_D: u32 = 0x14;
const L_ARG_E: u32 = 0x18;
/// Icon binding results.
const S_ICON_SHEET: u32 = 0x00;
const S_ICON_INDEX: u32 = 0x40;
const S_ICON_MODE: u32 = 0x42;
const S_ICON_FRAME: u32 = 0x44;
/// Handle, pending flag, slide and lifetime.
const S_HANDLE: u32 = 0x64;
const S_PENDING: u32 = 0x68;
const S_SLIDE: u32 = 0x6c;
const S_LIFETIME: u32 = 0x7c;

/// 0x228ac8 hud_icon_bind(slot, id): look up icon `id` in the icon table and store its sheet id (+0), table index
/// (u16 +0x40), mode byte (+0x42) and first frame (+0x44) in the slot.
pub fn hud_icon_bind(c: &mut Cpu) -> bool {
    let slot = c.arg(0);
    let id = c.arg(1);
    run_keep(c, hud_slots::hud_icon_index, &[id], &[]);
    let index = c.g(2) as u32;
    let table = c.r32(ICON_TABLE_PTR);
    let entry = table + index * 8;
    let sheet = c.r16(entry) as u32;
    let frame = c.r16(entry + 4) as u32;
    c.w32(slot + S_ICON_SHEET, sheet);
    c.w16(slot + S_ICON_INDEX, index as u16);
    let mode = c.r8(entry + 6);
    c.w8(slot + S_ICON_MODE, mode);
    c.w32(slot + S_ICON_FRAME, frame);
    set(c, 2, table);
    set(c, 3, frame);
    set(c, 4, entry);
    set(c, 5, sheet);
    set(c, 6, ICON_GLOBALS);
    true
}

/// 0x228950 hud_slot_commit(slot): bind the pending icon, copy the pending request into the live fields, install the
/// init callback and run it once with the slot, then clear the pending flag.
pub fn hud_slot_commit(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let slot = c.arg(0);
    let v = c.r32(slot + P_SHEET);
    set(c, 5, v);
    hud_icon_bind(c);
    let flags = c.r32(slot + P_FLAGS);
    let arg_d = c.r32(slot + P_ARG_D);
    let arg_e = c.r32(slot + P_ARG_E);
    let kind = c.r32(slot + P_KIND);
    let arg_b = c.r32(slot + P_ARG_B);
    let init = c.r32(slot + P_INIT);
    c.w32(slot + L_FLAGS, flags);
    c.w32(slot + L_ARG_D, arg_d);
    c.w32(slot + L_ARG_E, arg_e);
    c.w32(slot + L_KIND, kind);
    c.w32(slot + L_ARG_B, arg_b);
    c.w32(slot + L_INIT, init);
    set(c, 2, flags);
    set(c, 3, arg_d);
    set(c, 4, arg_e);
    set(c, 5, kind);
    set(c, 6, arg_b);
    set(c, 7, init);
    if init != 0 {
        call_keep(c, init, &[slot, kind, arg_b, init], &[]);
    }
    c.w32(slot + S_PENDING, 0);
    true
}

/// 0x2289b8 hud_slots_clear(): every HUD slot gets an empty request (flags 0xffff), slide -6, lifetime 0, then a commit.
pub fn hud_slots_clear(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    for i in 0..SLOT_COUNT {
        let slot = SLOT_TABLE + i * SLOT_SIZE;
        run_keep(c, hud_slots::hud_slot_request, &[i, 0xffff, 0, 0, 0, 0, 1], &[]);
        c.w32(slot + S_LIFETIME, 0);
        c.w32(slot + S_SLIDE, (-6i32) as u32);
        set(c, 4, slot);
        hud_slot_commit(c);
    }
    true
}

/// 0x228a48 hud_slot_close(handle) -> 1 if a slot holds `handle` (it is re-requested empty), else 0.
pub fn hud_slot_close(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let handle = c.arg(0);
    let found = (0..SLOT_COUNT).find(|&i| c.r32(SLOT_TABLE + i * SLOT_SIZE + S_HANDLE) == handle);
    match found {
        Some(i) => {
            // hud_slot_request returns early (a1 stays 0xffff, not 1) when the slot already holds this empty request;
            // that path is not modelled, so decline it and let retail run.
            let s = SLOT_TABLE + i * SLOT_SIZE;
            if c.r32(s + 0x20) == 0xffff && c.r32(s + 0x24) == 0 && c.r32(s + 0x30) == 0
                && c.r32(s + 0x34) == 0 && c.r32(s + 0x38) == 0
            {
                return false;
            }
            run_keep(c, hud_slots::hud_slot_request, &[i, 0xffff, 0, 0, 0, 0, 0], &[]);
            // The retail fill path leaves a0 = 0x180000 and a1 = 1 behind.
            set(c, 4, 0x18_0000);
            set(c, 5, 1);
            set(c, 2, 1);
        }
        None => {
            set(c, 2, 0);
            set(c, 3, SLOT_TABLE + SLOT_COUNT * SLOT_SIZE + S_HANDLE);
            set(c, 4, SLOT_COUNT);
            set(c, 5, handle);
        }
    }
    true
}

/// Sets integer register `r` to the sign-extended 32-bit value `v`.
fn set(c: &mut Cpu, r: usize, v: u32) {
    c.s(r, v as i32 as i64 as u64);
}
