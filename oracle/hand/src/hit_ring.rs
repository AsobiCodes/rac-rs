//! Hit-record ring: copies a 48-byte hit record into the 64-slot hit ring at 0x178200 for one object.
use crate::euler_basis;
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Base of the hit ring: 64 slots of 64 bytes.
const RING_BASE: u32 = 0x0017_8200;
/// Ring table; its word at +0x14 is the next slot index.
const RING_TABLE: u32 = 0x0017_3f40;
const RING_INDEX: u32 = RING_TABLE + 0x14;
/// Object byte holding the object's current slot index (0xff = none).
const OBJ_SLOT: u32 = 0xa4;
const NO_SLOT: u32 = 0xff;
/// Slot fields: sort key (float) and owning object.
const SLOT_KEY: u32 = 0x2c;
const SLOT_OWNER: u32 = 0x34;

/// Retail frame size and saved registers (s0..s6, ra), as stored by its prologue.
const FRAME: u32 = 0xb0;
const SAVES: [(usize, u32, u8); 8] = [
    (16, 0x30, h::SQ),
    (17, 0x40, h::SQ),
    (18, 0x50, h::SQ),
    (19, 0x60, h::SQ),
    (20, 0x70, h::SQ),
    (21, 0x80, h::SQ),
    (22, 0x90, h::SQ),
    (31, 0xa0, h::SQ),
];

/// 0x24c648 hit_ring_push(obj, rec) -> void: copies the 48-byte hit record at rec into the hit ring slot of obj,
/// unless obj's current slot is owned by obj and rec's key is below that slot's key; then advances the ring.
pub fn hit_ring_push(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let rec = c.arg(1);
    h::frame(c, FRAME, &SAVES, |c, sp| push(c, obj, rec, sp));
    true
}

/// Body of hit_ring_push inside the frame; `sp` is the frame's local copy of the record.
fn push(c: &mut Cpu, obj: u32, rec: u32, sp: u32) {
    c.s(5, NO_SLOT as u64);
    for off in [0u32, 0x10, 0x20] {
        let q = c.rv(rec + off);
        c.wv(sp + off, q);
    }
    let key = c.rf(sp + 0x1c);
    let slot = c.r8(obj + OBJ_SLOT) as u32;
    if slot != NO_SLOT {
        let cur = RING_BASE.wrapping_add(slot << 6);
        let owner = c.r32(cur + SLOT_OWNER);
        c.s(2, cur as u64);
        c.s(3, owner as i32 as i64 as u64);
        c.s(4, RING_BASE as u64);
        if owner == obj {
            let slot_key = c.rf(cur + SLOT_KEY);
            c.f[0] = key.0;
            c.f[1] = slot_key.0;
            if key < slot_key {
                return;
            }
        }
    }
    write_slot(c, obj, sp);
}

/// Fills the next ring slot from the local record and advances the ring index.
fn write_slot(c: &mut Cpu, obj: u32, sp: u32) {
    let idx = c.r32(RING_INDEX);
    let entry = RING_BASE.wrapping_add(idx << 6);
    let _ = h::run(c, euler_basis::zero_vec4, &[entry], &[]);
    let quad = c.rv(sp);
    c.wv(entry + 0x10, quad);
    let w = c.r32(sp + 0x10);
    c.w32(entry + 0x20, w);
    let w = c.r32(sp + 0x14);
    c.w32(entry + 0x24, w);
    let b = c.r8(sp + 0x18);
    c.w8(entry + 0x28, b);
    let b = c.r8(sp + 0x19);
    c.w8(entry + 0x29, b);
    let hw = c.r16(sp + 0x1a);
    c.w16(entry + 0x2a, hw);
    let k = c.r32(sp + 0x1c);
    c.w32(entry + SLOT_KEY, k);
    let w = c.r32(sp + 0x20);
    c.w32(entry + 0x30, w);
    c.w32(entry + SLOT_OWNER, obj);
    c.w32(entry + 0x38, 0);
    c.w8(obj + OBJ_SLOT, idx as u8);
    let next = (idx + 1) & 0x3f;
    c.w32(RING_INDEX, next);
    // Registers as retail leaves them at return.
    let v0 = c.r32(sp + 0x20) as i32 as i64 as u64;
    let v1 = c.r16(sp + 0x1a) as u64;
    c.s(2, v0);
    c.s(3, v1);
    c.s(4, next as u64);
    c.s(5, RING_TABLE as u64);
    c.f[0] = k;
}
