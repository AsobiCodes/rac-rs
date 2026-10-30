//! Retail polygon clipper for props: draw-distance alphas, per-prop verdicts and the clipped polygon queue.

use super::*;

impl Gpu {
	/// NOV-5.9c: this frame's view basis for the CPU clipper (None disables it for the calls that follow).
	pub(in crate::viewer) fn set_clip_view(&mut self, view: Option<(&Camera, f32)>) {
		let wfar = self.world_far();
		if let Some(scene) = &mut self.scene {
			scene.clip_view = view.map(|(cam, aspect)| super::clip::ClipView::new(cam, aspect).with_far(wfar));
			// REN-CULL-OCC: the mask is rebuilt once per frame, before any tie / fragment / moby test; with no view (menus) it
			// stays as it was and nothing tests it.
			if let Some(v) = &scene.clip_view {
				scene.occ.forced = crate::retail::OCC_MASK.lock().ok().and_then(|g| *g);
				scene.occ.update(v.e);
			}
		}
	}

	/// BRIDGE-33: per prop slot, this frame's draw verdict (empty before the first `set_props` of a scene).
	pub(in crate::viewer) fn props_drawn(&self) -> &[bool] {
		self.scene.as_ref().map_or(&[], |s| &s.props_drawn)
	}

	/// FRAME-1a: the draw verdict of one tick's render from that tick's camera, with its own occlusion state (`occ_tick`) so the
	/// display frame's mask rebuild does not disturb it. Empty without a scene.
	pub(in crate::viewer) fn props_verdict(&mut self, cam: &Camera, aspect: f32, props: &[Prop], centres: &[[f32; 3]], radii: &[Option<f32>]) -> Vec<bool> {
		let far = self.far_draw();
		let Some(scene) = &mut self.scene else { return Vec::new() };
		let v = super::clip::ClipView::new(cam, aspect).with_far(super::fardraw::far_plane(far));
		scene.occ_tick.update(v.e);
		// As `set_props`, only the scene's prop slots are tested; the rest stay not drawn.
		let n = props.len().min(scene.props_ranges.len());
		let mut drawn = prop_verdicts(&v, &scene.occ_tick.mask, &props[..n], &prop_alphas(Some((v.e, v.f)), &props[..n], centres, radii, far), centres, radii, far);
		drawn.resize(props.len(), false);
		drawn
	}
}

/// `clip_props`: the slots' vertices, as the records `vs_props` poses.
#[derive(Clone, Copy)]
pub(super) enum ClipSrc<'a> {
	/// Records, the pose entries (the CPU mirror; in the direct mode of gpu/pose_reuse.rs empty, the poses are then the jobs'
	/// sources: `slot_pose`) and per-slot model-space bounding radii.
	Gpu(&'a [super::gpuskin::PropInst], &'a [super::gpuskin::PoseEntry], Option<&'a [super::pose_reuse::PoseJob<'a>]>, &'a [f32]),
}

/// Initial capacity of the clipped-polygon overlay (vertices, indices); it only grows (NOV-5.9c Order 7).
pub(super) const CLIP_VERTS: usize = 1024;

pub(super) const CLIP_INDICES: usize = 3072;

/// `far`: [PC] Draw distance Medium / High (fardraw.rs), the fade tests see the depth `scale` times shorter and nothing past
/// the level's cap.
pub(super) fn prop_alphas(view: Option<([f32; 3], [f32; 3])>, props: &[Prop], centres: &[[f32; 3]], radii: &[Option<f32>], far: Option<super::fardraw::Level>) -> Vec<Option<u8>> {
	props
		.iter()
		.enumerate()
		.map(|(pi, p)| {
			let fade = match (view, centres.get(pi)) {
				(Some((e, f)), Some(c)) => {
					let d: [f32; 3] = std::array::from_fn(|k| c[k] - e[k] * 1024.0);
					let z = f[0] * d[0] + f[1] * d[1] + f[2] * d[2];
					let k = far.map_or(1.0, |l| l.scale());
					let z = match far {
						Some(l) => super::fardraw::moby_depth(l, z)?,
						None => z,
					};
					let fade = moby_fade(p.draw_dist(), z)?;
					// SCH-6b: an in-game prop on the deferred-list path (flags 0x400 / 0x800) fades by the unclamped distance and the
					// sphere's near edge, in integers.
					match radii.get(pi).copied().flatten() {
						Some(w) if p.sched_flags() & 0xc00 != 0 => moby_fade_alt(p.draw_dist(), z, w / k),
						_ => fade,
					}
				}
				_ => 0x80,
			};
			Some(moby_draw_alpha(fade, p.moby_alpha()))
		})
		.collect()
}

