//! GS packet queue: the packet writer at 0x1eacb8 and two gate functions (0x1eb698, 0x1eb7b8) that guard calls into retail.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Word at 0x161140: write pointer of the packet buffer (advanced by the writer).
const PKT_PTR: u32 = 0x0016_1140;
/// Word at 0x15f5b8: busy flag; nonzero makes both gates return at once.
const GATE_BUSY: u32 = 0x0016_0000 - 0x0a48;
/// Byte at 0x141401: second gate flag.
const GATE_BYTE: u32 = 0x0014_0000 + 0x1401;
/// Word at 0x15f5a4: mode value; mode 3 selects the flag write.
const MODE_WORD: u32 = 0x0016_0000 - 0x0a5c;
/// Word at 0x15f3f4: set to 0x7f in mode 3.
const MODE_FLAG: u32 = 0x0016_0000 - 0x0c0c;
/// Table at 0x16cb40: three words (0x230, 0x234, 0x238) packed into the packet's second argument.
const TABLE: u32 = 0x0017_0000 - 0x34c0;
/// Packet tag words written by the writer.
const TAG_A: u32 = 0x3000_0013;
const TAG_B: u32 = 0x5000_0013;
const TAG_C: u32 = 0x3000_000b;
const TAG_D: u32 = 0x5000_000b;
/// Packet register-list words: low and high halves of the address pairs.
const REGS_A: u32 = 0x0014_0000 - 0x3040;
const REGS_B: u32 = 0x0014_0000 - 0x30f0;
/// Packet writer (retail callee) and its arguments.
const WRITER_FN: u32 = 0x0029_13c8;
const WRITER_ARG0: u32 = 0x3d;
/// Gate callee for mode_gate and idle callee for idle_hook (retail).
const GATE_FN: u32 = 0x001e_ae20;
const IDLE_FN: u32 = 0x0026_9f00;
/// gp-relative offsets: the packet-end slot and the gate flag word.
const GP_END_SLOT: u32 = 0x5ac0;
const GP_GATE_FLAG: u32 = 0x780c;
/// Frame size of the retail prologue shared by the three functions.
const FRAME: u32 = 0x10;

/// Writes `v` at the current packet pointer plus `off` (the pointer is reloaded for each store, as retail does).
fn put(c: &mut Cpu, off: u32, v: u32) {
    let p = c.r32(PKT_PTR);
    c.w32(p.wrapping_add(off), v);
}

/// 0x1eacb8 gs_queue_write() -> void: writes the packet tags and register-list words at the write pointer, advances
/// the pointer by 0x10 twice (0x20 total), stores the new pointer plus 0x10 in the gp slot and calls 0x2913c8.
pub fn gs_queue_write(c: &mut Cpu) -> bool {
    h::frame(c, FRAME, &[(31, 0x00, h::SQ)], |c, _sp| {
        let gp = c.g(28) as u32;
        put(c, 0x00, TAG_A);
        put(c, 0x04, REGS_A);
        put(c, 0x08, 0);
        put(c, 0x0c, TAG_B);
        let p = c.r32(PKT_PTR);
        c.w32(PKT_PTR, p.wrapping_add(0x10));
        c.w32(p.wrapping_add(0x10), TAG_C);
        put(c, 0x04, REGS_B);
        put(c, 0x08, 0);
        put(c, 0x0c, TAG_D);
        // Packed argument: three table words, the middle one shifted 8 and the last shifted 16 (64-bit, as dsll).
        let w230 = c.r32(TABLE + 0x230) as i32 as i64 as u64;
        let w234 = c.r32(TABLE + 0x234) as i32 as i64 as u64;
        let w238 = c.r32(TABLE + 0x238) as i32 as i64 as u64;
        let a1 = w230 | (w234 << 8) | (w238 << 16);
        let a2 = w238 << 16;
        let end = c.r32(PKT_PTR).wrapping_add(0x10);
        c.w32(gp.wrapping_sub(GP_END_SLOT), end);
        c.s(4, WRITER_ARG0 as u64);
        c.s(5, a1);
        c.s(6, a2);
        c.s(7, TABLE as u64);
        c.s(8, TAG_D as u64);
        c.s(9, TAG_C as u64);
        h::call_keep(c, WRITER_FN, &[], &[]);
    });
    true
}

/// 0x1eb698 gs_queue_gate() -> void: unless the busy word or the gate byte is set, writes 0x000100ff to gp-0x780c,
/// writes 0x7f to the mode flag when the mode is 3, then calls 0x1eae20.
pub fn gs_queue_gate(c: &mut Cpu) -> bool {
    h::frame(c, FRAME, &[(31, 0x00, h::SQ)], |c, _sp| {
        if c.r32(GATE_BUSY) != 0 || c.r8(GATE_BYTE) != 0 {
            return;
        }
        let gp = c.g(28) as u32;
        c.w32(gp.wrapping_sub(GP_GATE_FLAG), 0x0001_00ff);
        if c.r32(MODE_WORD) == 3 {
            c.w32(MODE_FLAG, 0x7f);
        }
        h::call_keep(c, GATE_FN, &[], &[]);
    });
    true
}

/// 0x1eb7b8 gs_queue_idle() -> void: unless the busy word is set, calls 0x269f00 with a0 = 0.
pub fn gs_queue_idle(c: &mut Cpu) -> bool {
    h::frame(c, FRAME, &[(31, 0x00, h::SQ)], |c, _sp| {
        if c.r32(GATE_BUSY) != 0 {
            return;
        }
        h::call_keep(c, IDLE_FN, &[0], &[]);
    });
    true
}
