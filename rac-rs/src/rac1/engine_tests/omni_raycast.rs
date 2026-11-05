//! Tests of engine/omni/raycast.rs (engine split stage E2, 2026-10-26): they run with R&C1's part of the mod host (rac1/omni_bind.rs: binding,
//! spec files, RAM words), so they live in the R&C1 pack; moved from the engine file's test modules, each test hands that part over first.
#![allow(unused_imports)]
mod tests {
	use crate::omni::raycast::*;
	use crate::omni::events;
	use crate::omni::funcs;
	use crate::omni::native_host;
	use ee_oracle::cpu::Cpu;
	use ee_oracle::hs_reloc::Reloc;
	use std::ffi::c_void;
	use std::sync::Mutex;
	use std::sync::OnceLock;
	use crate::toc;

	#[test]
	fn the_spec_parses_with_the_length_of_the_function() {
		crate::omni_bind::for_tests();
		assert_eq!(reloc().len, 0x10f0);
	}

	#[test]
	fn vectors_are_xyz_and_w_one_little_endian() {
		crate::omni_bind::for_tests();
		let b = vec_bytes([1.5, -2.0, 3.25]);
		assert_eq!(&b[0..4], &1.5f32.to_le_bytes());
		assert_eq!(&b[4..8], &(-2.0f32).to_le_bytes());
		assert_eq!(&b[8..12], &3.25f32.to_le_bytes());
		assert_eq!(&b[12..16], &1.0f32.to_le_bytes());
	}

	#[test]
	fn the_two_vectors_sit_16_aligned_below_sp_and_a_tiny_stack_is_refused() {
		crate::omni_bind::for_tests();
		let (sp, s, e) = plan(0x01ff_ff00).unwrap();
		assert_eq!((s, e, sp % 16), (sp, sp + 16, 0));
		assert!(sp < 0x01ff_ff00 && sp + 32 <= 0x01ff_ff00);
		assert!(plan(0x10_0010).is_none());
	}

	#[test]
	fn the_normal_is_normalised_and_a_zero_normal_stays_zero() {
		crate::omni_bind::for_tests();
		let o = out6([1.0, 2.0, 3.0], [0.0, 0.0, 4.0]);
		assert_eq!(o, [1.0, 2.0, 3.0, 0.0, 0.0, 1.0]);
		assert_eq!(out6([0.0; 3], [0.0; 3])[3..], [0.0, 0.0, 0.0]);
		let d = out6([0.0; 3], [3.0, 0.0, 4.0]);
		assert!((d[3] - 0.6).abs() < 1e-6 && (d[5] - 0.8).abs() < 1e-6);
	}

	#[test]
	fn the_floats_are_read_back_from_ram() {
		crate::omni_bind::for_tests();
		let mut ram = vec![0u8; 64];
		ram[16..20].copy_from_slice(&7.0f32.to_le_bytes());
		ram[24..28].copy_from_slice(&(-1.0f32).to_le_bytes());
		assert_eq!(f3(&ram, 16), [7.0, 0.0, -1.0]);
		assert_eq!(f3(&ram, 62), [0.0, 0.0, 0.0]);
	}

	#[test]
	fn no_level_means_cannot_test() {
		crate::omni_bind::for_tests();
		// A fresh machine has level word 0 but empty code: the line test is not found, so nothing can be tested.
		let mut c = Cpu::new(ee_oracle::mem::Mem::new());
		assert_eq!(cast(&mut c, false, [0.0; 3], [0.0, 0.0, -1.0]), None);
	}
}