/// FRAME-1a / BRIDGE-33: the draw verdict of `set_props` without the vertex writes: occlusion mask, near / side planes and
/// the 48-entry deferred list (SCH-6b).
/// `far`: [PC] Draw distance Medium / High, the 48-entry list has no limit (far mobys must not push near ones out).
fn prop_verdicts(view: &super::clip::ClipView, mask: &[u8; 128], props: &[Prop], alphas: &[Option<u8>], centres: &[[f32; 3]], radii: &[Option<f32>], far: Option<super::fardraw::Level>) -> Vec<bool> {
	let mut drawn = vec![false; props.len()];
	let mut committed = 0;
	for (pi, p) in props.iter().enumerate() {
		if alphas.get(pi).copied().flatten().is_none() || !p.draw_shown() || !crate::occlusion::occ_pass(mask, p.sched_occ()) {
			continue;
		}
		let side_out = match (centres.get(pi), radii.get(pi).copied().flatten()) {
			(Some(&c), Some(r)) => view.moby_side_near_out(c, r),
			_ => false,
		};
		drawn[pi] = !side_out;
		if p.sched_flags() & 0x800 != 0 && !side_out {
			committed += 1;
			if committed > DEFERRED_CAP && far.is_none() {
				drawn[pi] = p.sched_over() == 0;
			}
		}
	}
	drawn
}

pub(super) fn clip_props(queue: &wgpu::Queue, scene: &mut Scene, src: ClipSrc, shown: &[bool]) {
	let mut taken: Vec<(u32, u32)> = Vec::new();
	if let Some(view) = scene.clip_view {
		let (band, port) = (view.band(), view.viewport());
		// [PC] perf: the slots are scanned and clipped on the frame worker pool, then queued in slot / face order.
		// [PC] lane nprops: only the shown slots with faces are handed to the pool, and only those with polygons are kept.
		let (ranges, model) = (&scene.props_ranges, &scene.props_model);
		let todo: Vec<usize> = (0..scene.prop_faces.len()).filter(|&pi| shown.get(pi).copied().unwrap_or(false) && !scene.prop_faces[pi].is_empty()).collect();
		let per_slot: Vec<(usize, Vec<(u32, super::clip::ClipPoly)>)> = todo
			.par_iter()
			.filter_map(|&pi| {
				let faces = &scene.prop_faces[pi];
				// One flag scan per slot replaces a bounding sphere: nothing outside the band, or everything past one viewport plane.
				// (PC perf: one pass, no projected copy.)
				let straddles = |positions: &mut dyn Iterator<Item = [f32; 3]>| {
					let (mut inside, mut all_out) = (true, 0x3fu8);
					for q in positions {
						let p = view.view(q);
						inside &= band.flags(p) == 0;
						all_out &= port.flags(p);
					}
					!inside && all_out == 0
				};
				// `slot` holds the vertices from index `base` on (the faces store batch indices).
				let clip = |slot: &[Vertex], base: usize| faces.iter().enumerate().filter_map(|(fi, f)| super::clip::clip_face(&view, f.idx.map(|i| slot[i as usize - base]), f.phase, f.tex, f.cull).map(|poly| (fi as u32, poly))).collect::<Vec<_>>();
				let r = ranges[pi].clone();
				let polys = match src {
					// [PC] perf round 3: the slot's vertices as `vs_props` poses them (positions only for the scan).
					ClipSrc::Gpu(inst, mirror, jobs, rad) => {
						let s = &inst[pi];
						// A slot whose bounding sphere settles the scan skips it (same verdict, no per-vertex work).
						if super::gpuskin::slot_sphere(s, rad[pi]).is_some_and(|(c, r)| super::gpuskin::sphere_skips(&view, c, r)) {
							return None;
						}
						// The slot's pose entries: the mirror's (record as is), or in the direct mode converted from the pose's source.
						let (s, pose) = super::pose_reuse::slot_pose(mirror, jobs, s);
						if !straddles(&mut model[r.clone()].iter().enumerate().map(|(j, m0)| super::gpuskin::emulate_pos(&s, &pose, m0.pos, j))) {
							return None;
						}
						let slot: Vec<Vertex> = model[r.clone()].iter().enumerate().map(|(j, m0)| super::gpuskin::emulate(&s, &pose, m0, j)).collect();
						clip(&slot, r.start)
					}
				};
				(!polys.is_empty()).then_some((pi, polys))
			})
			.collect();
		for (pi, polys) in per_slot {
			for (fi, poly) in polys {
				taken.push((pi as u32, fi));
				scene.clip_queue.push(poly);
			}
		}
	}
	if taken != scene.clip_masked {
		let write = |(pi, fi): (u32, u32), idx: Option<[u32; 3]>, scene: &Scene| {
			let f = &scene.prop_faces[pi as usize][fi as usize];
			let at = scene.slot_runs[pi as usize].get(f.run as usize).map_or(f.slot, |r| r.1 + f.slot);
			queue.write_buffer(&scene.props.ibuf, at as u64 * 4, bytemuck::cast_slice(&idx.unwrap_or([0; 3])));
		};
		let (old, new): (HashSet<_>, HashSet<_>) = (scene.clip_masked.iter().copied().collect(), taken.iter().copied().collect());
		for &k in old.difference(&new) {
			write(k, Some(scene.prop_faces[k.0 as usize][k.1 as usize].idx), scene);
		}
		for &k in new.difference(&old) {
			write(k, None, scene);
		}
		scene.clip_masked = taken;
	}
}

