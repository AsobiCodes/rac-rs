//! `set_props`' debris half (moved out of props_set.rs, lane nprops): every live piece instanced into the per-frame debris batch.

use super::*;

/// Debris: every live piece is instanced into the per-frame debris batch (the moby pool bounds the count).
/// `view`: the clip view's (eye, forward) for the draw-distance fade; `cull`: Culling On.
#[allow(clippy::too_many_arguments)]
pub(super) fn build(scene: &mut Scene, queue: &wgpu::Queue, table: &[Option<dynlight::SlotLight>; 8], moby_dyn: &mut Vec<([f32; 3], [f32; 3])>, cull: bool, view: Option<([f32; 3], [f32; 3])>, debris: &[particles::Debris], insts: &[particles::WorldInst]) {
	let Some(batch) = &mut scene.debris else { return };
	let (mut dv, mut by_tex): (Vec<Vertex>, BTreeMap<usize, Vec<u32>>) = (Vec::new(), BTreeMap::new());
	let mut di = 0;
	// NOV-10.5a-draw: game-owned instances (teleporter rings) ride the same batch.
	for (class, pos, mm, sc, draw) in debris.iter().map(|l| (l.class, l.pos, l.m, l.scale, Some(l.draw))).chain(insts.iter().map(|i| (i.class, i.pos, i.m, i.scale, None))) {
		let Some((model, faces, phases)) = scene.debris_model.get(&class) else { continue };
		// SCH-6b: past the piece's draw distance it is not drawn (the opaque batch carries no fade).
		if cull && effect_fade(view, draw, pos).is_none() {
			continue;
		}
		let n_idx: usize = faces.iter().map(|f| f.1.len()).sum();
		if dv.len() + model.len() > scene.debris_cap.0 || di + n_idx > scene.debris_cap.1 {
			break;
		}
		di += n_idx;
		let base = dv.len() as u32;
		// The crate sink fade (alpha -4 per tick) is not drawn: the batch is opaque, and the 0.9 per tick
		// shrink hides the piece first [P].
		let (c, p) = (mm, pos);
		let m = [c[0][0], c[0][1], c[0][2], 0.0, c[1][0], c[1][1], c[1][2], 0.0, c[2][0], c[2][1], c[2][2], 0.0, p[0], p[1], p[2], 1.0];
		let light = Gpu::moby_light(None);
		let id = dyn_id(table, moby_dyn, p);
		for v in model {
			let mut v = *v;
			v.colour[3] = id;
			v.light = light;
			let (a, n) = (v.pos.map(|c| c * sc), v.normal);
			for j in 0..3 {
				v.pos[j] = m[j] * a[0] + m[4 + j] * a[1] + m[8 + j] * a[2] + m[12 + j];
				v.normal[j] = m[j] * n[0] + m[4 + j] * n[1] + m[8 + j] * n[2];
			}
			dv.push(v);
		}
		for (k, (t, idx)) in faces.iter().enumerate() {
			for (fi, f) in idx.chunks_exact(3).enumerate() {
				let phase = phases[k].1[fi];
				let tri = [f[0], f[1], f[2]].map(|i| dv[(i + base) as usize]);
				// NOV-5.9d path 1: a face the retail clipper takes leaves the batch; its polygon goes to the overlay.
				if let Some(poly) = scene.clip_view.filter(|_| phase != crate::mesh::NO_PHASE).and_then(|view| super::super::clip::clip_face(&view, tri, phase, *t, 0)) {
					scene.clip_queue.push(poly);
				} else {
					by_tex.entry(*t).or_default().extend(f.iter().map(|i| i + base));
				}
			}
		}
	}
	let (mut indices, mut draws) = (Vec::new(), Vec::new());
	for (texture, idx) in by_tex {
		draws.push(Draw { inst: 0, cull: 0, texture, start: indices.len() as u32, count: idx.len() as u32 });
		indices.extend(idx);
	}
	if !dv.is_empty() {
		queue.write_buffer(&batch.vbuf, 0, bytemuck::cast_slice(&dv));
		queue.write_buffer(&batch.ibuf, 0, bytemuck::cast_slice(&indices));
	}
	batch.draws = draws;
}
