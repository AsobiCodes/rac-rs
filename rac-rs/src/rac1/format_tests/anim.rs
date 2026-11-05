//! Disc tests of formats/anim.rs (engine split stage E2, 2026-10-26): they read the R&C1 disc through `toc`, so they live in
//! the R&C1 pack; moved unchanged from the formats file's test modules.
#![allow(unused_imports)]
mod tests {
	use crate::anim::*;
	use crate::mobies::moby_classes;
	use crate::{iso, toc};
	use std::path::PathBuf;
	use crate::bytes::f32_at;
	use crate::bytes::span;
	use crate::bytes::u16_at;
	use crate::bytes::u32_at;
	use crate::bytes::R;
	use crate::level::Level;
	use crate::mobies::MobyClass;
	use crate::anim_pose::JointFx;
	use crate::toc::menu_clips;
	use crate::toc::MENU_CLIPS;

	fn frame(rotation: Vec<[f32; 4]>) -> Frame {
		Frame { speed: 1.0, number: 0.0, rotation, scale: Vec::new(), post_scale: Vec::new(), translation: Vec::new() }
	}

	/// UI-3b test 1, 2, 4: the boots' attach points 2 / 3 are hero joints 99 / 93, the spawn scale 0x3c23d70a is
	/// 0.01, and owned extra items 35 / 33 resolve to classes 407 / 614.
	#[test]
	fn worn_attach_points_and_classes() {
		let mut disc = crate::iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = crate::toc::read_levels(&mut disc).unwrap();
		let l = crate::level::Level::load(&mut disc, &levels[0]).unwrap();
		assert_eq!((attach_joint(&l, 0, 2), attach_joint(&l, 0, 3)), (Some(99), Some(93)));
		assert_eq!(f32::from_bits(0x3c23d70a), 0.01f32);
		assert_eq!((crate::inventory::preview_class(35), crate::inventory::preview_class(33)), (Some(407), Some(614)));
	}

	#[test]
	fn menu_clips_load() {
		let (mut disc, _) = open();
		let c = menu_clips(&mut disc, 111).unwrap();
		assert_eq!(c.len(), MENU_CLIPS);
		assert_eq!((c[0].frames.len(), c[3].frames.len(), c[27].frames.len()), (146, 253, 72));
		assert_eq!((c[0].speed, c[3].speed), (0.5, 0.0));
	}

	fn open() -> (iso::Disc, Vec<toc::Level>) {
		let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"));
		let mut disc = iso::Disc::open(&path).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		(disc, levels)
	}

	fn bbox(p: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
		let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
		for v in p {
			for i in 0..3 {
				lo[i] = lo[i].min(v[i]);
				hi[i] = hi[i].max(v[i]);
			}
		}
		(lo, hi)
	}

