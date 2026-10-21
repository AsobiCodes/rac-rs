//! ARI-45: Swingshot acquisition (decomp/notes/aridia_gadgets.md "ARI-45 port spec"): the swing selector, the pull
//! selector, their common scorer and the dispatcher case 0xc that enters the swing (0x2c) or the pull (0x24).

use crate::gameplay::pal_ticks;
use crate::gameplay::PAL_DT;







// ARI-47a (decomp/notes/aridia_gadgets.md "ARI-47a port spec"): hero swing state 0x2c.











// ARI-46 (decomp/notes/aridia_gadgets.md "ARI-46 port spec"): the Swingshot pull states 0x24 (aim, hook flying),
// 0x25 (reel toward the target) and 0x26 (orbit at a fixed radius).










#[cfg(test)]
mod tests {
	use super::*;
	use crate::physics::{Scene, World};

	fn flat_world() -> World {
		World::new([[[-50.0, -50.0, 0.0], [50.0, -50.0, 0.0], [50.0, 50.0, 0.0]], [[-50.0, -50.0, 0.0], [50.0, 50.0, 0.0], [-50.0, 50.0, 0.0]]], 4.0)
	}

	// ARI-45 test (1): the pull score with and without the 10.0 penalty.
	// ARI-45 test (2): the swing beats the penalised pull; the dispatcher clears pull_ok and enters 0x2c.
	// ARI-45 test (3): hero z - target z <= 3.0 inclusive.
	// ARI-45 test (4): a pull closer than 4.0 in XY is skipped and the previous pull is kept.
	// ARI-45 test (5): a swing target at a yaw gap of pi/2 is rejected.
	// ARI-45 test (6): the cooldown blocks the entry until it reaches 0; the entry sets scale(40) = 33.
	// ARI-45 test (7): the press windows scale(8) = 7 in the air, scale(5) = 4 grounded; held threshold scale(15) = 12.
	// ARI-47a tests (1)-(5): the constant bits. The `calc:` products are the EE-truncating `vmul`; the port's runtime
	// `dt * K` products round to nearest like the rest of the hero code (at most 1 ulp apart).
	// ARI-47a test (7): the entry attaches the selected anchor (the pvar +0x14 write is game-side, aridia.rs tests).
	// ARI-47a test (6): the release needs fire up after scale(10) = 8 ticks; swing_early is set below tick 33 and never
	// cleared.
	// ARI-47b test (1): the long flag of a low release sets on scale(18) = 15, not 14.
	// ARI-47b test (2): the early long landing locks scale(10) = 8 (6 after state 6); 15 without support after tick 42.
	// ARI-47b test (4): the pivot turn.
	// ARI-47b test (5): above 1.0 the long flag sets on the first tick; the fall gravity is 27.
	// BRIDGE-75: the tilt springs back outside 0x2c / 0x2d, at most dt * 5.2359877 per tick.
	// ARI-46 test (1): the pull constants (D2 is PAL_DT2 = 0x39d1b718; the spec's 4e-4 literal is 1 ulp lower).
	// ARI-46 test (2): the brake roots at distance 5.0.
	// ARI-46: dispatch 0x24 on a selected pull; the aim entry counts the stat and holds frames 10..=14 of 0x2f.
	// ARI-46 test (3): the reel accelerates by D2 * 80 from 0 (left 50 on the first tick) and caps at dt * 27 on tick 17.
	// ARI-46 tests (4) / (4a): the down-pull and the airborne aim gravity with the steep-face push.
	// ARI-46 test (5): fire up early sets swing_early; high -> fall at 29 with clip 10, low -> stand with the idle clip.
	// ARI-46 test (6): the orbit radius slews dt * 10 per tick toward pvar +4; fire up falls from 0.4 up, else stands.
	// ARI-46 test (8): the aim turns toward the target and ends when the Swingshot is put away; a lost target releases.
}
