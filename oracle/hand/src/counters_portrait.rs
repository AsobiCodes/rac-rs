//! Controller idle-run counter and tick bookkeeping (0x288690), and the portrait sequence install (0x28ab50).
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Sign-extends a 32-bit value into a 64-bit GPR the way lw and addiu leave it.
fn sx(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// Controller block at 0x13c940: a gate word at +0x1a0, four float fields read in this order.
const CTRL_BLOCK: u32 = 0x0014_0000 - 0x36c0;
const CTRL_GATE: u32 = 0x1a0;
const CTRL_FIELDS: [u32; 4] = [0x108, 0x10c, 0x100, 0x104];
/// gp-relative: idle run count (gp - 0x5c90), copy of the second tick word (gp - 0x7d5c), and the word
/// compared against the scaled float (gp - 0x7dd8).
const GP_RUN: u32 = (-0x5c90i32) as u32;
const GP_SEEN: u32 = (-0x7d5ci32) as u32;
const GP_LIMIT: u32 = (-0x7dd8i32) as u32;
/// Page 0x16 words: tick A (0x15f5ac), tick B (0x15eea4), gate C (0x15f578), limit tick D (0x15ee28).
const PAGE16: u32 = 0x0016_0000;
const TICK_A: u32 = PAGE16 - 0x0a54;
const TICK_B: u32 = PAGE16 - 0x115c;
const TICK_GATE: u32 = PAGE16 - 0x0a88;
const TICK_D: u32 = PAGE16 - 0x11d8;
/// Retail float_to_int_scaled's helper argument and the 60.0 scale (0x42700000).
const SCALE_ARG: u32 = 0xf;
const SCALE_60: F = F(0x4270_0000);
/// Retail helper at 0x118a80 (called with a0 = 0 by the portrait install).
const RETAIL_118A80: u32 = 0x0011_8a80;

/// The gate word and the four float fields tested as the retail chain does: f1 becomes 0.0 when the first field is
/// read, f0 keeps the last field loaded. True when the gate is clear and every field is 0.0.
fn block_idle(c: &mut Cpu) -> bool {
    if c.r32(CTRL_BLOCK + CTRL_GATE) != 0 {
        return false;
    }
    c.f[1] = F(0).0;
    for off in CTRL_FIELDS {
        let x = c.rf(CTRL_BLOCK + off);
        c.f[0] = x.0;
        if x != F(0) {
            return false;
        }
    }
    true
}

/// 0x288690 controller_tail_counters() -> none: idle-run counter, tick counters, and a limit bump when the run is
/// below 60 * float_to_int_scaled(15).
pub fn controller_tail_counters(c: &mut Cpu) -> bool {
    let gp = c.g(28) as u32;
    let run_addr = gp.wrapping_add(GP_RUN);
    let idle = block_idle(c);
    let run = if idle { c.r32(run_addr).wrapping_add(1) } else { 0 };
    c.w32(run_addr, run);

    // Tick words: A is bumped in place, B's bump is kept in gp (v1 in the register file).
    let tick_a = c.r32(TICK_A).wrapping_add(1);
    let tick_b = c.r32(TICK_B).wrapping_add(1);
    c.w32(TICK_A, tick_a);
    c.w32(gp.wrapping_add(GP_SEEN), tick_b);
    c.s(2, sx(tick_a));
    c.s(3, sx(tick_b));
    c.s(1, sx(PAGE16));

    let gate = c.r32(TICK_GATE);
    c.s(4, sx(gate));
    let mut fire = false;
    if gate != 0 {
        let (v0, _) = crate::h::run_keep(c, crate::mathx::float_to_int_scaled, &[SCALE_ARG], &[]);
        let prod = F::from_int(v0 as i32) * SCALE_60;
        let t = prod.to_int();
        c.f[0] = prod.0;
        c.f[1] = t as u32;
        c.s(3, sx(t as u32));
        fire = (run as i32) < t;
    }

    if gate == 0 {
        // Retail beqz a0 jumps straight to the end, so v0 still holds tick A and the limit is never read.
        c.s(2, sx(tick_a));
        return true;
    }
    let limit = c.r32(gp.wrapping_add(GP_LIMIT));
    if fire {
        c.w32(TICK_D, limit.wrapping_add(1));
        c.s(2, sx(limit.wrapping_add(1)));
        c.s(1, sx(PAGE16));
    } else {
        c.s(2, sx(limit));
    }
    true
}

/// Portrait install constants: descriptor pointer slot, record table, and sequence slot layout.
const DESC_PTR: u32 = 0x001c_a558;
/// Table of record pointers at 0x197400, indexed by the byte at 0x197ccc.
const SEQ_TABLE: u32 = 0x0019_7400;
const SEQ_BYTE: u32 = 0x0019_7ccc;
const SEQ_COUNT: u32 = 16;
/// Record offset of the 16 sequence slots, and the bias added to the unpacked base (0x80000).
const SEQ_SLOTS: u32 = 0x48;
const UNPACK_BIAS: u32 = 0x0008_0000;

/// 0x28ab50 portrait_seqs_install() -> none: unpacks the portrait WAD into the buffer at base + 0x80000, then rebases
/// the 16 sequence slots of the selected record by the descriptor base.
pub fn portrait_seqs_install(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let base = c.r32(DESC_PTR);
    c.s(2, sx(UNPACK_BIAS));
    let dst = base.wrapping_add(UNPACK_BIAS);
    c.s(4, 0);
    crate::h::call_keep(c, RETAIL_118A80, &[0], &[]);

    let src = c.r32(DESC_PTR);
    crate::h::run_keep(c, crate::spr_stream::spr_lzo_unpack, &[dst, src], &[]);
    crate::h::call_keep(c, RETAIL_118A80, &[0], &[]);

    c.s(4, sx(0x0019_0000));
    c.s(3, sx(SEQ_TABLE));
    let slot = SEQ_TABLE.wrapping_add(4 * c.r8(SEQ_BYTE) as u32);
    c.s(2, sx(slot));
    let rec = c.r32(slot);
    let mut out = rec.wrapping_add(SEQ_SLOTS);
    let mut ptr = c.r32(DESC_PTR);
    for i in 0..SEQ_COUNT {
        c.s(5, sx(i));
        let entry = c.r32(ptr);
        c.s(3, sx(entry));
        c.s(4, sx(rec));
        ptr = ptr.wrapping_add(4);
        if entry != 0 {
            let v = c.r32(DESC_PTR).wrapping_add(entry);
            c.s(2, sx(v));
            c.w32(out, v);
            crate::h::run_keep(c, crate::record_pool::rebase_record, &[rec, i], &[]);
        }
        let more = i + 1 < SEQ_COUNT;
        c.s(2, more as u64);
        if !more {
            break;
        }
        out = out.wrapping_add(4);
    }
    true
}
