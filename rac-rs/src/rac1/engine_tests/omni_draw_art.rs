//! Tests of engine/omni/draw_art.rs (engine split stage E2, 2026-10-26): they run with R&C1's part of the mod host (rac1/omni_bind.rs: binding,
//! spec files, RAM words), so they live in the R&C1 pack; moved from the engine file's test modules, each test hands that part over first.
#![allow(unused_imports)]
mod tests {
	use crate::omni::draw_art::*;
	use crate::hud::Glyph;
	use std::ffi::c_char;
	use std::ffi::c_void;
	use std::ffi::CString;
	use std::sync::Arc;
	use std::sync::Mutex;
	use std::sync::OnceLock;
	use crate::toc;
	use crate::omni_bind::ICONS as ITEMS;
	const FIRST_ITEM: u8 = crate::omni_bind::ICON_IDS.0;
	const LAST_ITEM: u8 = crate::omni_bind::ICON_IDS.1;

	#[test]
	fn names_are_unique_and_stable() {
		crate::omni_bind::for_tests();
		let d = defs();
		assert_eq!(d[0].name, "bolt");
		assert_eq!(d[1].name, "nanotech");
		let mut seen = std::collections::HashSet::new();
		assert!(d.iter().all(|i| seen.insert(i.name.clone())));
		assert_eq!(index_of("weapon.blaster"), d.iter().position(|i| i.src == Src::Item(15)));
		assert_eq!(index_of("weapon.blaster.hud"), d.iter().position(|i| i.src == Src::Hud(15)));
		assert_eq!(index_of("gadget.omniwrench"), d.iter().position(|i| i.src == Src::Item(8)));
		assert_eq!(index_of("item.2").map(|i| d[i].src), Some(Src::Item(2)));
		assert!(index_of("nope").is_none());
	}

	#[test]
	fn every_item_has_one_icon_and_every_weapon_a_hud_icon() {
		crate::omni_bind::for_tests();
		let d = defs();
		for id in FIRST_ITEM..=LAST_ITEM {
			assert_eq!(d.iter().filter(|i| i.src == Src::Item(id)).count(), 1, "item {id}");
		}
		let weapons = ITEMS.iter().filter(|i| i.2).count();
		assert_eq!(d.iter().filter(|i| matches!(i.src, Src::Hud(_))).count(), weapons);
		assert_eq!(d.len(), 2 + (LAST_ITEM - FIRST_ITEM + 1) as usize + weapons);
		assert!((LAST_ITEM as usize) < crate::inventory::GADGETS);
	}

	#[test]
	fn the_c_entries_walk_the_table() {
		crate::omni_bind::for_tests();
		// SAFETY: the callbacks take no pointers and return pointers into static tables.
		unsafe {
			let n = icon_count_cb(std::ptr::null_mut());
			assert_eq!(n as usize, defs().len());
			let first = std::ffi::CStr::from_ptr(icon_name_cb(std::ptr::null_mut(), 0));
			assert_eq!(first.to_str(), Ok("bolt"));
			assert!(icon_name_cb(std::ptr::null_mut(), n).is_null());
		}
	}
}
