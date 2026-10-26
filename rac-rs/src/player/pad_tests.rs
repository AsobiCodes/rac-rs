//! Test helpers (a floor world, a position check) for the hero pad-movement tests.
//! Test-only (`cfg(test)`) leftover from the retired hand port; the test bodies are gone. Do not extend.
use super::*;
use crate::camera::TICK;
fn floor() -> World {
	World::typed(vec![([[-40.0, -40.0, 2.0], [40.0, -40.0, 2.0], [40.0, 40.0, 2.0]], 31u8), ([[-40.0, -40.0, 2.0], [40.0, 40.0, 2.0], [-40.0, 40.0, 2.0]], 31u8)], 4.0)
}

fn near(p: &Player, x: f32, y: f32, yaw: f32) {
	assert!((p.pos[0] - x).abs() < 1e-3 && (p.pos[1] - y).abs() < 1e-3 && (p.yaw - yaw).abs() < 1e-3, "{:?} {}", p.pos, p.yaw);
}
