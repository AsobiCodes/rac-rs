//! Marker moby spawn: allocates the single "current" marker moby (class 0x5f3) and an update that copies the
//! caller's quadword into it.

use crate::actor;
use crate::h;
use crate::ps2::*;
use crate::slot_setup;
use ee_oracle::cpu::Cpu;

/// Moby class of the marker.
const MARKER_CLASS: u32 = 0x05f3;
/// Global holding the current marker moby pointer, as an offset below gp.
const CUR_MARKER_GP_OFF: u32 = 0x58e4;
/// Base of the marker's global table (the `lui $at, 0x16` base); the stores below are negative offsets from it.
const MARKER_TABLE: u32 = 0x0016_0000;

/// Address of the current marker pointer (`gp - 0x58e4`).
fn cur_marker_addr(c: &Cpu) -> u32 {
    (c.r[28] as u32).wrapping_sub(CUR_MARKER_GP_OFF)
}

/// Stores a 64-bit value as two little-endian words.
fn w64(c: &mut Cpu, addr: u32, v: u64) {
    c.w32(addr, v as u32);
    c.w32(addr.wrapping_add(4), (v >> 32) as u32);
}

/// 0x29b5d0 alloc_marker_moby(a1 = quadword ptr) -> v0: allocates a class 0x5f3 moby; if none is current yet, makes it
/// the current marker, sets its fields, copies the quadword at a1 into it and claims a colour slot for it. Returns
/// the new moby, or the existing current marker when one is already set, or 0 if the allocation failed.
pub fn alloc_marker_moby(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let src = c.arg(1);
    h::frame(c, 0x40, &[(16, 0x00, h::SQ), (17, 0x10, h::SQ), (18, 0x20, h::SQ), (31, 0x30, h::SQ)], |c, _sp| {
        alloc_body(c, src)
    });
    true
}

/// Body of `alloc_marker_moby`, run inside its retail frame.
fn alloc_body(c: &mut Cpu, src: u32) {
    let (moby, _) = h::run_keep(c, actor::alloc_moby, &[MARKER_CLASS], &[]);
    if moby == 0 {
        c.ret(0);
        return;
    }
    let cur_addr = cur_marker_addr(c);
    let cur = c.r32(cur_addr);
    if cur != 0 {
        c.ret(cur);
        return;
    }

    c.w8(moby + 0x30, 0xff);
    c.w8(moby + 0x31, 1);
    c.w16(moby + 0x32, 0xff);
    c.w8(moby + 0x20, 0);
    let q = c.rv(src);
    c.wv(moby + 0x10, q);
    let slot = c.r32(moby + 0x78);
    let flags: u64 = (0xf800u64 << 21) | 0x42;

    c.w32(MARKER_TABLE.wrapping_sub(0xcdc), 0xff);
    c.w32(MARKER_TABLE.wrapping_sub(0xccc), 0x00ff_ffff);
    w64(c, MARKER_TABLE.wrapping_sub(0xcc8), flags);
    c.w32(cur_addr, moby);
    w64(c, MARKER_TABLE.wrapping_sub(0xcd8), 0);
    c.w32(MARKER_TABLE.wrapping_sub(0xce0), 0);
    c.w32(MARKER_TABLE.wrapping_sub(0xcd0), 0);

    c.w32(slot.wrapping_add(0x10), 0);
    // The colour claim reads the scratch registers retail left set: v1 = 0xff, a2 = 0x00ffffff, a3 = flags.
    c.r[3] = 0xff;
    c.r[6] = 0x00ff_ffff;
    c.r[7] = flags as u128;
    let (colour, _) = h::run_keep(c, slot_setup::claim_slot_color, &[src, 0], &[F(0), F(0)]);
    c.w16(slot.wrapping_add(0x2c), colour as u16);
    c.ret(moby);
}

/// 0x29b6b0 marker_update(a0, a1 = quadword ptr): if no marker is current, spawns one (with a1 as its quadword); if the
/// current marker has class 0x5f3, copies the quadword at a1 into its slot and sets the flag at its record + 0x2e.
pub fn marker_update(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let a0 = c.arg(0);
    let quad = c.rv(c.arg(1));
    h::frame(c, 0x20, &[(31, 0x10, h::SQ)], |c, sp| {
        c.wv(sp, quad);
        update_body(c, a0, sp)
    });
    true
}

/// Body of `marker_update`, run inside its retail frame (`sp` holds the quadword).
fn update_body(c: &mut Cpu, a0: u32, sp: u32) {
    if c.r32(cur_marker_addr(c)) == 0 {
        h::run_keep(c, alloc_marker_moby, &[a0, sp], &[]);
        if c.r32(cur_marker_addr(c)) == 0 {
            return;
        }
    }
    let cur = c.r32(cur_marker_addr(c));
    let class = c.r16(cur + 0xa6);
    let rec = c.r32(cur + 0x78);
    // Scratch registers as retail leaves them: v0 = halfword, v1 = class, a0 = marker, a1 = its record.
    c.r[2] = class as i16 as i64 as u64 as u128;
    c.r[3] = MARKER_CLASS as u128;
    c.r[4] = cur as u128;
    c.r[5] = rec as u128;
    if u32::from(class) == MARKER_CLASS {
        let q = c.rv(sp);
        c.wv(cur + 0x10, q);
        c.w16(rec + 0x2e, 1);
        c.r[2] = c.mem.rd128(sp);
        c.r[3] = 1;
    }
}