/// Uploads the queued clipped polygons into `clip_batch` (growing both buffers to the next power of two when needed) and
/// builds one draw per (cull, texture) run; an empty queue leaves no draws.
pub(super) fn flush_clip(device: &wgpu::Device, queue: &wgpu::Queue, scene: &mut Scene) {
	let mut verts: Vec<Vertex> = Vec::new();
	let mut by_run: BTreeMap<(u8, usize), Vec<u32>> = BTreeMap::new();
	for poly in scene.clip_queue.drain(..) {
		let base = verts.len();
		verts.extend(poly.verts);
		by_run.entry((poly.cull, poly.tex)).or_default().extend(poly.tris.iter().flatten().map(|&i| (base + i) as u32));
	}
	scene.clip_batch.draws.clear();
	if verts.is_empty() {
		return;
	}
	let (mut indices, mut draws) = (Vec::new(), Vec::new());
	for ((cull, texture), idx) in by_run {
		draws.push(Draw { inst: 0, cull, texture, start: indices.len() as u32, count: idx.len() as u32 });
		indices.extend(idx);
	}
	if verts.len() > scene.clip_cap.0 || indices.len() > scene.clip_cap.1 {
		let cap = (verts.len().next_power_of_two().max(scene.clip_cap.0), indices.len().next_power_of_two().max(scene.clip_cap.1));
		scene.clip_batch.vbuf = buffer(device, "clip vertices", &vec![0u8; cap.0 * std::mem::size_of::<Vertex>()], wgpu::BufferUsages::VERTEX);
		scene.clip_batch.ibuf = buffer(device, "clip indices", &vec![0u8; cap.1 * 4], wgpu::BufferUsages::INDEX);
		scene.clip_cap = cap;
	}
	queue.write_buffer(&scene.clip_batch.vbuf, 0, bytemuck::cast_slice(&verts));
	queue.write_buffer(&scene.clip_batch.ibuf, 0, bytemuck::cast_slice(&indices));
	scene.clip_batch.draws = draws;
}
