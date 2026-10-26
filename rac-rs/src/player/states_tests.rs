//! Test helpers and notes for the hero state tests (wall and landing worlds, landing counts); one test is left, `exit_blend_counts_scale`.
//! Test-only (`cfg(test)`) leftover from the retired hand port; the test bodies are gone. Do not extend.
use super::*;

fn quad(z: f32, h: f32, kind: u8) -> Vec<([V3; 3], u8)> {
	vec![([[-h, -h, z], [h, -h, z], [h, h, z]], kind), ([[-h, -h, z], [h, h, z], [-h, h, z]], kind)]
}

/// A floor and a wall at x = 0.6 facing -x; the hero at height 2 facing it in a stand jump.
fn wall_world(flip: bool) -> World {
	let h = 40.0;
	let mut t = quad(0.0, h, 2);
	let w = [[0.6, -h, 0.0], [0.6, h, 0.0], [0.6, h, 20.0]];
	let w2 = [[0.6, -h, 0.0], [0.6, h, 20.0], [0.6, -h, 20.0]];
	if flip {
		t.push(([w[0], w[2], w[1]], 2));
		t.push(([w2[0], w2[2], w2[1]], 2));
	} else {
		t.push((w, 2));
		t.push((w2, 2));
	}
	World::typed(t, 4.0)
}

// HERO-11: the pending / given jump impulse model.
// CAM-7.4 helper: FUN_00247dd0.
// CAM-7.4: land_ticks is written only while the hero clock's next-frame byte is under the kind's frame (stand jump 12).
// CAM-7.4b: a double jump writes land_ticks only from anim frame 37.
// HERO-JUMPLAND: the landing keeps the jump's ids; the exits run from the second landed tick.
fn land_world() -> World {
	World::typed(quad(0.0, 60.0, 2), 4.0)
}

fn landed_count(p: &Player) -> Option<u32> {
	match p.state {
		State::Jump { landed, .. } => landed,
		_ => None,
	}
}

// HERO-2 tests: the ground-ray face cache, the CAM-6 face id and the downhill push.
// HERO-1 tests: the forward moby / crate probe (every third frame) and the detector cast height it selects.
// CAM-7.4c tests: the jump anim rate fit, takeoff and airborne rules.
// CAM-7.4d tests: the run-jump early re-land and the Double fit onto `double_rate`.
// HERO-FALLLAND tests: the fall touchdown is a SetState into idle / walk / skid with the input lock and dust table.
/// REN-6c: the PAL scale of the exit-site blend counts (EE round-toward-zero).
#[test]
fn exit_blend_counts_scale() {
	let t = |n| crate::gameplay::pal_ticks(n);
	assert_eq!([t(8), t(9), t(12), t(13), t(15), t(17), t(18)], [7, 7, 10, 11, 12, 14, 15]);
}

fn want_first(seed: u32) -> u32 {
	let mut r = crate::particles::Rng::shared();
	r.reseed(seed);
	r.next()
}

// KER-9: the 0x13f542 jump block counts down beside `platform_ride` and refuses the jump case while nonzero.