//! [PC] perf (lane P7): the per-class work of `Gpu::append_props`. A retail pool growth (a respawn spawns the level's mobys
//! again) appends hundreds of slots at once but only a dozen classes, and every slot of a class gets the same vertices,
//! radius and clippable faces (the faces differ only by the slot's first vertex). Each is built once per class (the vertex
//! builds in parallel) and copied to the slots, instead of once per slot: the same values, a fraction of the time.

use super::clip::PropFace;
use super::*;

/// The new slots' vertices in slot order, each slot's vertex range (counted from `v0`) and its bounding radius.
pub(super) struct NewSlots {
	pub(super) verts: Vec<Vertex>,
	pub(super) ranges: Vec<std::ops::Range<usize>>,
	pub(super) rad: Vec<f32>,
}

/// `push_mesh_vertices` for every slot's mesh (no baked colours), the radius `max |pos|` per slot. A mesh that several
/// slots share (same reference: `slot_meshes` hands out the class's mesh) is built once.
pub(super) fn new_slots(meshes: &[&Mesh], v0: usize) -> NewSlots {
	let mut first: HashMap<*const Mesh, usize> = HashMap::new();
	let mut distinct: Vec<&Mesh> = Vec::new();
	let which: Vec<usize> = meshes
		.iter()
		.map(|&m| {
			*first.entry(m as *const Mesh).or_insert_with(|| {
				distinct.push(m);
				distinct.len() - 1
			})
		})
		.collect();
	let built: Vec<(Vec<Vertex>, f32)> = distinct
		.par_iter()
		.map(|m| {
			let (mut v, mut lo, mut hi) = (Vec::new(), [f32::MAX; 3], [f32::MIN; 3]);
			push_mesh_vertices(m, 0, None, &mut v, &mut lo, &mut hi);
			let rad = v.iter().map(|v| (v.pos[0] * v.pos[0] + v.pos[1] * v.pos[1] + v.pos[2] * v.pos[2]).sqrt()).fold(0.0, f32::max);
			(v, rad)
		})
		.collect();
	let total: usize = which.iter().map(|&w| built[w].0.len()).sum();
	let (mut verts, mut ranges, mut rad) = (Vec::with_capacity(total), Vec::with_capacity(meshes.len()), Vec::with_capacity(meshes.len()));
	for &w in &which {
		let base = v0 + verts.len();
		verts.extend_from_slice(&built[w].0);
		ranges.push(base..v0 + verts.len());
		rad.push(built[w].1);
	}
	NewSlots { verts, ranges, rad }
}

/// Every face of every mesh uses a texture the scene has uploaded (slots of one class share their mesh: each is checked once).
pub(super) fn meshes_known(meshes: &[&Mesh], tex_slots: &HashMap<u32, usize>) -> bool {
	let mut seen: std::collections::HashSet<*const Mesh> = std::collections::HashSet::new();
	meshes.iter().filter(|&&m| seen.insert(m as *const Mesh)).all(|m| m.faces.iter().all(|&(t, _)| tex_slots.contains_key(&(t & crate::mobies::SLOT_MASK))))
}

/// A class's clippable faces as the faces of a slot whose vertices start at index 0 (`clip::replay_faces`).
pub(super) fn class_prop_faces(cf: &ClassFaces, tex_slots: &HashMap<u32, usize>) -> Vec<PropFace> {
	cf.clipped
		.iter()
		.map(|&(g, rank, idx, phase)| {
			let key = cf.groups[g].0;
			PropFace { slot: rank * 3, run: g as u32, idx, phase, tex: tex_slots.get(&(key & crate::mobies::SLOT_MASK)).copied().unwrap_or(0), cull: ((key >> 24) & 3) as u8 }
		})
		.collect()
}

/// `class_prop_faces`' faces for a slot whose first vertex is `base`.
pub(super) fn faces_at(base: u32, template: &[PropFace]) -> Vec<PropFace> {
	template.iter().map(|f| PropFace { idx: f.idx.map(|i| i + base), ..*f }).collect()
}

