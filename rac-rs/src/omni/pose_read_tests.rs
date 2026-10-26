//! Tests of the PURE `pose_read` (hand and gun points from the attach matrices in RAM, other joints from the rendered slot, the mirror plane) on the RAM
//! images of three savestates. The reference numbers are what the retail call `moby_attach_to_joint` gave on the same states (stage B3, recorded
//! before that path was removed): attach point 0 = right hand, 1, 6, 4, 14 = LEFT hand, 11 = chest (joint 6); each as three basis rows then the position.

use super::*;
use crate::omni::binding::Binding;
use crate::omni::pose::TEST_LOCK;

const HERO_CELL: usize = 0x14_13d0;
/// Rilgar (Blaster in hand), Kerwan (the replay's level) and Novalis.
const STATES: [&str; 3] = ["fg1/7_0284", "fg1/3_0086", "fg1/1_0016"];

/// Retail-call values per state: (hero position, attach 0, 1, 6, 4, 14, 11).
type Ref = ([f32; 3], [[f32; 12]; 6]);
const REF_7_0284: Ref = (
	[265.12857, 71.65756, 45.765625],
	[
		[0.1340954, -0.062467672, 0.9889936, 0.8627707, -0.4835799, -0.14752516, 0.48747465, 0.8730617, -0.010955334, 265.2409, 71.93329, 46.330708],
		[-0.588498, 0.3664045, 0.7206984, 0.22588652, -0.78139186, 0.5817178, 0.7762937, 0.50513685, 0.37708157, 265.23752, 71.865486, 46.374863],
		[-0.42310837, 0.45013723, 0.7863487, 0.18524635, -0.8065527, 0.5613826, 0.8869329, 0.38319477, 0.25787207, 265.24698, 71.82798, 46.39676],
		[-0.93566835, 0.1495536, -0.31959376, -0.14736193, -0.9885838, -0.031172158, -0.32061043, 0.017922148, 0.9470371, 265.1308, 71.67687, 46.672813],
		[-0.6825082, 0.47832766, 0.5525863, -0.5812574, 0.103090465, -0.8071426, -0.4430508, -0.8720905, 0.20768073, 265.0034, 71.494225, 46.27291],
		[-0.8619677, 0.50435215, -0.051269285, -0.5018863, -0.8632417, -0.053970538, -0.0714796, -0.020794233, 0.99722224, 265.1356, 71.67847, 46.608208],
	],
);
const REF_3_0086: Ref = (
	[286.452, 110.64974, 66.0],
	[
		[0.8574565, 0.18646826, 0.47947964, -0.43274403, -0.24261972, 0.86822754, 0.27825046, -0.95198417, -0.12737182, 286.589, 110.37013, 66.507545],
		[0.8133127, -0.4726096, 0.33925137, -0.36749908, 0.03474632, 0.929356, -0.4510136, -0.8805478, -0.14543335, 286.5304, 110.381355, 66.61914],
		[0.8133127, -0.4726096, 0.33925137, -0.36749908, 0.03474632, 0.929356, -0.4510136, -0.8805478, -0.14543335, 286.51126, 110.38491, 66.6591],
		[0.88150084, 0.4600465, -0.106339194, -0.46506912, 0.88485557, -0.027115753, 0.0816189, 0.07335818, 0.9939584, 286.4653, 110.61677, 66.886826],
		[0.74141705, -0.024596632, 0.6705637, 0.57659686, 0.5345056, -0.6178967, -0.34322032, 0.8447881, 0.41048616, 286.40872, 110.885506, 66.50957],
		[0.885003, 0.4639553, -0.03885556, -0.46506912, 0.88485557, -0.027115753, 0.021799518, 0.04206857, 0.99887526, 286.4641, 110.61398, 66.826805],
	],
);
/// Novalis 1_0016: the hero is in state 0x64 and hidden (visible byte 0), the stored matrices belong to a place about 11 m from the hero moby
/// (retail's own place for the held items in that state); the retail call, which uses the moby's own position, gave this hand instead.
const REF_1_0016_RETAIL_CALL_HAND: [f32; 12] = [-0.052150995, 0.8179393, 0.5729337, 0.17751683, -0.55698454, 0.81132823, 0.9827336, 0.14401706, -0.11615094, 162.82126, 136.50896, 60.522587];

