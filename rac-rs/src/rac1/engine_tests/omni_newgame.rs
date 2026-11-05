//! Tests of engine/omni/newgame.rs (engine split stage E2, 2026-10-26): they run with R&C1's part of the mod host (rac1/omni_bind.rs: binding,
//! spec files, RAM words), so they live in the R&C1 pack; moved from the engine file's test modules, each test hands that part over first.
#![allow(unused_imports)]
mod tests {
	use crate::omni::newgame::*;
	use crate::omni::Omni;
	use ee_oracle::cpu::Cpu;
	use ee_oracle::hs_reloc::Reloc;
	use ee_oracle::hs_reloc::RelocHand;
	use std::sync::atomic::AtomicU8;
	use std::sync::atomic::Ordering;
	use std::sync::OnceLock;
	use crate::toc;

	#[test]
	fn the_ng_plus_spec_parses_with_its_length() {
		crate::omni_bind::for_tests();
		assert_eq!(ng_plus_reloc().len, 0x2fc);
	}

	#[test]
	fn take_returns_the_flag_once() {
		crate::omni_bind::for_tests();
		PENDING.store(false, Ordering::Relaxed);
		assert!(!take());
		let mut c = Cpu::new(ee_oracle::mem::Mem::new());
		assert!(!on_ng_plus(&mut c));
		assert!(take());
		assert!(!take());
	}

	#[test]
	fn leaving_the_title_without_a_load_is_a_new_game() {
		crate::omni_bind::for_tests();
		let mut f = FrontWatcher::default();
		assert!(!f.step(false, -1, false));
		assert!(!f.step(true, 0, false));
		assert!(!f.step(true, 0, false));
		assert!(f.step(false, 0, false));
		assert!(!f.step(false, 0, false));
	}

	#[test]
	fn leaving_the_title_after_a_card_load_is_not() {
		crate::omni_bind::for_tests();
		let mut f = FrontWatcher::default();
		f.step(true, 0, false);
		f.step(true, 0, true);
		assert!(!f.step(false, 3, false));
		// The next trip to the title starts over.
		f.step(true, 3, false);
		assert!(f.step(false, 0, false));
	}

	#[test]
	fn a_load_in_the_field_the_title_is_left_counts_and_a_level_other_than_veldin_does_not() {
		crate::omni_bind::for_tests();
		let mut f = FrontWatcher::default();
		f.step(true, 0, false);
		assert!(!f.step(false, 0, true));
		f.step(true, 5, false);
		assert!(!f.step(false, 5, false));
	}
}
