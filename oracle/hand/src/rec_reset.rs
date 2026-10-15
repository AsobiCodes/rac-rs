//! Helpers for the 0x1516d0 record (retail s2 = 0x150000, record at +0x16d0): slot reset and fixed halfword writes.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const S2: usize = 18;
const RA: usize = 31;

/// Block base loaded with `lui 0x15`; the record sits at +0x16d0.
const BLOCK: u32 = 0x0015_0000;
const REC: u32 = BLOCK + 0x16d0;

/// Poll helper called with the record pointer until a pending word clears.
const POLL_FN: u32 = 0x0012_dc80;
/// Called once with the record pointer after the polls.
const LINK_FN: u32 = 0x0012_ebd0;
/// Called with 1 after the second poll loop.
const RELEASE_FN: u32 = 0x0012_ed30;
const PENDING: u32 = 0xffff_ffff;

/// Words polled (while equal to -1) before the reset.
const POLL_OFFS: [u32; 2] = [0x34, 0x6c];
const OFF_STATE_BYTE_A: u32 = 0x22;
const OFF_STATE_BYTE_B: u32 = 0x23;

/// Signed halfword limit written into the record's pairs.
const LIMIT: i16 = -0x8000;

/// 0x257840 reset_slots() -> (): polls three pending words of record 0x1516d0 until they clear (calling 0x12dc80
/// each time), calls 0x12ebd0, polls 0x12dc80 until it returns 0, calls 0x12ed30(1), then zeroes the record's
/// timers and sets its two state bytes to -1 (the first state byte is kept in the halfword at +0x38 when not -1).
pub fn reset_slots(c: &mut Cpu) -> bool {
    let saves = [(16, 0x00, h::SQ), (S2, 0x20, h::SQ), (17, 0x10, h::SQ), (RA, 0x30, h::SQ)];
    h::frame(c, 0x40, &saves, |c, _sp| {
        c.s(S2, BLOCK as u64);
        for off in POLL_OFFS {
            while c.r32(REC + off) == PENDING {
                h::call_keep(c, POLL_FN, &[REC], &[]);
            }
        }
        // retail 0x257900 loop: its delay slot is a nop, so re-calls pass whatever a0 the callee left
        c.s(A0, REC as u64);
        while c.r32(REC + 0x50) == PENDING {
            let a0 = c.arg(0);
            h::call_keep(c, POLL_FN, &[a0], &[]);
        }
        let a0 = c.arg(0);
        h::call_keep(c, LINK_FN, &[a0], &[]);
        loop {
            let a0 = c.arg(0);
            let (v0, _) = h::call_keep(c, POLL_FN, &[a0], &[]);
            if v0 == 0 {
                break;
            }
        }
        h::call_keep(c, RELEASE_FN, &[1], &[]);

        let state = c.r8(REC + OFF_STATE_BYTE_A) as i8 as i64;
        c.s(V0, REC as u64);
        c.s(V1, state as u64);
        c.s(A0, u64::MAX);
        c.w16(REC + 0x3e, 0);
        c.w16(REC + 0x3c, 0);
        c.w32(REC + 0x34, 0);
        if state != -1 {
            c.w16(REC + 0x38, state as u16);
        }
        c.w8(REC + OFF_STATE_BYTE_B, 0xff);
        c.w16(REC + 0x5a, 0);
        c.w16(REC + 0x58, 0);
        c.w32(REC + 0x50, 0);
        c.w16(REC + 0x76, 0);
        c.w16(REC + 0x74, 0);
        c.w32(REC + 0x6c, 0);
        c.w16(REC + 0x20, 0);
        c.w8(REC + OFF_STATE_BYTE_A, 0xff);
    });
    true
}

/// 0x2579a8 set_pair_limits(flag) -> (): when flag (a0) is nonzero, writes the limit pair at +0x5c/+0x5e of record
/// 0x1516d0; always writes the limit pairs at +0x78/+0x7a and +0x40/+0x42.
pub fn set_pair_limits(c: &mut Cpu) -> bool {
    if c.g(A0) != 0 {
        c.w16(REC + 0x5c, LIMIT as u16);
        c.w16(REC + 0x5e, 0);
    }
    c.w16(REC + 0x78, LIMIT as u16);
    c.w16(REC + 0x7a, 0);
    c.w16(REC + 0x40, LIMIT as u16);
    c.w16(REC + 0x42, 0);
    c.s(V0, REC as u64);
    c.s(V1, LIMIT as i64 as u64);
    c.s(A1, BLOCK as u64);
    true
}

/// 0x2579e0 set_four() -> (): writes halfword 4 at +0x5c, +0x40 and +0x78 of record 0x1516d0.
pub fn set_four(c: &mut Cpu) -> bool {
    c.w16(REC + 0x5c, 4);
    c.w16(REC + 0x40, 4);
    c.w16(REC + 0x78, 4);
    c.s(V0, REC as u64);
    c.s(V1, 4);
    true
}
