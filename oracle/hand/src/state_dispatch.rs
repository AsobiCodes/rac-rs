//! Hand-written 0x282520: state-indexed dispatch through a 9-entry jump table, guarded by a busy flag.

use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Busy flag word (lui 0x16, lw -0xa48): non-zero means "do nothing".
const BUSY_FLAG: u32 = 0x0015_F5B8;
/// Current state word (lui 0x14, lw -0x1fb0).
const STATE_WORD: u32 = 0x0013_E050;
/// Jump table of 9 handler addresses (lui 0x1e, addiu -0x4f60).
const JUMP_TABLE: u32 = 0x001D_B0A0;
const STATE_COUNT: u32 = 9;
/// Stack frame size popped by the function (addiu sp, -0x10).
const FRAME: u32 = 0x10;
/// Upper half of the state-word load address (lui 0x14).
const STATE_WORD_HI: u32 = 0x0014_0000;
/// Handler A (states 0 and 8): calls 0x1eb6f8.
const HANDLER_A: u32 = 0x0028_2560;
/// Handler B (states 3 and 7): stores 0x7f to the gp slot and calls 0x1eae20.
const HANDLER_B: u32 = 0x0028_2570;
/// Handler C (state 4): calls 0x2808c0.
const HANDLER_C: u32 = 0x0028_2584;
const CALL_A: u32 = 0x001E_B6F8;
const CALL_B: u32 = 0x001E_AE20;
const CALL_C: u32 = 0x0028_08C0;
/// Value stored by handler B, at gp - 0x780c.
const HANDLER_B_VALUE: u32 = 0x7F;
const GP_SLOT_OFFSET: u32 = 0x780C;

/// Sign-extends a 32-bit value into a full GPR.
fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.r[reg] = v as i32 as i64 as u64 as u128;
}

/// 0x282520 dispatch_282520(): if the busy flag is clear and the state is below 9, calls the handler for the state
/// (the jump table entry), then returns. Pops its 16-byte frame and restores ra from it.
pub fn dispatch_282520(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    // Resolve the jump before any write: a table target other than A/B/C is a handler this module does not
    // model, so retail runs it (decline with no side effects).
    if c.r32(BUSY_FLAG) == 0 {
        let state = c.r32(STATE_WORD);
        if state < STATE_COUNT {
            let target = c.r32(state.wrapping_mul(4).wrapping_add(JUMP_TABLE));
            if target != HANDLER_A && target != HANDLER_B && target != HANDLER_C {
                return false;
            }
        }
    }
    let outer_sp = c.g(29) as u32;
    let sp = outer_sp.wrapping_sub(FRAME);
    set_gpr(c, 29, sp);
    let ra = c.r[31];
    c.mem.wr128(sp, ra);

    let busy = c.r32(BUSY_FLAG);
    set_gpr(c, 2, busy);
    if busy == 0 {
        let state = c.r32(STATE_WORD);
        set_gpr(c, 2, STATE_WORD_HI);
        set_gpr(c, 4, state);
        if state < STATE_COUNT {
            let slot = state.wrapping_mul(4).wrapping_add(JUMP_TABLE);
            let target = c.r32(slot);
            set_gpr(c, 2, JUMP_TABLE);
            set_gpr(c, 3, slot);
            set_gpr(c, 4, target);
            match target {
                HANDLER_A => {
                    h::call_keep(c, CALL_A, &[target], &[]);
                }
                HANDLER_B => {
                    set_gpr(c, 2, HANDLER_B_VALUE);
                    let gp = c.g(28) as u32;
                    c.w32(gp.wrapping_sub(GP_SLOT_OFFSET), HANDLER_B_VALUE);
                    h::call_keep(c, CALL_B, &[target], &[]);
                }
                HANDLER_C => {
                    h::call_keep(c, CALL_C, &[target], &[]);
                }
                _ => {}
            }
        }
    }
    c.r[31] = c.mem.rd128(sp);
    set_gpr(c, 29, outer_sp);
    true
}
