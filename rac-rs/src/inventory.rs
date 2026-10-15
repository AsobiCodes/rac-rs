//! The hero's gadgets and ammo (decomp/notes/glove_prereq_inventory_input.md section 1): owned flags
//! (0x13d4c0), ammo (0x13d428 + id*4), the quick-select list (0x141ea0) and the equipped / last-chosen
//! weapon. Item ids are the game's own (0..0x24): 8 = Omniwrench, 10 = Bomb Glove.

pub const GADGETS: usize = 37;
pub const WRENCH: u8 = 8;
pub const BOMB_GLOVE: u8 = 10;
pub const PYROCITOR: u8 = 16;
/// The Visibomb (item 13, held class 163) and the Blaster (item 15, held class 168) (WPN-6.1e3).
pub const VISIBOMB: u8 = 13;
pub const BLASTER: u8 = 15;
/// The Devastator (item 11, held class 157, ammo pickup class 204, WPN-6.2f1).
pub const DEVASTATOR: u8 = 11;
/// The Mine Glove (item 17, held class 190, mine class 74, WPN-6.4a).
pub const MINE_GLOVE: u8 = 17;
/// The Glove of Doom (item 20, held class 229, capsule class 230, agent class 186, WPN-6.5a).
pub const GLOVE_OF_DOOM: u8 = 20;
/// The Suck Cannon (item 9, held class 849, WPN-5.1).
pub const SUCK_CANNON: u8 = 9;
/// The Taunter (item 14, held class 175, WPN-6.9a).
pub const TAUNTER: u8 = 14;
/// The Trespasser (item 26, held class 188, ARI-41r).
pub const TRESPASSER: u8 = 26;
/// Levels below this grant the glove at load (FUN_0023cd80); 0x13 and up grant everything.
const GRANT_LEVELS: u32 = 0x13;

/// The bits of the item row (0x179ac0) and the price/ammo table (0x1c4030) rac-rs uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemInfo {
	/// Ammo cap; 0 = no ammo concept (infinite).
	pub max: u16,
	pub start: u16,
	/// Ammo per pickup.
	pub pickup: u16,
	/// Held model class.
	pub held_class: u32,
	/// Ammo pickup moby class.
	pub ammo_class: u32,
	/// Equip slot 0 = main hand (goes in the quick-select list).
	pub slot: u8,
}


pub fn info(id: u8) -> Option<ItemInfo> {
	match id {
		WRENCH => Some(ItemInfo { max: 0, start: 0, pickup: 0, held_class: 71, ammo_class: 0, slot: 0 }),
		BOMB_GLOVE => Some(ItemInfo { max: 40, start: 10, pickup: 3, held_class: 192, ammo_class: 226, slot: 0 }),
		// decomp/notes/vendor.md: start 120, max 240, 20 per pickup, held class 176; ammo pickup class 214
		// (item record +0x32, weapons_framework.md).
		PYROCITOR => Some(ItemInfo { max: 240, start: 120, pickup: 20, held_class: 176, ammo_class: 214, slot: 0 }),
		// Row 9 (0x17a16c): held class 849, ammo class -1 (no ammo concept), hero fire sequence 51 like the Pyrocitor row.
		SUCK_CANNON => Some(ItemInfo { max: 0, start: 0, pickup: 0, held_class: 849, ammo_class: 0, slot: 0 }),
		// Row 14 (WPN-6.9a, dat 0x17a2e8 w4 = 175; price/ammo row 0x1c4580 = 2500, 1250, then all ammo halfwords 0): no ammo concept.
		TAUNTER => Some(ItemInfo { max: 0, start: 0, pickup: 0, held_class: 175, ammo_class: 0, slot: 0 }),
		// Row 17 (WPN-6.4a, price/ammo table 0x1c4430): pickup 5, max 50, start 25; held class 190, ammo class 225.
		MINE_GLOVE => Some(ItemInfo { max: 50, start: 25, pickup: 5, held_class: 190, ammo_class: 225, slot: 0 }),
		// Row 20 (WPN-6.5a, price table 0x1c4610: max 10, start 3, pickup 1; item row 0x17a4b8: held 229, ammo class 223).
		GLOVE_OF_DOOM => Some(ItemInfo { max: 10, start: 3, pickup: 1, held_class: 229, ammo_class: 223, slot: 0 }),
		// Row 11 (WPN-6.2f1, price/ammo table 0x1c4538 shorts 2/3/4): pickup 2, max 20, start 10; held class 157 (0x17a20c
		// word 2), ammo pickup class 204 (0x1c25e4 word 0).
		DEVASTATOR => Some(ItemInfo { max: 20, start: 10, pickup: 2, held_class: 157, ammo_class: 204, slot: 0 }),
		// WPN-6.1e3: price/ammo rows 0x1c4430 + id*0x18 (Novalis overlay) and item rows 0x179ec8 + id*0x4c (+8 held
		// class, +0x32 ammo pickup class); the Blaster spends 1 per shot (FUN_00248fb8(-1, 1)).
		VISIBOMB => Some(ItemInfo { max: 20, start: 10, pickup: 1, held_class: 163, ammo_class: 222, slot: 0 }),
		BLASTER => Some(ItemInfo { max: 200, start: 100, pickup: 25, held_class: 168, ammo_class: 1006, slot: 0 }),
		// Row 26 (ARI-41r, 0x17a800): kind 0 (main hand), held class 188, no ammo pickup class (retail -1);
		// price/ammo row 0x1c4828 all zero, so no ammo concept.
		TRESPASSER => Some(ItemInfo { max: 0, start: 0, pickup: 0, held_class: 188, ammo_class: 0, slot: 0 }),
		_ => None,
	}
}

