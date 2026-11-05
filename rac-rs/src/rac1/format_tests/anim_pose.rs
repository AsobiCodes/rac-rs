//! Disc tests of formats/anim_pose.rs (engine split stage E2, 2026-10-26): they read the R&C1 disc through `toc`, so they live in
//! the R&C1 pack; moved unchanged from the formats file's test modules.
#![allow(unused_imports)]
mod tests {
	use crate::anim_pose::*;
	use crate::anim::{animation, AnimClock, LayerPose};
	use crate::level::Level;
	use crate::anim::Animated;
	use crate::toc;

	/// The 42 mirror pairs of the hero (class 0, 111 joints), POSE_OVERRIDE.md section 1.1; every other joint is on the centre line.
	const PAIRS: [(usize, usize); 42] = [
		(16, 17), (19, 20), (21, 23), (22, 24), (25, 29), (26, 30), (27, 31), (28, 32), (33, 35), (34, 36), (39, 40), (41, 44), (42, 45),
		(43, 46), (47, 48), (49, 50), (52, 72), (53, 73), (54, 74), (55, 75), (56, 76), (57, 77), (58, 78), (59, 79), (60, 80), (61, 81),
		(62, 82), (63, 83), (64, 84), (65, 85), (66, 86), (67, 87), (68, 88), (69, 89), (70, 90), (71, 91), (92, 98), (93, 99), (94, 100),
		(95, 101), (96, 102), (97, 103),
	];

	fn hero() -> Animated {
		let path = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"));
		let mut disc = crate::iso::Disc::open(&path).unwrap();
		let levels = crate::toc::read_levels(&mut disc).unwrap();
		let level = Level::load(&mut disc, &levels[0]).unwrap();
		animation(&level, 0).unwrap().unwrap()
	}

	/// Skin matrices of sequence `seq` at `frame`, under the Blaster hold layer (sequence 44, the right-arm layer the
	/// POSE_OVERRIDE.md proof states carry) when `layer` is set.
	fn posed(a: &Animated, seq: usize, frame: usize, layer: bool, fx: &[JointFx]) -> Vec<[f32; 12]> {
		let clock = AnimClock::new(&a.sequences[seq], seq as u8, frame);
		let l = LayerPose { seq: 44, frame: 11, from_seq: 44, from_frame: 11, t: 0.0, weight: 1.0 };
		a.pose_held(&clock, None, fx, layer.then_some(&l), None, &[None, None])
	}

	fn bits(m: &[[f32; 12]]) -> Vec<[u32; 12]> {
		m.iter().map(|r| r.map(f32::to_bits)).collect()
	}

	fn pos(a: &Animated, m: &[[f32; 12]], j: usize) -> [f32; 3] {
		let w = a.joint_world(m, j);
		[w[3], w[7], w[11]]
	}

	/// `p` reflected across the body plane (model space y = 0).
	fn reflect(p: [f32; 3]) -> [f32; 3] {
		[p[0], -p[1], p[2]]
	}

	// The partner table is the bind-symmetry pairing of POSE_OVERRIDE.md: 42 pairs, 27 centre joints, nothing unpaired.
	#[test]
	fn veldin_partner_table_has_the_42_pairs() {
		let a = hero();
		let t = a.partner_table();
		assert_eq!(t.len(), 111);
		let mut want: Vec<Option<usize>> = (0..111).map(Some).collect();
		for (x, y) in PAIRS {
			want[x] = Some(y);
			want[y] = Some(x);
		}
		assert_eq!(t, want);
		assert_eq!(t.iter().enumerate().filter(|(j, p)| **p == Some(*j)).count(), 27);
	}

