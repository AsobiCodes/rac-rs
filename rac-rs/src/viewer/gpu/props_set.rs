//! Per-frame prop slot rebuild (set_props): slot jobs and sources, GPU records, preview and panel poses, debris.

use super::*;

impl Gpu {
	/// Registers the debris classes: unposed vertices and faces by texture slot per class, and the per-frame
	/// instance buffers sized for the most pieces the moby pool can hold (`MOBY_LIMIT`) of the largest class.
	pub(in crate::viewer) fn load_debris(&mut self, classes: &[u32], prop_meshes: &HashMap<u32, Mesh>, textures: &HashMap<u32, Texture>) {
		let Some(mut scene) = self.scene.take() else { return };
		let mut model = HashMap::new();
		let (mut max_v, mut max_i) = (0, 0);
		for &c in classes {
			let Some(m) = prop_meshes.get(&c) else { continue };
			let mut slots = HashMap::new();
			let (_, _, _, verts) = self.batch(&[m], None, false, textures, &mut scene.bind_groups, &mut slots);
			let mut by_tex: BTreeMap<usize, Vec<u32>> = BTreeMap::new();
			let mut phases: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
			let phased = m.strip_phase.len() == m.faces.len();
			for (fi, &(tex, f)) in m.faces.iter().enumerate() {
				let slot = slots[&(tex & crate::mobies::SLOT_MASK)];
				by_tex.entry(slot).or_default().extend(f);
				phases.entry(slot).or_default().push(if phased { m.strip_phase[fi] } else { crate::mesh::NO_PHASE });
			}
			max_v = max_v.max(verts.len());
			max_i = max_i.max(m.faces.len() * 3);
			model.insert(c, (verts, by_tex.into_iter().collect::<Vec<_>>(), phases.into_iter().collect::<Vec<_>>()));
		}
		let cap = (max_v * particles::MOBY_LIMIT, max_i * particles::MOBY_LIMIT);
		scene.debris = Some(Batch {
			vbuf: buffer(&self.device, "debris vertices", &vec![0u8; cap.0 * std::mem::size_of::<Vertex>()], wgpu::BufferUsages::VERTEX),
			ibuf: buffer(&self.device, "debris indices", &vec![0u8; cap.1 * 4], wgpu::BufferUsages::INDEX),
			draws: Vec::new(),
		});
		scene.debris_model = model;
		scene.debris_cap = cap;
		self.scene = Some(scene);
	}

	/// Poses the player model at `feet`, turned to `yaw` (None hides it); `skinned` replaces the
	/// model-space positions and normals with an animated pose.
	/// Re-poses hero preview `key` (model-space positions and normals, one per vertex of its mesh).
	pub(in crate::viewer) fn set_preview_pose(&mut self, key: u32, ps: &[[f32; 3]], ns: &[[f32; 3]]) {
		let Some(scene) = &self.scene else { return };
		let (Some(model), Some((batch, ..))) = (scene.preview_models.get(&key), scene.previews.get(&key)) else { return };
		if model.len() != ps.len() || ns.len() != ps.len() {
			return;
		}
		let verts: Vec<Vertex> = model.iter().zip(ps.iter().zip(ns)).map(|(v, (p, n))| Vertex { pos: *p, normal: *n, ..*v }).collect();
		self.queue.write_buffer(&batch.vbuf, 0, bytemuck::cast_slice(&verts));
	}

	/// NOV-30d (1)/(3): the shop panel-1 item A (slot +0x100) is an animated moby (seq 0, then seq 1 when the class has
	/// more, stepped every tick) that retail draws skinned into the item target; `posed` = (class, model-space positions,
	/// normals) at its retail clock. The class posed before gets its bind vertices back when the item changes or goes.
	pub(in crate::viewer) fn pose_panel_item(&mut self, posed: Option<(u32, Vec<[f32; 3]>, Vec<[f32; 3]>)>) {
		let key = posed.as_ref().map(|p| p.0);
		if let (Some(prev), Some(scene)) = (self.panel_posed.filter(|&c| Some(c) != key), &self.scene) {
			if let (Some(model), Some((batch, ..))) = (scene.preview_models.get(&prev), scene.previews.get(&prev)) {
				self.queue.write_buffer(&batch.vbuf, 0, bytemuck::cast_slice(model));
			}
		}
		self.panel_posed = key;
		if let Some((c, ps, ns)) = posed {
			self.set_preview_pose(c, &ps, &ns);
		}
	}

