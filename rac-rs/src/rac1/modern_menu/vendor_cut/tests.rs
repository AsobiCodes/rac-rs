//! Tests of the vendor screen cut (`vendor_cut.rs`): the screen joint set, which faces make a screen face on a synthetic model, the
//! collapse of a posed model, and the real class 11 of each disc present (every face on joints 0..36 is a screen face, whatever its
//! texture: frame, black plate, hub; the pedestal stays).
use super::*;
use crate::mobies::Skin;
use std::path::Path;

fn on(j: u8) -> Skin {
	Skin::single(j)
}

#[test]
fn the_screen_joints_are_the_six_blocks_of_six() {
	let all: Vec<u8> = (0..=255u8).filter(|&j| is_screen_joint(j)).collect();
	assert_eq!(all, (0..36).collect::<Vec<u8>>());
	assert!(!is_screen_joint(36) && !is_screen_joint(37) && !is_screen_joint(44));
}

#[test]
fn the_dominant_joint_is_the_heaviest_one_and_the_first_on_a_tie() {
	let s = Skin { joints: [9, 3, 7], weights: [0.25, 0.5, 0.25] };
	assert_eq!(dominant_joint(&s), 3);
	let tie = Skin { joints: [9, 3, 7], weights: [0.5, 0.5, 0.0] };
	assert_eq!(dominant_joint(&tie), 9);
	assert_eq!(dominant_joint(&on(5)), 5);
}

/// Vertices 0-3 on screen corner joints (frame texture 1), 4-7 on screen hub joints (body texture 0, the black plate and the hub),
/// 8-10 on the pedestal's joints (texture 2).
fn model() -> (Vec<(u32, [u32; 3])>, Vec<Skin>) {
	let faces = vec![(1, [0, 1, 2]), (1, [1, 2, 3]), (0, [4, 5, 6]), (0, [5, 6, 7]), (2, [8, 9, 10])];
	let skin = [2, 3, 4, 5, 1, 6, 7, 12, 37, 38, 39].map(on).to_vec();
	(faces, skin)
}

#[test]
fn every_face_on_screen_joints_is_cut_whatever_its_texture_and_the_pedestal_stays() {
	let (faces, skin) = model();
	let m = frame_mask(&faces, &skin, 11);
	assert_eq!(m, [true, true, true, true, true, true, true, true, false, false, false]);
}

#[test]
fn a_vertex_a_pedestal_face_also_uses_does_not_move() {
	let (mut faces, skin) = model();
	faces.push((2, [0, 8, 9])); // a pedestal face using screen vertex 0: moving it would bend the pedestal
	let m = frame_mask(&faces, &skin, 11);
	assert!(!m[0], "a vertex a non-screen face also uses is not moved");
	assert!(m[1] && m[2] && m[3] && m[4]);
	assert!(!m[8..].iter().any(|&b| b));
	// A face with one vertex off the screens is no screen face: its screen vertices stay unless another screen face uses them.
	let faces = vec![(1, [0, 1, 8])];
	assert!(frame_mask(&faces, &skin, 11).iter().all(|&b| !b));
}

#[test]
fn the_texture_id_does_not_matter_and_an_empty_model_marks_nothing() {
	let (_, skin) = model();
	let faces = vec![(0x0100_0001, [0, 1, 2]), (0x0200_0007, [1, 2, 3])];
	assert_eq!(frame_mask(&faces, &skin, 11).iter().filter(|&&b| b).count(), 4);
	assert!(frame_mask(&[], &[], 0).is_empty());
	assert_eq!(frame_mask(&[], &skin, 3), [false; 3]);
}

#[test]
fn collapse_moves_masked_vertices_onto_the_first_unmasked_one() {
	let ps: Vec<[f32; 3]> = (0..6).map(|i| [i as f32, 10.0 + i as f32, 20.0 + i as f32]).collect();
	let ns: Vec<[f32; 3]> = (0..6).map(|i| [0.0, 0.0, i as f32]).collect();
	let mask = [true, true, false, true, false, false];
	let (cp, cn) = collapse(&(ps.clone(), ns.clone()), &mask).expect("collapses");
	assert_eq!(cp.len(), ps.len());
	assert_eq!(cn, ns, "normals are kept");
	let anchor = ps[2];
	assert_eq!((cp[0], cp[1], cp[3]), (anchor, anchor, anchor));
	assert_eq!((cp[2], cp[4], cp[5]), (ps[2], ps[4], ps[5]), "unmasked vertices stay");
	// Metal vertices after the mesh stay (the pose is longer than the mask).
	let mut long = ps.clone();
	long.push([9.0; 3]);
	let mut long_n = ns.clone();
	long_n.push([0.0; 3]);
	let (cp, _) = collapse(&(long, long_n), &mask).unwrap();
	assert_eq!(cp[6], [9.0; 3]);
	// The bounds never grow: every moved vertex lies on a vertex that was there.
	assert!(cp.iter().all(|p| ps.contains(p) || *p == [9.0; 3]));
}

