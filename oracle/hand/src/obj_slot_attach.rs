//! Object slot attach/detach of the Eudora overlay: an object (word +0x44) holds one pooled slot record. A row
//! table gives the wanted slot id; a replacement slot inherits the old slot's data and owner link.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Word holding the pointer to the current scene context (lui 0x1c, lw -0x620c).
const CTX_PTR: u32 = 0x001b_9df4;
/// Table of 0x4c-byte rows, indexed by a halfword taken from the context (lui 0x18, addiu -0x6440).
const ROW_TABLE: u32 = 0x0017_9bc0;
const ROW_STRIDE: i32 = 0x4c;
/// Defaults for a freshly allocated slot (lui 0x16, addiu 0x6d80): floats at +0x140, +0x144, +0x148.
const SLOT_DEFAULTS: u32 = 0x0016_6d80;
/// Word stored into a fresh slot at +0x74 (lui 0x27, addiu -0x1580).
const SLOT_KIND: u32 = 0x0026_ea80;
/// Retail helpers: allocate a slot for an id (returns it, or 0), and release a slot.
const SLOT_ALLOC: u32 = 0x0027_5d20;
const SLOT_FREE: u32 = 0x0027_5dc0;
/// Offsets inside the object and the slot record.
const OBJ_SLOT: u32 = 0x44;
const SLOT_KIND_OFF: u32 = 0x74;
const SLOT_OWNER_OFF: u32 = 0x78;
const SLOT_ID_OFF: u32 = 0xa6;
const ROW_ID_OFF: u32 = 0x3a;
const NO_ID: i32 = -1;
/// Float constants: 6.0 (added to the default at +0x140), 0.3 (subtracted from +0x148), 3.14159 (pi-like).
const SIX: F = F(0x40c0_0000);
const POINT_THREE: F = F(0x3e99_999a);
const PI_LIKE: F = F(0x4049_0fdb);

/// Register numbers used below.
const AT: usize = 1;
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;

fn store128(c: &mut Cpu, addr: u32, v: u128) {
    for i in 0..4u32 {
        c.w32(addr + 4 * i, (v >> (32 * i)) as u32);
    }
}

/// Sets a GPR to a 32-bit result, sign-extended the way MIPS 32-bit ops and lw leave it.
fn set_gpr(c: &mut Cpu, n: usize, v: u32) {
    c.s(n, v as i32 as i64 as u64);
}

fn read_id(c: &mut Cpu, addr: u32) -> i32 {
    c.r16(addr) as i16 as i32
}

/// 0x26e820 obj_slot_attach(obj) -> 0: points obj's slot (+0x44) at the pool slot for the id in the row table.
/// No id: release the slot. Same id already attached: nothing. Nothing attached: allocate and initialize one.
/// Another id attached: allocate a new slot, copy the old slot's data over, release the old one.
pub fn obj_slot_attach(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    // The prologue spills s0, s1 and ra into the 0x30-byte frame, and retail leaves them there.
    let frame = (c.g(29) as u32).wrapping_sub(0x30);
    let (s0, s1, ra) = (c.r[16], c.r[17], c.r[31]);
    store128(c, frame, s0);
    store128(c, frame + 0x10, s1);
    store128(c, frame + 0x20, ra);

    let ctx = c.r32(CTX_PTR);
    let block = c.r32(ctx + 0x40);
    let cell = c.r32(block + 0x3c).wrapping_mul(10).wrapping_add(c.r32(block + 0x48));
    let row_index = read_id(c, cell + 6);
    let slot = c.r32(obj + OBJ_SLOT);
    let cur_id = if slot == 0 { NO_ID } else { read_id(c, slot + SLOT_ID_OFF) };
    let row = (row_index.wrapping_mul(ROW_STRIDE) as u32).wrapping_add(ROW_TABLE);
    let want = read_id(c, row + ROW_ID_OFF);
    // Registers as retail leaves them before its first call (the callees may read some of them).
    set_gpr(c, V0, row);
    set_gpr(c, V1, ROW_TABLE);
    set_gpr(c, A0, want as u32);
    set_gpr(c, A1, cur_id as u32);
    set_gpr(c, A2, NO_ID as u32);
    set_gpr(c, A3, slot);

    if want == NO_ID {
        set_gpr(c, A0, slot);
        let (released, _) = h::call_keep(c, SLOT_FREE, &[slot], &[]);
        c.w32(obj + OBJ_SLOT, released);
    } else if cur_id == NO_ID {
        let (new, _) = h::call_keep(c, SLOT_ALLOC, &[want as u32], &[]);
        set_gpr(c, A1, new);
        if new != 0 {
            attach_fresh(c, obj, new);
        }
    } else if cur_id != want {
        let (new, _) = h::call_keep(c, SLOT_ALLOC, &[want as u32], &[]);
        let old = c.r32(obj + OBJ_SLOT);
        if new != 0 {
            c.w16(new + 0x34, 0);
            let lo = c.rv(old + 0x10);
            c.wv(new + 0x10, lo);
            let hi = c.rv(old + 0x40);
            c.wv(new + 0x40, hi);
            let kind = c.r32(old + SLOT_KIND_OFF);
            let owner = c.r32(new + SLOT_OWNER_OFF);
            c.w32(new + SLOT_KIND_OFF, kind);
            c.w32(owner, obj);
            set_gpr(c, A1, new + 0x40);
            set_gpr(c, A2, kind);
            set_gpr(c, V1, old + 0x40);
        }
        set_gpr(c, A0, old);
        h::call_keep(c, SLOT_FREE, &[old], &[]);
        c.w32(obj + OBJ_SLOT, new);
    }
    c.ret(0);
    true
}

/// Initializes a slot that was just allocated for obj (new != 0) and links it back to obj.
fn attach_fresh(c: &mut Cpu, obj: u32, new: u32) {
    c.w32(obj + OBJ_SLOT, new);
    c.w16(new + 0x34, 0);
    let top = c.rf(SLOT_DEFAULTS + 0x140) + SIX;
    let f0 = c.rf(SLOT_DEFAULTS + 0x144);
    let f1 = c.rf(SLOT_DEFAULTS + 0x148) - POINT_THREE;
    c.wf(new + 0x10, top);
    c.wf(new + 0x14, f0);
    c.wf(new + 0x48, PI_LIKE);
    c.w32(new + SLOT_KIND_OFF, SLOT_KIND);
    c.wf(new + 0x18, f1);
    let owner = c.r32(new + SLOT_OWNER_OFF);
    c.w32(owner, obj);
    // Scratch registers as retail leaves them: $at holds the last constant, v1 the kind word, a0 the owner.
    set_gpr(c, AT, 0x4049_0fdb);
    set_gpr(c, V1, SLOT_KIND);
    set_gpr(c, A0, owner);
    c.f[0] = f0.0;
    c.f[1] = f1.0;
    c.f[2] = PI_LIKE.0;
    c.f[3] = POINT_THREE.0;
}
