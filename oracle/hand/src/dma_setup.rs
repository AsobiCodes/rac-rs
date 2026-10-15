//! Packet-cursor tag writers. Each emits a 16-byte GS/VIF tag block at the current packet cursor (a pointer kept at
//! 0x161140), then advances the cursor by 0x10 and stores the new cursor in a gp-relative slot.
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Pointer to the current packet write position.
const CURSOR_PTR: u32 = 0x0016_1140;
/// Pointer to the packet base; its word plus a small offset forms the first tag's address field.
const BASE_PTR: u32 = 0x0015_eeb8;
/// gp-relative slot that receives the advanced cursor.
const CURSOR_SLOT_GP: i32 = -0x5ac0;
/// Retail flush routine, called when no packet buffer is set up.
const FLUSH_FN: u32 = 0x0012_21b0;
/// Keeps the low 28 bits of the address field.
const ADDR_MASK: u32 = 0x0fff_ffff;
/// Tag words for the 0x09 block (word 0 and word 3).
const TAG_LO_09: u32 = 0x3000_0009;
const TAG_HI_09: u32 = 0x5000_0009;
/// Tag words for the GS block (word 0 and word 3).
const TAG_LO_26: u32 = 0x3000_0026;
const TAG_HI_26: u32 = 0x5000_0026;
/// Second word of the GS block.
const GS_ADDR_26: u32 = 0x0015_1900;

/// Writes the four words of a 16-byte block at `p`; word 2 is zero.
fn write_block(c: &mut Cpu, p: u32, w0: u32, w1: u32, w3: u32) {
    c.w32(p, w0);
    c.w32(p + 4, w1);
    c.w32(p + 8, 0);
    c.w32(p + 0xc, w3);
}

/// Stores the advanced cursor (`p + 0x10`) in the gp slot and returns it.
fn advance(c: &mut Cpu, p: u32) -> u32 {
    let next = p.wrapping_add(0x10);
    let slot = (c.g(28) as u32).wrapping_add(CURSOR_SLOT_GP as u32);
    c.w32(slot, next);
    next
}

/// Leaves a0..v1 holding the values retail's last instructions produce.
fn set_regs(c: &mut Cpu, a0: u32, a1: u32, v0: u32, v1: u32) {
    c.s(4, a0 as i32 as i64 as u64);
    c.s(5, a1 as i32 as i64 as u64);
    c.s(2, v0 as i32 as i64 as u64);
    c.s(3, v1 as i32 as i64 as u64);
}

/// Emits the 0x09 block at cursor `p`; its address word is `(base word + base_off) & ADDR_MASK`.
fn emit_09(c: &mut Cpu, p: u32, base_off: u32) {
    let w1 = c.r32(BASE_PTR).wrapping_add(base_off) & ADDR_MASK;
    write_block(c, p, TAG_LO_09, w1, TAG_HI_09);
    let next = advance(c, p);
    set_regs(c, p, TAG_HI_09, next, p);
}

/// 0x1f4050 emit_tags_or_flush(): with a packet cursor, emits a 0x09 block (base + 0x30); without one, calls the flush
/// routine 0x1221b0 with (base + 0x30). Returns nothing.
pub fn tag_emit_or_flush(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let sp = c.g(29) as u32;
    let p = c.r32(CURSOR_PTR);
    // The prologue saves ra at the new stack slot before branching on the cursor.
    let ra = c.g(31);
    c.wv(sp.wrapping_sub(0x10), V([ra as u32, (ra >> 32) as u32, 0, 0]));
    if p != 0 {
        emit_09(c, p, 0x30);
    } else {
        let arg = c.r32(BASE_PTR).wrapping_add(0x30);
        c.set_arg(0, arg);
        c.call(FLUSH_FN);
        // The epilogue reloads ra from the saved slot.
        c.s(31, ra);
    }
    true
}

/// 0x1f4150 emit_tag_09(): emits a 0x09 block at the packet cursor (base + 0xc0) and advances the cursor.
pub fn tag_emit(c: &mut Cpu) -> bool {
    let p = c.r32(CURSOR_PTR);
    emit_09(c, p, 0xc0);
    true
}

/// 0x1f4400 emit_tag_gs(): emits the GS block (tag 0x26, address 0x151900) at the packet cursor and advances it.
pub fn tag_emit_gs(c: &mut Cpu) -> bool {
    let p = c.r32(CURSOR_PTR);
    write_block(c, p, TAG_LO_26, GS_ADDR_26, TAG_HI_26);
    let next = advance(c, p);
    set_regs(c, TAG_HI_26, p, next, p);
    true
}