#[test]
fn collapse_does_nothing_without_work_or_for_a_foreign_pose() {
	let pose: Posed = (vec![[1.0; 3]; 4], vec![[0.0; 3]; 4]);
	assert!(collapse(&pose, &[false; 4]).is_none(), "no frame vertex");
	assert!(collapse(&pose, &[true; 4]).is_none(), "no vertex to collapse onto");
	assert!(collapse(&pose, &[true, false, true, false, true]).is_none(), "a pose shorter than the mask is another class's");
	assert!(collapse(&(vec![[1.0; 3]; 4], vec![[0.0; 3]; 3]), &[true, false, true, false]).is_none(), "normals shorter than the mask");
}

const PAL: [&str; 2] = [concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"), concat!(env!("CARGO_MANIFEST_DIR"), "/../../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso")];
const USA: [&str; 2] = [
	concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (USA) (En,Fr,De,Es,It)/Ratchet & Clank (USA) (En,Fr,De,Es,It).iso"),
	concat!(env!("CARGO_MANIFEST_DIR"), "/../../../../Ratchet & Clank (USA) (En,Fr,De,Es,It)/Ratchet & Clank (USA) (En,Fr,De,Es,It).iso"),
];

/// Class 11 of each disc present: the mask exists, holds screen-joint vertices only, and leaves the pedestal alone. On the PAL disc the
/// numbers are the measured ones: 1664 screen faces cut (552 frame, 1112 body: the black plates and the hubs), the pedestal's 464 faces
/// keep theirs. Without a disc the test returns early.
#[test]
fn class_11_on_each_disc_has_a_screen_mask_that_spares_the_pedestal() {
	for (name, isos) in [("PAL", PAL), ("USA", USA)] {
		let Some(iso) = isos.iter().find(|p| Path::new(p).is_file()) else { continue };
		let mut d = crate::iso::Disc::open(Path::new(iso)).unwrap();
		let levels = crate::toc::read_levels(&mut d).unwrap();
		for lv in [1usize, 3, 11] {
			let l = crate::level::Level::load(&mut d, &levels[lv]).unwrap();
			let class = crate::mobies::moby_classes(&l).unwrap().remove(&VENDOR_CLASS).unwrap_or_else(|| panic!("{name} level {lv}: no class 11"));
			let mask = frame_vertices(&class);
			assert_eq!(mask.len(), class.mesh.positions.len(), "{name} level {lv}");
			assert!(mask.iter().any(|&m| m), "{name} level {lv}: no screen vertex found");
			for (v, _) in mask.iter().enumerate().filter(|(_, m)| **m) {
				assert!(is_screen_joint(dominant_joint(&class.skin[v])), "{name} level {lv}: masked vertex {v} is off the screen joints");
			}
			let (mut cut, mut kept) = (0, 0);
			for (_, f) in &class.mesh.faces {
				let n = f.iter().filter(|&&v| mask[v as usize]).count();
				assert!(n == 0 || n == 3, "{name} level {lv}: a face with {n} of 3 vertices masked would be a bent triangle");
				if n == 3 {
					cut += 1;
				} else {
					kept += 1;
				}
			}
			assert!(cut > 0 && kept > 0, "{name} level {lv}: cut faces {cut}, kept faces {kept}");
			let pedestal = (0..class.skin.len()).filter(|&v| dominant_joint(&class.skin[v]) >= 36).all(|v| !mask[v]);
			assert!(pedestal, "{name} level {lv}: a pedestal vertex (joint 36 and up) is masked");
			if name == "PAL" {
				assert_eq!((class.mesh.positions.len(), class.mesh.faces.len()), (2327, 2128), "level {lv}");
				assert_eq!((cut, kept), (1664, 464), "level {lv}: 1664 screen faces cut (552 frame + 1112 body), the pedestal's 464 kept");
			}
			// A real pose: every masked vertex lands on one point, all others keep theirs.
			let pose: Posed = (class.mesh.positions.clone(), class.normals.clone());
			let (cp, _) = collapse(&pose, &mask).expect("collapses");
			let at = cp[mask.iter().position(|m| !*m).unwrap()];
			assert!(cp.iter().zip(&mask).all(|(p, &m)| if m { *p == at } else { true }));
		}
	}
}
