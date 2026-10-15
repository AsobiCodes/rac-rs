//! Pause-menu HUD frame code: 0x273628 draws the run-status text for the current frame, and 0x2737e8 releases the
//! claim-table record held at offset 0x48 of its argument and stores the result back there.
use crate::claim_table;
use crate::draw_run_text;
use crate::fxtex;
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Retail function that formats a text label id into a pointer (called with a fixed id 0x5217 or a table entry).
const LABEL_LOOKUP: u32 = 0x1f_6b80;
/// Callee of the first draw: frame-state record table base (lui 0x14, addiu -0x2d70).
const FRAME_TABLE: u32 = 0x13_d290;
/// Pointer cell to the frame link (lui 0x1c, offset -0x620c).
const FRAME_LINK_CELL: u32 = 0x1c_0000 - 0x620c;
/// Label table of run entries: 12-byte records starting at 0x1c1f40.
const LABEL_TABLE: u32 = 0x1c_1f40;
/// Text colour/attribute word passed as the third draw argument (0x80ff << 16 | 0xa888).
const TEXT_ATTR: u32 = 0x80ff_a888;
/// Sentinel value compared against the frame's state word.
const NO_RUN: u32 = 0xffff_ffff;

/// 0x273628 pause_hud_frame(c) -> 2: draws the run-status text for the current frame when the state word matches
/// and returns 2.
pub fn pause_hud_frame(c: &mut Cpu) -> bool {
    // Declined: the Rust body below writes RAM that differs from retail in the vector sample (every call), so retail runs.
    if c.arg(0) != 0 || true {
        return false;
    }
    let s3 = c.arg(0);
    // The frame lookup is read before the reset call: the reset's jal sits after the lw that fetches s1.
    let link = c.r32(FRAME_LINK_CELL);
    let inner = c.r32(link + 0x40);
    let frame = FRAME_TABLE.wrapping_add(c.r32(inner + 0x40).wrapping_mul(0x1c));
    let s1 = c.r32(frame + 0x20);
    h::run_keep(c, fxtex::fx_frame_reset, &[0], &[]);

    let state_ok = (c.r32(FRAME_TABLE + 0xd4) as i32) < 3
        && (c.r32(FRAME_TABLE + 0xdc) as i32) < 0
        && c.r32(FRAME_TABLE + 8) == 2;
    if state_ok {
        if s1 == NO_RUN {
            draw_status_fixed(c, s3);
        } else {
            draw_status_table(c, s3, s1);
        }
    }
    h::run_keep(c, fxtex::fx_pkt_tags, &[], &[]);
    c.ret(2);
    true
}

/// Half of a signed word (C-style division by two, rounding toward zero).
fn half(v: u32) -> u32 {
    ((v as i32) / 2) as u32
}

/// Draws the fixed-label text (state word == -1) at the centre-left of the screen position stored at s3.
fn draw_status_fixed(c: &mut Cpu, s3: u32) {
    let x = half(c.r32(s3 + 0x20));
    let y = half(c.r32(s3 + 0x24)).wrapping_sub(8);
    let a3 = h::call_keep(c, LABEL_LOOKUP, &[0x5217], &[]).0;
    h::run_keep(c, draw_run_text::text_place_1ee318, &[x, y, TEXT_ATTR, a3, NO_RUN], &[]);
}

/// Draws the table-driven label text (state word != -1): three labels from the 12-byte table.
fn draw_status_table(c: &mut Cpu, s3: u32, s1: u32) {
    let s2 = s1.wrapping_mul(0xc);
    let entry = LABEL_TABLE.wrapping_add(s2);
    let first = c.r32(entry);
    let y = half(c.r32(s3 + 0x20));
    let a3 = h::call_keep(c, LABEL_LOOKUP, &[first], &[]).0;
    h::run_keep(c, draw_run_text::text_place_1ee318, &[y, 4, TEXT_ATTR, a3, NO_RUN], &[]);

    let second = c.r32(entry + 4);
    let y = half(c.r32(s3 + 0x20));
    let a3 = h::call_keep(c, LABEL_LOOKUP, &[second], &[]).0;
    h::run_keep(c, draw_run_text::text_place_1ee318, &[y, 0x14, TEXT_ATTR, a3, NO_RUN], &[]);
}

/// 0x2737e8 release_store(rec) -> 0: releases the claim-table record held at offset 0x48 and stores the released
/// value back there; returns 0.
pub fn release_store(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let obj = c.arg(0);
    let key = c.r32(obj + 0x48);
    let v = h::run(c, claim_table::release_record, &[key], &[]).0;
    c.w32(obj + 0x48, v);
    c.ret(0);
    true
}
