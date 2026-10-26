//! A stop at the real return of a retail call made for a mod (frame_wait.rs `call`: a hook's `original`, `call_game`).
//!
//! Compiled code that leaves a function through a jump out of its compiled span goes on through oracle `Cpu::tail`, which takes r31 as
//! the return address. Where r31 is still the return of the body's last `jal` (the ammo pickup update jumps through its state jump table
//! to case code past its span), the machine runs on past the function's real return into the caller and the rest of the frame. Without
//! a mod that is harmless (the same instructions run, only nested deeper in Rust frames), but inside a mod's call it ran the rest of the
//! frame inside the hook up to the frame wait (2026-10-23: "the original function waits for the next frame" on ammo pickups; the
//! pickup hook then never ran again and the pickup was never collected).
//!
//! So while such a call runs, an interpreter stub sits at its return address: when the machine gets there with the stack pointer of the
//! call and r31 = that address (the function's own `jr ra`), the stub sets `Cpu::stop` to `MARK`, which ends every compiled and nested
//! loop at once; the PC stays at the return address and the instruction there is not run (the caller runs it). The call's runner then
//! clears the mark. Same instructions in the same order as without the stop, so the game state and instruction count do not change.

use ee_oracle::cpu::{Cpu, Stub};
use std::cell::RefCell;

/// What the stub leaves in `Cpu::stop` (the runner that set the stop clears it).
pub const MARK: &str = "omni: a mod's retail call reached its return address";
/// The stub's name in `Cpu::stubs`.
const NAME: &str = "omni return stop";

/// One call in progress: its return address and stack pointer; `owner`: this entry put the stub there (and whether the stub bit was
/// already set by something else, so removing the stub leaves it).
#[derive(Clone, Copy)]
struct Entry {
	ret: u32,
	sp: u32,
	owner: bool,
	had_bit: bool,
}

thread_local! {
	/// The calls in progress on this thread, innermost last (a nested call's stop applies first). Per thread: the machine is used by one
	/// thread at a time (the game thread, or the host thread between two fields), and the stub fires on the thread that made the call.
	static ACTIVE: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
}

fn bit(c: &Cpu, at: u32) -> bool {
	let i = ((at & 0x01ff_ffff) >> 2) as usize;
	c.stub_bits[i >> 6] >> (i & 63) & 1 != 0
}

fn set_bit(c: &mut Cpu, at: u32, on: bool) {
	let i = ((at & 0x01ff_ffff) >> 2) as usize;
	if on {
		c.stub_bits[i >> 6] |= 1 << (i & 63);
	} else {
		c.stub_bits[i >> 6] &= !(1 << (i & 63));
	}
}

/// The stub: at the innermost call's return address with its stack pointer and r31 pointing there, stop (true: the CPU "returns" to
/// r31, which is this same address, so nothing moves). Anything else (another pass through the address) runs on.
fn at_return(c: &mut Cpu) -> bool {
	let (pc, sp, ra) = (c.pc, c.g(29) as u32, c.g(31) as u32);
	let hit = ACTIVE.with_borrow(|v| v.last().is_some_and(|e| e.ret == pc && e.sp == sp && ra == pc));
	if hit {
		c.stop = Some(MARK.to_string());
	}
	hit
}

/// Puts the stop at `ret` for a call made with stack pointer `sp`. False when the address already holds another stub (then the call
/// runs without the stop, as before).
pub fn arm(c: &mut Cpu, ret: u32, sp: u32) -> bool {
	let owner = match c.stubs.get(&ret) {
		None => true,
		Some(s) if s.name == NAME => false,
		Some(_) => return false,
	};
	let had_bit = owner && bit(c, ret);
	if owner {
		c.stubs.insert(ret, Stub { ret: 0, name: NAME.to_string(), sig: None, host: Some(at_return) });
		set_bit(c, ret, true);
	}
	ACTIVE.with_borrow_mut(|v| v.push(Entry { ret, sp, owner, had_bit }));
	true
}

/// Takes the innermost stop away again (after `arm` said true), and clears `Cpu::stop` when it holds the mark. True: the call stopped
/// at its return address.
pub fn disarm(c: &mut Cpu) -> bool {
	let Some(e) = ACTIVE.with_borrow_mut(Vec::pop) else { return false };
	if e.owner {
		c.stubs.remove(&e.ret);
		if !e.had_bit {
			set_bit(c, e.ret, false);
		}
	}
	let ours = c.stop.as_deref() == Some(MARK);
	if ours {
		c.stop = None;
	}
	ours && c.pc == e.ret
}

#[cfg(test)]
#[path = "return_stop_tests.rs"]
mod tests;