/// The per-item view record the pause previews read (decomp/notes/ingame_pause_menu.md "Item previews"):
/// item row 0x179ac0 + id*0x4c: (preview moby class = +0x10, attach point = +0x0c, mode = +0x18). The
/// previews put the class's moby at hero attach point `joint` (FUN_0024f338); mode 0/1/2 selects how the
/// update callbacks FUN_00282150.. pose it. Row 0 is empty.
pub const ITEM_VIEW: [(u32, u8, u8); GADGETS] = [
	(0, 0, 0), (601, 5, 0), (607, 5, 0), (608, 5, 0), (609, 5, 0), (433, 4, 0), (1289, 4, 0), (1290, 4, 0), (71, 0, 0), (849, 0, 2), (192, 6, 0), (157, 0, 2), (208, 1, 1), (163, 0, 2), (175, 0, 1), (168, 0, 1), (176, 0, 2), (190, 6, 0), (180, 1, 0), (177, 0, 1), (229, 6, 0), (185, 0, 1), (1251, 0, 2), (454, 0, 2), (483, 6, 0), (562, 6, 0), (188, 0, 1), (585, 0, 1), (173, 2, 0), (195, 2, 0), (439, 2, 0), (483, 0, 1), (619, 0, 1), (614, 7, 0), (618, 7, 0), (407, 8, 0), (157, 0, 2),
];


/// [PC] the weapons the debug grant key hands out (WPN-6.8).
pub const DEBUG_WEAPONS: [u8; 7] = [11, 13, 15, 17, 18, 20, 21];

/// Item row w15 high half: the gold-upgrade level cap (1 for ids 9, 10, 11, 15, 16, 17, 19, 20, 21, 25, 36).
pub const GOLD_CAP: [u8; GADGETS] = {
	let mut t = [0u8; GADGETS];
	let ids = [9, 10, 11, 15, 16, 17, 19, 20, 21, 25, 36];
	let mut i = 0;
	while i < ids.len() {
		t[ids[i]] = 1;
		i += 1;
	}
	t
};

/// The preview moby class of item `id` (row +0x10); `None` for id 0 or out of range.
pub fn preview_class(id: u8) -> Option<u32> {
	ITEM_VIEW.get(id as usize).map(|v| v.0).filter(|&c| c != 0)
}

/// The view row the hand attach callback FUN_00282150 reads for item `id`: the first row whose class equals the
/// item's own (class, attach point, mode), so item 31 (class 483, row 31 attach 0 mode 1) resolves to row 24
/// (attach 6, mode 0).
pub fn view_row(id: u8) -> Option<(u32, u8, u8)> {
	let class = preview_class(id)?;
	ITEM_VIEW.iter().skip(1).find(|r| r.0 == class).copied()
}

