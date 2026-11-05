//! Hero attach and pose helpers: back objects (Clank, jetpack), the Clank light, held gadget rigs (Held, PyroRig),
//! wrench / glove joint constants, the prop pose key; with the back-object tests.

use super::*;

/// BRIDGE-21: hero back objects (classes 607 / 601) and the hero attach point they follow.
pub const BACK_D0_CLASS: u32 = 607;
pub const BACK_D4_CLASS: u32 = 601;
pub const BACK_ATTACH_POINT: usize = 5;
/// BRIDGE-21c: the Clank light class, its d4 joint, class-scale factor, and the glow (half size, camera bias, colour; z lift).
pub const CLANK_LIGHT_CLASS: u32 = 1204;
pub const CLANK_LIGHT_JOINT: usize = 6;
pub const CLANK_LIGHT_SCALE: f32 = 1.3;
pub const CLANK_LIGHT_GLOW: (f32, f32, u32) = (0.057, 0.08, 0x280000c0);
pub const CLANK_LIGHT_GLOW_Z: f32 = 0.015;

/// Vertices after the suck cannon in the hero mesh: d0, d4 and the Clank light.
pub fn back_tail(back: &Option<Held>, clank: &Option<Held>, light: &Option<mobies::MobyClass>) -> usize {
	rig_len(back) + rig_len(clank) + light.as_ref().map_or(0, |c| c.mesh.positions.len())
}
/// BRIDGE-40 (WPN-6.6a): the three whole-hero afterimage copies, last in the hero mesh tail.
pub fn hero_ghost_len(hero: &mobies::MobyClass) -> usize {
	3 * hero.mesh.positions.len()
}
/// Vertices of a back rig's mesh (0 when absent): its slot in the hero mesh tail.
pub(crate) fn rig_len(r: &Option<Held>) -> usize {
	r.as_ref().map_or(0, |h| h.class.mesh.positions.len())
}

/// A held gadget model posed by its own skeleton.
pub struct Held {
	pub class: mobies::MobyClass,
	pub anim: anim::Animated,
}

/// The Pyrocitor (gadget class 176) at the hero joint of its attach point, its own joint for attach point 0 (the
/// muzzle) and the class 179 nozzle in bind pose (mesh, positions, normals). decomp/notes/weapons_framework.md
/// "Pyrocitor".
pub struct PyroRig {
	pub held: Held,
	pub joint: usize,
	pub muzzle: Option<usize>,
	pub nozzle: Option<(Mesh, Vec<[f32; 3]>, Vec<[f32; 3]>)>,
}

/// Hidden appended vertices (a held model not in hand): all collapsed, so their triangles vanish.
/// BRIDGE-21: append one back object (`n` vertices) through the shared back matrix; collapsed when there is none
/// (`clank_hidden`, no joint). `posed` is its skinned (positions, normals).
pub fn append_back_object(ps: &mut Vec<[f32; 3]>, ns: &mut Vec<[f32; 3]>, n: usize, posed: (Vec<[f32; 3]>, Vec<[f32; 3]>), bj: Option<&[f32; 12]>) {
	let Some(bj) = bj else { return hide(ps, ns, n) };
	let (mut gp, mut gn) = posed;
	gp.resize(n, [0.0; 3]);
	gn.resize(n, [0.0, 0.0, 1.0]);
	ps.extend(gp.iter().map(|&v| apply(bj, v, 1.0)));
	ns.extend(gn.iter().map(|&v| apply(bj, v, 0.0)));
}

pub fn hide(ps: &mut Vec<[f32; 3]>, ns: &mut Vec<[f32; 3]>, n: usize) {
	ps.extend(std::iter::repeat([0.0; 3]).take(n));
	ns.extend(std::iter::repeat([0.0, 0.0, 1.0]).take(n));
}

/// Retail hides only the hero moby while he is hidden (flag +0x34 bit 0: the first-person look with L1, shops); a wrench he threw is a moby of
/// its own and keeps flying in view. The marker model carries the loose wrenches as extra vertex ranges, so while the hero is hidden every
/// vertex outside `keep` (`[a, b)` ranges) collapses the way `hide` does. Returns false and leaves the pose alone when `keep` holds
/// no vertex: nothing of the model stays visible then and the marker is dropped instead (frame_upload.rs).
pub fn collapse_outside(ps: &mut [[f32; 3]], ns: &mut [[f32; 3]], keep: &[super::loose_range::LooseRange]) -> bool {
	if keep.iter().all(|r| r.a >= r.b) {
		return false;
	}
	for (i, (p, n)) in ps.iter_mut().zip(ns.iter_mut()).enumerate() {
		if !keep.iter().any(|r| i >= r.a && i < r.b) {
			*p = [0.0; 3];
			*n = [0.0, 0.0, 1.0];
		}
	}
	true
}

