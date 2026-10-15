//! Bubble bounce step of the HUD aquarium and Morph-o-Ray meters, and a pass-through wrapper.

use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// 17.0f: the lower bound the bubbles bounce off.
const LOW_BOUND: F = F(0x4188_0000);
/// 116.0f: the upper bound the bubbles bounce off.
const HIGH_BOUND: F = F(0x42e8_0000);
/// Frame counter word (`lw` from 0x15f3f8, which is 0x160000 - 0xc08).
const COUNTER_ADDR: u32 = 0x15_f3f8;
/// First bubble record: 0x180000 - 0x18a0 (lui a0,0x18; addiu a0,-0x18a0).
const BUBBLE_BASE: u32 = 0x17_e760;
/// Each bubble record is 0x10 bytes; a step moves the walk two records (0x20 bytes) on.
const RECORD: u32 = 0x10;
/// Walk stops once the record index reaches this count.
const BUBBLE_COUNT: i32 = 100;
/// gp-relative float thresholds: picked by the record index (below 10, below 30, from 30 on).
const GP_THRESH_LOW: i32 = -0x74d0;
const GP_THRESH_MID: i32 = -0x74cc;
const GP_THRESH_HIGH: i32 = -0x74c8;

fn gp(c: &Cpu, off: i32) -> u32 {
    (c.g(28) as u32).wrapping_add(off as u32)
}

/// 0x229e40 bubble_bounce_step(): frame tick of the bubble meters: runs 0x229268, then walks the bubble records
/// (two per step, from the frame counter's parity) and bounces each record's position off the bounds.
pub fn bubble_bounce_step(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    h::frame(c, 0x10, &[(31, 0, h::SQ)], |c, _sp| {
        h::call_keep(c, 0x229268, &[], &[]);
        bounce_bubbles(c);
    });
    true
}

/// The record walk of 0x229e40, after the call to 0x229268. Every load and store is done in retail's order.
fn bounce_bubbles(c: &mut Cpu) {
    // (counter - (counter / 2) * 2) is counter % 2 in -1..=1, so the walk starts at record 0, 1 or -1.
    let mut index = (c.r32(COUNTER_ADDR) as i32) % 2;
    let mut rec = BUBBLE_BASE.wrapping_add((index << 4) as u32);
    if index < BUBBLE_COUNT {
        let f8 = c.rf(gp(c, GP_THRESH_HIGH));
        let f9 = c.rf(gp(c, GP_THRESH_MID));
        let f10 = c.rf(gp(c, GP_THRESH_LOW));
        c.f[11] = HIGH_BOUND.0;
        c.f[12] = LOW_BOUND.0;
        let (mut f0, mut f1, mut f2, mut f3, mut f4, mut f5, mut f6, mut f7);
        loop {
            let thresh = if index < 10 {
                f10
            } else if index < 30 {
                f9
            } else {
                f8
            };
            f2 = c.rf(rec);
            f3 = thresh - LOW_BOUND;
            f5 = c.rf(rec + 8);
            f4 = thresh - HIGH_BOUND;
            f1 = c.rf(rec + 4);
            f2 = f2 + f5;
            f0 = c.rf(rec + 0xc);
            f6 = -f3;
            f1 = f1 + f0;
            f7 = -f4;
            c.wf(rec, f2);
            c.wf(rec + 4, f1);
            if f2 < f3 {
                f0 = -f5;
                c.wf(rec, f3);
                c.wf(rec + 8, f0);
            }
            f0 = c.rf(rec);
            if f6 < f0 {
                f0 = c.rf(rec + 8);
                c.wf(rec, f6);
                f0 = -f0;
                c.wf(rec + 8, f0);
            }
            f0 = c.rf(rec + 4);
            if f0 < f4 {
                f0 = c.rf(rec + 0xc);
                c.wf(rec + 4, f4);
                f0 = -f0;
                c.wf(rec + 0xc, f0);
            }
            f0 = c.rf(rec + 4);
            index += 2;
            if f7 < f0 {
                f0 = c.rf(rec + 0xc);
                c.wf(rec + 4, f7);
                f0 = -f0;
                c.wf(rec + 0xc, f0);
            }
            rec = rec.wrapping_add(2 * RECORD);
            if index >= BUBBLE_COUNT {
                break;
            }
        }
        // Scratch float registers as the last pass leaves them.
        c.f[0] = f0.0;
        c.f[1] = f1.0;
        c.f[2] = f2.0;
        c.f[3] = f3.0;
        c.f[4] = f4.0;
        c.f[5] = f5.0;
        c.f[6] = f6.0;
        c.f[7] = f7.0;
        c.f[8] = f8.0;
        c.f[9] = f9.0;
        c.f[10] = f10.0;
    }
    c.s(1, HIGH_BOUND.0 as i32 as i64 as u64); // $at still holds the last lui (116.0 bits)
    c.s(2, 0);
    c.s(3, rec as i32 as i64 as u64);
    c.s(4, BUBBLE_BASE as i32 as i64 as u64);
    c.s(5, index as i64 as u64);
}

/// 0x22a8d8 passthrough_228ec0(): frame, call 0x228ec0 with the arguments as they came in, return its results.
pub fn passthrough_228ec0(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    h::frame(c, 0x10, &[(31, 0, h::SQ)], |c, _sp| {
        h::call_keep(c, 0x228ec0, &[], &[]);
    });
    true
}
