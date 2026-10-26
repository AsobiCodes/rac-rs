//! Prop slot layout: class faces, draw layout, slot allocation, and growing the props batch (append / regrow).

use super::*;

impl Gpu {
	pub(super) fn prop_slots(&self, props: &[u32], prop_meshes: &HashMap<u32, Mesh>, textures: &HashMap<u32, Texture>, bind: &mut Vec<wgpu::BindGroup>, slots: &mut HashMap<u32, usize>) -> PropSlots {
		self.ctx().prop_slots(props, prop_meshes, textures, bind, slots)
	}

	/// [PC] Retail pool growth (retail.rs `sync` / `take_grow`): rebuilds only the loaded scene's prop slots for the new
	/// slot order, with the textures the scene already uploaded, so the level, its audio and the other batches stay as
	/// they are. The slots come out as `load_scene` would build them for `props`. False (scene untouched) when there is
	/// no scene or a slot needs a texture the scene never uploaded; the caller then reloads the level.
	pub(in crate::viewer) fn regrow_props(&mut self, props: &[u32], prop_meshes: &HashMap<u32, Mesh>) -> bool {
		let Some(scene) = self.scene.as_ref() else { return false };
		let empty = Mesh::new(String::new());
		let known = slot_meshes(props, prop_meshes, &empty).iter().all(|m| m.faces.iter().all(|&(t, _)| scene.tex_slots.contains_key(&(t & crate::mobies::SLOT_MASK))));
		let had_prompt = scene.prompt.is_some();
		if !known || (!had_prompt && props.contains(&game::vendor::PROMPT_CLASS) && prop_meshes.get(&game::vendor::PROMPT_CLASS).is_some_and(|m| !m.positions.is_empty())) {
			return false;
		}
		let mut slots = std::mem::take(&mut self.scene.as_mut().unwrap().tex_slots);
		let mut bind = Vec::new();
		let p = self.prop_slots(props, prop_meshes, &HashMap::new(), &mut bind, &mut slots);
		debug_assert!(bind.is_empty());
		let scene = self.scene.as_mut().unwrap();
		scene.tex_slots = slots;
		scene.props = p.batch;
		scene.props_ibuf_spare = None;
		scene.props_model = p.model;
		scene.props_sj = p.sj;
		scene.props_rad = p.rad;
		scene.props_inst = Vec::new();
		scene.props_pose = Vec::new();
		scene.props_ranges = p.ranges;
		scene.props_shown = false;
		scene.props_alpha = false;
		scene.props_drawn = Vec::new();
		scene.props_vis = Vec::new();
		scene.slot_runs = p.slot_runs;
		scene.props_order = (0..p.classes.len()).collect();
		scene.props_class = p.classes;
		scene.class_faces.extend(p.class_faces);
		scene.props_late = Vec::new();
		scene.prop_faces = p.faces;
		scene.clip_masked = Vec::new();
		scene.clip_view = None;
		scene.clip_queue = Vec::new();
		scene.prompt_slot = p.prompt_slot.filter(|_| scene.prompt.is_some());
		true
	}