/// Skinned model-space positions and normals.
pub type Posed = (Vec<[f32; 3]>, Vec<[f32; 3]>);

/// Ratchet's sequences by hero state (SetState, decomp/notes/veldin_enemies.md section 5).
pub const IDLE: usize = 0;

pub const WRIST: usize = 56;
pub const WRENCH_HELD: usize = 1;
/// The Bomb Glove model, the hero joint it rides (54, not the wrench's wrist 56), its draw sequence
/// (0, then 1 idle) and the glove joint holding the bomb with the bomb's offset in it.
pub const GLOVE_CLASS: u32 = 192;
pub const GLOVE_JOINT: usize = 54;
pub const GLOVE_BOMB_JOINT: usize = 2;
pub const GLOVE_BOMB_OFFSET: [f32; 3] = [0.0, -0.0856, -0.0199];

/// Animated props pose by (class, clock state), so props in step share one skinning (props fading from a snapshot are
/// posed on their own, handler.rs).
pub type PoseKey = (u32, usize, u8, i32, i32, u32);
pub fn pose_key(p: &Prop) -> PoseKey {
	(p.class_id, p.anim.seq, p.anim.clock.old_seq, p.anim.clock.cur, p.anim.clock.next, p.anim.clock.p.to_bits())
}

#[cfg(test)]
mod back_object_tests {
	use super::*;

	#[test]
	fn back_object_uses_shared_matrix_and_hides() {
		let bj = norm_cols([2.0, 0.0, 0.0, 1.0, 0.0, 2.0, 0.0, 2.0, 0.0, 0.0, 2.0, 3.0]);
		let posed = (vec![[1.0, 2.0, 3.0]], vec![[0.0, 0.0, 1.0]]);
		let (mut ps, mut ns) = (Vec::new(), Vec::new());
		append_back_object(&mut ps, &mut ns, 1, posed.clone(), Some(&bj));
		assert_eq!(ps[0], apply(&bj, [1.0, 2.0, 3.0], 1.0));
		append_back_object(&mut ps, &mut ns, 1, posed, None);
		assert_eq!(ps[1], [0.0; 3]);
		assert_eq!(ps.len(), 2);
	}

	#[test]
	fn clank_light_glow_constants_and_hide() {
		assert_eq!(CLANK_LIGHT_GLOW, (0.057, 0.08, 0x280000c0));
		assert_eq!(CLANK_LIGHT_GLOW_Z, 0.015);
		// Hidden (no back matrix): the light range collapses like the back objects.
		let (mut ps, mut ns) = (Vec::new(), Vec::new());
		append_back_object(&mut ps, &mut ns, 3, (vec![[1.0; 3]; 3], vec![[0.0; 3]; 3]), None);
		assert!(ps.iter().all(|p| *p == [0.0; 3]) && ps.len() == 3);
	}

	#[test]
	fn collapse_outside_keeps_only_the_loose_ranges() {
		let mut ps: Vec<[f32; 3]> = (0..10).map(|i| [i as f32 + 1.0, 2.0, 3.0]).collect();
		let mut ns = vec![[1.0, 0.0, 0.0]; 10];
		let before = ps.clone();
		let r = |a, b| super::super::loose_range::LooseRange { a, b, pos: [0.0; 3], light: None };
		// Nothing to keep: the pose is untouched and the caller drops the marker.
		assert!(!collapse_outside(&mut ps, &mut ns, &[]));
		assert!(!collapse_outside(&mut ps, &mut ns, &[r(4, 4)]));
		assert_eq!(ps, before);
		// A thrown wrench at 3..5 and an afterimage at 7..9 stay; the hero (0..3), the gap and the tail collapse like `hide`.
		assert!(collapse_outside(&mut ps, &mut ns, &[r(3, 5), r(7, 9)]));
		for i in 0..10 {
			if (3..5).contains(&i) || (7..9).contains(&i) {
				assert_eq!((ps[i], ns[i]), (before[i], [1.0, 0.0, 0.0]), "vertex {i} kept");
			} else {
				assert_eq!((ps[i], ns[i]), ([0.0; 3], [0.0, 0.0, 1.0]), "vertex {i} collapsed");
			}
		}
		// A range past the end is harmless.
		let (mut ps, mut ns) = (vec![[1.0; 3]; 2], vec![[0.0; 3]; 2]);
		assert!(collapse_outside(&mut ps, &mut ns, &[r(1, 99)]));
		assert_eq!((ps[0], ps[1]), ([0.0; 3], [1.0; 3]));
	}
}