	// mirror_from 52..71 -> 72..91 puts every left arm joint (the hand 76 too) at its right arm joint reflected across the body plane,
	// to 1e-3 game units, in several poses with and without the Blaster hold layer (firing poses turn the chest: the reflection is
	// still across the body, not the chest); the right arm and every joint outside the left arm are untouched.
	#[test]
	fn veldin_mirror_puts_left_hand_at_the_reflected_right_hand() {
		let a = hero();
		let fx: Vec<JointFx> = (52..72).map(|j| JointFx::mirror_from(j + 20, j, 1.0)).collect();
		for (seq, frame, layer) in [(0, 0, false), (0, 0, true), (3, 5, true), (4, 7, false), (48, 1, true), (64, 3, true)] {
			let base = posed(&a, seq, frame, layer, &[]);
			let m = posed(&a, seq, frame, layer, &fx);
			assert_eq!(m.len(), 111);
			for j in 52..72 {
				// Right arm: the mirror leaves it alone.
				assert_eq!(bits(&m[j..j + 1]), bits(&base[j..j + 1]), "seq {seq} right joint {j}");
				// Left arm: the reflected right arm, joint by joint.
				let (r, l) = (pos(&a, &m, j), pos(&a, &m, j + 20));
				let want = reflect(r);
				for i in 0..3 {
					assert!((want[i] - l[i]).abs() < 1e-3, "seq {seq} layer {layer} joint {} {:?} want {:?}", j + 20, l, want);
				}
			}
			for j in (0..72).chain(92..111) {
				assert_eq!(bits(&m[j..j + 1]), bits(&base[j..j + 1]), "seq {seq} joint {j} outside the left arm changed");
			}
			// The pose really moved (the animation's own left arm is not already the mirror).
			assert_ne!(bits(&m[72..92]), bits(&base[72..92]), "seq {seq}");
		}
	}

	// No ops (or no effective ops) = bit-identical matrices: an empty list, a weight-0 mirror, and a mirror on a joint
	// outside the skeleton all give the very same bits as the plain pose; the list order of a record and a mirror op that
	// reads the record's joint does not matter (mirrors run after the records).
	#[test]
	fn veldin_no_effective_ops_means_identical_matrices() {
		let a = hero();
		for (seq, frame, layer) in [(0, 0, false), (3, 5, true)] {
			let base = bits(&posed(&a, seq, frame, layer, &[]));
			let zero: Vec<JointFx> = (52..72).map(|j| JointFx::mirror_from(j + 20, j, 0.0)).collect();
			assert_eq!(bits(&posed(&a, seq, frame, layer, &zero)), base);
			assert_eq!(bits(&posed(&a, seq, frame, layer, &[JointFx::mirror_from(500, 52, 1.0), JointFx::mirror_from(72, 500, 1.0)])), base);
		}
		let rec = JointFx::rot(56, [0.0, 0.3826834, 0.0, 0.9238795]);
		let mir = JointFx::mirror_from(76, 56, 1.0);
		let (ab, ba) = (posed(&a, 3, 5, true, &[rec, mir]), posed(&a, 3, 5, true, &[mir, rec]));
		assert_eq!(bits(&ab), bits(&ba));
		// The record moved joint 56, and joint 76 followed it through the mirror.
		let plain = posed(&a, 3, 5, true, &[mir]);
		assert_ne!(bits(&ab[56..57]), bits(&plain[56..57]));
		assert_ne!(bits(&ab[76..77]), bits(&plain[76..77]));
	}

	// A half-weight mirror lands between the animation's own pose and the full mirror.
	#[test]
	fn veldin_half_weight_mirror_is_between() {
		let a = hero();
		let full: Vec<JointFx> = (52..72).map(|j| JointFx::mirror_from(j + 20, j, 1.0)).collect();
		let half: Vec<JointFx> = (52..72).map(|j| JointFx::mirror_from(j + 20, j, 0.5)).collect();
		let (b, f, h) = (posed(&a, 3, 5, true, &[]), posed(&a, 3, 5, true, &full), posed(&a, 3, 5, true, &half));
		let d = |x: [f32; 3], y: [f32; 3]| (0..3).map(|i| (x[i] - y[i]).powi(2)).sum::<f32>().sqrt();
		let (pb, pf, ph) = (pos(&a, &b, 76), pos(&a, &f, 76), pos(&a, &h, 76));
		assert!(d(pb, pf) > 0.05, "base and full mirror coincide: {}", d(pb, pf));
		assert!(d(ph, pb) > 0.005 && d(ph, pf) > 0.005 && d(ph, pb) < d(pb, pf) && d(ph, pf) < d(pb, pf), "{pb:?} {ph:?} {pf:?}");
	}
}