	/// [PC] Retail pool growth without a rebuild: one new prop slot per class in `classes`, inserted at slot `at` (after the
	/// game's props, before the scene actors); `order` is the new slot order the faces are drawn in (a rebuilt scene's:
	/// each class's new slots after its old ones). The existing slots keep their vertices; the new slots' vertices go into
	/// the buffer's spare room after them, and the indices, draws, `slot_runs` and `prop_faces` come out as `load_scene`
	/// builds them for that slot order. False (scene untouched) when there is no scene, a
	/// slot needs a texture the scene never uploaded, or a class is the purchase prompt; the caller then rebuilds the slots.
	/// Short of room the prop buffers grow first (GPU copy of the old slots).
	/// [PC] perf (lane A7): when `order` is the old order with the new slots put in (`grow_plan`, the usual case) the old
	/// indices are not rebuilt: a GPU copy moves them into the other index buffer around the new slots' faces, which are
	/// the only indices written; the old slots' runs only move and their faces (run-relative) stay (unit tests: `grow_tests`).
	pub(in crate::viewer) fn append_props(&mut self, at: usize, classes: &[u32], prop_meshes: &HashMap<u32, Mesh>, order: &[usize]) -> bool {
		let (device, queue, Some(scene)) = (&self.device, &self.queue, self.scene.as_mut()) else { return false };
		let _s = super::profile::span("ap_head");
		let empty = Mesh::new(String::new());
		let meshes = slot_meshes(classes, prop_meshes, &empty);
		let (n0, m) = (scene.props_ranges.len(), classes.len());
		let n = n0 + m;
		if at > n0 || scene.props_class.len() != n0 || order.len() != n || classes.contains(&game::vendor::PROMPT_CLASS) || !meshes.iter().all(|m| m.faces.iter().all(|&(t, _)| scene.tex_slots.contains_key(&(t & crate::mobies::SLOT_MASK)))) {
			return false;
		}
		let mut all_classes = scene.props_class.clone();
		all_classes.splice(at..at, classes.iter().copied());
		let v0 = scene.props_model.len();
		let (mut verts, mut ranges) = (Vec::new(), Vec::new());
		let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
		for (mi, m) in meshes.iter().enumerate() {
			let base = v0 + verts.len();
			push_mesh_vertices(m, mi, None, &mut verts, &mut lo, &mut hi);
			ranges.push(base..v0 + verts.len());
		}
		for (&c, &mesh) in classes.iter().zip(&meshes) {
			scene.class_faces.entry(c).or_insert_with(|| ClassFaces::of(mesh));
		}
		let mut all_ranges = scene.props_ranges.clone();
		all_ranges.splice(at..at, ranges.iter().cloned());
		let mut plan = {
			let _p = super::profile::span("ap_plan");
			grow_plan(&GrowOld::of(scene), at, classes, &ranges, order)
		};
		let n_idx = match &plan {
			Some(p) => p.n_idx,
			None => slot_meshes(&all_classes, prop_meshes, &empty).iter().map(|m| m.faces.len() * 3).sum(),
		};
		// Room short: larger buffers (a 16th spare again), the old slots' vertices and (slot, vertex) attributes moved over by
		// a GPU copy; the indices are written whole below (rebuild) or copied over (`plan`).
		let vsz = std::mem::size_of::<Vertex>();
		let need = v0 + verts.len();
		let vcap = (scene.props.vbuf.size() as usize / vsz).min(scene.props_sj.as_ref().map_or(usize::MAX, |b| b.size() as usize / 8));
		let mut enc = None;
		if need > vcap || (plan.is_none() && n_idx > scene.props.ibuf.size() as usize / 4) {
			let _g = super::profile::span("ap_grow");
			let enc = enc.get_or_insert_with(|| device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("props grow") }));
			if need > vcap {
				let cap = need + (need / 16).max(1 << 16);
				let vbuf = sized_buffer(device, "vertices", cap * vsz, wgpu::BufferUsages::VERTEX);
				enc.copy_buffer_to_buffer(&scene.props.vbuf, 0, &vbuf, 0, (v0 * vsz) as u64);
				scene.props.vbuf = vbuf;
				if let Some(old) = scene.props_sj.as_ref() {
					let sj = sized_buffer(device, "props slot attribute", cap * 8, wgpu::BufferUsages::VERTEX);
					enc.copy_buffer_to_buffer(old, 0, &sj, 0, (v0 * 8) as u64);
					scene.props_sj = Some(sj);
				}
			}
			if plan.is_none() && n_idx > scene.props.ibuf.size() as usize / 4 {
				scene.props.ibuf = sized_buffer(device, "indices", (n_idx + (n_idx / 16).max(3 << 16)) * 4, wgpu::BufferUsages::INDEX);
				scene.props_ibuf_spare = None;
			}
		}
		drop(_s);
		let inserted = plan.is_some();
		let (prop_faces, clip_masked) = match plan.as_mut() {
			Some(plan) => {
				let _s = super::profile::span("ap_insert");
				// The old indices around the new slots' faces into the spare index buffer (as large as the current one; larger,
				// a 16th spare again, when the indices outgrow it).
				let size = scene.props.ibuf.size() as usize;
				let ibuf = match scene.props_ibuf_spare.take() {
					Some(b) if b.size() as usize >= size.max(n_idx * 4) => b,
					_ => sized_buffer(device, "indices", if n_idx * 4 > size { (n_idx + (n_idx / 16).max(3 << 16)) * 4 } else { size }, wgpu::BufferUsages::INDEX),
				};
				let e = enc.get_or_insert_with(|| device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("props grow") }));
				for &(src, dst, len) in &plan.copies {
					e.copy_buffer_to_buffer(&scene.props.ibuf, src as u64 * 4, &ibuf, dst as u64 * 4, len as u64 * 4);
				}
				{
					let _c = super::profile::span("ap_submit");
					queue.submit(enc.take().map(|e| e.finish()));
				}
				for (dst, r) in &plan.chunks {
					queue.write_buffer(&ibuf, *dst as u64 * 4, bytemuck::cast_slice(&plan.fresh[r.clone()]));
				}
				scene.props_ibuf_spare = Some(std::mem::replace(&mut scene.props.ibuf, ibuf));
				// The old runs only move (and change draw numbers when a texture key is new); the new slots' runs go in at `at`.
				let slot_runs = plan.moved_runs(std::mem::take(&mut scene.slot_runs), at);
				scene.props.draws = std::mem::take(&mut plan.draws);
				scene.slot_runs = slot_runs;
				// The masked faces moved with their indices: the same (slot, face) pairs stay masked, the actors' slots move up.
				let mut masked = std::mem::take(&mut scene.clip_masked);
				for k in masked.iter_mut().filter(|k| k.0 as usize >= at) {
					k.0 += m as u32;
				}
				(std::mem::take(&mut scene.prop_faces), masked)
			}
			None => {
				let _s = super::profile::span("ap_index");
				let (draws, slot_runs, indices) = prop_layout(&scene.class_faces, &scene.tex_slots, &all_classes, &all_ranges, order);
				if let Some(e) = enc.take() {
					queue.submit(Some(e.finish()));
				}
				queue.write_buffer(&scene.props.ibuf, 0, bytemuck::cast_slice(&indices));
				scene.props.draws = draws;
				scene.slot_runs = slot_runs;
				// The indices were written whole: nothing is masked for the clipper (as after a rebuild).
				(std::mem::take(&mut scene.prop_faces), Vec::new())
			}
		};
		// NOV-5.9c: the clippable faces' index positions (`clip::replay_faces`), run-relative: the old slots' faces stay (after
		// an insert only the new slots' are built; a rebuild also checks the old ones).
		let _s = super::profile::span("ap_pfaces");
		let mut prop_faces = prop_faces;
		let (cf, tex_slots) = (&scene.class_faces, &scene.tex_slots);
		let faces_of = |s: usize| {
			let (cf, base) = (&cf[&all_classes[s]], all_ranges[s].start as u32);
			cf.clipped.iter().map(|&(g, rank, idx, phase)| {
				let key = cf.groups[g].0;
				super::clip::PropFace { slot: rank * 3, run: g as u32, idx: idx.map(|i| i + base), phase, tex: tex_slots.get(&(key & crate::mobies::SLOT_MASK)).copied().unwrap_or(0), cull: ((key >> 24) & 3) as u8 }
			}).collect::<Vec<_>>()
		};
		prop_faces.splice(at..at, (at..at + m).map(faces_of));
		if !inserted {
			prop_faces.par_iter_mut().enumerate().filter(|(s, faces)| faces.len() != cf[&all_classes[*s]].clipped.len()).for_each(|(s, faces)| *faces = faces_of(s));
		}
		drop(_s);
		let _s = super::profile::span("ap_write");
		queue.write_buffer(&scene.props.vbuf, (v0 * std::mem::size_of::<Vertex>()) as u64, bytemuck::cast_slice(&verts));
		if let Some(sj) = &scene.props_sj {
			// The new slots' (slot, vertex) attribute, and the scene actors' slot numbers moved up by `m` (one write per run of
			// slots whose vertices follow each other).
			let put = |slots: &mut dyn Iterator<Item = (usize, &std::ops::Range<usize>)>| {
				let (mut at_v, mut attr) = (0usize, Vec::<[u32; 2]>::new());
				for (s, r) in slots.filter(|(_, r)| !r.is_empty()) {
					if !attr.is_empty() && r.start != at_v + attr.len() {
						queue.write_buffer(sj, at_v as u64 * 8, bytemuck::cast_slice(&attr));
						attr.clear();
					}
					if attr.is_empty() {
						at_v = r.start;
					}
					attr.extend((0..r.len() as u32).map(|j| [s as u32, j]));
				}
				if !attr.is_empty() {
					queue.write_buffer(sj, at_v as u64 * 8, bytemuck::cast_slice(&attr));
				}
			};
			put(&mut ranges.iter().enumerate().map(|(k, r)| (at + k, r)));
			put(&mut scene.props_ranges.iter().enumerate().skip(at).map(|(s, r)| (s + m, r)));
		}
		drop(_s);
		let rad: Vec<f32> = ranges.iter().map(|r| verts[r.start - v0..r.end - v0].iter().map(|v| (v.pos[0] * v.pos[0] + v.pos[1] * v.pos[1] + v.pos[2] * v.pos[2]).sqrt()).fold(0.0, f32::max)).collect();
		scene.props_rad.splice(at..at, rad);
		scene.prop_faces = prop_faces;
		scene.clip_masked = clip_masked;
		scene.props_model.extend(verts);
		scene.props_ranges = all_ranges;
		scene.props_class = all_classes;
		scene.props_order = order.to_vec();
		scene.prompt_slot = scene.prompt_slot.map(|p| if p >= at { p + m } else { p });
		for pi in scene.props_late.iter_mut().filter(|pi| **pi >= at) {
			*pi += m;
		}
		if scene.props_vis.len() >= at {
			scene.props_vis.splice(at..at, vec![false; m]);
		}
		true
	}
}

