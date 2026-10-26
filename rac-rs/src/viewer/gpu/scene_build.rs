//! Level scene build (terrain, ties, shrubs, sky) on the build context, and its parallel vertex helpers.

use super::*;

impl Gpu {
	pub(in crate::viewer) fn load_scene(&mut self, meshes: &[Mesh], shrub_set: &[shrubs::ShrubInstance], tie_set: &[ties::LitTie], textures: &HashMap<u32, Texture>, sky: Option<&sky::Sky>, hero: Option<&Mesh>, hero_tint: Option<([f32; 3], usize, usize)>, props: &[u32], prop_meshes: &HashMap<u32, Mesh>, preview_meshes: &HashMap<u32, Mesh>, occ: crate::occlusion::Occlusion, tfrag_set: Vec<terrain::TfragLods>, tfrag_b: f32, terrain_chains: &[Option<Vec<Texture>>], shrub_chains: &[Option<Vec<Texture>>], tie_chains: &[Option<Vec<Texture>>]) {
		let order = props.to_vec();
		let scene = self.ctx().build_scene(meshes, shrub_set, tie_set, textures, sky, hero, hero_tint, move |_| Some(order.clone()), prop_meshes, preview_meshes, occ, tfrag_set, tfrag_b, terrain_chains, shrub_chains, tie_chains);
		if std::env::var("RAC_PERF_LOG").is_ok_and(|v| v == "1") {
			if let Some(sc) = &scene {
				eprintln!("scene: {} prop slots, {} prop vertices", sc.props_ranges.len(), sc.props_model.len());
			}
		}
		self.scene = scene;
	}
}

/// [PC] perf: the vertex count of one work item of the frame worker pool (`par_chunked`, the marker build).
const PAR_CHUNK: usize = 4096;

/// [PC] perf round 3: per tie vertex the index of its slot triple in its (instance, LOD) list of distinct fat-vertex triples
/// (u32::MAX for a regular vertex), and those lists: `cull_ties` blends each triple once per rewrite.
pub(super) fn tie_triples(slots: &[ties::TieSlot], vranges: &[[std::ops::Range<usize>; 3]]) -> (Vec<u32>, Vec<[Vec<[usize; 3]>; 3]>) {
	let mut tri = vec![u32::MAX; slots.len()];
	let lists = vranges
		.iter()
		.map(|lods| {
			lods.clone().map(|r| {
				let (mut list, mut at): (Vec<[usize; 3]>, HashMap<[usize; 3], u32>) = (Vec::new(), HashMap::new());
				for k in r {
					if slots[k].1 {
						tri[k] = *at.entry(slots[k].0).or_insert_with(|| {
							list.push(slots[k].0);
							list.len() as u32 - 1
						});
					}
				}
				list
			})
		})
		.collect();
	(tri, lists)
}

/// [PC] perf: `par_ranges` with every range cut into pieces of at most `PAR_CHUNK` vertices: `f(job, offset in the job's
/// range, piece)`. For per-vertex work that does not depend on the other vertices of its range.
pub(super) fn par_chunked(verts: &mut [Vertex], ranges: &[std::ops::Range<usize>], f: impl Fn(usize, usize, &mut [Vertex]) + Sync) {
	if ranges.windows(2).any(|w| w[1].start < w[0].end) {
		for (k, r) in ranges.iter().enumerate() {
			f(k, 0, &mut verts[r.clone()]);
		}
		return;
	}
	let mut pieces: Vec<(usize, usize, &mut [Vertex])> = Vec::with_capacity(ranges.len());
	let (mut rest, mut at) = (verts, 0);
	for (k, r) in ranges.iter().enumerate() {
		let (_, tail) = std::mem::take(&mut rest).split_at_mut(r.start - at);
		let (mut mid, tail) = tail.split_at_mut(r.len());
		let mut off = 0;
		while mid.len() > PAR_CHUNK {
			let (a, b) = std::mem::take(&mut mid).split_at_mut(PAR_CHUNK);
			pieces.push((k, off, a));
			(mid, off) = (b, off + PAR_CHUNK);
		}
		pieces.push((k, off, mid));
		(rest, at) = (tail, r.end);
	}
	pieces.into_par_iter().for_each(|(k, off, v)| f(k, off, v));
}

