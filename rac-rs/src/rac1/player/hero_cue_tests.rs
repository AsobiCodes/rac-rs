//! Test helpers (a jump state, a water world) and notes for the hero sound-cue tests (AUD-11e, HERO-23).
//! Test-only (`cfg(test)`) leftover from the retired hand port; the test bodies are gone. Do not extend.
use super::*;

// AUD-11e test 2: coming up from under water (prev group 0x11) into the surface state queues the breath cues.
// AUD-11e test 3: the crouch cue needs a gait step strictly above dt * 3.5 and no blocking face.
// AUD-11e test 4: a loop slot starts once and a group change stops it.
// AUD-11e test 5: entering the water state from the surface group plays the dive sound, from the under group not.
fn jump_state() -> State {
	State::Jump { kind: JumpKind::Stand, t: 0.0, z0: 0.0, heading: 0.0, landed: None }
}

/// HERO-23: a water face (kind 0) at z 1.0 for the water ray, plus a kind-31 floor at 0.25 when `floor`.
fn water_world(floor: bool) -> World {
	let quad = |z: f32, kind: u8| [([[-40.0, -40.0, z], [40.0, -40.0, z], [40.0, 40.0, z]], kind), ([[-40.0, -40.0, z], [40.0, 40.0, z], [-40.0, 40.0, z]], kind)];
	let mut f = quad(1.0, 0).to_vec();
	if floor {
		f.extend(quad(0.25, 31));
	}
	World::typed(f, 4.0)
}

// AUD-11e.2 tests 1-2: the water-landing cue fires on the first tick under the surface after the takeoff tick.
// AUD-11e.2 test 3: back above the water with a floor below resets the counter.
// AUD-11e.2 test 4: the thrown wrench loop slot starts once outbound and stops near the hand (with the loop range)
// or at the catch.
// HERO-AFTERIMAGE-15 tests 1-3 and 5: the afterimage ring records the position before each step; the two copies show the
// entries 3 and 5 pushes back and stay hidden until the ring holds that many.
// AUD-11e.2 test 5: a rising jump does not enter the swim state, a falling one does.
// AUD-11g tests 2-3: the frame crossing window.
// AUD-11g test 4: walking state 2 gates on tick pal_ticks(15) = 12 and plays foot 0 at tick 18.
// AUD-11g test 5: surface class bits.
// AUD-11g test 6: a fall landing plays both feet in order.
// WPN-1 tests 1 and 2: the weapon stow flag.