/// NOV-5.9c driver: finds this frame's faces the retail clipper would take, queues their clipped polygons and masks the
/// original triangles out of `props.ibuf` (restoring those that are no longer clipped). `shown[pi]` = the slot is drawn.
/// MOBY-FADE: per prop the draw-distance fade alpha from the view (eye, forward) basis; None = past the limit (culled).
/// `Gpu::prop_slots`'s output, moved into the `Scene` fields of the same names.
pub(super) struct PropSlots {
	pub(super) ranges: Vec<std::ops::Range<usize>>,
	pub(super) prompt_slot: Option<usize>,
	pub(super) batch: Batch,
	pub(super) model: Vec<Vertex>,
	pub(super) slot_runs: Vec<Vec<(u32, u32, u32)>>,
	pub(super) faces: Vec<Vec<super::clip::PropFace>>,
	pub(super) sj: Option<wgpu::Buffer>,
	pub(super) rad: Vec<f32>,
	pub(super) classes: Vec<u32>,
	pub(super) class_faces: HashMap<u32, ClassFaces>,
}

/// A class mesh's faces as `Gpu::batch` groups them: per texture key (ascending) its indices in mesh order, and the
/// clippable faces in mesh order as (group, rank in the group, mesh indices, strip phase) (`clip::replay_faces`).
pub(in crate::viewer) struct ClassFaces {
	pub(super) groups: Vec<(u32, Vec<u32>)>,
	pub(super) clipped: Vec<(usize, u32, [u32; 3], u8)>,
}

