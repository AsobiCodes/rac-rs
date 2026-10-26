use super::*;

/// The vblank wait of the PAL boot, where a run that is not stopped ends here (no frame callback installed).
const WAIT: u32 = 0x118cb0;
/// Case code past a function's compiled span: `lw ra, 0(sp)`; `jr ra`; `addiu sp, sp, 16` (restores the return address, pops, returns).
const CASE: u32 = 0x10_0400;
/// The caller's code at the return address: `addiu v0, zero, 7` (seen in v0 when it ran), then `j WAIT`; nop.
const RET: u32 = 0x10_1000;
/// A stale r31 (the return of the body's last `jal`), never reached again.
const STALE: u32 = 0x10_0900;
const SP: u32 = 0x1f_0000;

/// A machine stopped in a function's case code with its frame still on the stack (saved ra = RET) and a stale r31: what compiled code
/// hands to `Cpu::tail` after a jump out of its span.
fn in_case_code() -> Cpu {
	let mut c = Cpu::new(ee_oracle::mem::Mem::new());
	for (i, w) in [0x8fbf_0000u32, 0x03e0_0008, 0x27bd_0010].into_iter().enumerate() {
		c.mem.wr32(CASE + 4 * i as u32, w);
	}
	for (i, w) in [0x2402_0007u32, 0x0800_0000 | (WAIT >> 2), 0].into_iter().enumerate() {
		c.mem.wr32(RET + 4 * i as u32, w);
	}
	c.s(29, (SP - 16) as u64);
	c.mem.wr32(SP - 16, RET);
	c.s(31, STALE as u64);
	c.pc = CASE;
	c.npc = CASE + 4;
	c
}

#[test]
fn a_tail_jump_with_a_stale_ra_stops_at_the_real_return_without_running_the_callers_code() {
	let mut c = in_case_code();
	assert!(arm(&mut c, RET, SP));
	c.tail();
	assert_eq!(c.stop.as_deref(), Some(MARK), "stopped by the return stub");
	assert_eq!((c.pc, c.npc, c.g(29) as u32, c.g(31) as u32), (RET, RET + 4, SP, RET));
	assert_eq!(c.g(2), 0, "the caller's instruction at the return address did not run");
	assert!(disarm(&mut c), "the call stopped at its return");
	assert!(c.stop.is_none(), "the mark is cleared");
	assert!(!c.stubs.contains_key(&RET) && !bit(&c, RET), "the stub is gone");
	// The caller goes on from the return address: its instruction runs once, now.
	c.step();
	assert_eq!(c.g(2), 7);
}

#[test]
fn without_the_stop_the_same_tail_runs_on_into_the_caller() {
	let mut c = in_case_code();
	c.tail();
	assert_eq!(c.pc, WAIT, "ran past the return to the frame wait (the 2026-10-23 bug)");
	assert_eq!(c.g(2), 7);
}

#[test]
fn passing_the_return_address_with_another_stack_pointer_runs_on() {
	let mut c = in_case_code();
	assert!(arm(&mut c, RET, SP - 0x100));
	c.tail();
	assert!(c.stop.is_none());
	assert_eq!((c.pc, c.g(2)), (WAIT, 7));
	assert!(!disarm(&mut c));
	assert!(!c.stubs.contains_key(&RET));
}

#[test]
fn an_address_with_another_stub_is_left_alone() {
	let mut c = Cpu::new(ee_oracle::mem::Mem::new());
	c.add_stub(RET, 0, "someone else");
	assert!(!arm(&mut c, RET, SP));
	assert_eq!(c.stubs.get(&RET).map(|s| s.name.as_str()), Some("someone else"));
}

#[test]
fn nested_calls_to_the_same_return_share_the_stub_until_the_outer_one_ends() {
	let mut c = Cpu::new(ee_oracle::mem::Mem::new());
	c.add_watch(RET + 0x40);
	assert!(arm(&mut c, RET, SP));
	assert!(arm(&mut c, RET, SP - 0x40));
	assert!(!disarm(&mut c));
	assert!(c.stubs.contains_key(&RET), "the outer call still has its stop");
	assert!(!disarm(&mut c));
	assert!(!c.stubs.contains_key(&RET) && !bit(&c, RET));
	assert!(bit(&c, RET + 0x40), "other stub bits stay");
}

#[test]
fn a_stop_that_is_not_the_mark_is_left_for_the_caller() {
	let mut c = Cpu::new(ee_oracle::mem::Mem::new());
	assert!(arm(&mut c, RET, SP));
	c.stop = Some("fault".into());
	assert!(!disarm(&mut c));
	assert_eq!(c.stop.as_deref(), Some("fault"));
}
