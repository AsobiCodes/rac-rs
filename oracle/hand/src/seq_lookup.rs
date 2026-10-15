//! Small lookup helpers: the hold-sequence map (0x200660), a jump-table code store (0x200678), a
//! code-to-id map (0x200ab8), and the euler pose vec4 builder (0x1fc280).
use crate::euler_basis;
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Sets a general register to a sign-extended 32-bit value.
fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// Sets float register `reg`.
fn set_fpr(c: &mut Cpu, reg: usize, f: F) {
    c.f[reg] = f.0;
}

/// Sequence id that means "use the hero's hold sequence".
const SEQ_HOLD: u32 = 0xff;
/// Byte offset of the hold sequence id in the hero block.
const HERO_HOLD_SEQ: u32 = 0xa5;

/// 0x200660 hold_seq_map(hero, seq) -> seq: seq 0xff maps to the hero's hold sequence byte, others pass through.
pub fn hold_seq_map(c: &mut Cpu) -> bool {
    let hero = c.arg(0);
    let seq = c.arg(1);
    if seq == SEQ_HOLD {
        let hold = c.r8(hero.wrapping_add(HERO_HOLD_SEQ)) as u32;
        c.set_arg(1, hold);
        c.ret(hold);
    } else {
        c.ret(seq);
    }
    true
}

/// Jump table base: one word per code, indexed by code - CODE_BASE.
const JUMP_TABLE: u32 = 0x1d_9930;
const CODE_BASE: u32 = 0x1c;
const CODE_COUNT: u32 = 0x53;
/// Case targets the table can hold (retail addresses).
const CASE_ZERO: u32 = 0x2006f8; // writes nothing more, returns 0
const CASE_ONE: u32 = 0x2006bc; // returns 1
const CASE_CONST: u32 = 0x2006a4; // stores CONST_A, returns 1
const CASE_LIMIT: u32 = 0x2006c4; // stores CONST_B when LIMIT_VALUE is above LIMIT, returns 1
/// Float bits stored by the constant cases.
const CONST_A: u32 = 0xbf75_be0b;
const CONST_B: u32 = 0xbfa7_8d36;
/// The limit float (25.0) and the RAM word the limit case reads.
const LIMIT: u32 = 0x41c8_0000;
const LIMIT_VALUE: u32 = 0x13_fdf8;

/// 0x200678 code_store(code, out) -> flag: clears *out, then a jump-table case for code (0x1c..0x6e) may store a
/// float to *out and returns 1; codes outside the table return 0.
pub fn code_store(c: &mut Cpu) -> bool {
    let index = c.arg(0).wrapping_sub(CODE_BASE);
    let out = c.arg(1);
    if index >= CODE_COUNT {
        c.w32(out, 0);
        set_gpr(c, 3, index); // v1 keeps code - 0x1c
        c.ret(0);
        return true;
    }
    let slot = JUMP_TABLE.wrapping_add(index * 4);
    let case = c.r32(slot);
    if !matches!(case, CASE_ZERO | CASE_ONE | CASE_CONST | CASE_LIMIT) {
        return false;
    }
    c.w32(out, 0);
    set_gpr(c, 3, slot); // v1 = table slot address
    set_gpr(c, 4, case); // a0 = case target loaded from the slot
    match case {
        CASE_ZERO => c.ret(0),
        CASE_ONE => c.ret(1),
        CASE_CONST => {
            set_gpr(c, 1, CONST_A); // at
            c.wf(out, F(CONST_A));
            c.fret(F(CONST_A));
            c.ret(1);
        }
        _ => {
            set_gpr(c, 1, LIMIT); // at = 25.0 bits
            set_fpr(c, 1, F(LIMIT)); // f1 = 25.0
            let limit = c.rf(LIMIT_VALUE);
            if F(LIMIT) < limit {
                set_gpr(c, 1, CONST_B);
                c.wf(out, F(CONST_B));
                c.fret(F(CONST_B));
            } else {
                c.fret(limit);
            }
            c.ret(1);
        }
    }
    true
}

/// 0x200ab8 code_to_id(n) -> id: 1, 2, 3 map to 0x79, 0x77, 0x75; every other value maps to 0.
pub fn code_to_id(c: &mut Cpu) -> bool {
    let id = match c.arg(0) as i32 {
        1 => 0x79,
        2 => 0x77,
        3 => 0x75,
        _ => 0,
    };
    c.ret(id);
    true
}

/// Retail callee that fills the stack vec4 (x, y, z lanes from the caller).
const SUB_FN: u32 = 0x1fc178;
/// 0.6f, stored in the stack vec4 after zeroing.
const Z_LANE: u32 = 0x3f19_999a;

/// 0x1fc280 euler_pose_vec4(x, y, z): zeroes a stack vec4, sets lane 2 to 0.6, and calls 0x1fc178(vec, x, y, z).
pub fn euler_pose_vec4(c: &mut Cpu) -> bool {
    let x = c.farg(0);
    let y = c.farg(1);
    let z = c.farg(2);
    h::frame(c, 0x40, &[(31, 0x10, h::SQ)], |c, sp| {
        c.wf(sp + 0x20, x);
        c.wf(sp + 0x28, y);
        c.wf(sp + 0x30, z);
        h::run_keep(c, euler_basis::zero_vec4, &[sp], &[]);
        set_gpr(c, 1, Z_LANE); // at
        set_fpr(c, 0, F(Z_LANE)); // f0
        c.wf(sp + 8, F(Z_LANE));
        h::call_keep(c, SUB_FN, &[sp], &[x, y, z]);
    });
    true
}