	/// Poses every gameplay prop (None hides them all, outside play mode). `posed` holds animated
	/// model-space (positions, normals) per class, shared by all its instances.
	/// `actors` fills the scene-actor slots after the props: a skinned pose (root-relative, facing
	/// baked in) at a world position, or None to hide.
	pub(in crate::viewer) fn set_props(&mut self, props: Option<&[Prop]>, posed: &HashMap<PoseKey, Posed>, fx_posed: &HashMap<usize, Posed>, actors: &[Option<(Posed, [f32; 3])>], debris: &[particles::Debris], insts: &[particles::WorldInst], centres: &[[f32; 3]], radii: &[Option<f32>]) {
		let _s = super::profile::span("set_props");
		if let Some(props) = props {
			if !crate::retail::splitview::redo() { self.waterfx.track_props(props); } // [PC] split view: a re-run (frame_split.rs) counts no second still frame
		}
		let prop_v = self.prop_v.take(); self.moby_dyn.truncate(crate::retail::splitview::dyn_base(self.moby_dyn.len())); // [PC] split view: a re-run drops the first run's dynamic-light entries
		let far = self.far_draw();
		let cull = self.culling();
		let Some(scene) = &mut self.scene else { return };
		let Some(props) = props else {
			scene.props_shown = false;
			scene.props_alpha = false;
			scene.clip_queue.clear();
			scene.clip_batch.draws.clear();
			return;
		};
		// Draw-distance fade (MOBY-FADE): None = past the limit (culled), else the moby draw alpha.
		let view = scene.clip_view.as_ref().map(|v| (v.e, v.f));
		let (alphas, past) = open_alphas(prop_alphas(view, props, centres, radii, far), props, cull);
		scene.props_alpha = props.iter().zip(&alphas).any(|(p, a)| p.draw_shown() && a.is_some_and(|a| moby_alpha_encoding(0.0, a).1));
		let n_slots = scene.props_ranges.len();
		let mut shown = vec![false; n_slots];
		scene.props_drawn = vec![false; props.len()];
		let masked = scene.clip_view.is_some();
		// SCH-6b: the deferred list holds 48 committed entries per frame (flags 0x800 props that passed the side test).
		let mut committed = 0;
		// BRIDGE-45: the drawn props whose metal packets `set_metal_props` builds (prop, placement, scale, dynamic-light id).
		let mut metal_jobs = Vec::new();
		// [PC] perf: what each slot's vertices are built from this frame, in slot order: written by the CPU path below
		// or posed by `vs_props` from one record per slot (gpuskin.rs).
		let mut srcs: Vec<SlotSrc> = Vec::with_capacity(n_slots);
		for (pi, (p, r)) in props.iter().zip(&scene.props_ranges).enumerate() {
			// REN-CULL-OCC-MOBY: a prop whose occlusion bytes fail this frame's mask is not drawn (no view: nothing is tested).
			// [PC] Culling Off: an occluded prop is drawn too (unless no camera cell sees it, `Occlusion::ever_visible`); retail's verdict for it stays "not drawn" (no side test, no list entry).
			let occluded = masked && !crate::occlusion::occ_pass(&scene.occ.mask, p.sched_occ());
			let Some(alpha) = alphas[pi].filter(|_| (p.draw_shown() || p.cast) && !(!cull && occluded && !scene.occ.ever_visible_moby(p.sched_occ()))) else {
				srcs.push(SlotSrc::Hidden);
				continue;
			};
			let (ps, ns) = slot_pose(fx_posed, posed, pi, p, r.len());
			if !(p.draw_shown() && (!occluded || !cull)) || past.get(pi) == Some(&true) {
				srcs.push(culled_src(pi, p, ps, ns, alpha, cull, self.class_uv.get(&(p.class_id as u32)).copied().unwrap_or([0.0; 2])));
				continue;
			}
			// REN-MOBY-DRAWN: the verdict also needs the near / side planes (the GPU clips, so the prop is still drawn).
			let side_out = match (&scene.clip_view, centres.get(pi), radii.get(pi).copied().flatten()) {
				(Some(v), Some(&c), Some(r)) => v.moby_side_near_out(c, r),
				_ => false,
			};
			scene.props_drawn[pi] = !side_out && !occluded;
			if p.sched_flags() & 0x800 != 0 && !side_out && !occluded {
				committed += 1;
				if committed > DEFERRED_CAP && far.is_none() && cull {
					// List full: not drawn; the verdict is 0 when the override was set, else the side pass stands.
					scene.props_drawn[pi] = p.sched_over() == 0;
					srcs.push(SlotSrc::Deferred { ps, ns });
					continue;
				}
			}
			shown[pi] = true;
			// PC-FPS-1: the placement drawn between the last two ticks when smooth motion blends it.
			let (m, scale) = super::smooth::prop_xf(pi).unwrap_or_else(|| (place_mirrored(p.pos, p.rot, p.mirror), p.scale));
			let light = Self::moby_light(p.light);
			let id = dyn_id(&self.table, &mut self.moby_dyn, [m[12], m[13], m[14]]);
			let tint = PROP_TINTS.with(|t| t.borrow().get(pi).copied().flatten());
			metal_jobs.push((pi, m, scale, id));
			let alpha_code = moby_alpha_encoding(id, alpha).0;
			if crate::retail::dbg_class() == Some(p.class_id as u32) {
				eprintln!("DBGDRAW {} slot {pi} alpha {alpha} moby_alpha {} light {light:?} dyn {id} dl {:?} scale {scale} l0 {:?}", p.class_id, p.moby_alpha(), (id > 0.0).then(|| self.moby_dyn.get(id as usize - 1)), self.lighting.as_ref().and_then(|lt| lt.lights.first()).map(|l| (l.colour, l.wrap, l.dir)));
			}
			// WPN-6.3e: the aux 179 texture scroll (FUN_00263928 adds the delta to every vertex V, as the Pyrocitor nozzle);
			// a scrolled slot is rebuilt every frame.
			let cuv = self.class_uv.get(&(p.class_id as u32)).copied().unwrap_or([0.0; 2]);
			let (du, dv) = (cuv[0], cuv[1] + prop_v.filter(|&(i, _)| i == pi).map_or(0.0, |(_, d)| d));
			log_uv(p, pi, &m, scale, cuv);
			srcs.push(SlotSrc::Prop(SlotJob { ps, ns, m, scale, light, alpha_code, prop_tint: p.tint, tint, alpha, dv, du }));
		}
		// Debris: every live piece is instanced into the per-frame debris batch (the moby pool bounds the count).
		if let Some(batch) = &mut scene.debris {
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
				let light = Self::moby_light(None);
				let id = dyn_id(&self.table, &mut self.moby_dyn, p);
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
						if let Some(poly) = scene.clip_view.filter(|_| phase != crate::mesh::NO_PHASE).and_then(|view| super::clip::clip_face(&view, tri, phase, *t, 0)) {
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
				self.queue.write_buffer(&batch.vbuf, 0, bytemuck::cast_slice(&dv));
				self.queue.write_buffer(&batch.ibuf, 0, bytemuck::cast_slice(&indices));
			}
			batch.draws = draws;
		}
		for (i, r) in scene.props_ranges.iter().enumerate().skip(props.len()) {
			match actors.get(i - props.len()).and_then(|a| a.as_ref()).filter(|((ps, _), _)| ps.len() == r.len()) {
				Some(((ps, ns), at)) => {
					shown[i] = true;
					let light = self.scene_actor_light.unwrap_or_else(|| Self::moby_light(Some(([0x38; 3], 0)))); // SCN-3: the hero's word at scene start, else the 0x38 constant
					let tint = ACTOR_TINTS.with(|t| t.borrow().get(i - props.len()).copied().flatten());
					let id = dyn_id(&self.table, &mut self.moby_dyn, *at);
					srcs.push(SlotSrc::Actor { ps, ns, at: *at, light, id, tint });
				}
				None => srcs.push(SlotSrc::Hidden),
			}
		}
		scene.props_vis = shown.clone();
		{
			let pg = &mut self.props_gpu;
			// [PC] perf round 3: one record per slot and each distinct pose once; `vs_props` poses the static model.
			let (mut inst, mut pose, mut rad) = (std::mem::take(&mut scene.props_inst), std::mem::take(&mut scene.props_pose), Vec::new());
			{
				let _s = super::profile::span("props_slots");
				build_records(&srcs, &scene.props_ranges, &scene.props_rad, &mut inst, &mut pose, &mut rad);
			}
			// BRIDGE-36 (c2): with a HUD split the posed prompt goes to its own batch and leaves the props batch (and the clip overlay).
			if let (Some(_), Some(pi), Some(prompt)) = (self.hud_split, scene.prompt_slot, scene.prompt.as_ref()) {
				if pi < props.len() {
					let r = scene.props_ranges[pi].clone();
					let vs: Vec<Vertex> = scene.props_model[r].iter().enumerate().map(|(j, m0)| super::gpuskin::emulate(&inst[pi], &pose, m0, j)).collect();
					self.queue.write_buffer(&prompt.vbuf, 0, bytemuck::cast_slice(&vs));
					inst[pi].k[0] |= super::gpuskin::FLAG_PROMPT;
				}
			}
			{
				let _s = super::profile::span("props_clip");
				clip_props(&self.queue, scene, ClipSrc::Gpu(&inst, &pose, &rad), &shown);
				flush_clip(&self.device, &self.queue, scene);
			}
			let _u = super::profile::span("props_upload");
			pg.upload(&self.device, &self.queue, &inst, &pose);
			(scene.props_inst, scene.props_pose) = (inst, pose);
			scene.props_shown = true;
		}
		let _m = super::profile::span("props_metal");
		self.set_metal_props(props, &metal_jobs, posed, fx_posed, centres, radii);
	}

