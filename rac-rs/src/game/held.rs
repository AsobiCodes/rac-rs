//! BRIDGE-38 HELD-RIG (WPN-6.0): the generic equipped-weapon actor. A held class without a bespoke rig (Blaster 168,
//! Devastator 157, Visibomb 163) is a fresh actor on sequence 0 with its own clock, stepped once per tick, whose
//! model the viewer poses at the hero's attach point and whose joint 0 (the muzzle) comes back each frame
//! (decomp/notes/weapons_framework.md "WPN-6.0 port spec"). The per-weapon updaters hook in `update_held`.

use super::*;

/// Held classes served by the generic rig (168, Devastator 157, 163, Walloper 180 / WPN-6.6a, Mine Glove 190 / WPN-6.4a, Morph-o-Ray 185 / WPN-6.7a,
/// Taunter 175 / WPN-6.9a); the wrench, Pyrocitor, Suck Cannon and glove keep their own paths. Registry: one entry per
/// line in its historical (load) order, which the viewer's rig loading keeps; add new held classes at their own line.
pub const HELD_RIG_CLASSES: &[u32] = &[
	168,
	157,
	163,
	180,
	190,
	229, // Glove of Doom
	185,
	175,
	188, // Trespasser (ARI-42a)
	208, // Swingshot (ARI-47c)
];

/// ARI-47c / BRIDGE-76: the held Swingshot class and its hook model (class 209, no inventory item: the viewer loads it
/// beside the held rig).
pub const SWINGSHOT_HELD: u32 = 208;

/// Retail mode only (retail.rs `sync`): the hand items rac-rs has no game rig for, drawn by the same generic held path
/// from retail's held moby clock: Tesla Claw 177 (item 19), Hydrodisplacer 1251 (22), R.Y.N.O. 454 (23), 483 (24 / 31),
/// Decoy Glove 562 (25), 585 (27), 619 (32) (inventory.rs `ITEM_VIEW` rows with attach point 0, 1 or 6).
pub const RETAIL_HELD_EXTRA: &[u32] = &[177, 1251, 454, 483, 562, 585, 619];

/// The classes the viewer loads a generic held rig for and poses in this order: `HELD_RIG_CLASSES`, then in retail
/// mode `RETAIL_HELD_EXTRA`.
pub fn rig_classes() -> Vec<u32> {
	let extra = if crate::retail::active() { RETAIL_HELD_EXTRA } else { &[] };
	HELD_RIG_CLASSES.iter().chain(extra).copied().collect()
}
pub const SWINGSHOT_HOOK: u32 = 209;

/// The equipped weapon's actor state: its class and its own animation clock, the Devastator updater state (WPN-6.2a)
/// and the BRIDGE-38 draw ticks 0x140400 (3 on an item change, +1 per tick after).
pub struct HeldRig {
	pub class: u32,
	pub anim: crate::anim::PropAnim,
	/// ARI-47c: the held Swingshot updater state and its hook.
	pub(super) swing: super::swingshot::Held208,
}

impl Game {

	/// The held rig's class and clock, for the viewer to pose.
	pub fn held_pose(&self) -> Option<(u32, &crate::anim::PropAnim)> {
		self.held.as_ref().map(|h| (h.class, &h.anim))
	}

	/// A held class's sequence timings (viewer load).
	pub fn set_held_timing(&mut self, class: u32, seqs: std::rc::Rc<Vec<crate::anim::Sequence>>) {
		self.held_seqs.insert(class, seqs);
		if let Some(h) = self.held.as_mut().filter(|h| h.class == class) {
			h.anim.attach(self.held_seqs.get(&class).cloned());
		}
		// BRIDGE-76: the Swingshot hook (class 209) timings.
		if class == super::swingshot::HOOK {
			if let Some(k) = self.held.as_mut().and_then(|h| h.swing.hook.as_mut()) {
				k.anim.attach(self.held_seqs.get(&class).cloned());
			}
		}
	}





	/// The point the held class updaters fire from: the posed muzzle, else the same fallback as `muzzle`.
	pub(super) fn held_muzzle_point(&self) -> V3 {
		let (s, c) = self.player.yaw.sin_cos();
		self.held_muzzle.unwrap_or(add(self.player.pos, [c * 0.5, s * 0.5, 0.9]))
	}
}