/// The class of the Gadgets panel item model (FUN_0027b808): the view row's class, 0x1df for item 0x18; None for
/// a class of 0 or -1.
pub fn panel_class(id: u8) -> Option<u32> {
	if id == 0x18 {
		return Some(0x1df);
	}
	preview_class(id)
}

/// Hero attach point of item `id` (row +0x0c).
pub fn attach_point(id: u8) -> Option<u8> {
	ITEM_VIEW.get(id as usize).filter(|v| v.0 != 0).map(|v| v.1)
}

/// FUN_0020d018: the gloves (items 10 Bomb Glove, 17 Mine Glove, 20 Glove of Doom, 25) whose held model takes the hero
/// hand pose copy in play instead of its own animation, with unnormalised axes (HeroItemsAttach FUN_0020f0c8,
/// decomp/notes/bomb_glove.md "HELD-GLOVE").
pub fn hand_pose_copy(id: u8) -> bool {
	matches!(id, 10 | 17 | 20 | 25)
}

/// Pickup message table 0x1b0340 (decomp/notes/ingame_menu_leftovers.md P1-b): (item, first text,
/// repeat text or 0). -1 rows (the wrench, the glove, ...) give silently.
const POPUPS: [(u8, u32, u32); 21] = [
	(2, 0xbc7, 0),
	(3, 0x2b04, 0),
	(4, 0x2ee6, 0),
	(5, 0x7e0, 0),
	(6, 0x2b05, 0),
	(7, 0x232a, 0),
	(9, 0xfa7, 0x53df),
	(12, 0xbc8, 0),
	(21, 0x36b2, 0),
	(22, 0x177c, 0),
	(23, 0x1396, 0x53e2),
	(26, 0x7df, 0),
	(27, 0x1f49, 0),
	(28, 0x271b, 0),
	(29, 0x177d, 0),
	(30, 0x7e1, 0),
	(31, 0x3e86, 0),
	(32, 0x36b3, 0x53dd),
	(33, 0x3e85, 0x53dc),
	(34, 0x3aa3, 0x53db),
	(35, 0x2b06, 0x53de),
];

/// The pickup popup text for `id` (`again` = already owned), None = silent.
pub fn popup_text(id: u8, again: bool) -> Option<u32> {
	let &(_, first, repeat) = POPUPS.iter().find(|p| p.0 == id)?;
	Some(if again && repeat != 0 { repeat } else { first })
}

/// Equip category per item, word 0 of the 0x4c-byte item records at 0x179ac8 (overlay data): 0 hand,
/// 1 feet, 2 head, 3 back; 4 and 5 are not worn from the Gadgets page; -1 no item.
const CATEGORY: [i8; GADGETS] = [-1, 3, 3, 3, 3, 2, 2, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 0, 0, 4, 4, 5, 0];

pub fn category(id: u8) -> Option<usize> {
	CATEGORY.get(id as usize).and_then(|&c| usize::try_from(c).ok())
}

/// Name text per item: u16 at +0x3e of the 0x4c-byte item records at 0x179ac8 (overlay data; Heli-Pack
/// 2, Pilot's Helmet 7, OmniWrench 8, Bomb Glove 10). Items 0 and 1 have none.
const NAMES: [u16; GADGETS] = [0x0, 0x0, 0x4e6e, 0x4e6d, 0x4e6f, 0x4e6a, 0x4e6b, 0x4e6c, 0x4e46, 0x4e57, 0x4e56, 0x4e4d, 0x4e61, 0x4e5a, 0x4e51, 0x4e47, 0x4e49, 0x4e54, 0x4e45, 0x4e58, 0x4e5b, 0x4e55, 0x4e5c, 0x4e4e, 0x4e53, 0x4e59, 0x4e63, 0x4e5e, 0x4e67, 0x4e68, 0x4e69, 0x4e5d, 0x4e5f, 0x4e70, 0x4e78, 0x4e73, 0x4e4d];

