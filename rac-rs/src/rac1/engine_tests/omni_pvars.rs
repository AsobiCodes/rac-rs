//! Tests of engine/omni/pvars.rs (engine split stage E2, 2026-10-26): they run with R&C1's part of the mod host (rac1/omni_bind.rs: binding,
//! spec files, RAM words), so they live in the R&C1 pack; moved from the engine file's test modules, each test hands that part over first.
#![allow(unused_imports)]
mod tests {
	use crate::omni::pvars::*;
	use crate::omni::binding::Binding;
	use crate::omni::binding::PvarDef;
	use crate::omni::binding::Ty;
	use crate::omni::cabi::PvarEmit;
	use crate::omni::host::cpu;
	use crate::omni::host::state;
	use std::ffi::c_char;
	use std::ffi::c_void;
	use std::ffi::CString;
	use crate::toc;

	const BINDING: &str = r#"{"game":"rac1","hooks":[],"fields":[],
	  "structs":{"moby":{"fields":[{"name":"class","offset":"0xa6","type":"i16"},{"name":"pvar","offset":"0x78","type":"u32"}]},
	             "enemy":{"fields":[{"name":"health","offset":"0x20","type":"f32"},{"name":"junk","offset":{"pal":"0x24","ntsc":"0x24"},"type":"u8"},{"name":"bad","offset":"zz","type":"u8"}]}},
	  "pvars":{"749":"enemy","12":[{"name":"x","offset":"0x4","type":"u16"}],"7":"nosuch","oops":"enemy"},"unknown_key":1}"#;

	fn ram_with(moby: usize, class: i16, pvar: u32, at: usize, bytes: &[u8]) -> Vec<u8> {
		let mut ram = vec![0u8; 0x40_0000];
		ram[moby + 0xa6..moby + 0xa8].copy_from_slice(&class.to_le_bytes());
		ram[moby + 0x78..moby + 0x7c].copy_from_slice(&pvar.to_le_bytes());
		ram[at..at + bytes.len()].copy_from_slice(bytes);
		ram
	}

	#[test]
	fn enemy_class_gives_health() {
		crate::omni_bind::for_tests();
		let b = Binding::parse(BINDING).unwrap();
		let mut bytes = 2.5f32.to_le_bytes().to_vec();
		bytes.extend_from_slice(&[9, 0, 0, 0]);
		let ram = ram_with(0x20_0000, 749, 0x30_0000, 0x30_0020, &bytes);
		assert_eq!(pvars_of(&b, &ram, 0x20_0000), Some(vec![("health".to_string(), 2.5), ("junk".to_string(), 9.0)]));
	}

	#[test]
	fn list_entry_and_misses() {
		crate::omni_bind::for_tests();
		let b = Binding::parse(BINDING).unwrap();
		let ram = ram_with(0x20_0000, 12, 0x30_0000, 0x30_0004, &[0x34, 0x12]);
		assert_eq!(pvars_of(&b, &ram, 0x20_0000), Some(vec![("x".to_string(), 4660.0)]));
		let other = ram_with(0x20_0000, 99, 0x30_0000, 0x30_0000, &[]);
		assert_eq!(pvars_of(&b, &other, 0x20_0000), None, "a class the section does not list");
		let unknown_struct = ram_with(0x20_0000, 7, 0x30_0000, 0x30_0000, &[]);
		assert_eq!(pvars_of(&b, &unknown_struct, 0x20_0000), None, "an entry naming a missing struct is dropped");
		assert_eq!(pvars_of(&b, &ram, 0), None, "null moby");
		assert_eq!(pvars_of(&b, &ram, 0x7fff_0000), None, "moby outside RAM");
		let nopvar = ram_with(0x20_0000, 749, 0, 0, &[]);
		assert_eq!(pvars_of(&b, &nopvar, 0x20_0000), None, "null pvar");
	}

	#[test]
	fn without_the_sections_nothing_is_found() {
		crate::omni_bind::for_tests();
		let b = Binding::parse(r#"{"game":"rac1","hooks":[]}"#).unwrap();
		assert_eq!(pvars_of(&b, &vec![0u8; 0x40_0000], 0x20_0000), None);
	}

	#[test]
	fn real_binding_lists_the_enemy_classes() {
		crate::omni_bind::for_tests();
		let b = Binding::parse(crate::omni_bind::BINDING).unwrap();
		assert_eq!(b.struct_offset("moby", "class"), Some(0xa6));
		assert_eq!(b.struct_offset("moby", "pvar"), Some(0x78));
		assert_eq!(b.struct_offset("enemy", "health"), Some(0x20));
		assert_eq!(b.pvars.len(), 94, "70 enemy classes (B3) + 18 weapon pvar classes (P2a) + 6 support weapon classes (Arsenal Mastery fields)");
		assert_eq!(b.struct_offset("held_morph", "time_left"), Some(0x18));
		assert_eq!(b.struct_offset("decoy", "health"), Some(0x48));
		assert_eq!(b.struct_offset("moby", "draw_dist"), Some(0x32));
		assert_eq!(b.pvars_of(749).map(|d| d.len()), Some(1));
		assert!(b.pvars_of(1440).is_some() && b.pvars_of(1).is_none());
	}
}