	/// SCN-3: capture the hero's light for the scene actors when a scene starts, release it when the scene ends.
	pub(in crate::viewer) fn scene_actor_start(&mut self, active: bool, feet: [f32; 3]) {
		let mut held = self.scene_actor_light;
		let (lighting, placed, env) = (&self.lighting, self.hero_placed, &mut self.hero_env);
		scene_light_snapshot(&mut held, active, || Self::hero_light(lighting, feet, placed, env));
		self.scene_actor_light = held;
	}
}

/// A hidden prop slot's vertices sit far below the world (collapsed to a point).
pub(in crate::viewer) const HIDDEN_POS: [f32; 3] = [0.0, 0.0, -1.0e6];

/// `set_props`: one shown prop slot's rebuild from the model (pose, placement, colour).
pub(in crate::viewer) struct SlotJob<'a> {
	/// The pose's first positions and normals (empty: the model's).
	pub(in crate::viewer) ps: &'a [[f32; 3]],
	pub(in crate::viewer) ns: &'a [[f32; 3]],
	pub(in crate::viewer) m: [f32; 16],
	pub(in crate::viewer) scale: f32,
	pub(in crate::viewer) light: [f32; 4],
	pub(in crate::viewer) alpha_code: f32,
	pub(in crate::viewer) prop_tint: Option<[f32; 3]>,
	pub(in crate::viewer) tint: Option<([f32; 3], usize)>,
	pub(in crate::viewer) alpha: u8,
	/// WPN-6.3e: the aux 179 texture scroll added to every vertex V.
	pub(in crate::viewer) dv: f32,
	/// The class U scroll of a retail-mode class block (`retail::asset_feed`), added to every vertex U.
	pub(in crate::viewer) du: f32,
}

