//! HUD widget slot table (13 slots of 0x90 bytes at 0x17dd50) and the HUD icon table lookup (Eudora overlay).
use crate::calls::*;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const SLOT_TABLE: u32 = 0x17_dd50;
const SLOT_SIZE: u32 = 0x90;
const SLOT_COUNT: u32 = 13;
/// Pointer to the icon table (8-byte entries, u16 sheet id first, ends with 0xffff).
const ICON_TABLE_PTR: u32 = 0x17_e5f4;
/// Next handle to hand out.
const NEXT_HANDLE: u32 = 0x17_e5d8;
/// Game-mode word (5 = a mode in which only slots 0 and 2 accept requests).
const GAME_MODE: u32 = 0x15_f5a4;
const SLOT_REQUEST_EXTRA: u32 = 0x22_8950;

/// Slot fields.
const S_FLAGS_ACTIVE: u32 = 0x04;
const S_ARG_A: u32 = 0x20;
const S_FLAGS: u32 = 0x24;
const S_ARG_B: u32 = 0x28;
const S_KIND: u32 = 0x2c;
const S_ARG_C: u32 = 0x30;
const S_ARG_D: u32 = 0x34;
const S_ARG_E: u32 = 0x38;
const S_HANDLE: u32 = 0x64;
const S_PENDING: u32 = 0x68;
const S_ZERO_70: u32 = 0x70;
const S_TIMER: u32 = 0x7c;

/// 0x227f88 hud_icon_index(id) -> index of the first icon entry with that sheet id (or of the terminator).
pub fn hud_icon_index(c: &mut Cpu) -> bool {
    preserved(c, true, false, hud_icon_index_body)
}

fn hud_icon_index_body(c: &mut Cpu) -> bool {
    let id = c.arg(0);
    let mut e = c.r32(ICON_TABLE_PTR);
    let mut idx = 0;
    loop {
        let v = c.r16(e) as u32;
        if v == 0xffff || v == id {
            break;
        }
        e += 8;
        idx += 1;
    }
    c.ret(idx);
    true
}

/// Index of the slot holding `handle`, if any.
fn slot_of_handle(c: &mut Cpu, handle: u32) -> Option<u32> {
    (0..SLOT_COUNT).find(|i| c.r32(SLOT_TABLE + SLOT_SIZE * i + S_HANDLE) == handle)
}

/// 0x228840 hud_slot_request(flags|slot, a, b, c, d, kind, e) -> handle: slot = flags & 0xf; in mode 5 only slots 0
/// and 2 accept requests (others return 0). An identical request keeps its handle; else the slot is rewritten with a
/// fresh handle and marked pending.
pub fn hud_slot_request(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    preserved(c, true, false, hud_slot_request_body)
}

fn hud_slot_request_body(c: &mut Cpu) -> bool {
    let (a0, a1, a2, a3) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3));
    let (t0, t1, t2) = (c.arg(4), c.arg(5), c.arg(6));
    let (slot, flags) = (a0 & 0xf, a0 & 0xfff0);
    let s = SLOT_TABLE + SLOT_SIZE * slot;
    if c.r32(GAME_MODE) == 5 && slot != 2 && slot != 0 {
        c.ret(0);
        return true;
    }
    let same = c.r32(s + S_KIND) == t1
        && c.r32(s + S_ARG_B) == t2
        && c.r32(s + S_ARG_A) == a1
        && c.r32(s + S_FLAGS) == flags
        && c.r32(s + S_ARG_C) == a2
        && c.r32(s + S_ARG_D) == a3
        && c.r32(s + S_ARG_E) == t0;
    if !same {
        let handle = c.r32(NEXT_HANDLE);
        let old_active = c.r32(s + S_FLAGS_ACTIVE);
        c.w32(s + S_KIND, t1);
        c.w32(s + S_HANDLE, handle);
        c.w32(s + S_ARG_B, t2);
        c.w32(NEXT_HANDLE, handle.wrapping_add(1));
        c.w32(s + S_ARG_A, a1);
        c.w32(s + S_ARG_C, a2);
        c.w32(s + S_ARG_D, a3);
        c.w32(s + S_ARG_E, t0);
        c.w32(s + S_PENDING, 1);
        c.w32(s + S_FLAGS, flags);
        c.w32(s + S_TIMER, 0);
        c.w32(s + S_ZERO_70, 0);
        if flags & old_active & 0x20 != 0 {
            call(c, SLOT_REQUEST_EXTRA, &[s], &[]);
        }
    }
    let h = c.r32(s + S_HANDLE);
    c.ret(h);
    true
}

/// 0x228b38 hud_slot_release(handle, flags): sets the slot's flags; when nothing is pending also its active flags.
pub fn hud_slot_release(c: &mut Cpu) -> bool {
    let (handle, flags) = (c.arg(0), c.arg(1));
    if let Some(i) = slot_of_handle(c, handle) {
        let s = SLOT_TABLE + SLOT_SIZE * i;
        c.w32(s + S_FLAGS, flags);
        if c.r32(s + S_PENDING) == 0 {
            c.w32(s + S_FLAGS_ACTIVE, flags);
        }
    }
    true
}

/// 0x228f58 hud_slot_extend(handle, n) -> 0 when the slot is live (timer = max(timer, n)), else 1.
pub fn hud_slot_extend(c: &mut Cpu) -> bool {
    preserved(c, true, false, hud_slot_extend_body)
}

fn hud_slot_extend_body(c: &mut Cpu) -> bool {
    let (handle, n) = (c.arg(0), c.arg(1));
    if let Some(i) = slot_of_handle(c, handle) {
        let s = SLOT_TABLE + SLOT_SIZE * i;
        if c.r32(s + S_PENDING) == 0 {
            let t = c.r32(s + S_TIMER) as i32;
            c.w32(s + S_TIMER, if (n as i32) < t { t as u32 } else { n });
            c.ret(0);
            return true;
        }
    }
    c.ret(1);
    true
}
