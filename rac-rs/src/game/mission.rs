//! Novalis story actors (decomp/notes/novalis_flow.md, owner): the mission NPCs that land when the
//! hero arrives and hand over the Infobot once their guards are down (classes 790/730,
//! FUN_002fa698), the Plumber's dialogue (class 774 FUN_002fea18 on the shared helpers FUN_0027a940 /
//! FUN_0027ae68), the checkpoints (class 805 FUN_002ffb50) and the ship pad (FUN_002a1570). Scenes
//! go to the Flow through `Game::scene_requests`; `Game::scene_playing` stands for DAT_0015f5a4 == 2.
//! Story bytes live in `Game::story` (save.rs `Story`, global and carried across levels), addressed
//! by their game address.

use super::Game;
use super::Event;
use crate::physics::V3;

pub(super) const MISSION_NPC: u32 = 790;
pub(super) const MISSION_NPC_2: u32 = 730;
pub(super) const PLUMBER: u32 = 774;
pub(super) const INFOBOT: u32 = 750;
/// The ship the pad callback spawns (DAT_001604c8[ship type 0] = 531) at the level header's ship
/// transform (settings +0x2c, gameplay.rs `ship`).
pub const HEADER_SHIP: u32 = 531;

/// Popup text ids by planet (level_transition.md "Unlock persistence", table 0x209fc0 order).
pub(super) const UNLOCK_TEXT: [u32; 19] = [0, 0x6, 0x3f1, 0x3f2, 0xbc6, 0x177b, 0xfa6, 0x1394, 0x1b58, 0x1f47, 0x1f48, 0x2719, 0x271a, 0x2ee5, 0x32c9, 0x36b1, 0x3aa1, 0x3aa2, 0x4269];




/// NOV-14.1 gold-weapon offers (decomp/notes/novalis_checklist.md "NOV-14.1"): the case class and its displays.
pub(super) const GOLD_CASE: u32 = 304;
pub(super) const GOLD_DISPLAY_FIRST: u32 = 1456;
pub(super) const GOLD_DISPLAY_LAST: u32 = 1465;






/// The ship pad: the ship prop, and whether the hero is in reach (`+0xc`).
#[derive(Clone)]
pub(super) struct Pad {
	prop: usize,
}

impl Game {





	/// FUN_00275268: a planet's coordinates are known; appended to the unlock order. The popup shows
	/// when it is not the current level.
	pub(super) fn unlock_planet(&mut self, n: u32) {
		let Some(u) = self.story.unlock.get_mut(n as usize) else { return };
		if *u != 0 {
			return;
		}
		*u = 1;
		let count = self.story.unlock.iter().filter(|&&b| b != 0).count();
		if let Some(o) = self.story.order.get_mut(count - 1) {
			*o = n;
		}
		self.log.push(format!("planet {n} unlocked"));
		if n as i32 != self.level {
			if let Some(&t) = UNLOCK_TEXT.get(n as usize).filter(|&&t| t != 0) {
				self.popups.push((t, crate::gameplay::pal_ticks(0x49c) as u32));
			}
		}
	}

	/// Level start (FUN_00275268(current), silent; skipped on Veldin): the level unlocks itself.
	pub fn arrive(&mut self, level: usize) {
		self.level = level as i32;
		if let Some(v) = self.story.visited.get_mut(level) {
			*v = (*v).max(1);
		}
		if level != 0 {
			self.unlock_planet(level as u32);
		}
	}

	/// The planets unlocked so far in unlock order (A20), for the travel screen.
	pub fn planets(&self) -> Vec<usize> {
		let n = self.story.unlock.iter().filter(|&&b| b != 0).count();
		self.story.order[..n].iter().map(|&p| p as usize).collect()
	}





