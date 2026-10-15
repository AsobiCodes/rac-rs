//! Slot record fill: sets up the record at 0x179510 from the upload's halfwords (retail overlay code, Eudora addresses).
use crate::h;
use crate::ps2::Ee;
use crate::slot_dispatch;
use ee_oracle::cpu::Cpu;

/// The slot record the function fills (retail s1): 0x180000 - 0x6af0.
const REC: u32 = 0x0017_9510;
/// Two flag bytes (0x160000 - 0x11e3 and 0x160000 - 0x11e4); when either is set, the setup routine runs.
const FLAG_A: u32 = 0x0015_EE1D;
const FLAG_B: u32 = 0x0015_EE1C;
/// Pointer to the 16-byte records (0x160000 - 0x9c0); the record index is the word at REC + 0x20.
const RECORD_BASE: u32 = 0x0015_F640;
/// Setup routine run when a flag is set (retail call with a0 = 0, a1 = 1, a2 = 0).
const SETUP: u32 = 0x0027_E5C8;
/// Upload routine (retail, eight register arguments and a stack word of 7).
const UPLOAD: u32 = 0x001E_EAB8;
/// Word read for the geometry (0x140000 - 0x1afc).
const GEOM_WORD: u32 = 0x0013_E504;

/// Frame size of the function.
const FRAME: u32 = 0x60;

/// Stores a 128-bit quadword as four little-endian words.
fn store_quad(c: &mut Cpu, addr: u32, v: u128) {
    for i in 0..4u32 {
        c.w32(addr + 4 * i, (v >> (32 * i)) as u32);
    }
}

/// 0x1f6c30 slot_rec_fill(): fills the slot record at 0x179510 (flag, setup call when a flag byte is set, record
/// pointer from the table at 0x15f640, upload into the caller frame, then the geometry words from the halfwords the
/// upload left at sp+0x1c and sp+0x1e).
pub fn fill_slot_record(c: &mut Cpu) -> bool {
    h::with_stack(c, FRAME, |c, sp| fill_frame(c, sp));
    true
}

/// The body, running with sp at the function's frame (retail `addiu sp, sp, -0x60`).
fn fill_frame(c: &mut Cpu, sp: u32) {
    let (saved_s0, saved_ra, saved_s1) = (c.r[16], c.r[31], c.r[17]);
    store_quad(c, sp + 0x30, saved_s0);
    store_quad(c, sp + 0x50, saved_ra);
    store_quad(c, sp + 0x40, saved_s1);

    c.w32(REC, 1);
    c.w32(REC + 4, 0);
    if c.r8(FLAG_A) != 0 || c.r8(FLAG_B) != 0 {
        h::call(c, SETUP, &[0, 1, 0], &[]);
    }

    let base = c.r32(RECORD_BASE);
    let index = c.r32(REC + 0x20);
    let s0 = c.r32(base.wrapping_add(index << 4));

    c.w32(sp, 7);
    let buf = sp + 0x10;
    h::call(c, UPLOAD, &[buf, 0xf0, 0x1e0, 0x2c, 0x1d4, 0x100, 0x168, 0x10], &[]);
    // Retail leaves a1 zero-extended (ori/dsll), a2 sign-extended (lw), a3 = -1: set them directly.
    c.s(5, 0x80ff_a888);
    c.s(6, s0 as i32 as i64 as u64);
    c.s(7, u64::MAX);
    h::run(c, slot_dispatch::upload_then_fill_2, &[buf], &[]);

    let t1 = c.r32(GEOM_WORD) as i32;
    let x = (c.r16(buf + 0xe) as i16 as i32) >> 1;
    let y = (c.r16(buf + 0xc) as i16 as i32) >> 1;
    let a3 = t1.wrapping_sub(0x3c);
    let a2 = x.wrapping_add(5);
    let a1 = a3.wrapping_add(a2);
    c.w32(REC + 8, y.wrapping_add(0xa) as u32);
    c.w32(REC + 0x10, 0x100);
    c.w32(REC + 0x1c, 8);
    c.w32(REC + 0xc, a2 as u32);
    c.w32(REC + 0x18, 8);
    let overlaps = t1.wrapping_sub(0xc) < a1;
    c.w32(REC + 0x14, a3 as u32);
    if overlaps {
        let v = t1.wrapping_sub(x.wrapping_add(0x11));
        c.w32(REC + 0x14, v as u32);
    }
    // Scratch registers as retail leaves them (32-bit results sign-extended, as the MIPS ops do).
    let v0_final = if overlaps { t1.wrapping_sub(x.wrapping_add(0x11)) } else { y.wrapping_add(0xa) };
    c.s(2, v0_final as i64 as u64);
    c.s(3, overlaps as u64);
    c.s(4, 0x100);
    c.s(5, a1 as i64 as u64);
    c.s(6, a2 as i64 as u64);
    c.s(7, a3 as i64 as u64);
    c.s(8, 8);
    c.s(9, t1 as i64 as u64);
    c.s(10, x as i64 as u64);
}
