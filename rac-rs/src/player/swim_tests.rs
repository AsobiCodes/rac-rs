use super::*;

fn quad(z: f32, h: f32, kind: u8) -> Vec<([V3; 3], u8)> {
	vec![([[-h, -h, z], [h, -h, z], [h, h, z]], kind), ([[-h, -h, z], [h, h, z], [-h, h, z]], kind)]
}

/// A 4.75 deep pool: floor at 0.25, water (face id 0) at 5 (HERO-23: the water ray's end clamps at z 0, and
/// `ray_face` misses a face exactly on the segment end, so the floor sits above 0).
fn pool() -> World {
	let mut f = quad(0.25, 40.0, 31);
	f.extend(quad(5.0, 40.0, 0));
	World::typed(f, 4.0)
}

// NOV-14.4 test (7): the re-entry window is scale(3) = 2 ticks.
/// HERO-23: a 0.25 floor (kind 31) and a water face (kind 0) at 10.
fn deep_pool(extra: &[f32]) -> World {
	let mut f = quad(0.25, 40.0, 31);
	f.extend(quad(10.0, 40.0, 0));
	for &z in extra {
		f.extend(quad(z, 40.0, 31));
	}
	World::typed(f, 4.0)
}

fn under_at(z: f32) -> Player {
	let mut p = Player::new([0.0, 0.0, z], 0.0);
	p.state = State::Swim { mode: Swim::Under, t: 1.0 };
	p.under_wz = 10.0;
	p
}
