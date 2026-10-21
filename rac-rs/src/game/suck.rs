//! The Suck Cannon, held side (decomp/notes/weapons_framework.md WPN-5.1b scan and pickup, WPN-5.1d held updater): the
//! weapon's state machine (wind-up, suction, stored, launch), the scan that registers victims, and the pickups it
//! draws through the tube. The per-victim half is `suckcomp.rs`, the tube is `tube.rs`. Per-tick units throughout.

use super::Game;
use crate::anim::PropAnim;






/// The held class-849 moby's private block.
#[derive(Clone, Debug)]
pub(super) struct SuckState {
	/// The weapon's animation clock.
	pub anim: PropAnim,
	/// The suction loop cue is playing (+0x04 handle valid).
	pub cue: bool,
}

impl SuckState {
	pub(super) fn new() -> SuckState {
		SuckState {  anim: PropAnim::new(0), cue: false,        }
	}
}

impl Game {
	/// The weapon's animation clock while the Suck Cannon is held.
	pub fn suck_pose(&self) -> Option<&PropAnim> {
		self.suck.as_ref().map(|s| &s.anim)
	}

	/// Whether the suction loop (class 849 cue 2) should be playing.
	pub fn suck_loop(&self) -> bool {
		self.suck.as_ref().is_some_and(|s| s.cue)
	}

	/// The weapon's sequence timings (anim.rs `share_timing`) for its anim clock (viewer::load).
	pub fn set_suck_timing(&mut self, seqs: std::rc::Rc<Vec<crate::anim::Sequence>>) {
		self.suck_seqs = Some(seqs);
		if let Some(s) = &mut self.suck {
			s.anim.attach(self.suck_seqs.clone());
		}
	}
















}

