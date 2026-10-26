//! Test helpers (floor, wall and mud worlds) for the hero ground tests.
//! Test-only (`cfg(test)`) leftover from the retired hand port; the test bodies are gone. Do not extend.
use super::*;

fn floor(x0: f32, x1: f32) -> World {
	World::typed(vec![([[x0, -40.0, 2.0], [x1, -40.0, 2.0], [x1, 40.0, 2.0]], 31u8), ([[x0, -40.0, 2.0], [x1, 40.0, 2.0], [x0, 40.0, 2.0]], 31u8)], 4.0)
}

/// A vertical wall at x facing -x (stored winding a, c, b of the counter-clockwise a, b, c seen from the front).
fn wall(x: f32, kind: u8) -> MobySolid {
	let (a, b, c, d) = ([x, -3.0, -1.0], [x, -3.0, 3.0], [x, 3.0, -1.0], [x, 3.0, 3.0]);
	MobySolid { tris: vec![([a, c, b], kind), ([c, d, b], kind)], ..Default::default() }
}

// HERO-VEL
// NOV-20.1d: the hard-landing flag ripples only inside the swim states, so a fall entry makes no water cue.
/// ARI-48: a mud face (surface 3, attr 0x83) at `mud` over an ordinary floor at `floor` (both flat, 80 x 80).
fn mud_world(floor: f32, mud: f32) -> World {
	let quad = |z: f32, k: u8| [([[-40.0, -40.0, z], [40.0, -40.0, z], [40.0, 40.0, z]], k), ([[-40.0, -40.0, z], [40.0, 40.0, z], [-40.0, 40.0, z]], k)];
	World::typed(quad(floor, 31).into_iter().chain(quad(mud, 0x83)).collect::<Vec<_>>(), 4.0)
}
