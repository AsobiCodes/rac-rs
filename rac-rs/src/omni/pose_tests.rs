//! Tests of the pose store (pose.rs).

use super::*;
use crate::omni::binding::Binding;
use crate::omni::{pose_host, pose_read};

const BINDING: &str = include_str!("rac1.json");

fn op(joint: u32, kind: u32, w: f32, blend: u32) -> OmniPoseOp {
	OmniPoseOp { joint, op: kind, source: 52, q: [0.0, 0.0, 0.0, 1.0], weight: w, blend_ticks: blend, ..OmniPoseOp::default() }
}

#[test]
fn a_mirror_op_ramps_up_and_fades_out() {
	let mut s = Store::default();
	assert_eq!(s.set(7, 111, &[op(72, OP_MIRROR_FROM, 1.0, 4)]), Ok(1));
	assert!(s.records(7).is_empty());
	s.step();
	assert_eq!(s.records(7)[0].blend, Some(0.25));
	for _ in 0..3 {
		s.step();
	}
	assert_eq!(s.records(7)[0].blend, Some(1.0));
	let mut r = op(72, OP_MIRROR_FROM, 1.0, 2);
	r.flags = FLAG_REMOVE;
	assert_eq!(s.set(7, 111, &[r]), Ok(1));
	s.step();
	s.step();
	s.step();
	assert!(s.is_empty());
}

#[test]
fn bad_ops_are_refused_without_effect() {
	let mut s = Store::default();
	assert_eq!(s.set(7, 111, &[op(111, OP_MIRROR_FROM, 1.0, 0)]), Err(ERR_BAD_JOINT));
	assert_eq!(s.set(7, 111, &[op(72, OP_MIRROR_FROM, 2.0, 0)]), Err(ERR_RANGE));
	let _t = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	assert_eq!(s.set(7, 111, &[op(72, OP_HOLD, 1.0, 0)]), Err(ERR_RANGE), "a hold naming a snapshot nobody made");
	assert!(s.is_empty());
}

// ---- the binding section ----

#[test]
fn the_binding_has_the_hero_skeleton() {
	let sk = Binding::parse(BINDING).unwrap().skeleton.expect("skeletons.hero");
	assert_eq!((sk.class, sk.joint_count, sk.chest, sk.chest_attach), (0, 111, 6, 11));
	assert_eq!(sk.pairs.len(), 42);
	assert_eq!(sk.centre.len(), 27);
	assert!(sk.names.iter().any(|(n, j)| n == "right_clavicle" && *j == 52));
	assert!(sk.names.iter().any(|(n, j)| n == "left_clavicle" && *j == 72));
}

// ---- the store ----

#[test]
fn replacing_an_op_keeps_its_weight_and_a_zero_blend_jumps() {
	let mut s = Store::default();
	s.set(7, 111, &[op(72, OP_MIRROR_FROM, 1.0, 4)]).unwrap();
	s.step();
	s.step();
	assert_eq!(s.records(7)[0].blend, Some(0.5));
	// renewed every tick: the weight carries on (0.5 -> 0.75), no pop back to 0
	s.set(7, 111, &[op(72, OP_MIRROR_FROM, 1.0, 4)]).unwrap();
	assert_eq!(s.records(7)[0].blend, Some(0.5));
	s.step();
	assert_eq!(s.records(7)[0].blend, Some(0.75));
	// blend 0: full weight at once
	s.set(7, 111, &[op(72, OP_MIRROR_FROM, 1.0, 0)]).unwrap();
	assert_eq!(s.records(7)[0].blend, Some(1.0));
	assert_eq!(s.len(), 1);
}

#[test]
fn a_duration_fades_the_op_out_by_itself() {
	let mut s = Store::default();
	let mut o = op(72, OP_MIRROR_FROM, 1.0, 2);
	o.duration_ticks = 5;
	s.set(7, 111, &[o]).unwrap();
	for _ in 0..4 {
		s.step();
	}
	assert_eq!(s.len(), 1);
	assert_eq!(s.records(7)[0].blend, Some(1.0));
	for _ in 0..4 {
		s.step();
	}
	assert!(s.is_empty());
}

