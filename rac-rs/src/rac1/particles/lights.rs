//! Point-light slots of the effect pool (8 slots): allocation (FUN_002522c0), `hold_light`, the level's wandering lights and the
//! slot tables handed to the renderer.
//! Leftover from the retired hand port; still compiled because retail mode and the renderer still use parts of `particles` (`Rng`, sprite and quad types). Do not extend.
use super::*;

impl Particles {

	/// FUN_002522c0: the first free slot, None when all 8 are used or the frame load (DAT_0015f5b4) is above 0.8.
	pub(super) fn light_alloc(&mut self, owner: Owner, light: PointLight) -> Option<usize> {
		if self.alloc_load > ALLOC_LOAD_LIMIT {
			return None;
		}
		let k = self.lslots.iter().position(Option::is_none)?;
		self.serial = self.serial.wrapping_add(1);
		self.lslots[k] = Some(LightSlot { owner, light, serial: self.serial });
		Some(k)
	}






	/// A moby's own slot, rewritten every tick (FUN_002522c0 once, then direct writes; FUN_002523c0 frees it on
	/// None). Refused like a flash's when all 8 slots are used or the load gate is shut.
	pub fn hold_light(&mut self, owner: u32, l: Option<PointLight>) {
		let at = self.lslots.iter().position(|s| s.is_some_and(|s| s.owner == Owner::Held(owner)));
		let Some(l) = l else {
			if let Some(k) = at {
				self.lslots[k] = None;
			}
			return;
		};
		match at {
			Some(k) => {
				if let Some(s) = &mut self.lslots[k] {
					s.light = l;
				}
			}
			None => {
				self.light_alloc(Owner::Held(owner), l);
			}
		}
	}


	/// The level's wandering lights (class 1504 placements).
	pub fn set_wanders(&mut self, w: Vec<Wander>) {
		for k in 0..self.wanders.len() {
			self.hold_light(WANDER_OWNER + k as u32, None);
		}
		self.wanders = w;
	}

	/// The live slots, index = slot.
	pub fn point_lights(&self) -> [Option<PointLight>; LIGHT_SLOTS] {
		self.lslots.map(|l| l.map(|l| l.light))
	}

	/// The slots with their allocation serials, for the assignment tracker (`dynlight::Tracker::step`).
	pub fn light_table(&self) -> [Option<SlotLight>; LIGHT_SLOTS] {
		self.lslots.map(|l| l.map(|l| SlotLight { light: l.light, serial: l.serial }))
	}
}