	/// The ship pad's prop index, position and yaw (the ship stream's frame, FUN_002a39b0 via +0x10 / +0xc0).
	pub fn ship_pad(&self) -> Option<(usize, V3, f32)> {
		let pad = self.pad.as_ref()?;
		let p = self.props.get(pad.prop)?;
		Some((pad.prop, p.pos, p.rot[2]))
	}










}








/// The pause-menu button-code table (retail 0x1b9fa0): code `c` expects the 20 presses `T[(c * (i + 1)) & 0xff]`.
const CODE_TABLE: [u8; 256] = [
	3, 1, 4, 1, 5, 4, 2, 1, 2, 1, 3, 2, 3, 1, 3, 2, 4, 3, 1, 3, 4, 1, 0, 1, 0, 4, 1, 0, 2, 5, 3, 5,
	3, 4, 5, 4, 0, 4, 5, 4, 0, 2, 0, 1, 0, 5, 4, 3, 1, 3, 5, 0, 1, 5, 4, 2, 3, 2, 3, 0, 2, 3, 1, 1,
	1, 0, 3, 4, 5, 4, 0, 1, 0, 2, 5, 5, 3, 3, 3, 1, 2, 1, 0, 1, 0, 3, 2, 3, 2, 1, 0, 1, 0, 3, 2, 3,
	1, 1, 0, 2, 4, 5, 0, 4, 2, 3, 0, 2, 1, 5, 4, 2, 3, 0, 5, 5, 5, 5, 2, 2, 1, 0, 2, 3, 4, 4, 4, 3,
	3, 3, 1, 1, 1, 4, 0, 1, 4, 5, 3, 0, 1, 4, 4, 2, 4, 1, 0, 2, 0, 4, 0, 1, 3, 3, 5, 0, 0, 5, 3, 1,
	5, 0, 5, 4, 4, 4, 4, 5, 2, 5, 0, 0, 1, 3, 1, 4, 5, 0, 4, 4, 1, 3, 5, 0, 4, 3, 3, 0, 2, 5, 4, 3,
	2, 0, 0, 2, 1, 2, 2, 3, 3, 0, 5, 0, 2, 1, 5, 2, 1, 2, 4, 3, 0, 5, 3, 2, 0, 3, 1, 2, 5, 2, 1, 1,
	3, 0, 3, 1, 1, 4, 3, 5, 3, 4, 3, 4, 2, 3, 1, 3, 2, 3, 3, 2, 5, 4, 4, 1, 3, 4, 4, 0, 2, 5, 5, 3,
];

impl Game {
	/// FUN_002988a0, the combo-released arm: the entry count resets while the code combo is not held.
	pub fn code_hold(&mut self, combo: bool) {
		if !combo {
			self.code_n = 0;
		}
	}

	/// FUN_002988a0, one press with the combo held (up 0, down 1, left 2, right 3, square 4, circle 5): records it, and
	/// on the 20th entry looks the code up and applies its unlock (items, planets, story bytes, skill points).
	pub fn code_press(&mut self, v: u8) {
		if self.code_n >= 20 {
			return;
		}
		self.code_buf[self.code_n] = v;
		self.code_n += 1;
		if self.code_n < 20 {
			return;
		}
		let Some(c) = (2usize..0x93).find(|&c| (0..20).all(|i| CODE_TABLE[(c * (i + 1)) & 0xff] == self.code_buf[i])) else { return };
		let id = c - 2;
		if id < 0x25 {
			self.inv.have[id] = true;
			self.story.new_flags[id] = 1;
		} else if id - 0x25 < 0x12 {
			self.unlock_planet((id - 0x24) as u32);
		} else if id >= 0x37 && id - 0x37 < 6 {
			self.story.a5[id - 0x37] = 1;
		} else if id >= 0x3d && id - 0x3d < 0x1e && self.skill_points[id - 0x3d] == 0 {
			self.skill_points[id - 0x3d] = 1;
			self.events.push(Event::GlobalCue(1));
			self.popups.push((0x53d6, crate::gameplay::pal_ticks(0xb4) as u32));
		}
	}
}