/// [PC] perf: `f(job, slice)` for every range of `verts` on the frame worker pool (`init_pool`), results in job order. Each
/// job owns its slice, so the vertices are the same as one thread's; ranges that overlap or are out of ascending order run
/// on this thread in job order.
pub(super) fn par_ranges<R: Send>(verts: &mut [Vertex], ranges: &[std::ops::Range<usize>], f: impl Fn(usize, &mut [Vertex]) -> R + Sync) -> Vec<R> {
	if ranges.windows(2).any(|w| w[1].start < w[0].end) {
		return ranges.iter().enumerate().map(|(k, r)| f(k, &mut verts[r.clone()])).collect();
	}
	let mut slices: Vec<&mut [Vertex]> = Vec::with_capacity(ranges.len());
	let (mut rest, mut at) = (verts, 0);
	for r in ranges {
		let (_, tail) = std::mem::take(&mut rest).split_at_mut(r.start - at);
		let (mid, tail) = tail.split_at_mut(r.len());
		slices.push(mid);
		(rest, at) = (tail, r.end);
	}
	slices.into_par_iter().enumerate().map(|(k, v)| f(k, v)).collect()
}

impl SceneCtx {
	pub(in crate::viewer) fn build_scene(&self, meshes: &[Mesh], shrub_set: &[shrubs::ShrubInstance], tie_set: &[ties::LitTie], textures: &HashMap<u32, Texture>, sky: Option<&sky::Sky>, hero: Option<&Mesh>, hero_tint: Option<([f32; 3], usize, usize)>, mut props_fn: impl FnMut(Option<&[u32]>) -> Option<Vec<u32>>, prop_meshes: &HashMap<u32, Mesh>, preview_meshes: &HashMap<u32, Mesh>, occ: crate::occlusion::Occlusion, tfrag_set: Vec<terrain::TfragLods>, tfrag_b: f32, terrain_chains: &[Option<Vec<Texture>>], shrub_chains: &[Option<Vec<Texture>>], tie_chains: &[Option<Vec<Texture>>]) -> Option<Scene> {
		let (mut bind, mut slots) = (Vec::new(), HashMap::new());
		*self.tex_log.borrow_mut() = self.bindless_layout.is_some().then(HashMap::new); self.repl_begin(); // [PC] texture packs: take this level's replacements
		let refs: Vec<&Mesh> = meshes.iter().collect();
		let (world, lo, hi, world_verts) = self.batch(&refs, None, false, textures, &mut bind, &mut slots);
		crate::loadlog::say(|| "gpu: world batch".into());
		// Ties: three LOD meshes per instance in their own batch (BRIDGE-29); the bounds still take LOD 0 so the scene
		// centre and radius are what the merged world batch gave.
		let tie_refs: Vec<&Mesh> = tie_set.iter().flat_map(|t| t.lods.iter()).collect();
		let (mut ties, _, _, tie_verts, tie_vr) = self.batch_instances(&tie_refs, textures, &mut bind, &mut slots, None, tie_chains, if super::tiegpu::wanted() { wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC } else { wgpu::BufferUsages::empty() });
		crate::loadlog::say(|| "gpu: ties batch".into());
		let tie_draws = std::mem::take(&mut ties.draws);
		let (mut lo, mut hi) = (lo, hi);
		for (_, vr) in tie_vr.iter().step_by(3) {
			for p in &tie_verts[vr.clone()] {
				for j in 0..3 {
					lo[j] = lo[j].min(p.pos[j]);
					hi[j] = hi[j].max(p.pos[j]);
				}
			}
		}
		let tie_ranges: Vec<[std::ops::Range<u32>; 3]> = tie_vr.chunks(3).map(|c| std::array::from_fn(|k| c[k].0.start as u32..c[k].0.end as u32)).collect();
		let tie_vranges: Vec<[std::ops::Range<usize>; 3]> = tie_vr.chunks(3).map(|c| std::array::from_fn(|k| c[k].1.clone())).collect();
		let tie_morph: Vec<[f32; 3]> = tie_set.iter().flat_map(|t| t.morph.iter().flatten().copied()).collect();
		let tie_fat: Vec<[bool; 3]> = tie_set.iter().map(|t| std::array::from_fn(|k| t.morph[k].iter().any(|d| d.iter().any(|&c| c != 0.0)))).collect();
		let tie_lit: Vec<[&Mesh; 3]> = tie_set.iter().map(|t| [&t.lods[0], &t.lods[1], &t.lods[2]]).collect();
		// Terrain fragments (BRIDGE-31): all three LOD meshes of each in their own batch; the near meshes join the scene bounds.
		// Their face keys carry the material record index (REN-TEX-RESIDENCY: K differs per record) and the textures upload their
		// authored mip chains; the tfrag pass samples an explicit level.
		let tf_meshes: Vec<Mesh> = tfrag_set.iter().flat_map(|t| [(&t.far, &t.face_mat[0]), (&t.mid, &t.face_mat[1]), (&t.near, &t.face_mat[2])]).map(|(m, mats)| terrain_face_keys(m, mats)).collect();
		let tf_refs: Vec<&Mesh> = tf_meshes.iter().collect();
		let rule = TerrainRule { chains: terrain_chains, materials: tfrag_set.iter().flat_map(|t| [&t.materials[..]; 3]).collect() };
		let (mut tfrags, _, _, tfrag_verts, tf_vr) = self.batch_instances(&tf_refs, textures, &mut bind, &mut slots, Some(&rule), &[], wgpu::BufferUsages::empty());
		crate::loadlog::say(|| "gpu: tfrag batch".into());
		let tfrag_draws = std::mem::take(&mut tfrags.draws);
		for (_, vr) in tf_vr.iter().skip(2).step_by(3) {
			for p in &tfrag_verts[vr.clone()] {
				for j in 0..3 {
					lo[j] = lo[j].min(p.pos[j]);
					hi[j] = hi[j].max(p.pos[j]);
				}
			}
		}
		let tfrag_ranges: Vec<[std::ops::Range<u32>; 3]> = tf_vr.chunks(3).map(|c| std::array::from_fn(|k| c[k].0.start as u32..c[k].0.end as u32)).collect();
		let tfrag_vranges: Vec<[std::ops::Range<usize>; 3]> = tf_vr.chunks(3).map(|c| std::array::from_fn(|k| c[k].1.clone())).collect();
		let tfrag_cols: Vec<Vec<[f32; 4]>> = tfrag_set.iter().map(|t| t.lit.colours.clone()).collect();
		let tfrag_lit: Vec<&Mesh> = tfrag_set.iter().map(|t| &t.lit).collect();
		// Shrubs (BRIDGE-27): one batch with per-instance ranges, bounds merged into the scene's, chained last in the light world.
		let shrub_refs: Vec<&Mesh> = shrub_set.iter().map(|s| &s.mesh).collect();
		let (shrub_batch, slo, shi, shrub_verts, shrub_ranges) = self.batch_instances(&shrub_refs, textures, &mut bind, &mut slots, None, shrub_chains, wgpu::BufferUsages::empty());
		crate::loadlog::say(|| "gpu: shrub batch".into());
		let shrub_vkey = super::icefx::vertex_keys(shrub_refs.iter().copied().zip(shrub_ranges.iter().map(|r| r.1.start)), shrub_verts.len());
		for j in 0..3 {
			lo[j] = lo[j].min(slo[j]);
			hi[j] = hi[j].max(shi[j]);
		}
		let lit = dynlight::LitWorld::chain_tfrags(&tfrag_lit, meshes, &tie_lit, shrub_refs.iter().copied());
		crate::loadlog::say(|| "gpu: lit chain".into());
		let tie_first = world_verts.len();
		let shrub_first = tie_first + tie_verts.len();
		let tie_base: Vec<[f32; 3]> = tie_verts.iter().map(|v| v.pos).collect();
		// The LOD 1 / 2 meshes carry no `Lit`, so they take LOD 0's fog class tag and instance centre here.
		let mut tie_verts = tie_verts;
		let tie_vkey = super::icefx::vertex_keys(tie_refs.iter().copied().zip(tie_vr.iter().map(|r| r.1.start)), tie_verts.len());
		for r in &tie_vranges {
			if let Some(l) = tie_verts.get(r[0].start).map(|v| v.light) {
				for lod in 1..3 {
					for v in &mut tie_verts[r[lod].clone()] {
						v.light = l;
					}
				}
			}
		}
		self.wb(&ties.vbuf, 0, bytemuck::cast_slice(&tie_verts));
		let tie_slots: Vec<ties::TieSlot> = tie_set.iter().flat_map(|t| t.slots.iter().flatten().copied()).collect();
		let tie_cols: Vec<Vec<[f32; 4]>> = tie_set.iter().map(|t| t.cols.clone()).collect();
		let tie_stale = vec![[false; 3]; tie_set.len()];
		let mut clear = wgpu::Color { r: 0.05, g: 0.07, b: 0.15, a: 1.0 };
		let sky_src = sky;
		crate::loadlog::say(|| "gpu: ties written".into());
		let (sky, sky_shells) = sky
			.map(|s| {
				let mut t: HashMap<u32, Texture> = s.textures.iter().map(|(k, v)| (*k, Texture { w: v.w, h: v.h, rgba: v.rgba.clone() })).collect();
				t.insert(sky::UNTEXTURED, Texture { w: 1, h: 1, rgba: vec![[255; 4]] });
				let c = s.colour.map(|x| x as f64 / 255.0); // the target is gamma-space Unorm: raw GS bytes
				clear = wgpu::Color { r: c[0], g: c[1], b: c[2], a: 1.0 };
				// Retail draws the sky shells with CLAMP_1 = 5 (both axes clamped; fx_census_late.md GS census): wrap 3 in the face key.
				let clamped: Vec<Mesh> = s.meshes.iter().map(|m| Mesh { faces: m.faces.iter().map(|&(k, f)| (if k == sky::UNTEXTURED { k } else { k | 3 << shrubs::WRAP_SHIFT }, f)).collect(), ..m.clone() }).collect();
				let refs: Vec<&Mesh> = clamped.iter().collect();
				let (batch, _, _, verts) = self.batch(&refs, Some(&s.colours), true, &t, &mut bind, &mut slots);
				// The shells' vertices follow each other in shell order (an ordered batch).
				let mut start = 0;
				let ranges = s
					.meshes
					.iter()
					.map(|m| {
						let r = start..start + m.positions.len();
						start = r.end;
						r
					})
					.collect();
				let base = s.meshes.iter().map(|m| m.positions.clone()).collect();
				// ARI-4: draws of shells 0 and 1 (no run straddles a shell).
				let first_two = s.meshes.iter().take(2).map(|m| m.faces.len() as u32 * 3).sum::<u32>();
				let split = batch.draws.iter().take_while(|d| d.start < first_two).count();
				(batch, SkyShells { verts, ranges, base, turned: vec![false; s.meshes.len()], posed: None, split })
			})
			.unzip();
		let sky_sprites = sky_src.and_then(|s| {
			let keys = [4 << 16 | 2, 4 << 16 | 3];
			if !keys.iter().all(|k| s.textures.contains_key(k)) {
				return None;
			}
			let bind_of = |k: u32, bind: &mut Vec<wgpu::BindGroup>, slots: &mut HashMap<u32, usize>| match slots.get(&k) {
				Some(&i) => i,
				None => {
					bind.push(self.upload_texture(s.textures.get(&k)));
					slots.insert(k, bind.len() - 1);
					bind.len() - 1
				}
			};
			let tex = keys.map(|k| bind_of(k, &mut bind, &mut slots));
			let n = sky::SKY_SPRITES;
			let vbuf = buffer(&self.device, "sky sprites", bytemuck::cast_slice(&vec![Vertex::zeroed(); n * 4]), wgpu::BufferUsages::VERTEX);
			let idx: Vec<u32> = (0..n as u32).flat_map(|q| [0, 1, 2, 1, 2, 3].map(|i| q * 4 + i)).collect();
			let ibuf = buffer(&self.device, "sky sprite indices", bytemuck::cast_slice(&idx), wgpu::BufferUsages::INDEX);
			self.wb(&ibuf, 0, bytemuck::cast_slice(&idx));
			Some(SkySpriteDraw { vbuf, ibuf, bind: tex, counts: [0; 2] })
		});
		crate::loadlog::say(|| "gpu: sky done".into());
		let centre = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
		let d = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
		let radius = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() / 2.0;
		// The player model, in model space (feet at the origin, facing +X).
		const BOX: u32 = u32::MAX;
		let (boxed, mut t) = (box_mesh(BOX), HashMap::new());
		let hero = match hero {
			Some(m) => (m, textures),
			None => {
				t.insert(BOX, Texture { w: 1, h: 1, rgba: vec![[255, 140, 30, 255]] });
				(&boxed, &t)
			}
		};
		let (marker, _, _, marker_model) = self.batch(&[hero.0], None, false, hero.1, &mut bind, &mut slots);
		// [PC] perf (lane R2, gpuhero.rs): with RAC_HERO_GPU=1 the marker's vertex buffer is also a storage buffer a compute pass fills.
		let (marker, marker_gpu) = if super::gpuhero::gpu_hero_on() && !marker_model.is_empty() {
			let vb = sized_buffer(&self.device, "marker vertices", marker_model.len() * std::mem::size_of::<Vertex>(), wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::STORAGE);
			self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&marker_model));
			let hg = super::gpuhero::HeroGpu::new(&self.device, &self.queue, &marker_model, &vb);
			(Batch { vbuf: vb, ..marker }, hg)
		} else {
			(marker, None)
		};
		crate::loadlog::say(|| "gpu: marker".into());
		let marker_faces = super::clip::replay_faces(&[hero.0], &slots).pop().unwrap_or_default();
		// Gameplay props: one mesh copy per slot, posed every frame.
		// [PC] The level prefetch may build them for a speculative order (retail's pools a few fields before arrival): after the build
		// `props_fn(Some(built))` answers the order to keep (== built) or a newer one, rebuilt from the same texture slots (`bind`/`slots`
		// back to what they were here), so the result is the build of the final order alone.
		// (Before the prop slots: the prefetch thread has them done when the final order comes.)
		let tie_qdep: Vec<bool> = tie_slots.iter().zip(&tie_morph).map(|(s, d)| s.1 || *d != [0.0; 3]).collect();
		let (tie_tri, tie_tris) = tie_triples(&tie_slots, &tie_vranges);
		let tie_gpu = super::tiegpu::TieGpu::new(&self.device, &self.queue, &ties.vbuf, &tie_base, &tie_morph, &tie_slots, &tie_qdep, &tie_vranges, &tie_cols);
		let (bind0, slots0) = (bind.len(), slots.clone());
		let mut order_p = props_fn(None)?;
		let (ranges, prompt_slot, props, props_model, slot_runs, prop_faces, props_sj, props_rad, props_class, class_faces, prompt, portrait, portrait_model, clip_cap, clip_batch, previews, preview_models, previews_gold) = loop {
		let PropSlots { ranges, prompt_slot, batch: props, model: props_model, slot_runs, faces: prop_faces, sj: props_sj, rad: props_rad, classes: props_class, class_faces } = self.prop_slots(&order_p, prop_meshes, textures, &mut bind, &mut slots);
		crate::loadlog::say(|| "gpu: prop_slots".into());
		// BRIDGE-36 (c1): the purchase prompt (class 1137) as its own one-prop batch, drawn between HUD groups at `hud_split`.
		let prompt = prompt_slot.and_then(|_| prop_meshes.get(&game::vendor::PROMPT_CLASS)).filter(|m| !m.positions.is_empty()).map(|m| self.batch(&[m], None, false, textures, &mut bind, &mut slots).0);
		let prompt_slot = prompt_slot.filter(|_| prompt.is_some());
		// BRIDGE-37 (b): the shop portrait (class 12) as a one-prop batch, posed by `pose_portrait`.
		let (portrait, portrait_model) = match prop_meshes.get(&game::vendor::PORTRAIT_CLASS).filter(|m| !m.positions.is_empty()) {
			Some(m) => {
				let (b, _, _, verts) = self.batch(&[m], None, false, textures, &mut bind, &mut slots);
				(Some(b), verts)
			}
			None => (None, Vec::new()),
		};
		crate::loadlog::say(|| "gpu: prompt/portrait".into());
		let clip_cap = (CLIP_VERTS, CLIP_INDICES);
		let clip_batch = Batch { vbuf: buffer(&self.device, "clip vertices", &vec![0u8; CLIP_VERTS * std::mem::size_of::<Vertex>()], wgpu::BufferUsages::VERTEX), ibuf: buffer(&self.device, "clip indices", &vec![0u8; CLIP_INDICES * 4], wgpu::BufferUsages::INDEX), draws: Vec::new() };
		crate::loadlog::say(|| "gpu: clip batch".into());
		// Menu item previews: bind pose, centred by bounding box, lit as menu mobies (FUN_00282a70: ambient 0x20,
		// light slots 14/14, blend 0; lighting_fog.md "Reserved level-light slots 12-14").
		let mut previews = HashMap::new();
		let mut preview_models = HashMap::new();
		let mut previews_gold = HashMap::new();
		for (&class, m) in preview_meshes.iter().filter(|(_, m)| !m.positions.is_empty()) {
			let (batch, lo, hi, mut verts) = self.batch(&[m], None, false, textures, &mut bind, &mut slots);
			for v in &mut verts {
				v.colour[3] = 0.0;
				v.light = Gpu::pack_light([0.25; 3], MENU_LIGHT_SLOT, MENU_LIGHT_SLOT, 0.0);
			}
			self.wb(&batch.vbuf, 0, bytemuck::cast_slice(&verts));
			// LIT-7a: a second copy lit by the gold-weapon light word (ambient 38 / 28 / 0, slots 12 / 12).
			let (gold, _, _, mut gverts) = self.batch(&[m], None, false, textures, &mut bind, &mut slots);
			for v in &mut gverts {
				v.colour[3] = 0.0;
				v.light = Gpu::pack_light(GOLD_AMBIENT, GOLD_LIGHT_SLOT, GOLD_LIGHT_SLOT, 0.0);
			}
			self.wb(&gold.vbuf, 0, bytemuck::cast_slice(&gverts));
			previews_gold.insert(class, gold);
			// Every class keeps its bind vertices: hero previews re-pose them, and the shop panel item A (`pose_panel_item`).
			preview_models.insert(class, verts);
			let d = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
			previews.insert(class, (batch, [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0], (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() / 2.0));
		}
		let keep = props_fn(Some(&order_p))?;
		if keep == order_p {
			break (ranges, prompt_slot, props, props_model, slot_runs, prop_faces, props_sj, props_rad, props_class, class_faces, prompt, portrait, portrait_model, clip_cap, clip_batch, previews, preview_models, previews_gold);
		}
		crate::loadlog::say(|| "gpu: prop slots rebuilt for a newer order".into());
		bind.truncate(bind0);
		slots = slots0.clone();
		order_p = keep;
		};
		// [PC] perf round 3 (bindless.rs): the scene's texture slots as one array for the tie and shrub draws.
		let views = self.tex_log.borrow_mut().take();
		crate::loadlog::say(|| "gpu: previews done".into());
		let tex_array = self.bindless_layout.as_ref().zip(views.as_ref()).and_then(|(l, v)| super::bindless::array_group(l, &self.device, &bind, v, &self.sampler, &self.wrap_samplers));
		// The first-instance words of the moby draws, by `bind_groups` index (`moby_word`).
		let mut slot_words = vec![0u32; bind.len()];
		for (&key, &i) in &slots {
			slot_words[i] = moby_word(key, textures.get(&(key & crate::mobies::KEY_MASK)));
		}
		Some(Scene { previews, previews_gold, preview_models, world, world_verts, lit, ties, tie_draws, tie_ranges, tie_cull: tie_set.iter().map(|t| t.cull).collect(), tie_lod: vec![u8::MAX; tie_set.len()], occ_tick: occ.clone(), occ, tfrags, tfrag_set, tfrag_draws, tfrag_ranges, tfrag_vranges, tfrag_verts, tfrag_cols, tfrag_b, tfrag_lod: Vec::new(), tie_verts, tie_base, tie_morph, tie_vranges, tie_fat, tie_q: vec![[0.0; 3]; tie_set.len()], tie_qdep, tie_full: vec![[false; 3]; tie_set.len()], tie_slots, tie_tri, tie_tris, tie_cols, tie_stale, tie_first, shrubs: shrub_batch, shrub_ranges, shrub_cull: shrub_set.iter().map(|s| s.cull).collect(), shrub_order: shrub_order(shrub_set), shrub_bb: shrub_set.iter().map(|s| s.bb).collect(), shrub_verts, shrub_first, tie_vkey, shrub_soft: super::icefx::soft_keys(&shrub_vkey, textures), shrub_vkey, ice_level: usize::MAX, shrub_a: vec![0; shrub_set.len()], shrub_opaque: Vec::new(), shrub_faded: Vec::new(), shrub_bb_quads: Vec::new(), shrub_bb_tex: HashMap::new(), sky, sky_shells, sky_sprites, stars: crate::viewer::starfield::build(sky_src, &slots) /* [PC] Starfield */, marker, marker_verts: Vec::new(), marker_model, marker_tint: hero_tint.map(|(c, s, _)| (c, s)), marker_hero_len: hero_tint.map_or(0, |t| t.2), marker_shown: false, marker_alpha: false, props, props_ibuf_spare: None, props_model, props_sj, props_rad, props_inst: Vec::new(), props_pose: Vec::new(), props_ranges: ranges, props_shown: false, props_alpha: false, props_drawn: Vec::new(), props_vis: Vec::new(), slot_runs, props_order: (0..props_class.len()).collect(), props_class, props_late: Vec::new(), prop_faces, class_faces, clip_masked: Vec::new(), clip_batch, clip_cap, clip_view: None, clip_queue: Vec::new(), marker_faces, marker_masked: Vec::new(), marker_key: None, marker_skin: (Vec::new(), Vec::new()), marker_gpu, tie_pass: None, tie_jobs: Vec::new(), tie_gpu, debris_model: HashMap::new(), debris: None, debris_cap: (0, 0), warp_model: HashMap::new(), warp_batch: None, warp_shown: false, prompt, prompt_slot, portrait, portrait_model, clear_tex: clear_slots(&slots, bind.len(), textures), orig: self.orig_end(&bind, views.as_ref()) /* [PC] split view with a pack */, repl: self.repl_end(&bind, views), bind_groups: bind, tex_slots: slots, slot_words, tex_array, clear, centre, radius })
	}
}