/// The faces of the slots `at..at + m` (classes `all_classes`, vertex ranges `all_ranges`): one template per class, one copy per slot.
pub(super) fn new_slot_faces(class_faces: &HashMap<u32, ClassFaces>, tex_slots: &HashMap<u32, usize>, all_classes: &[u32], all_ranges: &[std::ops::Range<usize>], at: usize, m: usize) -> Vec<Vec<PropFace>> {
	let mut template: HashMap<u32, Vec<PropFace>> = HashMap::new();
	for &c in &all_classes[at..at + m] {
		template.entry(c).or_insert_with(|| class_prop_faces(&class_faces[&c], tex_slots));
	}
	(at..at + m).into_par_iter().map(|s| faces_at(all_ranges[s].start as u32, &template[&all_classes[s]])).collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	fn mesh(seed: u32, n: usize) -> Mesh {
		let positions: Vec<[f32; 3]> = (0..n).map(|i| [(i as u32 * 7 + seed) as f32 * 0.37, (i as u32 * 3 + seed) as f32 * -0.21, (i as u32 + seed * 5) as f32 * 0.5]).collect();
		let uvs = (0..n).map(|i| [i as f32 * 0.1, seed as f32 * 0.01]).collect();
		let faces = (0..n.saturating_sub(2)).map(|i| (seed % 3, [i as u32, i as u32 + 1, i as u32 + 2])).collect();
		Mesh { name: String::new(), positions, uvs, faces, colours: Vec::new(), lit: None, strip_phase: Vec::new(), normals: Vec::new() }
	}

	/// The per-slot loops of `append_props` before lane P7, as the reference.
	fn per_slot(meshes: &[&Mesh], v0: usize) -> NewSlots {
		let (mut verts, mut ranges) = (Vec::new(), Vec::new());
		let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
		for (mi, m) in meshes.iter().enumerate() {
			let base = v0 + verts.len();
			push_mesh_vertices(m, mi, None, &mut verts, &mut lo, &mut hi);
			ranges.push(base..v0 + verts.len());
		}
		let rad = ranges.iter().map(|r| verts[r.start - v0..r.end - v0].iter().map(|v| (v.pos[0] * v.pos[0] + v.pos[1] * v.pos[1] + v.pos[2] * v.pos[2]).sqrt()).fold(0.0, f32::max)).collect();
		NewSlots { verts, ranges, rad }
	}

	#[test]
	fn new_slots_match_the_per_slot_build() {
		let (a, b, c, empty) = (mesh(1, 9), mesh(2, 14), mesh(3, 5), Mesh::new(String::new()));
		let meshes: Vec<&Mesh> = vec![&a, &b, &a, &empty, &c, &b, &b, &a, &empty, &c];
		let (fast, slow) = (new_slots(&meshes, 1234), per_slot(&meshes, 1234));
		assert_eq!(fast.ranges, slow.ranges);
		assert_eq!(fast.rad, slow.rad);
		assert_eq!(bytemuck::cast_slice::<Vertex, u8>(&fast.verts), bytemuck::cast_slice::<Vertex, u8>(&slow.verts));
		let none = new_slots(&[], 7);
		assert!(none.verts.is_empty() && none.ranges.is_empty() && none.rad.is_empty());
	}

	#[test]
	fn slot_faces_match_the_per_slot_build() {
		let groups = vec![(5u32, vec![0, 1, 2, 3, 4, 5]), (2 | (1 << 24), vec![1, 2, 3])];
		let clipped = vec![(0usize, 0u32, [0u32, 1, 2], 0u8), (1, 0, [1, 2, 3], 1), (0, 1, [3, 4, 5], 0)];
		let (mut cf, mut tex_slots) = (HashMap::new(), HashMap::new());
		cf.insert(7u32, ClassFaces { groups: groups.clone(), clipped: clipped.clone() });
		cf.insert(8u32, ClassFaces { groups, clipped: Vec::new() });
		tex_slots.insert(5 & crate::mobies::SLOT_MASK, 3usize);
		let all_classes = [9u32, 9, 7, 8, 7, 7, 8];
		let all_ranges: Vec<std::ops::Range<usize>> = (0..7).map(|s| s * 100..s * 100 + 6).collect();
		let got = new_slot_faces(&cf, &tex_slots, &all_classes, &all_ranges, 2, 5);
		assert_eq!(got.len(), 5);
		for (k, faces) in got.iter().enumerate() {
			let s = 2 + k;
			let cfc = &cf[&all_classes[s]];
			let base = all_ranges[s].start as u32;
			let want: Vec<PropFace> = cfc
				.clipped
				.iter()
				.map(|&(g, rank, idx, phase)| {
					let key = cfc.groups[g].0;
					PropFace { slot: rank * 3, run: g as u32, idx: idx.map(|i| i + base), phase, tex: tex_slots.get(&(key & crate::mobies::SLOT_MASK)).copied().unwrap_or(0), cull: ((key >> 24) & 3) as u8 }
				})
				.collect();
			assert_eq!(format!("{faces:?}"), format!("{want:?}"));
		}
	}
}
