//! Hero attach and pose helpers: back objects (Clank, jetpack), the Clank light, held gadget rigs (Held, PyroRig),
//! wrench / glove joint constants, the prop pose key; with the back-object tests.

use super::*;

/// BRIDGE-21: hero back objects (classes 607 / 601) and the hero attach point they follow.
pub(super) const BACK_D0_CLASS: u32 = 607;
pub(super) const BACK_D4_CLASS: u32 = 601;
pub(super) const BACK_ATTACH_POINT: usize = 5;
/// BRIDGE-21c: the Clank light class, its d4 joint, class-scale factor, and the glow (half size, camera bias, colour; z lift).
pub(super) const CLANK_LIGHT_CLASS: u32 = 1204;
pub(super) const CLANK_LIGHT_JOINT: usize = 6;
pub(super) const CLANK_LIGHT_SCALE: f32 = 1.3;
pub(super) const CLANK_LIGHT_GLOW: (f32, f32, u32) = (0.057, 0.08, 0x280000c0);
pub(super) const CLANK_LIGHT_GLOW_Z: f32 = 0.015;

/// Vertices after the suck cannon in the hero mesh: d0, d4 and the Clank light.
pub(super) fn back_tail(back: &Option<Held>, clank: &Option<Held>, light: &Option<mobies::MobyClass>) -> usize {
	rig_len(back) + rig_len(clank) + light.as_ref().map_or(0, |c| c.mesh.positions.len())
}
/// BRIDGE-40 (WPN-6.6a): the three whole-hero afterimage copies, last in the hero mesh tail.
pub(super) fn hero_ghost_len(hero: &mobies::MobyClass) -> usize {
	3 * hero.mesh.positions.len()
}
/// Vertices of a back rig's mesh (0 when absent): its slot in the hero mesh tail.
pub(super) fn rig_len(r: &Option<Held>) -> usize {
	r.as_ref().map_or(0, |h| h.class.mesh.positions.len())
}

/// Vertices of every generic held rig mesh (their slot in the hero mesh, after the Suck Cannon).
pub(super) fn held_len(rigs: &HashMap<u32, PyroRig>, hook: &Option<Held>) -> usize {
	game::held::rig_classes().into_iter().filter_map(|c| rigs.get(&c)).map(|r| r.held.class.mesh.positions.len()).sum::<usize>() + hook.as_ref().map_or(0, |h| h.class.mesh.positions.len())
}

/// A held gadget model posed by its own skeleton.
pub(super) struct Held {
	pub(super) class: mobies::MobyClass,
	pub(super) anim: anim::Animated,
}

/// The Pyrocitor (gadget class 176) at the hero joint of its attach point, its own joint for attach point 0 (the
/// muzzle) and the class 179 nozzle in bind pose (mesh, positions, normals). decomp/notes/weapons_framework.md
/// "Pyrocitor".
pub(super) struct PyroRig {
	pub(super) held: Held,
	pub(super) joint: usize,
	pub(super) muzzle: Option<usize>,
	pub(super) nozzle: Option<(Mesh, Vec<[f32; 3]>, Vec<[f32; 3]>)>,
}

/// Hidden appended vertices (a held model not in hand): all collapsed, so their triangles vanish.
/// BRIDGE-21: append one back object (`n` vertices) through the shared back matrix; collapsed when there is none
/// (`clank_hidden`, no joint). `posed` is its skinned (positions, normals).
pub(super) fn append_back_object(ps: &mut Vec<[f32; 3]>, ns: &mut Vec<[f32; 3]>, n: usize, posed: (Vec<[f32; 3]>, Vec<[f32; 3]>), bj: Option<&[f32; 12]>) {
	let Some(bj) = bj else { return hide(ps, ns, n) };
	let (mut gp, mut gn) = posed;
	gp.resize(n, [0.0; 3]);
	gn.resize(n, [0.0, 0.0, 1.0]);
	ps.extend(gp.iter().map(|&v| apply(bj, v, 1.0)));
	ns.extend(gn.iter().map(|&v| apply(bj, v, 0.0)));
}

pub(super) fn hide(ps: &mut Vec<[f32; 3]>, ns: &mut Vec<[f32; 3]>, n: usize) {
	ps.extend(std::iter::repeat([0.0; 3]).take(n));
	ns.extend(std::iter::repeat([0.0, 0.0, 1.0]).take(n));
}

/// Skinned model-space positions and normals.
pub(super) type Posed = (Vec<[f32; 3]>, Vec<[f32; 3]>);

/// Ratchet's sequences by hero state (SetState, decomp/notes/veldin_enemies.md section 5).
pub(super) const IDLE: usize = 0;

pub(super) const WRIST: usize = 56;
pub(super) const WRENCH_HELD: usize = 1;
/// The Bomb Glove model, the hero joint it rides (54, not the wrench's wrist 56), its draw sequence
/// (0, then 1 idle) and the glove joint holding the bomb with the bomb's offset in it.
pub(super) const GLOVE_CLASS: u32 = 192;
pub(super) const GLOVE_JOINT: usize = 54;
pub(super) const GLOVE_BOMB_JOINT: usize = 2;
pub(super) const GLOVE_BOMB_OFFSET: [f32; 3] = [0.0, -0.0856, -0.0199];

/// Animated props pose by (class, clock state), so props in step share one skinning (props fading from a snapshot are
/// posed on their own, handler.rs).
pub(super) type PoseKey = (u32, usize, u8, i32, i32, u32);
pub(super) fn pose_key(p: &Prop) -> PoseKey {
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
	fn back_pulse_is_not_clamped() {
		let m = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
		let o = crate::player::pulse_item(m, 1.3, false, [0.0; 3]);
		assert!((o[0] - 1.3).abs() < 1e-6 && (o[5] - 1.3).abs() < 1e-6);
		assert_eq!(crate::player::pulse_item(m, 1.3, true, [0.0; 3])[0], 1.0);
	}
}
