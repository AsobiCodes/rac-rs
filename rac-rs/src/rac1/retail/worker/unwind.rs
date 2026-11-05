//! The periodic unwind of the game thread's recompiled frames (`UNWIND_FIELDS`, `Cpu::rec_unwind`): only while the game
//! is parked in one of retail's two vsync waits (`parked`).
//!
//! [PC] A long frame (the Galactic Map's preload decompresses the map bundles, ~5.5M instructions) runs past the field's
//! end: the field timer raises the vblank mid-computation (`cold::tick`) and the worker gets the field while the game is
//! anywhere, also inside a PC hook's `Cpu::call` (the prompts text hook calls the retail text lookup FUN_00225440 for the
//! map's list texts). Unwinding there threw away the hook's Rust frame while the MIPS call went on: its return went to
//! `CALL_RET` (0x10) and the EE ran low memory, the stuck map (2026-11-04, the user's keyboard recording rec_map/run1:
//! pc 0x10 at field ~1900, Triangle no longer left the map). At the wait no hook is in a call, so the unwind waits
//! for a field that ends there.

/// A due unwind that has waited this many fields runs anyway: a last resort for a wait no `parked` test knows. [PC] 500,
/// was 3000: the title menu after the 50/60 Hz switch grew the 256 MB stack > 85 KB a field and overflowed it at field
/// ~2350 (2026-11-04, release training run cold_60hz; v0.4.3 unwound every 10th field anywhere and passed).
const UNWIND_FORCE_FIELDS: u32 = 500;

/// Whether the game thread waits for the vsync, so no PC hook is inside a `Cpu::call`: at the vblank wait (`vblank_pc`), or
/// in sceGsSyncV's INTC_STAT poll (`poll_head`, the 7-instruction loop `Cpu::poll_ff` skips), where retail waits while no
/// vsync callback is registered: after the PAL 50/60 Hz switch on the title menu, in level loads and in levels whose start
/// does not register it again (Load Game) [C oracle rec/mod.rs `poll_ff`]. Before this the poll counted as mid-frame, so
/// those states never unwound.
pub(super) fn parked(pc: u32, vblank_pc: u32, poll_head: u32) -> bool {
	pc == vblank_pc || pc.wrapping_sub(poll_head) < 0x1c
}

/// The unwind after field number `field` (counted after the increment): `due` = fields a due unwind has waited (0 = none),
/// `parked` = the game thread is in a vsync wait (`parked`). Returns the new `due` and whether to unwind now.
pub(super) fn step(due: u32, field: u32, parked: bool) -> (u32, bool) {
	let due = if due > 0 {
		due.saturating_add(1)
	} else if field % super::UNWIND_FIELDS == 0 {
		1
	} else {
		0
	};
	// (Risk: the forced path can unwind inside a PC hook's `Cpu::call` (return into CALL_RET 0x10); ee-oracle exposes no call-depth counter to guard it, and 10 s (PAL; NTSC ~8.3 s) without a field in a vsync wait is a long load or a hang.)
	if due > 0 && (parked || due > UNWIND_FORCE_FIELDS) {
		(0, true)
	} else {
		(due, false)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn both_vsync_waits_are_parked() {
		let a = ee_oracle::ver::RC1_ADDRS;
		assert!(parked(a.vblank_wait, a.vblank_wait, a.poll_head));
		for i in 0..7 {
			assert!(parked(a.poll_head + 4 * i, a.vblank_wait, a.poll_head), "poll loop insn {i}");
		}
		assert!(!parked(a.poll_head - 4, a.vblank_wait, a.poll_head));
		assert!(!parked(a.poll_head + 0x1c, a.vblank_wait, a.poll_head), "the jal after the loop");
		assert!(!parked(0x10, a.vblank_wait, a.poll_head), "CALL_RET is never parked");
	}

	#[test]
	fn parked_unwinds_every_unwind_fields() {
		let n = super::super::UNWIND_FIELDS;
		let mut due = 0;
		let mut hits = Vec::new();
		for f in 1..=3 * n {
			let (d, u) = step(due, f, true);
			due = d;
			if u {
				hits.push(f);
			}
		}
		assert_eq!(hits, vec![n, 2 * n, 3 * n]);
		assert_eq!(due, 0);
	}

	#[test]
	fn mid_frame_defers_to_the_next_parked_field() {
		let n = super::super::UNWIND_FIELDS;
		// Due at field n while the game computes: nothing, then the first parked field unwinds.
		assert_eq!(step(0, n, false), (1, false));
		assert_eq!(step(1, n + 1, false), (2, false));
		assert_eq!(step(2, n + 2, true), (0, true));
		// Not due: a parked field does nothing.
		assert_eq!(step(0, n + 3, true), (0, false));
	}

	#[test]
	fn a_frame_that_never_parks_unwinds_after_the_cap() {
		let n = super::super::UNWIND_FIELDS;
		let mut due = 0;
		let mut at = None;
		for f in n..n + UNWIND_FORCE_FIELDS + 5 {
			let (d, u) = step(due, f, false);
			due = d;
			if u {
				at = Some(f - n);
				break;
			}
		}
		assert_eq!(at, Some(UNWIND_FORCE_FIELDS));
	}
}
