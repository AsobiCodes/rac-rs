//! R&C2 placed ties: the same `LitTie` the shared decoders give R&C1, without baked lights (engine split stage E5).
//!
//! R&C2 ties are coloured by the game's own per-instance colour stream (the gameplay +0x94 blob, expanded by the tie chain; not
//! decoded here [?]). So every vertex starts at the neutral colour 128 and the pack writes the game's colours into `LitTie::cols`:
//! `slots[lod][v]` names the entry of `cols` that vertex `v` of that LOD takes, one entry per vertex of the packet's own stream, so
//! the pack can lay the game's stream of each LOD out in `cols` in order (LOD 0 at 0, LOD 1 at `total_0`, LOD 2 at `total_0 +
//! total_1`; a LOD that does not decode shares LOD 0's entries and adds none).

use crate::bytes::{f32_at, i32_at, span, u32_at, R};
use crate::gamelayout::{layout, Game};
use crate::gameplay::DirLight;
use crate::level::Level;
use crate::mesh::Mesh;
use crate::ties::{instance_sphere, placement, tie_mesh_full, transformed, LitTie, TieCull, TieSlot};
use std::collections::HashMap;

/// A class decoded once for all its placements.
struct ClassLods {
	meshes: [Mesh; 3],
	morph: [Vec<[f32; 3]>; 3],
	/// Per LOD, each vertex's colour slot (already offset by the LOD's base in `cols`).
	slots: [Vec<TieSlot>; 3],
	/// Length of `cols`: the sum of the colour streams of the LODs that decoded.
	cols: usize,
	/// Bounding sphere of the LOD-0 mesh in class units (bbox centre, farthest vertex) [I: R&C2 headers have no class sphere at R&C1's offsets].
	centre: [f32; 3],
	radius: f32,
	/// LOD distances (class f32 +0x10, +0x14, +0x18; the R&C1 offsets, 12 / 25 / 40 on level 0).
	lod: [f32; 3],
}

fn bounding_sphere(m: &Mesh) -> ([f32; 3], f32) {
	if m.positions.is_empty() {
		return ([0.0; 3], 0.0);
	}
	let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
	for p in &m.positions {
		for k in 0..3 {
			lo[k] = lo[k].min(p[k]);
			hi[k] = hi[k].max(p[k]);
		}
	}
	let c: [f32; 3] = std::array::from_fn(|k| (lo[k] + hi[k]) * 0.5);
	let r2 = m.positions.iter().map(|p| (0..3).map(|k| (p[k] - c[k]).powi(2)).sum::<f32>()).fold(0.0, f32::max);
	(c, r2.sqrt())
}

fn decode_class(data: &[u8], remap: &[u8], class_id: u32) -> R<ClassLods> {
	let name = format!("tie_{class_id}");
	let mut meshes: Vec<Mesh> = Vec::new();
	let mut morph: Vec<Vec<[f32; 3]>> = Vec::new();
	let mut slots: Vec<Vec<TieSlot>> = Vec::new();
	let mut cols = 0usize;
	for k in 0..3 {
		let (mut stream, mut total) = (Vec::new(), 0u32);
		match tie_mesh_full(Game::Rc2, data, remap, name.clone(), k, &mut stream, &mut total) {
			Ok((m, _, d)) if !m.positions.is_empty() => {
				slots.push(stream.iter().map(|&i| ([cols + i as usize; 3], false)).collect());
				meshes.push(m);
				morph.push(d);
				cols += total as usize;
			}
			Err(e) if k == 0 => return Err(format!("tie class {class_id} lod 0: {e}")),
			Ok(_) if k == 0 => return Err(format!("tie class {class_id} lod 0: empty")),
			// A LOD that fails or is empty draws LOD 0 and shares its colours.
			_ => {
				meshes.push(meshes[0].clone());
				morph.push(morph[0].clone());
				slots.push(slots[0].clone());
			}
		}
	}
	let (centre, radius) = bounding_sphere(&meshes[0]);
	let lod = [f32_at(data, 0x10)?, f32_at(data, 0x14)?, f32_at(data, 0x18)?];
	let three = || String::from("tie class: three LODs expected");
	Ok(ClassLods { meshes: meshes.try_into().map_err(|_| three())?, morph: morph.try_into().map_err(|_| three())?, slots: slots.try_into().map_err(|_| three())?, cols, centre, radius, lod })
}