#[test]
fn the_cap_is_64_ops_per_moby_and_a_refused_batch_changes_nothing() {
	let mut s = Store::default();
	let batch: Vec<OmniPoseOp> = (0..64).map(|j| op(j, OP_MIRROR_FROM, 1.0, 0)).collect();
	assert_eq!(s.set(7, 111, &batch), Ok(64));
	assert_eq!(s.set(7, 111, &[op(100, OP_MIRROR_FROM, 1.0, 0)]), Err(ERR_CAP));
	assert_eq!(s.len(), 64);
	// replacing an existing joint is not a new op
	assert_eq!(s.set(7, 111, &[op(3, OP_MIRROR_FROM, 0.5, 0)]), Ok(1));
	assert_eq!(s.len(), 64);
	// another moby has its own 64
	assert_eq!(s.set(8, 111, &[op(100, OP_MIRROR_FROM, 1.0, 0)]), Ok(1));
}

#[test]
fn n_zero_fades_every_op_of_the_moby_and_leaves_the_others() {
	let mut s = Store::default();
	s.set(7, 111, &[op(72, OP_MIRROR_FROM, 1.0, 0), op(73, OP_MIRROR_FROM, 1.0, 0)]).unwrap();
	s.set(8, 111, &[op(72, OP_MIRROR_FROM, 1.0, 0)]).unwrap();
	s.fade_moby(7, 3);
	s.step();
	assert!(s.records(7).iter().all(|r| r.blend.is_some_and(|w| w < 1.0 && w > 0.0)));
	assert_eq!(s.records(8)[0].blend, Some(1.0));
	for _ in 0..4 {
		s.step();
	}
	assert!(s.records(7).is_empty());
	assert_eq!(s.len(), 1);
	s.clear_all();
	assert!(s.is_empty());
}

#[test]
fn rotation_ops_make_set_and_offset_records_in_joint_order() {
	let mut s = Store::default();
	let h = std::f32::consts::FRAC_1_SQRT_2;
	let mut a = op(30, OP_SET_ROTATION, 1.0, 0);
	a.q = [0.0, h, 0.0, h];
	let mut b = op(20, OP_OFFSET_ROTATION, 1.0, 0);
	b.q = [0.0, 0.0, h, h];
	s.set(7, 111, &[a, b]).unwrap();
	let r = s.records(7);
	assert_eq!(r.len(), 2);
	assert_eq!(r[0].joint, 20);
	assert_eq!(r[1].joint, 30);
	let mut bad = op(30, OP_SET_ROTATION, 1.0, 0);
	bad.q = [0.0, 0.0, 0.0, 0.0];
	assert_eq!(s.set(7, 111, &[bad]), Err(ERR_RANGE));
}

// ---- the partner and parent tables ----

#[test]
fn the_binding_pairs_are_the_renderers_partner_table() {
	let sk = Binding::parse(BINDING).unwrap().skeleton.unwrap();
	let (parent, partner) = pose_host::tables(&[0u8; 0x100], 0x40, 111, &sk);
	assert_eq!((parent.len(), partner.len()), (111, 111));
	let path = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"));
	let Ok(mut disc) = crate::iso::Disc::open(&path) else {
		eprintln!("skipped: disc not found");
		return;
	};
	let levels = crate::toc::read_levels(&mut disc).unwrap();
	let level = crate::level::Level::load(&mut disc, &levels[0]).unwrap();
	let a = crate::anim::animation(&level, 0).unwrap().unwrap();
	let want: Vec<u16> = a.partner_table().iter().map(|p| p.map_or(0xffff, |j| j as u16)).collect();
	assert_eq!(partner, want);
}

// ---- the savestate image (Rilgar fg1/7_0284, Blaster in hand) ----

fn blaster_ram() -> Option<Vec<u8>> {
	let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../oracle/states/fg1/7_0284/eeMemory.bin");
	let alt = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../oracle/states/fg1/7_0284/eeMemory.bin");
	let r = std::fs::read(&p).or_else(|_| std::fs::read(&alt));
	match r {
		Ok(v) if v.len() >= 0x200_0000 => Some(v),
		_ => {
			eprintln!("skipped: savestate oracle/states/fg1/7_0284 not found");
			None
		}
	}
}

