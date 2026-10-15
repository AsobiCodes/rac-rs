//! Record helpers: a stack record-and-call (retail 0x2580c0..0x258120) and the particle spawn (retail 0x25f838..0x25f99c).
use crate::h;
use crate::mathx;
use crate::ps2::*;
use crate::slot_registry;
use crate::talk_pad_scan;
use ee_oracle::cpu::Cpu;

const V0: usize = 2;
const V1: usize = 3;
const AT: usize = 1;
const A1: usize = 5;
const GP: usize = 28;
const RA: usize = 31;

/// Header word copied to the record (lwl/lwr at 0x151700).
const HEADER_SRC: u32 = 0x0015_1700;
/// Flag byte copied to record+1 (lbu at 0x15ed58, i.e. 0x160000 - 0x12a8).
const FLAG_SRC: u32 = 0x0015_ED58;
/// Word cleared before the call (sw at 0x15eebc, i.e. 0x160000 - 0x1144).
const CLEAR_WORD: u32 = 0x0015_EEBC;
/// Word cleared through gp (gp - 0x7d40).
const GP_CLEAR_OFF: u32 = 0x7d40;
/// Routine the record is passed to (retail, no Rust hand).
const CALLEE: u32 = 0x0012_1450;
/// Frame: 0x20 bytes, ra saved at +0x10.
const FRAME_SIZE: u32 = 0x20;
const RA_SAVE: u32 = 0x10;
/// Record byte offset of the flag byte.
const FLAG_OFF: u32 = 1;
/// Base of the 0x16 page used for the scratch registers.
const PAGE_16: u32 = 0x0016_0000;

fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// 0x2580c0 record_call(a0, a1, a2) -> v0: fills a stack record from the header word and flag byte, clears the two
/// words, calls 0x121450(a1, a2, a0, record) and returns 1.
pub fn record_call(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let a0 = c.g(4) as u32;
    let a1 = c.g(5) as u32;
    let a2 = c.g(6) as u32;
    h::frame(c, FRAME_SIZE, &[(RA, RA_SAVE, h::SQ)], |c, sp| {
        let header = c.r32(HEADER_SRC);
        c.w32(sp, header);
        let flag = c.r8(FLAG_SRC);
        c.w8(sp.wrapping_add(FLAG_OFF), flag);
        c.w32(CLEAR_WORD, 0);
        let gp = c.g(GP) as u32;
        c.w32(gp.wrapping_sub(GP_CLEAR_OFF), 0);
        set_gpr(c, 10, HEADER_SRC);
        set_gpr(c, AT, PAGE_16);
        set_gpr(c, V1, flag as u32);
        h::call_keep(c, CALLEE, &[a1, a2, a0, sp], &[]);
    });
    c.ret(1);
    true
}

/// Particle spawn: 0x25f838 frame size and saves (s0..s4 at 0x00..0x40, ra at 0x50, f20 at 0x60).
const SPAWN_FRAME: u32 = 0x70;
/// Kind argument of particle_alloc_mode0.
const ALLOC_KIND: u32 = 0x19;
const ONE: u32 = 0x3f80_0000;
const NEG_ONE: u32 = 0xbf80_0000;
/// 0.6f.
const POINT_SIX: u32 = 0x3f19_999a;
/// 4.0f, the float_to_int argument.
const FOUR: u32 = 0x4080_0000;
/// 5000.0f and 30000.0f, the rand_float range.
const RAND_LO: u32 = 0x459c_4000;
const RAND_HI: u32 = 0x46ea_6000;
/// Word holding a pointer to a flag byte (lw at 0x1b21e4 through the 0x1b page).
const SCALE_PTR_CELL: u32 = 0x001b_21e4;
const SCALE_PAGE: u32 = 0x001b_0000;
/// Argument of float_to_int_scaled.
const SCALE_ARG: u32 = 0x50;
/// Retail float_to_int_scaled (0x1f14b8), run as retail so its scratch registers (at) match.
const FLOAT_TO_INT_SCALED: u32 = 0x001f_14b8;
/// Kind byte stored at record+3, and the tag added to the float_to_int result before it is stored at record+9.
const KIND_BYTE: u8 = 0x48;
const TAG_ADD: u32 = 0x40;
/// The sub-record starts 0x20 into the particle record.
const SUB_OFF: u32 = 0x20;
const REC_FLAG: u32 = 1;
const REC_PTR_BYTE: u32 = 2;
const REC_KIND: u32 = 3;
const REC_COLOR: u32 = 4;
const REC_ZERO8: u32 = 8;
const REC_TAG: u32 = 9;
const REC_SCALED: u32 = 0xa;
const REC_RAND: u32 = 0xc;
const REC_QUAD: u32 = 0x10;

