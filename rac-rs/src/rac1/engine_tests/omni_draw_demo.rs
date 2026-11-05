//! Tests of engine/omni/draw_demo.rs (engine split stage E2, 2026-10-26): they run with R&C1's part of the mod host (rac1/omni_bind.rs: binding,
//! spec files, RAM words), so they live in the R&C1 pack; moved from the engine file's test modules, each test hands that part over first.
#![allow(unused_imports)]
mod tests {
	use crate::omni::draw_demo::*;
	use crate::omni::cabi::OmniDrawCmd;
	use crate::omni::draw_quads::KIND_BAR;
	use crate::omni::draw_quads::KIND_RECT;
	use crate::omni::draw_quads::KIND_TEXT;
	use ee_oracle::cpu::Cpu;
	use std::sync::OnceLock;
	use crate::toc;

	#[test]
	fn the_demo_list_is_well_formed() {
		crate::omni_bind::for_tests();
		let l = list(Some(([256.0, 224.0], true)));
		assert!(l.cmds.len() > 10);
		for c in l.cmds.iter().filter(|c| c.kind == KIND_TEXT) {
			assert!((c.text_off + c.text_len) as usize <= l.pool.len());
		}
		assert!(list(None).cmds.len() < l.cmds.len());
	}
}