impl ClassFaces {
	pub(super) fn of(m: &Mesh) -> ClassFaces {
		let mut by: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
		for &(key, f) in &m.faces {
			by.entry(key).or_default().extend(f);
		}
		let gi: HashMap<u32, usize> = by.keys().enumerate().map(|(g, &k)| (k, g)).collect();
		let phased = m.strip_phase.len() == m.faces.len();
		let mut rank = vec![0u32; gi.len()];
		let mut clipped = Vec::new();
		for (i, &(key, f)) in m.faces.iter().enumerate() {
			let g = gi[&key];
			if phased && m.strip_phase[i] != crate::mesh::NO_PHASE {
				clipped.push((g, rank[g], f, m.strip_phase[i]));
			}
			rank[g] += 1;
		}
		ClassFaces { groups: by.into_iter().collect(), clipped }
	}
}

/// [PC] The props batch's layout for the slot order `order` (`load_scene`'s for index order): one draw per texture key in
/// ascending order holding each slot's faces of that key in `order`; per slot its runs (draw, first index, count) in key
/// order; the indices (the class faces moved to each slot's vertices, `ranges`).
pub(super) fn prop_layout(class_faces: &HashMap<u32, ClassFaces>, tex_slots: &HashMap<u32, usize>, classes: &[u32], ranges: &[std::ops::Range<usize>], order: &[usize]) -> (Vec<Draw>, Vec<Vec<(u32, u32, u32)>>, Vec<u32>) {
	let mut count: BTreeMap<u32, u32> = BTreeMap::new();
	for c in classes {
		for (key, f) in &class_faces[c].groups {
			*count.entry(*key).or_default() += f.len() as u32;
		}
	}
	// One draw per texture key in ascending order (`batch`); `pos` = each draw's next free index.
	let mut draws = Vec::with_capacity(count.len());
	let mut draw_of: HashMap<u32, usize> = HashMap::with_capacity(count.len());
	let (mut pos, mut start) = (Vec::with_capacity(count.len()), 0);
	for (d, (&key, &c)) in count.iter().enumerate() {
		draws.push(Draw { inst: 0, cull: ((key >> 24) & 3) as u8, texture: tex_slots[&(key & crate::mobies::SLOT_MASK)], start, count: c });
		draw_of.insert(key, d);
		pos.push(start);
		start += c;
	}
	// Per slot (in `order`) its runs; per draw the (vertex base, faces) it holds in order.
	let mut slot_runs: Vec<Vec<(u32, u32, u32)>> = vec![Vec::new(); classes.len()];
	let mut per_draw: Vec<Vec<(u32, &[u32])>> = vec![Vec::new(); draws.len()];
	for &s in order {
		let base = ranges[s].start as u32;
		for (key, f) in &class_faces[&classes[s]].groups {
			let d = draw_of[key];
			slot_runs[s].push((d as u32, pos[d], f.len() as u32));
			per_draw[d].push((base, f));
			pos[d] += f.len() as u32;
		}
	}
	let mut indices = vec![0u32; start as usize];
	{
		let mut rest: &mut [u32] = &mut indices;
		let mut outs = Vec::with_capacity(draws.len());
		for d in &draws {
			let (a, b) = rest.split_at_mut(d.count as usize);
			outs.push(a);
			rest = b;
		}
		outs.into_par_iter().zip(per_draw.par_iter()).for_each(|(out, segs)| {
			let mut k = 0;
			for &(base, f) in segs {
				for (o, &i) in out[k..k + f.len()].iter_mut().zip(f) {
					*o = i + base;
				}
				k += f.len();
			}
		});
	}
	(draws, slot_runs, indices)
}

