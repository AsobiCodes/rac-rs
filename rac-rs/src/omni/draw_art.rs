//! The game's own HUD art for the mods' draw list: the default font (text in the game's look) and the icon set (`draw.icon`, `draw.icons()`).
//!
//! The art is decoded per level by viewer/hudart.rs (`HudArt`) and sits in the renderer's HUD texture slots. `hudart::publish_omni` hands this
//! module the font (slot + glyph table) and, per icon of the table below, its slot and size, right after the slots are uploaded
//! (viewer/app.rs, next to `set_hud_art`), so a slot given here always belongs to the textures on the GPU. The renderer (pass_omni.rs) reads it
//! through `get` only when a mod has drawn.
//!
//! The icon NAMES are a fixed table (the same on every level and in every run: a mod may store a name): `bolt`, `nanotech`, then the game's items
//! by id (`weapon.<name>`, `gadget.<name>`, `item.<id>` for items without a known name), and `weapon.<name>.hud` = the ammo HUD's own icon of
//! a weapon. An icon the current level's HUD art does not hold draws nothing.

use crate::hud::Glyph;
use std::ffi::{c_char, c_void, CString};
use std::sync::{Arc, Mutex, OnceLock};

/// Where an icon's texture comes from in `HudArt`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Src {
	/// The bolt counter's coin (frame 0 of sheet 0x754f).
	Bolt,
	/// The nanotech orb (frame 0 of sheet 0x7536).
	Nano,
	/// Frame 0 of item sheet 0xea60 + id: the item's icon (Quick Select wheel).
	Item(u8),
	/// Frame 3 of item sheet 0xea60 + id: the ammo HUD's weapon icon.
	Hud(u8),
}

pub struct IconDef {
	pub name: String,
	pub src: Src,
}

/// Items with a known name (decomp/notes/omni_weapons.md): (id, name, is a weapon).
const ITEMS: [(u8, &str, bool); 19] = [
	(8, "omniwrench", false),
	(9, "suck_cannon", true),
	(10, "bomb_glove", true),
	(11, "devastator", true),
	(12, "swingshot", false),
	(13, "visibomb", true),
	(14, "taunter", true),
	(15, "blaster", true),
	(16, "pyrocitor", true),
	(17, "mine_glove", true),
	(18, "walloper", true),
	(19, "tesla_claw", true),
	(20, "glove_of_doom", true),
	(21, "morph_o_ray", true),
	(22, "hydrodisplacer", false),
	(23, "ryno", true),
	(24, "drone_device", false),
	(25, "decoy_glove", true),
	(26, "trespasser", false),
];

/// The first and the last item id that has a sheet (inventory::GADGETS = 37 items; 0 and 1 are no items).
const FIRST_ITEM: u8 = 2;
const LAST_ITEM: u8 = 36;

fn build_defs() -> Vec<IconDef> {
	let mut v = vec![IconDef { name: "bolt".into(), src: Src::Bolt }, IconDef { name: "nanotech".into(), src: Src::Nano }];
	for id in FIRST_ITEM..=LAST_ITEM {
		match ITEMS.iter().find(|i| i.0 == id) {
			Some(&(_, n, weapon)) => {
				v.push(IconDef { name: format!("{}.{n}", if weapon { "weapon" } else { "gadget" }), src: Src::Item(id) });
				if weapon {
					v.push(IconDef { name: format!("weapon.{n}.hud"), src: Src::Hud(id) });
				}
			}
			None => v.push(IconDef { name: format!("item.{id}"), src: Src::Item(id) }),
		}
	}
	v
}

/// The icon table, in `icon_name` order (an `OmniDrawCmd.icon` is an index into it).
pub fn defs() -> &'static [IconDef] {
	static DEFS: OnceLock<Vec<IconDef>> = OnceLock::new();
	DEFS.get_or_init(build_defs)
}

/// The index of an icon name.
pub fn index_of(name: &str) -> Option<usize> {
	defs().iter().position(|d| d.name == name)
}

fn cnames() -> &'static [CString] {
	static NAMES: OnceLock<Vec<CString>> = OnceLock::new();
	NAMES.get_or_init(|| defs().iter().map(|d| CString::new(d.name.as_str()).unwrap_or_default()).collect())
}

pub unsafe extern "C" fn icon_count_cb(_user: *mut c_void) -> u32 {
	defs().len() as u32
}

pub unsafe extern "C" fn icon_name_cb(_user: *mut c_void, i: u32) -> *const c_char {
	cnames().get(i as usize).map_or(std::ptr::null(), |c| c.as_ptr())
}

/// What the renderer needs of the level's HUD art.
pub struct Art {
	/// The default font: (HUD texture slot, glyph table); text falls back to the built-in 5x7 font without it.
	pub font: Option<(usize, Vec<Glyph>)>,
	/// Per icon of `defs()`: (HUD texture slot, width, height in texels).
	pub icons: Vec<Option<(usize, f32, f32)>>,
}

static ART: Mutex<Option<Arc<Art>>> = Mutex::new(None);

pub fn set(a: Art) {
	*ART.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(a));
}

pub fn get() -> Option<Arc<Art>> {
	ART.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn names_are_unique_and_stable() {
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