impl SlotJob<'_> {
	/// One pass per vertex: model copy, pose, placement, colour (PC perf: no separate copy and pose passes).
	/// `dst` / `model`: the slot's vertices from `off` on.
	#[cfg(test)]
	pub(in crate::viewer) fn fill(&self, dst: &mut [Vertex], model: &[Vertex], off: usize) {
		let (m, scale) = (&self.m, self.scale);
		for (j, (v, m0)) in dst.iter_mut().zip(model).enumerate().map(|(j, x)| (j + off, x)) {
			*v = *m0;
			if j < self.ps.len() {
				v.pos = self.ps[j];
				v.normal = self.ns[j];
			}
			v.uv[0] += self.du;
			v.uv[1] += self.dv;
			v.colour[3] = self.alpha_code;
			v.light = self.light;
			let (a, n) = (v.pos.map(|c| c * scale), v.normal);
			for i in 0..3 {
				v.pos[i] = m[i] * a[0] + m[4 + i] * a[1] + m[8 + i] * a[2] + m[12 + i];
				v.normal[i] = m[i] * n[0] + m[4 + i] * n[1] + m[8 + i] * n[2];
			}
			if let Some(c) = self.prop_tint {
				for i in 0..3 {
					v.colour[i] *= c[i];
				}
			}
			v.colour = tint_override(v.colour, j, self.tint, self.alpha);
		}
	}
}