#[cfg(test)]
pub(super) fn same_draws(a: &[Draw], b: &[Draw]) -> bool {
	a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x.inst, x.cull, x.texture, x.start, x.count) == (y.inst, y.cull, y.texture, y.start, y.count))
}

/// [PC] perf (lane A7): a pool growth as an insert into the props batch's layout (`Gpu::append_props`): the new draws, the
/// old draw numbers' new ones (`dmap`), the old index positions the new faces go in before with the count put in up to
/// and including each (`ins`, ascending), the new slots' runs, their indices (`fresh`) and where each piece of them goes
/// (`chunks`: new position, range of `fresh`), and the old pieces kept (`copies`: old start, new start, length).
#[derive(Clone, Copy)]
pub(super) struct GrowOld<'a> {
	pub(super) cf: &'a HashMap<u32, ClassFaces>,
	pub(super) tex_slots: &'a HashMap<u32, usize>,
	pub(super) classes: &'a [u32],
	pub(super) order: &'a [usize],
	pub(super) draws: &'a [Draw],
	pub(super) runs: &'a [Vec<(u32, u32, u32)>],
}

impl<'a> GrowOld<'a> {
	pub(super) fn of(scene: &'a Scene) -> GrowOld<'a> {
		GrowOld { cf: &scene.class_faces, tex_slots: &scene.tex_slots, classes: &scene.props_class, order: &scene.props_order, draws: &scene.props.draws, runs: &scene.slot_runs }
	}
}

impl GrowPlan {
	/// The old slots' runs moved (and renumbered when a texture key is new), the new slots' runs put in at `at`.
	pub(super) fn moved_runs(&mut self, mut runs: Vec<Vec<(u32, u32, u32)>>, at: usize) -> Vec<Vec<(u32, u32, u32)>> {
		let (dmap, ins) = (&self.dmap, &self.ins);
		let shift = |x: u32| ins.partition_point(|&(p, _)| p <= x).checked_sub(1).map_or(0, |j| ins[j].1);
		for r in runs.iter_mut().flatten() {
			*r = (dmap[r.0 as usize], r.1 + shift(r.1), r.2);
		}
		runs.splice(at..at, std::mem::take(&mut self.runs));
		runs
	}