fn lit_tie(c: &ClassLods, mat: &[f32; 16], i: usize, dist: f32, occ: (u8, u8)) -> LitTie {
	let sphere = instance_sphere(mat, c.centre, c.radius, 1.0);
	let build = |k: usize| -> (Mesh, Vec<[f32; 3]>, Vec<TieSlot>) {
		let mut m = transformed(&c.meshes[k], mat, format!("tie_instance_{i}"));
		m.colours = vec![[128.0; 4]; m.positions.len()];
		// The delta is a direction: the matrix's linear part only.
		let morph = c.morph[k].iter().map(|d| std::array::from_fn(|r| mat[r] * d[0] + mat[4 + r] * d[1] + mat[8 + r] * d[2])).collect();
		(m, morph, c.slots[k].clone())
	};
	let ((m0, d0, s0), (m1, d1, s1), (m2, d2, s2)) = (build(0), build(1), build(2));
	LitTie {
		lods: [m0, m1, m2],
		morph: [d0, d1, d2],
		slots: [s0, s1, s2],
		cols: vec![[128.0; 4]; c.cols],
		cull: TieCull { centre: [sphere[0], sphere[1], sphere[2]], radius: sphere[3], dist, lod: c.lod, occ },
	}
}

/// One entry per tie record in file order (the order of `gameplay::tie_placements`): the placed tie, or None for a record whose class
/// the level does not have. The pack indexes the game's per-instance colour streams by this order.
pub fn lit_placements(level: &Level, _lights: &[DirLight]) -> R<Vec<Option<LitTie>>> {
	let mut classes: HashMap<u32, ClassLods> = HashMap::new();
	for e in level.table(0x20, 32)? {
		let (start, class_id) = (u32_at(e, 0)? as usize, u32_at(e, 4)?);
		if start == 0 || classes.contains_key(&class_id) {
			return Err(format!("duplicate or empty tie class {class_id}"));
		}
		classes.insert(class_id, decode_class(level.block(start)?, &e[16..32], class_id)?);
	}
	let g = &level.gameplay;
	let start = u32_at(g, 0x34)? as usize;
	if start == 0 {
		return Ok(Vec::new());
	}
	let count = u32_at(g, start)? as usize;
	let size = layout(Game::Rc2).tie_record;
	let records = span(g, start + 16, count * size)?;
	// Occlusion ids: record u32 +0xc; no mapping block leaves every tie unmapped (they pass the all-ones mask).
	let ids = records.chunks_exact(size).map(|r| u32_at(r, 0xc)).collect::<R<Vec<u32>>>()?;
	let occ = level.occlusion_map().map_or_else(|| vec![crate::occlusion::UNMAPPED; ids.len()], |m| crate::occlusion::tie_bytes(&m, &ids));
	records
		.chunks_exact(size)
		.enumerate()
		.map(|(i, r)| match classes.get(&u32_at(r, 0)?) {
			Some(c) => Ok(Some(lit_tie(c, &placement(r, "tie instance")?, i, i32_at(r, 4)? as f32, occ[i]))),
			None => Ok(None),
		})
		.collect()
}

/// `ties::lit_instances` for an R&C2 level: the placed ties of `lit_placements` without the records of unknown classes.
pub fn lit_instances(level: &Level, lights: &[DirLight]) -> R<Vec<LitTie>> {
	Ok(lit_placements(level, lights)?.into_iter().flatten().collect())
}