fn ram_of(name: &str) -> Option<(Vec<u8>, u32)> {
	let path = format!("{}/../oracle/states/{name}/eeMemory.bin", env!("CARGO_MANIFEST_DIR"));
	let Ok(mut ram) = std::fs::read(&path) else {
		eprintln!("skipped: savestate {name} not found");
		return None;
	};
	ram.truncate(32 << 20);
	let hero = u32_at(&ram, HERO_CELL)? & MASK;
	pose::hero_joints(&ram, hero)?;
	Some((ram, hero))
}

fn skeleton() -> SkeletonDef {
	Binding::parse(include_str!("rac1.json")).expect("binding").skeleton.expect("the hero skeleton section")
}

fn near(a: &[f32; 12], b: &[f32; 12], tol: f32) -> bool {
	a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
}

/// A synthetic rendered slot: every joint at the origin except `set` ones; `hero_pos` of the frame.
fn publish(sk: &SkeletonDef, set: &[(u32, [f32; 12])], hero_pos: [f32; 3]) {
	let mut world = vec![[0.0f32; 12]; sk.joint_count as usize];
	for (j, m) in set {
		world[*j as usize] = *m;
	}
	pose_capture::publish_world(world, hero_pos);
}

fn refs() -> [(&'static str, Ref); 2] {
	[("fg1/7_0284", REF_7_0284), ("fg1/3_0086", REF_3_0086)]
}

// The hand and the other stored points equal what the retail call gave (to 0.01) on the two states where the hero is in play.
#[test]
fn stored_points_match_the_recorded_retail_call() {
	let _t = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	let (sk, mut seen) = (skeleton(), 0);
	for (name, (_, mats)) in refs() {
		let Some((ram, hero)) = ram_of(name) else { continue };
		// attach 0 -> joint 56 (right hand), 1 -> 55, 6 -> 54, 4 -> 7
		for (m, joint) in [(0, 56), (1, 55), (2, 54), (3, 7)] {
			let got = read_ram(&ram, 1, &sk, hero, joint, 0).unwrap_or_else(|e| panic!("{name} joint {joint}: error {e}"));
			assert!(near(&got, &mats[m], 0.01), "{name} joint {joint}: {got:?} vs {:?}", mats[m]);
		}
		seen += 1;
	}
	assert!(seen > 0, "no savestate found");
}

// Which joints have a stored matrix: the nine attach points; the left hand, the chest and the fingers do not.
#[test]
fn the_stored_matrices_cover_the_nine_attach_points() {
	let _t = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	let mut seen = 0;
	for name in STATES {
		let Some((ram, hero)) = ram_of(name) else { continue };
		let class = u32_at(&ram, hero as usize + 0x24).unwrap() & MASK;
		for (joint, want) in [(56, Some(0)), (55, Some(1)), (99, Some(2)), (93, Some(3)), (7, Some(4)), (5, Some(5)), (54, Some(6)), (3, Some(7)), (76, None), (6, None), (57, None)] {
			assert_eq!(matrix_index(&ram, class, joint), want, "{name}: joint {joint}");
		}
		seen += 1;
	}
	assert!(seen > 0);
}

// Novalis 1_0016: the hero is hidden in state 0x64; the stored matrices (what retail's logic sees) are not where the retail call put the hand.
#[test]
fn a_hidden_hero_state_reads_the_stored_matrices() {
	let _t = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	let Some((ram, hero)) = ram_of("fg1/1_0016") else { return };
	assert_eq!((ram[hero as usize + 0x31], u32_at(&ram, 0x14_13d4)), (0, Some(0x64)), "the state is the hidden-hero one");
	let sk = skeleton();
	let got = read_ram(&ram, 1, &sk, hero, 56, 0).unwrap();
	assert_eq!(got, stored(&ram, 0).unwrap());
	let d = (0..3).map(|i| (got[9 + i] - REF_1_0016_RETAIL_CALL_HAND[9 + i]).powi(2)).sum::<f32>().sqrt();
	assert!(d > 5.0 && d < 15.0, "{d}");
}

// A read changes nothing: the reads take the RAM image by shared reference; reading every joint twice (with and without the mirror) gives
// identical answers and the image is byte-identical afterwards.
#[test]
fn reads_change_nothing() {
	let _t = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	let sk = skeleton();
	let mut seen = 0;
	for name in STATES {
		let Some((ram, hero)) = ram_of(name) else { continue };
		let before = ram.clone();
		pose_capture::clear();
		new_tick();
		publish(&sk, &[(76, [1.0; 12]), (sk.chest, [2.0; 12])], [0.0; 3]);
		for flags in [0, FLAG_MIRRORED] {
			for joint in 0..sk.joint_count {
				let a = read_ram(&ram, 7, &sk, hero, joint, flags);
				assert_eq!(a, read_ram(&ram, 7, &sk, hero, joint, flags), "{name}: joint {joint} flags {flags}");
			}
		}
		assert!(ram == before, "{name}: the image changed");
		seen += 1;
	}
	assert!(seen > 0);
	pose_capture::clear();
	new_tick();
}

// pose.mirrored: the stored matrix reflected across the plane through `hero position + n * offset`, n = the hero's left axis (row 1 at +0xd0).
// Checked with the formula written out again here, on a slot whose chest is where retail's chest point was.
#[test]
fn mirrored_is_the_reflection_of_the_read_matrix() {
	let _t = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	let sk = skeleton();
	let mut seen = 0;
	for (name, (hero_pos, mats)) in refs() {
		let Some((ram, hero)) = ram_of(name) else { continue };
		pose_capture::clear();
		new_tick();
		let mut chest = [0.0f32; 12];
		chest[9..12].copy_from_slice(&mats[5][9..12]);
		publish(&sk, &[(sk.chest, chest)], hero_pos);
		let h = hero as usize;
		let row1 = vec3_at(&ram, h + 0xd0).unwrap();
		let len = (row1[0] * row1[0] + row1[1] * row1[1] + row1[2] * row1[2]).sqrt();
		let n = row1.map(|c| c / len);
		let hp = vec3_at(&ram, h + 0x10).unwrap();
		let off = (0..3).map(|i| (mats[5][9 + i] - hero_pos[i]) * n[i]).sum::<f32>();
		let plain = read_ram(&ram, 1, &sk, hero, 56, 0).unwrap();
		let got = read_ram(&ram, 1, &sk, hero, 56, FLAG_MIRRORED).unwrap();
		// the reflection written out again
		let mut want = [0.0f32; 12];
		for r in 0..4 {
			let v = [plain[r * 3], plain[r * 3 + 1], plain[r * 3 + 2]];
			let rel = if r == 3 { (0..3).map(|i| (v[i] - (hp[i] + n[i] * off)) * n[i]).sum::<f32>() } else { (0..3).map(|i| v[i] * n[i]).sum::<f32>() };
			for i in 0..3 {
				want[r * 3 + i] = v[i] - 2.0 * rel * n[i];
			}
		}
		assert!(near(&got, &want, 1e-4), "{name}: {got:?} vs {want:?}");
		// it is a reflection: the position is as far on the other side of the plane
		let side = |p: &[f32; 12]| (0..3).map(|i| (p[9 + i] - (hp[i] + n[i] * off)) * n[i]).sum::<f32>();
		assert!((side(&got) + side(&plain)).abs() < 1e-3 && side(&plain).abs() > 0.1, "{name}: sides {} {}", side(&plain), side(&got));
		// reflecting twice gives the matrix back
		assert!(near(&reflect(&reflect(&plain, n, hp), n, hp), &plain, 1e-4));
		// the mirrored right hand lands near the real left hand (the plane does not follow the torso lean, so not exactly)
		let d = (0..3).map(|i| (got[9 + i] - mats[4][9 + i]).powi(2)).sum::<f32>().sqrt();
		eprintln!("{name}: mirrored right hand is {d:.3} m from the retail left hand");
		assert!(d < 0.2, "{name}: {d} m");
		seen += 1;
	}
	assert!(seen > 0);
	pose_capture::clear();
	new_tick();
}

// Before the first rendered frame: a mirrored read uses offset 0, a joint with no stored matrix fails with -1.
#[test]
fn without_a_rendered_slot_the_offset_is_zero_and_slot_joints_fail() {
	let _t = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	let Some((ram, hero)) = ram_of(STATES[0]) else { return };
	let sk = skeleton();
	pose_capture::clear();
	new_tick();
	assert_eq!(read_ram(&ram, 1, &sk, hero, 76, 0), Err(-1));
	assert!(pose_capture::reads_on(), "the first slot read asks the renderer to publish");
	let (plain, got) = (read_ram(&ram, 1, &sk, hero, 56, 0).unwrap(), read_ram(&ram, 1, &sk, hero, 56, FLAG_MIRRORED).unwrap());
	let (n, pos) = hero_axes(&ram, hero).unwrap();
	assert!(near(&got, &reflect(&plain, n, pos), 1e-5));
	assert_eq!(read_ram(&ram, 1, &sk, hero, 111, 0), Err(-3));
	assert_eq!(read_ram(&ram, 1, &sk, hero ^ 0x1000, 56, 0), Err(-2));
	pose_capture::clear();
	new_tick();
}

// Other joints come from the rendered slot, and one tick sees one frame: a later publish shows only from the next tick.
#[test]
fn slot_joints_are_copied_once_per_tick() {
	let _t = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	let Some((ram, hero)) = ram_of(STATES[0]) else { return };
	let sk = skeleton();
	let left_hand = REF_7_0284.1[4];
	pose_capture::clear();
	new_tick();
	publish(&sk, &[(76, left_hand)], REF_7_0284.0);
	assert_eq!(read_ram(&ram, 5, &sk, hero, 76, 0), Ok(left_hand));
	let mut moved = left_hand;
	moved[9] += 1.0;
	publish(&sk, &[(76, moved)], REF_7_0284.0);
	assert_eq!(read_ram(&ram, 5, &sk, hero, 76, 0), Ok(left_hand), "same tick: same frame");
	new_tick();
	assert_eq!(read_ram(&ram, 5, &sk, hero, 76, 0), Ok(moved), "next tick: the new frame");
	// a non-finite slot value is no answer
	let mut bad = left_hand;
	bad[3] = f32::NAN;
	publish(&sk, &[(76, bad)], REF_7_0284.0);
	new_tick();
	assert_eq!(read_ram(&ram, 5, &sk, hero, 76, 0), Err(-1));
	pose_capture::clear();
	new_tick();
}

// A mod reading four joints twice per tick (one stored, one from the slot, two mirrored): the cost of the reads.
#[test]
fn reads_are_cheap() {
	let _t = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	let Some((ram, hero)) = ram_of(STATES[0]) else { return };
	let sk = skeleton();
	pose_capture::clear();
	publish(&sk, &[(76, REF_7_0284.1[4]), (sk.chest, REF_7_0284.1[5])], REF_7_0284.0);
	let n = 2000u64;
	let t0 = std::time::Instant::now();
	for tick in 0..n {
		new_tick();
		for _ in 0..2 {
			for (joint, flags) in [(56, 0), (76, 0), (56, FLAG_MIRRORED), (76, FLAG_MIRRORED)] {
				std::hint::black_box(read_ram(&ram, tick, &sk, hero, joint, flags).unwrap());
			}
		}
	}
	let per_tick = t0.elapsed().as_secs_f64() * 1000.0 / n as f64;
	eprintln!("4 joints, read twice: {per_tick:.4} ms per tick");
	assert!(per_tick < 0.3, "{per_tick} ms per tick");
	pose_capture::clear();
	new_tick();
}
