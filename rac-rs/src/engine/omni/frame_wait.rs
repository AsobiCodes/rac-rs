//! Retail calls made on behalf of a mod (a hook's `original`, `call_game`) must never PARK the game thread at a frame wait.
//!
//! Normally the game thread, when it reaches the vblank wait (a frame boundary), runs the host's frame callback in place: it hands the
//! field to the host thread and sleeps until the next field. If that happens while a hook is on the stack, the hook's Rust and Luau
//! frames sleep with it, holding the one loader lock (lock.rs) and the Luau state in the middle of a call, and the host thread's
//! `tick`/`pad`/events then wait for that lock forever: the game freezes at about 0 % CPU. This was the gold-case purchase (the retail
//! updater runs the gadget fanfare scene, a loop that waits for frames, inside the hooked function).
//!
//! The fix is to take the frame callback away for the length of the nested retail call. With no callback, the vblank wait makes the
//! compiled code unwind (oracle `Cpu::vblank_hook` returns false, `hs::call` returns false, `Cpu::tail` leaves the PC at the wait): the
//! nested call is over for the hook, the hook's chain is told (a zero result and one run-log line), and the hand ends at
//! once (bridge.rs `run`). The rest of the hooked function is plain retail code: its state is all in the PS2 stack and registers, the
//! host loop goes on interpreting from the wait with the callback back in place, and the function returns to its real caller at the
//! same instruction count as without the hook. So the field boundary happens at the same instruction as in a run without the mod.

use ee_oracle::cpu::Cpu;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};

/// Runs `f` with `c.vblank_cb` taken away, and puts it back afterwards (also when `f` panics, which is then passed on).
/// With no callback installed (the host is parked in a callback itself, e.g. a call between fields) nothing changes.
pub fn without_frame_cb<R>(c: &mut Cpu, f: impl FnOnce(&mut Cpu) -> R) -> R {
	let saved = c.vblank_cb.take();
	let r = catch_unwind(AssertUnwindSafe(|| f(&mut *c)));
	if saved.is_some() {
		c.vblank_cb = saved;
	}
	match r {
		Ok(v) => v,
		Err(p) => resume_unwind(p),
	}
}

/// Whether a call left the machine at its return address with nothing stopped (the call is over, result in v0 / f0). A real unwind
/// stays at the frame wait (the game's vblank wait, `Game::vblank_wait`) or a stop; the wait is never a return address.
pub fn came_back(c: &Cpu, ret: u32) -> bool {
	c.stop.is_none() && c.pc == ret
}

/// A retail call for a mod (a hook's `original`, `call_game`): the callee runs with no frame callback (`without_frame_cb`) to its return
/// (`run_to_return`); false only for a real unwind at a frame wait or a stop.
pub fn call(c: &mut Cpu, entry: u32, ret: u32) -> bool {
	without_frame_cb(c, |c| run_to_return(c, entry, ret))
}

/// Runs the function at `entry` (arguments set; r31 is set to `ret` here, as the `jal` of a real call would) to its return, compiled as the game runs it, with a stop at the return
/// address while it runs (return_stop.rs): compiled code that leaves the function through a jump out of its span and runs on past the
/// real return (the ammo pickup update's state jump table) stops there instead of running the rest of the frame inside the hook.
fn run_to_return(c: &mut Cpu, entry: u32, ret: u32) -> bool {
	// r31 = the return address, every call: a hook that calls `original` twice (Arsenal Mastery's twin Devastator rockets) finds r31
	// clobbered by the first call (the aim hand's `Cpu::call` leaves its CALL_RET 0x10 there), and a hand at the entry that returns to
	// r31 (oracle `run_hand`) then sent the machine into kernel address 0x10: the game froze (2026-10-25).
	c.s(31, ret as u64);
	let sp = c.r[29] as u32;
	let armed = super::return_stop::arm(c, ret, sp);
	let r = catch_unwind(AssertUnwindSafe(|| ee_oracle::hs::call(c, entry, ret)));
	let stopped = armed && super::return_stop::disarm(c);
	let back = match r {
		Ok(b) => b,
		Err(p) => resume_unwind(p),
	};
	// `hs::call` answers false also when the compiled function left through a tail jump and `Cpu::tail` ran it on to the return address.
	if stopped || (!back && came_back(c, ret)) {
		c.npc = ret.wrapping_add(4);
		return true;
	}
	back
}
