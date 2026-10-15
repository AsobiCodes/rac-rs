//! Talk-slot init, pad-history reset, pad-history pair scan and the particle alloc entry (retail 0x258a20, 0x258ec8,
//! 0x258f80, 0x259a38).

use crate::h;
use crate::misc_leaf;
use crate::particles;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// 0x258950 talk_slot_find(player) -> index into the talk tables, or -1.
const TALK_SLOT_FIND: u32 = 0x258950;
/// Per-index entry pointer table (0 = no entry for that index).
const TALK_ENTRY_TABLE: u32 = 0x1b1778;
/// Per-index 16-byte talk records (+0xc: nonzero when the record is live).
const TALK_RECORDS: u32 = 0x13d5b0;
/// Word copied into the npc's +0x38 field.
const TALK_SOURCE: u32 = 0x15f5ac;
/// Size of one talk entry (the npc's entry index is scaled by this).
const TALK_ENTRY_SIZE: u32 = 0x1c;
/// Pad history block read by input_history_scan.
const PAD_HISTORY: u32 = 0x13c940;
/// Bitmap allocator body: the `j` target of the 0x259a38 entry (mode in a1).
const ALLOC_BODY: u32 = 0x259a40;

/// 0x258a20 npc_talk_init(player, npc) -> slot index or -1: looks the player up in the talk slots; when the slot has
/// an entry, sets the npc's set flag (+9), its entry fields (+4, +0x36, +0x38, +0x3c) and its auto flag (+8).
pub fn npc_talk_init(c: &mut Cpu) -> bool {
    let (player, npc) = (c.arg(0), c.arg(1));
    let saves = [(16, 0, h::SQ), (31, 0x10, h::SQ)];
    h::frame(c, 0x20, &saves, |c, _sp| npc_talk_init_body(c, player, npc));
    true
}

fn npc_talk_init_body(c: &mut Cpu, player: u32, npc: u32) {
    c.s(16, npc as u64);
    h::call_keep(c, TALK_SLOT_FIND, &[player], &[]);
    let idx = c.g(2) as u32;
    c.s(6, c.g(2));
    c.set_arg(3, u32::MAX);
    if idx == u32::MAX {
        c.ret(u32::MAX);
        return;
    }
    let slot_off = idx.wrapping_mul(4);
    let table_addr = slot_off.wrapping_add(TALK_ENTRY_TABLE);
    c.set_arg(0, slot_off);
    c.set_arg(1, table_addr);
    let entry = c.r32(table_addr);
    if entry == 0 {
        c.s(3, 0);
        c.ret(u32::MAX);
        return;
    }
    c.w8(npc + 9, 1);
    let rec = TALK_RECORDS.wrapping_add(idx << 4);
    let source = c.r32(TALK_SOURCE);
    c.w16(npc + 4, 0xffff);
    c.w32(npc + 0x38, source);
    c.w16(npc + 0x36, 0);
    c.w32(npc + 0x3c, entry);
    // Retail skips the copy when the record is empty or the entry's +8 halfword is set.
    let skip = c.r32(rec + 0xc) == 0 || c.r16(entry + 8) as i16 != 0;
    if !skip {
        let v = c.r16(entry + 6);
        c.w16(npc + 0x36, v);
    }
    let offset = (c.r16(npc + 0x36) as i16 as i32 as u32).wrapping_mul(TALK_ENTRY_SIZE);
    let addr = c.r32(npc + 0x3c).wrapping_add(offset);
    let auto = c.r8(addr + 0x10) & 1;
    c.w8(npc + 8, auto);
    c.set_arg(0, addr);
    c.set_arg(1, offset);
    c.s(3, auto as u64);
    c.ret(idx);
}

/// 0x258ec8 pad_history_reset() -> PAD_HISTORY: clears the history count word (+0x190) and the halfword before it.
pub fn pad_history_reset(c: &mut Cpu) -> bool {
    c.w32(PAD_HISTORY + 0x190, 0);
    c.w16(PAD_HISTORY + 0x18e, 0);
    c.ret(PAD_HISTORY);
    true
}

/// 0x258f80 pad_pair_scan(mask0, mask1, frames, thresh) -> 0/1: input_history_scan on mask0, then on mask1 (both must
/// hit); 1 when the absolute gap between the two ages is below thresh (signed compare).
pub fn pad_pair_scan(c: &mut Cpu) -> bool {
    let (mask0, mask1, frames, thresh) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3));
    let saves = [(17, 0x20, h::SQ), (16, 0x10, h::SQ), (19, 0x40, h::SQ), (18, 0x30, h::SQ), (31, 0x50, h::SQ)];
    h::frame(c, 0x60, &saves, |c, sp| pad_pair_scan_body(c, sp, [mask0, mask1, frames, thresh]));
    true
}

fn pad_pair_scan_body(c: &mut Cpu, sp: u32, args: [u32; 4]) {
    let [mask0, mask1, frames, thresh] = args;
    let mut result = 0;
    let hit0 = h::run(c, particles::input_history_scan, &[mask0, frames, sp], &[]).0;
    if hit0 != 0 {
        let hit1 = h::run(c, particles::input_history_scan, &[mask1, frames, sp + 4], &[]).0;
        if hit1 != 0 {
            let age_gap = c.r32(sp).wrapping_sub(c.r32(sp + 4));
            let (gap, _) = h::run_keep(c, misc_leaf::iabs, &[age_gap], &[]);
            result = ((gap as i32) < (thresh as i32)) as u32;
        }
    }
    c.ret(result);
}

/// 0x259a38 particle_alloc_mode0(a0, a2, a3) -> slot: `j 0x259a40` with a1 = 0, i.e. the bitmap allocator body run in
/// mode 0 with the caller's other arguments.
pub fn particle_alloc_mode0(c: &mut Cpu) -> bool {
    c.s(5, 0);
    h::call_keep(c, ALLOC_BODY, &[], &[]);
    true
}
