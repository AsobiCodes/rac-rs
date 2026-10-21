//! KER-60: the grind-rail probe of the hero tick (decomp/notes/kerwan_gadgets.md "KER-60 port spec"). Every hero tick
//! the probe clears its flag, and when the re-entry cooldown is over and the hero is in a ground/fall group (or a jump
//! that is still rising) it looks for the first rail of the level list near a point in front of and above the feet.
//! A hit inside the height window latches the rail point, rail, segment, distance along it and loop word for the grind
//! state (KER-61a).

use super::Player;
use super::State;
use crate::physics::{add, dot, length, scale, sub, vmul, MobySolid, World, V3};







#[cfg(test)]
mod tests {
	use super::*;
	use crate::gameplay::Polygon;
	use crate::player::{JumpKind, State};
	use std::rc::Rc;

	// KER-61a: a hero at `pos` with a latch at the start of a rail along +x (the point 1.0 on is (1, 0, 0)).
	const FALL: State = State::Fall { t: 0.0, long: false };

	// KER-61b: a grinding hero whose probe point (pos + (0.3, 0, 1.34) at yaw 0) is `foot`.
	fn square() -> crate::gameplay::Polygon {
		Polygon { points: vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 10.0, 0.0], [0.0, 10.0, 0.0]], marks: vec![10.0; 4] }
	}

	fn no_world() -> World {
		World::new(std::iter::empty::<[V3; 3]>(), 4.0)
	}

}
