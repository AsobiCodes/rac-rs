//! Tests of engine/omni/frame_wait.rs (engine split stage E2, 2026-10-26): they run with R&C1's part of the mod host (rac1/omni_bind.rs: binding,
//! spec files, RAM words), so they live in the R&C1 pack; moved from the engine file's test modules, each test hands that part over first.
#![allow(unused_imports)]
mod tests {
	use crate::omni::frame_wait::*;
	use std::sync::atomic::{AtomicU32, Ordering};
	use std::sync::Arc;
	use ee_oracle::cpu::Cpu;
	use std::panic::catch_unwind;
	use std::panic::resume_unwind;
	use std::panic::AssertUnwindSafe;
	use crate::toc;

	/// The vblank wait of the PAL boot (oracle `VBLANK_WAIT`), where the compiled code and the interpreter meet the frame callback.
	const WAIT: u32 = 0x118cb0;

	const ENTRY: u32 = 0x10_0000;

	const RET: u32 = 0x10_1000;

	/// A machine whose function at ENTRY is `j WAIT; nop` (a function that waits for the next frame), vsync armed, with a frame callback
	/// that counts its calls.
	fn waiting_machine() -> (Cpu, Arc<AtomicU32>) {
		let mut c = Cpu::new(ee_oracle::mem::Mem::new());
		c.mem.wr32(ENTRY, 0x0800_0000 | (WAIT >> 2));
		c.mem.wr32(ENTRY + 4, 0);
		c.vsync_armed = true;
		let calls = Arc::new(AtomicU32::new(0));
		let k = calls.clone();
		c.vblank_cb = Some(Box::new(move |_c: &mut Cpu| {
			k.fetch_add(1, Ordering::SeqCst);
		}));
		(c, calls)
	}

	#[test]
	fn a_nested_call_that_waits_for_a_frame_comes_back_unwound_without_running_the_frame_callback() {
		crate::omni_bind::for_tests();
		let (mut c, calls) = waiting_machine();
		c.s(31, RET as u64);
		let returned = without_frame_cb(&mut c, |c| ee_oracle::hs::call(c, ENTRY, RET));
		assert!(!returned, "the call must report that it did not come back");
		assert_eq!(c.pc, WAIT, "the machine stays at the wait, the host loop goes on from there");
		assert_eq!(calls.load(Ordering::SeqCst), 0, "the frame callback must not run inside the hook");
		assert!(c.vblank_cb.is_some(), "the frame callback is back for the host loop");
		// The host loop's next step at the wait runs the callback (the frame happens there, outside any hook).
		c.run_rec(1);
		assert_eq!(calls.load(Ordering::SeqCst), 1);
	}

	#[test]
	fn a_call_left_at_its_return_address_came_back_and_one_left_at_the_wait_or_stopped_did_not() {
		crate::omni_bind::for_tests();
		let mut c = Cpu::new(ee_oracle::mem::Mem::new());
		c.pc = RET;
		assert!(came_back(&c, RET), "a tail jump that ran on to the return address is a finished call");
		c.pc = WAIT;
		assert!(!came_back(&c, RET), "left at the frame wait: unwound");
		c.pc = ENTRY + 8;
		assert!(!came_back(&c, RET), "somewhere in the middle: not back");
		c.pc = RET;
		c.stop = Some("test stop".into());
		assert!(!came_back(&c, RET), "a stopped machine did not come back");
	}

	#[test]
	fn the_wrapper_reports_a_plain_call_as_back_and_a_frame_wait_as_unwound() {
		crate::omni_bind::for_tests();
		let (mut c, calls) = waiting_machine();
		c.mem.wr32(RET, 0x03e0_0008); // jr ra
		c.mem.wr32(RET + 4, 0);
		c.s(31, RET as u64);
		assert!(!call(&mut c, ENTRY, RET), "the waiting function unwinds");
		assert_eq!(calls.load(Ordering::SeqCst), 0);
		// A function that is just `jr ra; nop` comes back.
		c.mem.wr32(ENTRY, 0x03e0_0008);
		c.mem.wr32(ENTRY + 4, 0);
		c.pc = ENTRY;
		c.s(31, RET as u64);
		assert!(call(&mut c, ENTRY, RET));
		assert_eq!(c.pc, RET);
	}

	/// MIPS words for the tests below.
	fn j(t: u32) -> u32 {
		0x0800_0000 | (t >> 2 & 0x03ff_ffff)
	}

	fn jal(t: u32) -> u32 {
		0x0c00_0000 | (t >> 2 & 0x03ff_ffff)
	}

	fn addiu_sp(k: i16) -> u32 {
		0x27bd_0000 | k as u16 as u32
	}

	const SW_RA: u32 = 0xafbf_0000; // sw ra, 0(sp)

	const LW_RA: u32 = 0x8fbf_0000; // lw ra, 0(sp)