#[test]
fn the_hero_is_posable_on_a_savestate_and_point_11_gives_the_chest() {
	let Some(ram) = blaster_ram() else { return };
	let hero = u32_at(&ram, HERO_CELL) & MASK;
	assert_eq!(hero_joints(&ram, hero), Some(111));
	assert_eq!(hero_joints(&ram, hero + 0x80), None, "only the hero cell's moby");
	assert_eq!(hero_joints(&ram, 0), None);
	let class = u32_at(&ram, hero as usize + 0x24) & MASK;
	assert_eq!(pose_read::matrix_index(&ram, class, 56), Some(0), "the right hand is stored matrix 0");
	assert_eq!(pose_read::matrix_index(&ram, class, 6), None, "the chest has no stored matrix");
	assert!(!hero_dead(&ram));
	let sk = Binding::parse(BINDING).unwrap().skeleton.unwrap();
	let (parent, partner) = pose_host::tables(&ram, hero, 111, &sk);
	assert_eq!(parent[0], 0xffff);
	assert!(parent.iter().skip(1).filter(|&&p| p != 0xffff).count() >= 100, "the bone table gives parents");
	assert_eq!(partner[52], 72);
	assert_eq!(partner[72], 52);
	assert_eq!(partner[6], 6);
}

#[test]
fn the_auto_clear_rules_drop_ops() {
	let _t = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	let Some(mut ram) = blaster_ram() else { return };
	let hero = u32_at(&ram, HERO_CELL) & MASK;
	let put = |w| {
		with_store(|s| s.set(hero, 111, &[op(72, OP_MIRROR_FROM, w, 0)]).unwrap());
	};
	// (the store is a process static: this test is the only one that ticks it)
	clear_all();
	tick(&ram, 7, false, 0);
	put(1.0);
	tick(&ram, 7, false, 0);
	assert_eq!(snapshot_for(hero, true, false).len(), 1);
	assert!(snapshot_for(hero, true, true).is_empty(), "a scene draws the hero from scene frames");
	assert!(snapshot_for(hero, false, false).is_empty());
	tick(&ram, 7, false, 2);
	assert!(snapshot_for(hero, true, false).is_empty(), "cutscene clears");
	put(1.0);
	tick(&ram, 8, false, 0);
	assert!(snapshot_for(hero, true, false).is_empty(), "level change clears");
	put(1.0);
	tick(&ram, 8, true, 0);
	assert!(snapshot_for(hero, true, false).is_empty(), "level exit clears");
	put(1.0);
	ram[HERO_STATE..HERO_STATE + 4].copy_from_slice(&0x3du32.to_le_bytes());
	for _ in 0..8 {
		tick(&ram, 8, false, 0);
	}
	assert!(snapshot_for(hero, true, false).is_empty(), "death fades the ops out");
	put(1.0);
	clear_all();
	assert!(snapshot_for(hero, true, false).is_empty());
}

// ---- pose_read maths ----

#[test]
fn reflect_mirrors_across_the_chest_plane() {
	// the plane y = 10: normal (0, 1, 0) through (0, 10, 0); a frame at (3, 14, -2) with identity rows
	let (n, centre) = ([0.0f32, 1.0, 0.0], [0.0f32, 10.0, 0.0]);
	let m = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 3.0, 14.0, -2.0];
	let r = pose_read::reflect(&m, n, centre);
	assert_eq!(&r[9..12], &[3.0, 6.0, -2.0]);
	// x and z basis unchanged, the y basis flips
	assert_eq!(&r[0..3], &[1.0, 0.0, 0.0]);
	assert_eq!(&r[3..6], &[0.0, -1.0, 0.0]);
	assert_eq!(&r[6..9], &[0.0, 0.0, 1.0]);
	// reflecting twice is the identity
	assert_eq!(pose_read::reflect(&r, n, centre), m);
}
