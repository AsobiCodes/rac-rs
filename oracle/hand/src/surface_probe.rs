//! Surface probe helpers (rec/l4_16): 0x2b28b0 probes the point at obj+0x10 and picks the object's state;
//! 0x2b6d10 classifies the surface under the hit and calls 0x2b5e28 for some classes.

use crate::collision;
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const SP: usize = 29;
const AT: usize = 1;
/// Upper half of the hit helper address that retail leaves in at.
const HIT_AT: u32 = 0x0017_3f40;
/// lui value that the delay slot of the class-range branch leaves in v0.
const HI_1E: u32 = 0x001e_0000;
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const S0: usize = 16;
const RA: usize = 31;

/// Retail two-point probe: operands come from the frame copies, v0 non-zero on a hit.
const PROBE: u32 = 0x001e_24d8;
/// Retail callee run for the surface classes that have a call entry in the dispatch table.
const DISPATCH_CALL: u32 = 0x002b_5e28;
/// Table target that calls DISPATCH_CALL (the other targets return).
const TARGET_CALL: u32 = 0x002b_6d4c;
/// Table target that sets the state (the other target keeps the object's state).
const TARGET_SET_STATE: u32 = 0x002b_2978;
/// Pointer to the current probe record, and the field offsets read through it.
const RECORD_PTR: u32 = 0x0017_3f58;
const RECORD_KIND: u32 = 0x24;
const RECORD_TYPE: u32 = 0x46;
/// Record type that marks a deadly or enemy surface.
const RECORD_TYPE_WANTED: u32 = 5;
/// Jump tables of 14 words, indexed by the surface class.
const STATE_TABLE: u32 = 0x001d_b7b0;
const DISPATCH_TABLE: u32 = 0x001d_b8c0;
const CLASS_COUNT: u32 = 14;
/// State written on a hit (decimal 11).
const STATE_ON_SURFACE: u32 = 11;
/// Probe offsets: the y operand is shifted by -0.3 and the x operand by +1.0.
const PROBE_DY: u32 = 0x3e99_999a;
const ONE: u32 = 0x3f80_0000;

/// A 32-bit ALU or load result: the low 64 bits are sign-extended, the upper 64 bits are kept (as on the EE).
fn set(c: &mut Cpu, r: usize, v: u32) {
    c.r[r] = (c.r[r] >> 64 << 64) | (v as i32 as i64 as u64 as u128);
}

fn store_quad(c: &mut Cpu, addr: u32, v: u128) {
    c.mem.wr64(addr, v as u64);
    c.mem.wr64(addr + 8, (v >> 64) as u64);
}

/// hit_surface_class (0x1e5e40) through the Rust helper; retail leaves at = 0x173f40 (its lui/addiu address).
fn hit_class(c: &mut Cpu) -> u32 {
    let class = h::run_keep(c, collision::hit_surface_class, &[], &[]).0;
    c.r[V1] = c.r[V0];
    set(c, AT, HIT_AT);
    class
}

fn copy_quad(c: &mut Cpu, src: u32, dst: u32) {
    for i in 0..4 {
        let w = c.r32(src + 4 * i);
        c.w32(dst + 4 * i, w);
    }
}

/// Frame body of probe_surface_state: returns the object's new state.
fn probe_body(c: &mut Cpu, obj: u32, sp: u32, state: u32) -> u32 {
    copy_quad(c, obj + 0x10, sp);
    copy_quad(c, obj + 0x10, sp + 0x10);
    let x = F(c.r32(obj + 0x18));
    let f1 = x - F(PROBE_DY);
    let f0 = x + F(ONE);
    c.w32(sp + 0x8, f1.0);
    c.w32(sp + 0x18, f0.0);
    c.f[0] = f0.0;
    c.f[1] = f1.0;
    c.f[2] = PROBE_DY;
    c.f[3] = ONE;
    set(c, AT, ONE);
    let hit = h::call_keep(c, PROBE, &[sp + 0x10, sp, 6, obj, 0], &[]).0;
    if hit == 0 {
        return state;
    }
    let record = c.r32(RECORD_PTR);
    let flagged = record != 0 && {
        let kind = c.r32(record + RECORD_KIND);
        kind != 0 && c.r16(kind + RECORD_TYPE) as i16 as u32 == RECORD_TYPE_WANTED
    };
    if flagged {
        set(c, V1, RECORD_TYPE_WANTED);
        return STATE_ON_SURFACE;
    }
    let class = hit_class(c);
    if class >= CLASS_COUNT {
        set(c, V0, HI_1E);
        return state;
    }
    let slot = STATE_TABLE + 4 * class;
    let target = c.r32(slot);
    set(c, V0, STATE_TABLE);
    set(c, V1, slot);
    set(c, A0, target);
    if target == TARGET_SET_STATE {
        STATE_ON_SURFACE
    } else {
        state
    }
}

/// 0x2b28b0 probe_surface_state(obj) -> state: probes obj+0x10 (the probe point with y - 0.3 and x + 1.0 applied
/// to its word at +0x18). A hit on a flagged record, or on a surface class that the table maps to state 11, gives
/// state 11; otherwise the object keeps its state byte at +0x20.
pub fn probe_surface_state(c: &mut Cpu) -> bool {
    let obj = c.r[A0] as u32;
    let state = c.r8(obj + 0x20) as u32;
    let state = h::frame(c, 0x40, &[(S0, 0x20, h::SQ), (RA, 0x30, h::SQ)], |c, sp| {
        probe_body(c, obj, sp, state)
    });
    set(c, V0, state);
    true
}

/// 0x2b6d10 dispatch_surface_class(obj) -> (): classifies the surface under the hit (hit_surface_class); for the
/// classes whose table entry is 0x2b6d4c it calls 0x2b5e28(obj), and otherwise it returns.
pub fn dispatch_surface_class(c: &mut Cpu) -> bool {
    let obj = c.r[A0] as u32;
    h::frame(c, 0x20, &[(S0, 0x0, h::SQ), (RA, 0x10, h::SQ)], |c, _sp| {
        c.r[S0] = c.r[A0];
        let class = hit_class(c);
        if class >= CLASS_COUNT {
            set(c, V0, HI_1E);
            return;
        }
        let slot = DISPATCH_TABLE + 4 * class;
        let target = c.r32(slot);
        set(c, V0, DISPATCH_TABLE);
        set(c, V1, slot);
        set(c, A0, target);
        if target == TARGET_CALL {
            set(c, A0, obj);
            h::call_keep(c, DISPATCH_CALL, &[obj], &[]);
        }
    });
    true
}
