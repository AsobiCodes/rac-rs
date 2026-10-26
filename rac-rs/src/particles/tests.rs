//! Test helpers for the old `Particles` pool (counting effects, comparing vectors, reading puff / glow / flash effects).
//! Test-only (`cfg(test)`) leftover from the retired hand port; the test bodies are gone. Do not extend.
use super::*;

fn count(p: &Particles, f: impl Fn(&Effect) -> bool) -> usize {
	p.slots.iter().flatten().filter(|e| f(e)).count()
}

const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn near3(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
	(0..3).all(|i| (a[i] - b[i]).abs() <= tol)
}

// VEL-13b step S: the strand frame is x = unit(end - origin), y = (0,0,1) x x, z = x x y, unnormalised (FUN_002e1bf8).
/// VEL-13c: the glints one arc tick spawns, in spawn order (fresh pool: ascending slots), with the replayed draws.
struct ArcGlints {
	glints: Vec<([f32; 3], [f32; 3], f32)>,
	r: f32,
	ang: f32,
	origin_flash: bool,
	ve: [f32; 3],
}

fn puff_list(p: &Particles) -> Vec<&Puff> {
	p.slots.iter().flatten().filter_map(|e| if let Effect::Puff(q) = e { Some(q) } else { None }).collect()
}

const ROBOT_TEST_PRIMS: [([f32; 3], [f32; 3], f32); 2] = [([0.0, 0.0, 0.5], [0.0, 0.0, 1.0], 0.5), ([0.0, 0.0, 0.5], [0.0, 0.0, 1.25], 0.75)];

// PAR-BRIDGE
// PAR-BRIDGE remainder (gold constructors): no constructor draws, full pool -> None with the RNG untouched.
// PAR-BRIDGE test 2: an out-of-range spark is refused before claiming.
// PAR-BRIDGE test 3: a life-2 glow lives one tick and is freed on the second.
// NOV-8.4: spark colour is mixed toward alpha 0 by t = (L-1)/L of the decremented life, round-toward-zero.
// NOV-9.3 draw side: the type-0x16 lofting smoke sprite, growth, colour mix, buoyancy and free.
// PAR-BRIDGE test 1 (smoke_loft): one `next()` after the claim; a full pool returns None with the RNG untouched.
// PAR-BRIDGE test 5: a PipeChild-like producer (rf, rf, rf, ri, then the creator's rot) twice: slots ascend and the
// RNG stream is own x4, rot, own x4, rot.
// ARI-30 (C580): the lofting smoke's rot byte is the low byte of the one rand() word drawn after the claim.
// BRIDGE-1 test 4: a detached instance does not reset the shared pool.
// PAR-5 tests 2, 4, 6: the jet shot pair claims 2 slots with one raw draw each; the 0x4f trail ramps at timer 19
// (79 * 19 / 33 = 45), not at 20; a gone owner frees the trail.
// PAR-5 tests 5, 7, 8: a Birdbot trail (life 8) lives 7 updates without a ramp; the glint ctor draws size, rot,
// life after the claim; the glint fade and rotation wrap; the last tick frees after its rot draw.
// PAR-3 tests 1-2: the streak ribbon half-widths 0.09 / 0.045; the ember colour weight of c0 is life / life0.
// NOV-15.2 type-0x10 puff.
fn puff10_of(p: &Particles, i: usize) -> &Puff10 {
	let Some(Effect::Puff10(e)) = &p.slots[i] else { panic!("slot {i} is not a puff") };
	e
}

/// The first `Glow20` of the pool.
fn glow20(p: &Particles) -> Glow20 {
	p.slots.iter().flatten().find_map(|e| if let Effect::Glow20(g) = e { Some(g.clone()) } else { None }).unwrap()
}

fn flash5_of(p: &Particles, k: usize) -> Flash5 {
	let Some(Effect::Flash5(f)) = &p.slots[k] else { panic!("not a flash5") };
	f.clone()
}