/// 0x25f838 spawn_particle(src_pos, src_sub, flag) -> v0: allocates a particle record and fills it from the two
/// quadwords (a0 at +0x10, a1 at the sub-record) and the flag (a2 != 0 stores -1.0 in the sub-record, else 1.0).
pub fn spawn_particle(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let src_pos = c.g(4) as u32;
    let src_sub = c.g(5) as u32;
    let flag_zero = c.g(6) == 0;
    let saves = [
        (16, 0x00, h::SQ),
        (17, 0x10, h::SQ),
        (18, 0x20, h::SQ),
        (19, 0x30, h::SQ),
        (20, 0x40, h::SQ),
        (RA, 0x50, h::SQ),
    ];
    h::frame(c, SPAWN_FRAME, &saves, |c, sp| {
        let f20 = c.f[20];
        c.w32(sp.wrapping_add(0x60), f20);
        h::run_keep(c, talk_pad_scan::particle_alloc_mode0, &[ALLOC_KIND], &[]);
        if c.g(2) == 0 {
            return;
        }
        let rec = c.g(2) as u32;
        let sub = rec.wrapping_add(SUB_OFF);
        let quad = c.rv(src_pos);
        c.wv(rec.wrapping_add(REC_QUAD), quad);
        let fill = if flag_zero { ONE } else { NEG_ONE };
        h::run_keep(c, slot_registry::pack_rgba_f, &[], &[F(ONE), F(ONE), F(ONE), F(POINT_SIX)]);
        let color = c.g(2) as u32;
        c.w32(rec.wrapping_add(REC_COLOR), color);
        c.w32(sub.wrapping_add(0x10), fill);
        c.w32(sub.wrapping_add(0x14), fill);
        c.w32(sub.wrapping_add(0x18), fill);
        h::run_keep(c, mathx::float_to_int, &[], &[F(FOUR)]);
        let tag = (c.g(2) as u32).wrapping_add(TAG_ADD);
        c.w8(rec.wrapping_add(REC_FLAG), 0);
        c.w8(rec.wrapping_add(REC_TAG), tag as u8);
        c.w8(rec.wrapping_add(REC_KIND), KIND_BYTE);
        h::run_keep(c, crate::rand::rand_float, &[], &[F(RAND_LO), F(RAND_HI)]);
        let rnd = c.f[0];
        c.w32(rec.wrapping_add(REC_RAND), rnd);
        c.w8(rec.wrapping_add(REC_ZERO8), 0);
        c.f[0] = POINT_SIX;
        c.w32(sub.wrapping_add(0x1c), POINT_SIX);
        c.s(A1, SCALE_PAGE as u64);
        c.s(AT, POINT_SIX as u64);
        let cell = c.r32(SCALE_PTR_CELL);
        let byte = c.r8(cell);
        c.w8(rec.wrapping_add(REC_PTR_BYTE), byte);
        c.s(V1, byte as u64);
        h::call_keep(c, FLOAT_TO_INT_SCALED, &[SCALE_ARG], &[]);
        let scaled = c.g(2) as u16;
        c.w16(rec.wrapping_add(REC_SCALED), scaled);
        let quad2 = c.rv(src_sub);
        c.wv(sub, quad2);
        c.r[V0] = c.mem.rd128(src_sub);
    });
    true
}