	/// The new indices from the old ones on the CPU, as the GPU copy and the writes give them (tests).
	#[cfg(test)]
	pub(super) fn apply(&self, old: &[u32]) -> Vec<u32> {
		let mut out = vec![u32::MAX; self.n_idx];
		for &(src, dst, len) in &self.copies {
			out[dst as usize..(dst + len) as usize].copy_from_slice(&old[src as usize..(src + len) as usize]);
		}
		for (dst, r) in &self.chunks {
			out[*dst as usize..*dst as usize + r.len()].copy_from_slice(&self.fresh[r.clone()]);
		}
		out
	}
}

pub(super) struct GrowPlan {
	pub(super) draws: Vec<Draw>,
	pub(super) dmap: Vec<u32>,
	pub(super) ins: Vec<(u32, u32)>,
	pub(super) runs: Vec<Vec<(u32, u32, u32)>>,
	pub(super) fresh: Vec<u32>,
	pub(super) chunks: Vec<(u32, std::ops::Range<usize>)>,
	pub(super) copies: Vec<(u32, u32, u32)>,
	pub(super) n_idx: usize,
}

/// The insert that turns the scene's props layout (built for `props_order`) into `prop_layout`'s for `order`, with the slots
/// `at..at + classes.len()` (vertices `ranges`, class faces already in `class_faces`) new. None when `order` is not
/// `props_order` with the new slots put in, or the scene's draws are not the ones its classes give (then the caller
/// rebuilds the layout). Each new face run goes after the run of the same key of the nearest slot before it in `order`
/// that has that key (new slots before it at the same place first), else at the start of its key's draw.
/// `old` = the scene's class faces, texture slots, slot classes, slot order, draws and runs (`GrowOld::of`).
pub(super) fn grow_plan(old: &GrowOld, at: usize, classes: &[u32], ranges: &[std::ops::Range<usize>], order: &[usize]) -> Option<GrowPlan> {
	let GrowOld { cf, tex_slots, classes: old_classes, order: old_order, draws: old_draws, runs: old_runs } = *old;
	let m = classes.len();
	let new = at..at + m;
	// Where each new slot sits in `order`; the old slots must keep their order.
	let mut q_of = vec![usize::MAX; m];
	let mut old = old_order.iter();
	for (q, &s) in order.iter().enumerate() {
		if new.contains(&s) {
			if q_of[s - at] != usize::MAX {
				return None;
			}
			q_of[s - at] = q;
		} else if old.next() != Some(&if s < at { s } else { s - m }) {
			return None;
		}
	}
	if old.next().is_some() || q_of.contains(&usize::MAX) {
		return None;
	}
	// The old draws as the old slots' classes give them (one per key, ascending).
	let mut per_class: HashMap<u32, u32> = HashMap::new();
	for &c in old_classes {
		*per_class.entry(c).or_default() += 1;
	}
	let mut count: BTreeMap<u32, u32> = BTreeMap::new();
	for (c, &k) in &per_class {
		for (key, f) in &cf.get(c)?.groups {
			*count.entry(*key).or_default() += k * f.len() as u32;
		}
	}
	let keys: Vec<u32> = count.keys().copied().collect();
	let mut start = 0;
	if keys.len() != old_draws.len() || count.values().zip(old_draws).any(|(&c, d)| {
		let bad = c != d.count || d.start != start;
		start += d.count;
		bad
	}) {
		return None;
	}
	let old_n = old_draws.last().map_or(0, |d| d.start + d.count);
	let has = |c: u32, key: u32| cf[&c].groups.binary_search_by_key(&key, |g| g.0).ok();
	// Per new slot (in `order`) and face run the old index position it goes in before.
	let mut by_q: Vec<usize> = (0..m).collect();
	by_q.sort_unstable_by_key(|&k| q_of[k]);
	let mut ins_at: Vec<Vec<u32>> = vec![Vec::new(); m];
	let mut items: Vec<(u32, u32, usize, usize, usize)> = Vec::new();
	for &k in &by_q {
		let groups = &cf[&classes[k]].groups;
		let mut here = Vec::with_capacity(groups.len());
		for (g, &(key, _)) in groups.iter().enumerate() {
			let mut found = None;
			for &s in order[..q_of[k]].iter().rev() {
				if new.contains(&s) {
					if let Some(gj) = has(classes[s - at], key) {
						found = Some(ins_at[s - at][gj]);
						break;
					}
				} else {
					let so = if s < at { s } else { s - m };
					if let Some(gj) = has(old_classes[so], key) {
						let r = old_runs[so][gj];
						found = Some(r.1 + r.2);
						break;
					}
				}
			}
			let p = found.unwrap_or_else(|| old_draws.get(keys.partition_point(|&x| x < key)).map_or(old_n, |d| d.start));
			here.push(p);
			items.push((p, key, q_of[k], k, g));
		}
		ins_at[k] = here;
	}
	items.sort_unstable_by_key(|&(p, key, q, _, _)| (p, key, q));
	// The new draws.
	for &(_, key, _, k, g) in &items {
		*count.entry(key).or_default() += cf[&classes[k]].groups[g].1.len() as u32;
	}
	let new_keys: Vec<u32> = count.keys().copied().collect();
	let mut draws = Vec::with_capacity(count.len());
	let mut start = 0;
	for (&key, &c) in &count {
		draws.push(Draw { inst: 0, cull: ((key >> 24) & 3) as u8, texture: tex_slots[&(key & crate::mobies::SLOT_MASK)], start, count: c });
		start += c;
	}
	let dmap: Vec<u32> = keys.iter().map(|k| new_keys.binary_search(k).unwrap() as u32).collect();
	// The new faces in buffer order: one chunk per old position, the old pieces between them copied.
	let added: usize = items.iter().map(|&(_, _, _, k, g)| cf[&classes[k]].groups[g].1.len()).sum();
	let (mut fresh, mut chunks, mut copies, mut ins) = (Vec::with_capacity(added), Vec::new(), Vec::new(), Vec::new());
	let mut runs: Vec<Vec<(u32, u32, u32)>> = classes.iter().map(|c| vec![(0, 0, 0); cf[c].groups.len()]).collect();
	let (mut prev, mut cum, mut i) = (0u32, 0u32, 0);
	while i < items.len() {
		let p = items[i].0;
		if p > prev {
			copies.push((prev, prev + cum, p - prev));
		}
		let from = fresh.len();
		while i < items.len() && items[i].0 == p {
			let (_, key, _, k, g) = items[i];
			let f = &cf[&classes[k]].groups[g].1;
			let base = ranges[k].start as u32;
			runs[k][g] = (new_keys.binary_search(&key).unwrap() as u32, p + cum + (fresh.len() - from) as u32, f.len() as u32);
			fresh.extend(f.iter().map(|&x| x + base));
			i += 1;
		}
		chunks.push((p + cum, from..fresh.len()));
		cum += (fresh.len() - from) as u32;
		ins.push((p, cum));
		prev = p;
	}
	if old_n > prev {
		copies.push((prev, prev + cum, old_n - prev));
	}
	debug_assert_eq!(start as usize, old_n as usize + added);
	Some(GrowPlan { draws, dmap, ins, runs, fresh, chunks, copies, n_idx: start as usize })
}

/// The mesh of each prop slot (the bolt classes fall back to the bolt mesh, a class without one is empty).
fn slot_meshes<'a>(props: &[u32], prop_meshes: &'a HashMap<u32, Mesh>, empty: &'a Mesh) -> Vec<&'a Mesh> {
	let fallback = prop_meshes.get(&game::BOLT);
	props.iter().map(|&c| prop_meshes.get(&c).or(if (game::BOLT..game::BOLT + 4).contains(&c) { fallback } else { None }).unwrap_or(empty)).collect()
}

impl SceneCtx {
	/// The gameplay prop slots of a scene: one copy of the class mesh per slot in `props` order (`load_scene`, and
	/// `regrow_props` when a retail pool grows).
	pub(super) fn prop_slots(&self, props: &[u32], prop_meshes: &HashMap<u32, Mesh>, textures: &HashMap<u32, Texture>, bind: &mut Vec<wgpu::BindGroup>, slots: &mut HashMap<u32, usize>) -> PropSlots {
		let empty = Mesh::new(String::new());
		let slot_meshes = slot_meshes(props, prop_meshes, &empty);
		let mut ranges = Vec::new();
		let mut at = 0;
		for m in &slot_meshes {
			ranges.push(at..at + m.positions.len());
			at += m.positions.len();
		}
		let prompt_slot = props.iter().position(|&c| c == game::vendor::PROMPT_CLASS);
		// [PC] Room for grown retail pools (`append_props`): a 16th more vertices and indices, at least 64K / 192K.
		let spare = (at / 16).max(1 << 16) * (crate::retail::active() as usize);
		let n_idx: usize = slot_meshes.iter().map(|m| m.faces.len() * 3).sum();
		let (batch, _, _, model) = self.batch_spare(&slot_meshes, None, false, textures, bind, slots, (spare, (n_idx / 16).max(3 << 16) * (spare > 0) as usize));
		crate::loadlog::say(|| "ps: batch_spare done".into());
		let slot_runs = super::shadowvol::slot_runs(&slot_meshes);
		crate::loadlog::say(|| "ps: slot_runs done".into());
		// NOV-5.9c: where each clippable face sits in `props.ibuf`, and the overlay batch its clipped polygons go to.
		let mut faces = super::clip::replay_faces(&slot_meshes, slots);
		// [PC] perf (lane A7): each face's position counted from the start of its slot's run (`clip::PropFace`).
		faces.par_iter_mut().zip(&slot_runs).for_each(|(fs, runs)| {
			for f in fs {
				let r = runs.partition_point(|r| r.1 + r.2 <= f.slot);
				f.run = r as u32;
				f.slot -= runs[r].1;
			}
		});
		crate::loadlog::say(|| "ps: replay_faces done".into());
		// [PC] perf round 3: the static (slot, vertex) attribute of the vertex-shader posing (gpuskin.rs).
		let sj = Some({
			let sj = super::gpuskin::slot_attr(&ranges);
			let b = sized_buffer(&self.device, "props slot attribute", (sj.len() + spare) * 8, wgpu::BufferUsages::VERTEX);
			self.wb(&b, 0, bytemuck::cast_slice(&sj));
			b
		});
		crate::loadlog::say(|| "ps: sj done".into());
		let rad: Vec<f32> = ranges.iter().map(|r| model[r.clone()].iter().map(|v| (v.pos[0] * v.pos[0] + v.pos[1] * v.pos[1] + v.pos[2] * v.pos[2]).sqrt()).fold(0.0, f32::max)).collect();
		crate::loadlog::say(|| "ps: rad done".into());
		// [PC] Retail: each class's faces as `append_props` regroups them, built here (level load) so a pool growth does not.
		let mut uniq: Vec<(u32, &Mesh)> = if spare > 0 { props.iter().copied().zip(slot_meshes.iter().copied()).collect() } else { Vec::new() };
		uniq.sort_unstable_by_key(|u| u.0);
		uniq.dedup_by_key(|u| u.0);
		let class_faces: HashMap<u32, ClassFaces> = uniq.par_iter().map(|&(c, m)| (c, ClassFaces::of(m))).collect();
		PropSlots { ranges, prompt_slot, batch, model, slot_runs, faces, sj, rad, classes: props.to_vec(), class_faces }
	}
}