/// The item's display name text id (0 = none).
pub fn name_text(id: u8) -> u32 {
	NAMES.get(id as usize).map_or(0, |&n| n as u32)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Inventory {
	pub have: [bool; GADGETS],
	pub ammo: [u16; GADGETS],
	/// Quick-select ids, 0 = empty.
	pub quick: [u8; 8],
	/// The item in the hand (0x141424).
	pub equipped: u8,
	/// The last weapon chosen (0x141660), what Circle brings back out.
	pub last: u8,
	/// Worn item per category (feet, head, back at 1..3; 0 = none). The game keeps them in
	/// 0x141660[4] with 0x141408[slot] = item or 0x26 for none (ingame_pause_menu.md, Gadgets).
	pub worn: [u8; 4],
	/// A1000 / A1001 / A1002: ammo bought, picked up and used per item (savegame.md), counting only the
	/// quantity actually added or taken (FUN_002afa.. purchase, FUN_002db180 pickup, FUN_00248fb8 consume).
	pub stats: [[u32; GADGETS]; 3],
	/// A32 words 4..6 (category 4/5 selections and the unused word 6), kept as the save holds them: the game
	/// builds those items from ownership at load and never reads these words back (savegame.md).
	pub sel_tail: [u32; 3],
}

impl Default for Inventory {
	fn default() -> Self {
		Self::new()
	}
}

impl Inventory {
	/// Nothing owned; the wrench in hand (it is not granted through `have`).
	pub fn new() -> Inventory {
		Inventory { have: [false; GADGETS], ammo: [0; GADGETS], quick: [0; 8], equipped: WRENCH, last: 0, worn: [0; 4], stats: [[0; GADGETS]; 3], sel_tail: [0; 3] }
	}

	pub fn has_ammo_concept(id: u8) -> bool {
		info(id).is_some_and(|i| i.max != 0)
	}

	/// UnlockGadget FUN_00260470 (without the pickup message).
	pub fn unlock(&mut self, id: u8, equip_now: bool) {
		if (id as usize) < GADGETS {
			self.have[id as usize] = true;
		}
		// Gadgets and items rac-rs has no row for are only marked owned.
		let Some(i) = info(id) else { return };
		let k = id as usize;
		if i.max != 0 {
			self.ammo[k] = self.ammo[k].max(i.start);
		}
		if equip_now || i.slot == 0 {
			self.equipped = id;
		}
		if i.slot == 0 && !self.quick.contains(&id) {
			if let Some(q) = self.quick.iter_mut().find(|q| **q == 0) {
				*q = id;
			}
		}
	}

	/// FUN_0023cd80 at every level load: the glove is given silently, equipped, if not owned yet.
	pub fn ensure_glove(&mut self, level: u32) {
		if level < GRANT_LEVELS && !self.have[BOMB_GLOVE as usize] {
			self.unlock(BOMB_GLOVE, true);
			self.last = BOMB_GLOVE;
		}
	}

	/// AddAmmo FUN_00233a20: clamps to the cap and returns the overflow.
	pub fn add_ammo(&mut self, id: u8, n: u16) -> u16 {
		let Some(i) = info(id).filter(|i| i.max != 0) else { return 0 };
		let a = &mut self.ammo[id as usize];
		let sum = *a as u32 + n as u32;
		*a = sum.min(i.max as u32) as u16;
		(sum - *a as u32) as u16
	}


	/// [PC] debug grant (WPN-6.8, user decision 2026-10-06; no retail counterpart): the seven weapons the first visits do not
	/// reach (Devastator 11, Visibomb 13, Blaster 15, Mine Glove 17, Walloper 18, Glove of Doom 20, Morph-o-Ray 21) become
	/// owned with full ammo (where the item has an ammo row) and each takes a free quick-select slot.
	pub fn debug_grant_weapons(&mut self) {
		// The item in hand stays: `unlock` equips every slot-0 row (the Blaster, Visibomb and Mine Glove rows).
		let equipped = self.equipped;
		for id in DEBUG_WEAPONS {
			self.unlock(id, false);
			self.equipped = equipped;
			self.add_ammo(id, u16::MAX);
			if !self.quick.contains(&id) {
				if let Some(q) = self.quick.iter_mut().find(|q| **q == 0) {
					*q = id;
				}
			}
		}
		self.equipped = equipped;
	}

	/// Which item is in hand when a level starts (passive init FUN_0022ee88): the wrench if the last
	/// selection was the wrench (A22), else the last-chosen weapon (A32 word 0), else the wrench.
	/// A last-chosen item the save does not own stays the wrench [port guard].
	pub fn equip_on_load(&mut self, wrench_pref: bool) {
		self.equipped = if !wrench_pref && self.last != 0 && self.have.get(self.last as usize).copied().unwrap_or(false) { self.last } else { WRENCH };
	}


	/// GetAmmo FUN_00233a78: 1 for items without ammo.
	pub fn ammo(&self, id: u8) -> u16 {
		if Self::has_ammo_concept(id) { self.ammo[id as usize] } else { 1 }
	}




}

#[cfg(test)]
mod tests {
	// WPN-6.2f1 tests (1) and (7): the Devastator row, its consume gate and the debug grant.
	// WPN-6.1e3 tests (1)-(3): the Blaster and Visibomb rows, their ammo classes and the consume gate.
	#[test]
	fn hand_row_is_found_by_class() {
		assert_eq!(view_row(31), Some((483, 6, 0)));
		assert_eq!(view_row(24), Some((483, 6, 0)));
		assert_eq!(view_row(13), Some((163, 0, 2)));
		assert_eq!(view_row(0), None);
	}

	use super::*;

	#[test]
	fn new_owns_nothing() {
		let inv = Inventory::new();
		assert!(inv.have.iter().all(|h| !h));
		assert_eq!(inv.equipped, WRENCH);
		assert_eq!(inv.ammo(BOMB_GLOVE), 0);
	}

	#[test]
	fn unlock_glove() {
		let mut inv = Inventory::new();
		inv.unlock(BOMB_GLOVE, true);
		assert_eq!((inv.ammo(BOMB_GLOVE), inv.equipped, inv.quick[0]), (10, BOMB_GLOVE, BOMB_GLOVE));
		// A second unlock keeps the ammo (max(ammo, start)) and does not add a second quick slot.
		inv.ammo[BOMB_GLOVE as usize] = 25;
		inv.unlock(BOMB_GLOVE, true);
		assert_eq!(inv.ammo(BOMB_GLOVE), 25);
		inv.ammo[BOMB_GLOVE as usize] = 3;
		inv.unlock(BOMB_GLOVE, true);
		assert_eq!(inv.ammo(BOMB_GLOVE), 10);
		assert_eq!(inv.quick, [10, 0, 0, 0, 0, 0, 0, 0]);
	}

	#[test]
	fn equipped_on_load_follows_wrench_pref_then_last() {
		let mut inv = Inventory::new();
		inv.unlock(BOMB_GLOVE, true);
		inv.last = BOMB_GLOVE;
		inv.equip_on_load(true);
		assert_eq!(inv.equipped, WRENCH);
		inv.equip_on_load(false);
		assert_eq!(inv.equipped, BOMB_GLOVE);
		inv.last = 0;
		inv.equip_on_load(false);
		assert_eq!(inv.equipped, WRENCH);
	}

	#[test]
	fn add_ammo_overflow() {
		let mut inv = Inventory::new();
		inv.unlock(BOMB_GLOVE, true);
		inv.ammo[BOMB_GLOVE as usize] = 38;
		assert_eq!(inv.add_ammo(BOMB_GLOVE, 3), 1);
		assert_eq!(inv.ammo(BOMB_GLOVE), 40);
	}

	// WPN-6.8 [PC]: the debug grant owns the seven weapons and gives each a quick-select slot; it is repeatable.
	#[test]
	fn debug_grant_owns_seven_weapons_with_quick_slots() {
		let mut inv = Inventory::new();
		inv.debug_grant_weapons();
		inv.debug_grant_weapons();
		for id in DEBUG_WEAPONS {
			assert!(inv.have[id as usize], "{id} owned");
			assert_eq!(inv.quick.iter().filter(|&&q| q == id).count(), 1, "{id} in one quick slot");
		}
		assert_eq!(inv.quick.iter().filter(|&&q| q != 0).count(), 7);
		assert_eq!(inv.equipped, WRENCH);
		// Items with an ammo row reach the cap (max ammo applies to any DEBUG_WEAPONS id that gains a row).
		for id in DEBUG_WEAPONS {
			if let Some(i) = info(id).filter(|i| i.max != 0) {
				assert_eq!(inv.ammo[id as usize], i.max);
			}
		}
	}
}