	const JR_RA: u32 = 0x03e0_0008;

	#[test]
	fn a_body_that_jumps_out_of_its_span_with_a_stale_ra_comes_back_at_its_real_return() {
		crate::omni_bind::for_tests();
		// The ammo pickup update's shape: frame, a `jal`, then a jump to a case block past the function, which restores ra, pops the
		// frame and returns. r31 at the jump is the `jal`'s return, not the function's; the caller's code after RET runs on to the wait.
		const LEAF: u32 = 0x10_0800;
		const CASE: u32 = 0x10_0400;
		let (mut c, calls) = waiting_machine();
		for (i, w) in [addiu_sp(-16), SW_RA, jal(LEAF), 0, j(CASE), 0].into_iter().enumerate() {
			c.mem.wr32(ENTRY + 4 * i as u32, w);
		}
		for (i, w) in [LW_RA, JR_RA, addiu_sp(16)].into_iter().enumerate() {
			c.mem.wr32(CASE + 4 * i as u32, w);
		}
		c.mem.wr32(LEAF, JR_RA);
		c.mem.wr32(LEAF + 4, 0);
		c.mem.wr32(RET, j(WAIT));
		c.mem.wr32(RET + 4, 0);
		c.s(29, 0x1f_0000);
		c.s(31, RET as u64);
		assert!(call(&mut c, ENTRY, RET), "the call is over at its return");
		assert_eq!((c.pc, c.npc, c.g(29)), (RET, RET + 4, 0x1f_0000), "stopped at the return with the caller's stack");
		assert_eq!(calls.load(Ordering::SeqCst), 0);
		assert!(c.vblank_cb.is_some());
	}

	#[test]
	fn passing_the_return_address_deeper_in_the_stack_is_not_the_return() {
		crate::omni_bind::for_tests();
		// ENTRY calls itself once (a recursion through RET's address at a lower stack) before returning: only the outer return counts.
		const INNER: u32 = 0x10_0200;
		let (mut c, _calls) = waiting_machine();
		// ENTRY: frame; jal INNER; nop; lw ra; jr ra; pop.   INNER: a function whose own return address is also RET (set below).
		for (i, w) in [addiu_sp(-16), SW_RA, jal(INNER), 0, LW_RA, JR_RA, addiu_sp(16)].into_iter().enumerate() {
			c.mem.wr32(ENTRY + 4 * i as u32, w);
		}
		// INNER jumps to RET with the deeper stack (as a callee returning to an address that equals the outer RET would).
		c.mem.wr32(INNER, j(RET));
		c.mem.wr32(INNER + 4, 0);
		// At RET with the deeper stack: return to the instruction after ENTRY's jal (r31 = ENTRY + 0x10 from the jal).
		c.mem.wr32(RET, JR_RA);
		c.mem.wr32(RET + 4, 0);
		c.s(29, 0x1f_0000);
		c.s(31, RET as u64);
		assert!(call(&mut c, ENTRY, RET));
		assert_eq!((c.pc, c.g(29)), (RET, 0x1f_0000), "back at RET only with the caller's stack");
	}

	#[test]
	fn a_second_call_comes_back_to_its_return_address_when_the_first_one_left_r31_elsewhere() {
		crate::omni_bind::for_tests();
		// r31 holds 0x10 before each call (what the aim hand's `Cpu::call` left after the first twin rocket): the call itself must set
		// r31 to its return address, or the function's `jr ra` goes to 0x10.
		let (mut c, _calls) = waiting_machine();
		c.mem.wr32(ENTRY, JR_RA);
		c.mem.wr32(ENTRY + 4, 0);
		c.mem.wr32(RET, j(WAIT));
		c.mem.wr32(RET + 4, 0);
		c.s(29, 0x1f_0000);
		for _ in 0..2 {
			c.s(31, 0x10);
			assert!(call(&mut c, ENTRY, RET));
			assert_eq!((c.pc, c.g(31) as u32), (RET, RET));
		}
	}

	#[test]
	fn the_callback_is_restored_when_the_call_panics_and_the_panic_goes_on() {
		crate::omni_bind::for_tests();
		let (mut c, _calls) = waiting_machine();
		let r = catch_unwind(AssertUnwindSafe(|| without_frame_cb(&mut c, |c| {
			assert!(c.vblank_cb.is_none());
			panic!("boom")
		})));
		assert!(r.is_err());
		assert!(c.vblank_cb.is_some());
	}

	#[test]
	fn without_a_callback_installed_none_is_invented() {
		crate::omni_bind::for_tests();
		let mut c = Cpu::new(ee_oracle::mem::Mem::new());
		let v = without_frame_cb(&mut c, |c| {
			assert!(c.vblank_cb.is_none());
			7
		});
		assert_eq!(v, 7);
		assert!(c.vblank_cb.is_none());
	}
}