/// `set_props`: what one slot's vertices are built from this frame.
enum SlotSrc<'a> {
	/// The plain hidden copy of the model.
	Hidden,
	/// SCH-6b deferred-list overflow: the model with the pose's normals, collapsed to `HIDDEN_POS`.
	Deferred { ps: &'a [[f32; 3]], ns: &'a [[f32; 3]] },
	Prop(SlotJob<'a>),
	/// [PC] A prop only the sun shadow pass poses (`gpuskin::KIND_CAST`, `culled_src`).
	Cast(SlotJob<'a>),
	/// A scene actor: root-relative skinned pose at a world offset, its light word, dynamic-light id and tint.
	Actor { ps: &'a [[f32; 3]], ns: &'a [[f32; 3]], at: [f32; 3], light: [f32; 4], id: f32, tint: Option<([f32; 3], usize)> },
}

/// [PC] perf round 3: the slot records of `vs_props` (gpuskin.rs) and the pose entries they read, each distinct pose
/// (same position and normal buffers, same length) once.
/// `rad` gets each slot's bounding radius about its model origin for the clipper's sphere pre-test (`model_rad`: the model's;
/// infinite for an actor with unposed vertices).
/// NOV-35b: the pose of slot `pi` (`len` model vertices; none = the static model). A pose may carry the class's metal vertices
/// after the LOD ones (`anim::skin_all`): its first positions and normals replace the model's.
fn slot_pose<'a>(fx_posed: &'a HashMap<usize, Posed>, posed: &'a HashMap<PoseKey, Posed>, pi: usize, p: &Prop, len: usize) -> (&'a [[f32; 3]], &'a [[f32; 3]]) {
	let pose = fx_posed.get(&pi).or_else(|| posed.get(&pose_key(p))).filter(|(ps, _)| ps.len() >= len);
	pose.map_or((&[], &[]), |(ps, ns)| (&ps[..ps.len().min(ns.len())], &ns[..ps.len().min(ns.len())]))
}

/// RAC_WATER_LOG: a prop slot drawn with a class texture scroll.
fn log_uv(p: &Prop, pi: usize, m: &[f32; 16], scale: f32, cuv: [f32; 2]) {
	if water_log() && cuv != [0.0; 2] {
		eprintln!("water: uv-scroll class {} slot {pi} at {:?} scale {scale} uv {cuv:?}", p.class_id, [m[12], m[13], m[14]]);
	}
}

/// [PC] The slot of a prop the game camera alone keeps out of the picture: retail's draw culled it (`Prop::cast`: occlusion
/// cells, near / side planes) or our occlusion test drops it. It is still in the world, so it casts its sun shadow wherever
/// the camera looks (a crane beside the screen kept its shadow on the visible ground only while the crane was in view);
/// with Culling Off the main pass draws it too, lit without a dynamic light (the drawn props' light ids stay as they are).
/// [PC] Culling Off: no draw-distance fade or cut for any prop (`moby_draw_alpha` of a whole fade: a moby's own +0x23 alpha
/// stays), with which props were past their distance (they take `culled_src`: drawn, but no retail verdict). Culling On:
/// `alphas` as they are and no list.
fn open_alphas(alphas: Vec<Option<u8>>, props: &[Prop], cull: bool) -> (Vec<Option<u8>>, Vec<bool>) {
	if cull {
		return (alphas, Vec::new());
	}
	let past = alphas.iter().map(Option::is_none).collect();
	(props.iter().map(|p| Some(moby_draw_alpha(0x80, p.moby_alpha()))).collect(), past)
}