	/// Mean |posed edge length - bind edge length| / bbox height over all faces.
	fn distortion(class: &MobyClass, posed: &[[f32; 3]]) -> f32 {
		let d = |a: [f32; 3], b: [f32; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
		let (mut sum, mut n) = (0.0, 0);
		for (_, f) in &class.mesh.faces {
			for e in 0..3 {
				let (a, b) = (f[e] as usize, f[(e + 1) % 3] as usize);
				sum += (d(posed[a], posed[b]) - d(class.mesh.positions[a], class.mesh.positions[b])).abs();
				n += 1;
			}
		}
		let (lo, hi) = bbox(&class.mesh.positions);
		sum / n as f32 / (hi[2] - lo[2])
	}

	// AUD-11d test 1: Novalis hero sequence triggers (cs, time) and no hero loop byte.
	#[test]
	fn novalis_hero_triggers() {
		let (mut disc, levels) = open();
		let level = Level::load(&mut disc, &levels[1]).unwrap();
		let anim = animation(&level, 0).unwrap().unwrap();
		let trig = |n: usize| -> Vec<(u32, u32)> { anim.sequences[n].triggers.iter().map(|w| (w & 0xffff, w >> 16)).collect() };
		assert_eq!(trig(7), vec![(26, 8), (30, 160), (31, 208)]);
		assert_eq!(trig(16), vec![(33, 24), (39, 64), (31, 112)]);
		assert!(anim.sequences.iter().all(|s| s.loop_sound == 0xff));
	}

	// HELD-GLOVE (decomp/notes/bomb_glove.md): the Veldin gloves (192 Bomb, 190 Mine, 229 Doom) posed by the hero hand copy
	// at hero joint 54 put every glove joint k (1..16) on hero joint 54 + k: at idle the glove wrist (joint 2) is the hero
	// wrist 56 (-0.0112, -0.3130, 0.5244), also under a weapon layer and in seq 48 where the glove's own idle sequence 1
	// (the old draw) left the wrist 0.4 units off the hand.
	#[test]
	fn veldin_glove_hand_copy_follows_the_hand() {
		let (mut disc, levels) = open();
		let level = Level::load(&mut disc, &levels[0]).unwrap();
		let hero = animation(&level, 0).unwrap().unwrap();
		assert!(!hero.layer_joints.is_empty());
		// FUN_0020d018: only the gloves; the Blaster, Pyrocitor, Suck Cannon and Taunter keep their own clocks.
		let copied: Vec<u8> = (0..crate::inventory::GADGETS as u8).filter(|&id| crate::inventory::hand_pose_copy(id)).collect();
		assert_eq!(copied, [10, 17, 20, 25]);
		let world =|m: &[f32; 12], v: [f32; 3]| [0, 1, 2].map(|r| m[r * 4] * v[0] + m[r * 4 + 1] * v[1] + m[r * 4 + 2] * v[2] + m[r * 4 + 3]);
		let dist = |a: [f32; 3], b: [f32; 3]| (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt();
		let layer = LayerPose { seq: 44, frame: 11, from_seq: 44, from_frame: 11, t: 0.0, weight: 1.0 };
		for class in [192, 190, 229] {
			let glove = gadget_animation(&level, class).unwrap().unwrap();
			assert_eq!(glove.skeleton.parents.len(), HAND_COPY.len(), "class {class}");
			for (seq, frame, l) in [(0, 0, None), (48, 1, None), (0, 0, Some(&layer))] {
				let mut pa = PropAnim::new(seq);
				pa.attach(Some(share_timing(&hero.sequences)));
				pa.set(seq, frame);
				let mats = pa.pose_held(&hero, &[], l, None, &[None, None]);
				let gm = hand_copy_pose(&glove, &pa.local_held(&hero, &[], l, None, &[None, None]));
				let j54 = hero.joint_world(&mats, 54);
				let at = |k: usize| {
					let g = glove.joint_world(&gm, k);
					world(&j54, [g[3], g[7], g[11]])
				};
				for k in 1..HAND_COPY.len() {
					let h = hero.joint_world(&mats, 54 + k);
					assert!(dist(at(k), [h[3], h[7], h[11]]) < 1e-4, "class {class} seq {seq} joint {k}: {:?} vs {h:?}", at(k));
				}
				if (seq, l.is_none()) == (0, true) {
					assert!(dist(at(2), [-0.011197273, -0.31303102, 0.5243815]) < 1e-4, "{:?}", at(2));
				}
				if seq == 48 {
					let own = glove.joint_world(&glove.pose_frame(1, 0), 2);
					let h = hero.joint_world(&mats, 56);
					assert!(dist(world(&j54, [own[3], own[7], own[11]]), [h[3], h[7], h[11]]) > 0.3);
				}
			}
		}
	}

	// HERO-24 test 1: the hold sets (classes 1 / 2) load on Novalis and map onto the hero layer joints 12 / 13.
	#[test]
	fn novalis_hold_sets() {
		let (mut disc, levels) = open();
		let level = Level::load(&mut disc, &levels[1]).unwrap();
		let hero = animation(&level, 0).unwrap().unwrap();
		let n = hero.skeleton.parents.len();
		for (class, joints) in [(1, &hero.layer_joints), (2, &hero.layer_joints13)] {
			let a = animation(&level, class).unwrap().unwrap();
			assert_eq!((a.skeleton.parents.len(), a.sequences.len()), (20, 23), "class {class}");
			let h = hold_sequences(&level, class, n).unwrap();
			assert_eq!(h.len(), 23);
			assert!(h.iter().flat_map(|s| &s.frames).all(|f| f.rotation.len() == n));
			let f = &h[0].frames[0];
			assert!(!joints.is_empty());
			assert!(joints.iter().all(|&j| f.rotation[j] != [0.0; 4]), "class {class}");
		}
	}

	// HERO-20: the hero (class 0) and Clank (class 601) attach-slot maps of Novalis from the class data.
	#[test]
	fn novalis_hero_clank_slot_maps() {
		let (mut disc, levels) = open();
		let level = Level::load(&mut disc, &levels[1]).unwrap();
		let hero = animation(&level, 0).unwrap().unwrap();
		let want = [56, 55, 99, 93, 7, 5, 54, 98, 92, 0, 2, 6, 52, 72, 76, 23, 21, 24, 22, 50, 49, 18, 100, 94, 8, 104, 106, 108, 110, 3, 5];
		assert_eq!(hero.slot_map[..31], want.map(Some));
		assert_eq!(hero.slot_map[31], None);
		let clank = animation(&level, 601).unwrap().unwrap();
		// The four Clank blink records register on slots 2..5 (CLANK_JOINTS).
		assert_eq!(clank.slot_map[2..6], [Some(15), Some(17), Some(21), Some(23)]);
	}

	// NOV-35a test 2: the Novalis purchase prompt (class 1137) attach points 0 / 1 / 2 are joints 2 / 1 / 4.
	#[test]
	fn novalis_prompt_attach_points() {
		let (mut disc, levels) = open();
		let level = Level::load(&mut disc, &levels[1]).unwrap();
		let a = animation(&level, 1137).unwrap().unwrap();
		assert_eq!(a.attach0, Some(2));
		assert_eq!(a.attach12, [Some(1), Some(4)]);
		let w = a.joint_world(&a.pose_frame(1, 0), 4);
		for (got, want) in [w[3], w[7], w[11]].iter().zip([0.4607226, -0.41478366, 0.9498151]) {
			assert!((got - want).abs() < 1e-4, "{w:?}");
		}
	}

	// M3b: the shop icon / popup bolt (Eudora class 13) at seq 0 frame 0 vs its stored (bind) vertices.
	#[test]
	fn eudora_bolt_pose_vs_bind() {
		let (mut disc, levels) = open();
		let level = Level::load(&mut disc, &levels[4]).unwrap();
		let classes = moby_classes(&level).unwrap();
		let c = &classes[&13];
		let a = animation(&level, 13).unwrap().unwrap();
		let m = a.pose_frame(0, 0);
		// One joint, seq 0 has one frame and it is the bind pose (identity, translation < 1e-4): the shop icon / popup bolt
		// is the stored mesh, so its shape rides only on the slot matrix [C].
		assert_eq!((a.skeleton.parents.len(), a.sequences[0].frames.len(), c.mesh.positions.len()), (1, 1, 84));
		let id = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
		assert!(m[0].iter().zip(id).all(|(x, y)| (x - y).abs() < 1e-4), "{:?}", m[0]);
	}

	// Eudora class 186 (Glove of Doom bot), seq 0 frame 3 -> 4 at p = 1/6, no fade: retail's skin matrices S22 / S30
	// (columns, raw-unit translation) read from scratch 0x70000000 + j*0x40 at the return of the Eudora skin build
	// 0x2438a8 (interpreted e06_17 field 0, moby 0x1bd8f00) [C]. The same-sequence blend is unnormalised, so these leg
	// joints (frame-to-frame dot 0.07 / 0.44) shrink; the normalised blend is 0.3 off.
	#[test]
	fn eudora_doom_bot_same_seq_blend_unnormalised() {
		let (mut disc, levels) = open();
		let level = Level::load(&mut disc, &levels[4]).unwrap();
		let a = animation(&level, 186).unwrap().unwrap();
		let mut clock = AnimClock::new(&a.sequences[0], 0, 3);
		(clock.cur, clock.next, clock.old_seq, clock.new_seq, clock.p) = (3, 4, 0, 0, 1.0 / 6.0);
		assert!(clock.direct_blend());
		let m = a.pose_clock(&clock, None);
		let u = a.skeleton.unit;
		let retail: [(usize, [f32; 12]); 2] = [
			(22, [0.35086, -0.74263, -0.06451, -0.00712, 0.15316, -0.71372, 0.93759, 0.17134, 0.16590, -817.57257, 270.27353, 1063.25427]),
			(30, [0.66986, 0.06211, -0.51579, 0.04783, 1.02461, 0.13885, 0.55950, -0.14775, 0.60672, 441.55328, 98.64288, -9.95908]),
		];
		for (j, r) in retail {
			let s = &m[j];
			for k in 0..3 {
				for row in 0..3 {
					assert!((s[row * 4 + k] - r[k * 3 + row]).abs() < 2e-3, "S{j} col {k} row {row}: {} vs {}", s[row * 4 + k], r[k * 3 + row]);
				}
			}
			for row in 0..3 {
				assert!((s[row * 4 + 3] / u - r[9 + row]).abs() < 2.0, "S{j} t{row}: {} vs {}", s[row * 4 + 3] / u, r[9 + row]);
			}
		}
		// The flag: wrap to frame 0, a fade (old 0xff) or another old sequence clear it; a hold (next == cur) keeps it.
		for (cur, next, old, new, want) in [(4, 0, 0, 0, false), (3, 4, 0xff, 0, false), (3, 4, 1, 0, false), (3, 3, 0, 0, true), (2, 4, 0, 0, false)] {
			(clock.cur, clock.next, clock.old_seq, clock.new_seq) = (cur, next, old, new);
			assert_eq!(clock.direct_blend(), want, "{cur} {next} {old} {new}");
		}
		// Seq 7 (moby 0x1bdf200, 35-joint LOD record): scale records with flags 0 scale the joint's world matrix after the
		// chain (`post_scale_list`). F3 (7,0)->(7,1) p 0.5 direct: frame 1 re-appends 22, so 23 and 33 lose theirs, 26
		// (frame 1 only) blends from 1; F1 (7,3)->(7,0) p 0.5 normalised. Retail 0x2438a8 output, e06_17 [C].
		let check = |m: &[[f32; 12]], j: usize, r: [f32; 12]| {
			for k in 0..3 {
				for row in 0..3 {
					assert!((m[j][row * 4 + k] - r[k * 3 + row]).abs() < 2e-3, "S{j} col {k} row {row}: {} vs {}", m[j][row * 4 + k], r[k * 3 + row]);
				}
			}
			for row in 0..3 {
				assert!((m[j][row * 4 + 3] / u - r[9 + row]).abs() < 2.0, "S{j} t{row}: {} vs {}", m[j][row * 4 + 3] / u, r[9 + row]);
			}
		};
		let mut c7 = AnimClock::new(&a.sequences[7], 7, 0);
		(c7.cur, c7.next, c7.old_seq, c7.new_seq, c7.p) = (0, 1, 7, 7, 0.5);
		assert!(c7.direct_blend());
		let m = a.pose_clock(&c7, None);
		check(&m, 23, [0.60311, -0.00999, -0.66534, 0.03859, 0.99892, 0.01818, 0.72381, -0.03719, 0.60564, -1237.67493, 67.42755, -65.37485]);
		check(&m, 26, [0.56477, 0.03508, -0.66796, 0.01238, 0.99752, 0.04894, 0.73598, -0.03596, 0.57585, -1384.89209, 108.11121, -19.54456]);
		check(&m, 31, [0.75566, -0.00084, 0.38407, 0.00892, 0.99905, 0.00190, -0.44870, -0.01902, 0.87736, 737.49744, -9.77148, 118.93004]);
		check(&m, 33, [0.60408, 0.01443, -0.54783, 0.00892, 0.99905, 0.00190, 0.54734, -0.00632, 0.60197, 761.04291, -6.41254, -60.80092]);
		(c7.cur, c7.next) = (3, 0);
		assert!(!c7.direct_blend());
		let m = a.pose_clock(&c7, None);
		check(&m, 22, [0.45819, -0.05661, 0.87659, 0.07924, 0.99659, 0.02284, -1.14350, 0.07713, 0.60192, 1504.40845, 64.49805, 242.41187]);
		check(&m, 25, [0.69245, -0.00870, 0.72950, 0.05187, 0.99798, -0.03585, -0.72650, 0.06264, 0.68314, 1202.01172, 108.12280, -37.25961]);
	}

	// SCN-4.3c test 3: Novalis ship class 531 has 1 / 1 / 2 frames in sequences 0..2, and sequence 1's frame runs at speed 0.
	#[test]
	fn novalis_ship_flight_sequences() {
		let (mut disc, levels) = open();
		let level = Level::load(&mut disc, &levels[1]).unwrap();
		let a = animation(&level, 531).unwrap().unwrap();
		let n: Vec<usize> = a.sequences.iter().take(3).map(|s| s.frames.len()).collect();
		assert_eq!(n, vec![1, 1, 2]);
		assert_eq!(a.sequences[1].frames[0].speed.to_bits(), 0);
	}

	#[test]
	fn ratchet() {
		let (mut disc, levels) = open();
		let level = Level::load(&mut disc, &levels[0]).unwrap();
		let classes = moby_classes(&level).unwrap();
		let r = &classes[&0];
		// CLASS-tint test 2: class +0x40 and the 48-vertex tail packet.
		assert_eq!(r.glow, 0x808c8c8c);
		assert_eq!(r.colour_start, r.mesh.positions.len() - 48);
		let anim = animation(&level, 0).unwrap().unwrap();
		assert_eq!(anim.skeleton.parents.len(), 111);
		assert_eq!(anim.sequences.len(), 134);

		let (pos, nrm) = skin(r, &anim.pose_frame(0, 0));
		assert_eq!(pos.len(), nrm.len());
		let (lo, hi) = bbox(&pos);
		println!("seq 0 frame 0 bbox {lo:?} .. {hi:?}");
		let height = hi[2] - lo[2];
		assert!((height - 1.41).abs() < 0.05, "height {height}");
		assert!(lo[2].abs() < 0.06, "feet {}", lo[2]);

		for (seq, frac) in [(0, 0.5), (3, 0.5), (4, 0.3), (64, 0.5)] {
			let s = &anim.sequences[seq];
			let (pos, _) = skin(r, &anim.pose_after(seq, s.tick_len() * frac));
			let dist = distortion(r, &pos);
			println!("seq {seq} mid-pose edge distortion {dist:.4}");
			assert!(dist < 0.03, "seq {seq} distortion {dist}");
		}

		println!("seq frames speed  dur(s)  root(joint1 local) x-range y-range z-range  triggers");
		for (i, s) in anim.sequences.iter().enumerate() {
			if s.frames.is_empty() {
				continue;
			}
			let mut lo = [f32::MAX; 3];
			let mut hi = [f32::MIN; 3];
			for f in &s.frames {
				let t = f.translation.iter().find(|x| x.0 == 1).map_or(anim.skeleton.bind_translation[1], |x| x.1);
				for k in 0..3 {
					lo[k] = lo[k].min(t[k]);
					hi[k] = hi[k].max(t[k]);
				}
			}
			println!(
				"{i:3} {:3} {:.4} {:6.2}  x {:7.0}..{:7.0}  y {:7.0}..{:7.0}  z {:7.0}..{:7.0}  trig {}",
				s.frames.len(), s.speed, s.tick_len() / 50.0, lo[0], hi[0], lo[1], hi[1], lo[2], hi[2], s.triggers.len()
			);
		}
	}
}