fn culled_src<'a>(pi: usize, p: &Prop, ps: &'a [[f32; 3]], ns: &'a [[f32; 3]], alpha: u8, cull: bool, cuv: [f32; 2]) -> SlotSrc<'a> {
	let (m, scale) = super::smooth::prop_xf(pi).unwrap_or_else(|| (place_mirrored(p.pos, p.rot, p.mirror), p.scale));
	let tint = PROP_TINTS.with(|t| t.borrow().get(pi).copied().flatten());
	let job = SlotJob { ps, ns, m, scale, light: Gpu::moby_light(p.light), alpha_code: moby_alpha_encoding(0.0, alpha).0, prop_tint: p.tint, tint, alpha, dv: cuv[1], du: cuv[0] };
	if cull {
		SlotSrc::Cast(job)
	} else {
		SlotSrc::Prop(job)
	}
}

fn build_records(srcs: &[SlotSrc], ranges: &[std::ops::Range<usize>], model_rad: &[f32], inst: &mut Vec<super::gpuskin::PropInst>, pose: &mut Vec<super::gpuskin::PoseEntry>, rad: &mut Vec<f32>) {
	use super::gpuskin::{PropInst, KIND_ACTOR, KIND_CAST, KIND_DEFERRED, KIND_PROP};
	inst.clear();
	pose.clear();
	rad.clear();
	let mut bases: HashMap<(usize, usize, usize), (u32, f32)> = HashMap::new();
	let mut add = |ps: &[[f32; 3]], ns: &[[f32; 3]], n: usize| -> (u32, f32) {
		*bases.entry((ps.as_ptr() as usize, ns.as_ptr() as usize, n)).or_insert_with(|| {
			let b = pose.len() as u32;
			pose.extend(ps[..n].iter().zip(&ns[..n]).map(|(p, q)| [p[0], p[1], p[2], q[0], q[1], q[2]]));
			(b, ps[..n].iter().map(|p| (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt()).fold(0.0, f32::max))
		})
	};
	for (si, (src, r)) in srcs.iter().zip(ranges).enumerate() {
		let len = r.len();
		let mrad = model_rad.get(si).copied().unwrap_or(f32::INFINITY);
		let mut bound = mrad;
		let mut add = |ps: &[[f32; 3]], ns: &[[f32; 3]], n: usize, actor: bool| -> u32 {
			let (b, pr) = add(ps, ns, n);
			bound = match (n == len, actor) {
				(true, _) => pr,
				(false, false) => pr.max(mrad),
				(false, true) => f32::INFINITY,
			};
			b
		};
		inst.push(match src {
			SlotSrc::Hidden => PropInst::hidden(),
			SlotSrc::Deferred { ps, ns } => {
				let n = ps.len().min(ns.len()).min(len);
				PropInst { k: [KIND_DEFERRED, u32::MAX, add(ps, ns, n, false), n as u32], ..PropInst::hidden() }
			}
			SlotSrc::Prop(job) | SlotSrc::Cast(job) => {
				let kind = if matches!(src, SlotSrc::Cast(_)) { KIND_CAST } else { KIND_PROP };
				let n = job.ps.len().min(job.ns.len()).min(len);
				PropInst {
					m: job.m,
					light: job.light,
					tint: job.tint.map_or([0.0; 4], |(c, _)| [c[0], c[1], c[2], moby_baked_encoding(job.alpha)]),
					ptint: job.prop_tint.map_or([0.0; 4], |c| [c[0], c[1], c[2], 1.0]),
					a: [job.scale, job.alpha_code, job.dv, job.du],
					k: [kind, job.tint.map_or(u32::MAX, |(_, s)| s.min(u32::MAX as usize) as u32), add(job.ps, job.ns, n, false), n as u32],
				}
			}
			SlotSrc::Actor { ps, ns, at, light, id, tint } => {
				let n = ps.len().min(ns.len()).min(len);
				let mut m = [0.0; 16];
				m[12..15].copy_from_slice(at);
				PropInst {
					m,
					light: *light,
					tint: tint.map_or([0.0; 4], |(c, _)| [c[0], c[1], c[2], -2.0]),
					ptint: [0.0; 4],
					a: [0.0, *id, 0.0, 0.0],
					k: [KIND_ACTOR, tint.map_or(u32::MAX, |(_, s)| s.min(u32::MAX as usize) as u32), add(ps, ns, n, true), n as u32],
				}
			}
		});
		rad.push(bound);
	}
}
