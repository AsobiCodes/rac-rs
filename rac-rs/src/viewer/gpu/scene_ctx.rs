//! SceneCtx (the device handles of the scene build) and its texture upload and batching, with the Gpu wrappers around it.

use super::*;

impl Gpu {
	pub(in crate::viewer) fn ctx(&self) -> SceneCtx {
		SceneCtx { device: self.device.clone(), queue: self.queue.clone(), sampler: self.sampler.clone(), wrap_samplers: self.wrap_samplers.clone(), tex_layout: self.tex_layout.clone(), tex_log: std::cell::RefCell::new(None), repl: Default::default(), repl_pending: Default::default(), repl_cap: Default::default(), repl_origs: Default::default(), orig_on: Default::default(), extra_origs: Default::default(), bindless_layout: self.bindless.as_ref().map(|b| b.layout.clone()), paced: false }
	}

	pub(in crate::viewer) fn upload(&self, t: Option<&Texture>, sampler: &wgpu::Sampler) -> wgpu::BindGroup {
		self.ctx().upload(t, sampler)
	}

	pub(in crate::viewer) fn upload_texture(&self, t: Option<&Texture>) -> wgpu::BindGroup {
		self.ctx().upload_texture(t)
	}

	pub(in crate::viewer) fn upload_chain(&self, chain: &[Texture], sampler: &wgpu::Sampler) -> wgpu::BindGroup {
		self.ctx().upload_chain(chain, sampler)
	}

	pub(in crate::viewer) fn batch(&self, meshes: &[&Mesh], colours: Option<&[Vec<[f32; 4]>]>, ordered: bool, textures: &HashMap<u32, Texture>, bind: &mut Vec<wgpu::BindGroup>, slots: &mut HashMap<u32, usize>) -> (Batch, [f32; 3], [f32; 3], Vec<Vertex>) {
		self.ctx().batch(meshes, colours, ordered, textures, bind, slots)
	}
}

/// `buffer` of `bytes` (rounded up to 4, at least 4) for data written later (copyable: `append_props` grows the prop
/// buffers by a GPU copy).
pub(super) fn sized_buffer(device: &wgpu::Device, label: &str, bytes: usize, usage: wgpu::BufferUsages) -> wgpu::Buffer {
	device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: (bytes as u64).max(4).next_multiple_of(4), usage: usage | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false })
}

pub(super) fn push_mesh_vertices(m: &Mesh, mi: usize, colours: Option<&[Vec<[f32; 4]>]>, verts: &mut Vec<Vertex>, lo: &mut [f32; 3], hi: &mut [f32; 3]) {
	// Moby origin (w 1), or the world fog class tag in w (-1 tfrag, -2 shrub, -3 tie; instance centre in xyz).
	let origin = m.lit.as_ref().map_or([0.0; 4], |l| {
		let c = [l.sphere[0], l.sphere[1], l.sphere[2]];
		match l.kind {
			LitKind::Moby => [c[0], c[1], c[2], 1.0],
			_ if colours.is_some() => [0.0; 4],
			LitKind::Tfrag => [0.0, 0.0, 0.0, -1.0],
			LitKind::Shrub => [l.origin[0], l.origin[1], l.origin[2], -2.0],
			LitKind::Tie => [c[0], c[1], c[2], -3.0],
		}
	});
	// A moby mesh keeps its decoded vertex normals (retail VU0 prog0 lights with them, lighting_fog.md REN-5c); other
	// meshes sum their face normals.
	let normals = m.vertex_normals();
	for (i, p) in m.positions.iter().enumerate() {
		for j in 0..3 {
			lo[j] = lo[j].min(p[j]);
			hi[j] = hi[j].max(p[j]);
		}
		// Baked world colours (PS2 bytes, 0x80 = 1.0) are flagged with alpha -2 for the shader.
		let colour = match (colours, m.colours.get(i)) {
			(Some(c), _) => c[mi][i],
			(None, Some(b)) => [b[0] / 128.0, b[1] / 128.0, b[2] / 128.0, -2.0],
			(None, None) => [1.0, 1.0, 1.0, -1.0],
		};
		verts.push(Vertex { pos: *p, uv: m.uvs[i], normal: normals[i], colour, light: origin });
	}
}

/// Index and draw layout of `Gpu::batch_instances`: each mesh's faces grouped by texture key (ascending), meshes in order.
/// Returns the index buffer, the draws as (key, first index, index count), and per mesh its range in the draw list and its
/// vertex range.
pub(super) fn instance_layout(meshes: &[&Mesh]) -> (Vec<u32>, Vec<(u32, u32, u32)>, Vec<(std::ops::Range<usize>, std::ops::Range<usize>)>) {
	let (mut indices, mut draws, mut ranges) = (Vec::new(), Vec::new(), Vec::new());
	let mut base = 0u32;
	for m in meshes {
		// Ordered by key without the shrub material bits (then the full key): keys without them keep their order.
		let mut by_tex: BTreeMap<(u32, u32), Vec<u32>> = BTreeMap::new();
		for &(key, f) in &m.faces {
			let bare = if crate::shrubs::key_k(key).is_some() { key & !crate::shrubs::MATERIAL_BITS } else { key & !(3 << crate::shrubs::WRAP_SHIFT) };
			by_tex.entry((bare, key)).or_default().extend(f.map(|i| i + base));
		}
		let first = draws.len();
		for ((_, key), idx) in by_tex {
			draws.push((key, indices.len() as u32, idx.len() as u32));
			indices.extend(idx);
		}
		let n = m.positions.len();
		ranges.push((first..draws.len(), base as usize..base as usize + n));
		base += n as u32;
	}
	(indices, draws, ranges)
}

/// The alpha carrier of a faded shrub mesh or a billboard in the vertex colour alpha: -2 - a / 256 (a 1..=0x80), decoded by the
/// shrub fragment entries; every value stays below the -1.5 baked-colour tag.
pub(super) fn shrub_carrier(a: u8) -> f32 {
	-2.0 - a as f32 / 256.0
}

/// Mip state of the terrain textures of a tfrag batch: the authored chains by texture index and the material records of
/// each mesh of the batch (by mesh index), which give the draw its K.
pub(in crate::viewer) struct TerrainRule<'a> {
	pub(super) chains: &'a [Option<Vec<Texture>>],
	pub(super) materials: Vec<&'a [terrain::TfragMaterial]>,
}

/// A tfrag mesh copy whose face keys carry the material record index in bits 8..16 next to the texture index.
pub(super) fn terrain_face_keys(m: &Mesh, mats: &[u8]) -> Mesh {
	let mut m = m.clone();
	for (f, &mat) in m.faces.iter_mut().zip(mats) {
		f.0 |= (mat as u32) << 8;
	}
	m
}

/// The first-instance word of a tfrag draw: the raw 12-bit K (signed, 1/16 mip) and the top mip level in bits 12..14.
pub(super) fn terrain_inst(k: f32, mxl: usize) -> u32 {
	((mxl.min(3) as u32) << 12) | ((k * 16.0) as i32 as u32 & 0xfff)
}

/// [PC] The device-side handles the scene build needs (wgpu handles are cheap clones and Send + Sync), so `build_scene` can run on a
/// background thread (the level prefetch) while the viewer keeps drawing the level that is still playing.
pub(in crate::viewer) struct SceneCtx {
	pub(in crate::viewer) device: wgpu::Device,
	pub(in crate::viewer) queue: wgpu::Queue,
	pub(in crate::viewer) sampler: wgpu::Sampler,
	pub(in crate::viewer) wrap_samplers: [wgpu::Sampler; 3],
	pub(in crate::viewer) tex_layout: wgpu::BindGroupLayout,
	/// While `build_scene` runs: the texture view of every bind group `upload` / `upload_chain` made with the scene sampler (bindless.rs).
	pub(in crate::viewer) tex_log: std::cell::RefCell<Option<HashMap<wgpu::BindGroup, wgpu::TextureView>>>,
	/// [PC] texture packs (repl_upload.rs): the level's replacements while `build_scene` runs, and the slots still waiting for theirs.
	pub(in crate::viewer) repl: std::cell::RefCell<Option<std::sync::Arc<crate::texpack::repl::LevelRepl>>>,
	pub(in crate::viewer) repl_pending: std::cell::RefCell<Vec<super::repl_upload::PendRaw>>,
	/// What this scene build may still bind of the Ready replacements (`repl_upload::BuildCap`, set by `repl_begin`).
	pub(in crate::viewer) repl_cap: std::cell::Cell<super::repl_upload::BuildCap>,
	/// [PC] split view with a pack (split_orig.rs): (replacement bind group, the original's) of every slot uploaded with its replacement, and whether this build keeps them.
	pub(in crate::viewer) repl_origs: std::cell::RefCell<Vec<(wgpu::BindGroup, wgpu::BindGroup)>>,
	pub(in crate::viewer) orig_on: std::cell::Cell<bool>,
	/// Split view: the originals of textures a load hook replaced in the CPU tables (the sky shells), by scene texture key (`extra_hd::take_extra_originals`).
	pub(in crate::viewer) extra_origs: std::cell::RefCell<HashMap<u32, Texture>>,
	/// The bindless array layout (None: per-draw bind groups).
	pub(in crate::viewer) bindless_layout: Option<wgpu::BindGroupLayout>,
	/// Set on the level-prefetch thread: big buffer writes go in 4 MB pieces (see `wb`).
	pub(in crate::viewer) paced: bool,
}

/// The authored mip chains of the level's moby textures by texture key, registered by the level loader next to `all_textures`
/// (`Level::mip_chains`; the same 16-byte table entry layout as terrain, ties and shrubs).
static MOBY_CHAINS: std::sync::Mutex<Option<std::sync::Arc<HashMap<u32, Vec<Texture>>>>> = std::sync::Mutex::new(None);

pub(in crate::viewer) fn register_moby_chains(chains: HashMap<u32, Vec<Texture>>) {
	*MOBY_CHAINS.lock().unwrap() = Some(std::sync::Arc::new(chains));
}

/// The authored chain of a moby draw slot (key with a TEX1 K code), only when its level 0 is exactly the texture the draw uses
/// (the warp actors' texture keys 0..2 are other textures than the level's).
fn moby_chain(slot_key: u32, t: Option<&Texture>) -> Option<std::sync::Arc<Vec<Texture>>> {
	let t = t?;
	if crate::mobies::key_k(slot_key).is_none() || (slot_key & crate::mobies::KEY_MASK) >> 16 != 1 {
		return None;
	}
	let all = MOBY_CHAINS.lock().unwrap().clone()?;
	let chain = all.get(&(slot_key & crate::mobies::KEY_MASK))?;
	chain.first().filter(|c| c.w == t.w && c.h == t.h && c.rgba == t.rgba)?;
	Some(std::sync::Arc::new(chain.iter().map(|m| Texture { w: m.w, h: m.h, rgba: m.rgba.clone() }).collect()))
}

/// The first-instance word of a moby draw slot (`vs` / `vs_props` `moby_kmx`): TEX1 K (bits 2..14) and the top level of the
/// authored chain (bits 14..16); 0 when the material has no K or its texture no authored chain (plain sample).
pub(super) fn moby_word(slot_key: u32, t: Option<&Texture>) -> u32 {
	match (crate::mobies::key_k(slot_key), moby_chain(slot_key, t)) {
		(Some(k), Some(chain)) => (k & 0xfff) << 2 | (chain.len() as u32 - 1) << 14,
		_ => 0,
	}
}

impl SceneCtx {
	/// `queue.write_buffer`, but on the level-prefetch thread (`paced`) in 4 MB pieces with a 1 ms pause after each:
	/// wgpu copies a write into its staging memory while holding the queue's pending-writes lock, so one 100 MB
	/// write would stall the main thread's acquire/submit for 100+ ms. Same bytes, same buffer, same order.
	pub(super) fn wb(&self, buf: &wgpu::Buffer, off: u64, data: &[u8]) {
		const CHUNK: usize = 4 << 20;
		if !self.paced || data.len() <= CHUNK {
			self.queue.write_buffer(buf, off, data);
			return;
		}
		for (k, c) in data.chunks(CHUNK).enumerate() {
			self.queue.write_buffer(buf, off + (k * CHUNK) as u64, c);
			std::thread::sleep(std::time::Duration::from_millis(1));
		}
	}

	pub(in crate::viewer) fn upload_texture(&self, t: Option<&Texture>) -> wgpu::BindGroup {
		self.upload(t, &self.sampler)
	}

	pub(in crate::viewer) fn upload(&self, t: Option<&Texture>, sampler: &wgpu::Sampler) -> wgpu::BindGroup {
		let fallback = Texture { w: 1, h: 1, rgba: vec![[200, 0, 200, 255]] };
		let t = t.unwrap_or(&fallback);
		let size = wgpu::Extent3d { width: t.w as u32, height: t.h as u32, depth_or_array_layers: 1 };
		// Mipmaps for the scene sampler only (HUD art is drawn near 1:1 with `nearest`).
		let mips = if *sampler == self.sampler { mip_chain(t) } else { Vec::new() };
		let tex = self.device.create_texture(&wgpu::TextureDescriptor {
			label: None,
			size,
			mip_level_count: 1 + mips.len() as u32,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::Rgba8Unorm,
			usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
			view_formats: &[],
		});
		self.queue.write_texture(
			tex.as_image_copy(),
			bytemuck::cast_slice(&t.rgba),
			wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(t.w as u32 * 4), rows_per_image: Some(t.h as u32) },
			size,
		);
		for (i, (w, h, px)) in mips.iter().enumerate() {
			self.queue.write_texture(
				wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: i as u32 + 1, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
				bytemuck::cast_slice(px),
				wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(*h) },
				wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 },
			);
		}
		let view = tex.create_view(&Default::default());
		let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: None,
			layout: &self.tex_layout,
			entries: &[
				wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
				wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
			],
		});
		// bindless.rs: `load_scene` collects the views of the scene's slots (any sampler: the array shader picks the shrub wrap
		// sampler itself; a missing view turned the array path off on levels with clamped shrub materials).
		if let Some(log) = self.tex_log.borrow_mut().as_mut() {
			log.insert(bind.clone(), view);
		}
		bind
	}

	/// Faces grouped by texture (sorted by key), or in mesh order when `ordered` (sky shells blend in
	/// sequence). `colours` gives per-mesh vertex colours; otherwise white.
	pub(in crate::viewer) fn batch(&self, meshes: &[&Mesh], colours: Option<&[Vec<[f32; 4]>]>, ordered: bool, textures: &HashMap<u32, Texture>, bind: &mut Vec<wgpu::BindGroup>, slots: &mut HashMap<u32, usize>) -> (Batch, [f32; 3], [f32; 3], Vec<Vertex>) {
		self.batch_spare(meshes, colours, ordered, textures, bind, slots, (0, 0))
	}

	/// `batch` with buffers `spare` (vertices, indices) larger than the data: the prop slots append grown retail pools
	/// there (`append_props`).
	pub(super) fn batch_spare(&self, meshes: &[&Mesh], colours: Option<&[Vec<[f32; 4]>]>, ordered: bool, textures: &HashMap<u32, Texture>, bind: &mut Vec<wgpu::BindGroup>, slots: &mut HashMap<u32, usize>, spare: (usize, usize)) -> (Batch, [f32; 3], [f32; 3], Vec<Vertex>) {
		let mut verts: Vec<Vertex> = Vec::new();
		let mut runs: Vec<(u32, Vec<u32>)> = Vec::new();
		let mut by_tex: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
		let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
		// ARI-4: an ordered run never spans two meshes (the sky sprites draw between shells 1 and 2).
		let mut run_mesh = usize::MAX;
		for (mi, m) in meshes.iter().enumerate() {
			let base = verts.len() as u32;
			push_mesh_vertices(m, mi, colours, &mut verts, &mut lo, &mut hi);
			for &(tex, f) in &m.faces {
				let f = f.map(|i| i + base);
				if !ordered {
					by_tex.entry(tex).or_default().extend(f);
				} else if let Some(r) = runs.last_mut().filter(|r| r.0 == tex && run_mesh == mi) {
					r.1.extend(f);
				} else {
					runs.push((tex, f.to_vec()));
					run_mesh = mi;
				}
			}
		}
		if spare.0 > 0 { crate::loadlog::say(|| "bs: verts+groups".into()); }
		if !ordered {
			runs = by_tex.into_iter().collect();
		}
		let mut indices = Vec::new();
		let mut draws = Vec::new();
		for (tex, idx) in runs {
			let cull = ((tex >> 24) & 3) as u8;
			// Key bits 26..28 are the CLAMP wrap of the draw (`mobies::SLOT_MASK`): the slot's sampler clamps u, v or both.
			let wrap = (tex >> crate::shrubs::WRAP_SHIFT) & 3;
			let moby = (tex & crate::mobies::KEY_MASK) >> 16 == 1;
			let kcode = if moby { tex >> crate::mobies::K_CODE_SHIFT } else { 0 };
			let slot_key = tex & (crate::mobies::KEY_MASK | 3 << crate::shrubs::WRAP_SHIFT) | kcode << crate::mobies::K_CODE_SHIFT;
			let tex = tex & crate::mobies::KEY_MASK;
			let texture = match slots.get(&slot_key) {
				Some(&i) => i,
				None => {
					// A moby material with a TEX1 K reads the authored mip chain of its texture at the level the GS picks (`moby_word`).
					let chain = moby_chain(slot_key, textures.get(&tex)).filter(|_| kcode != 0);
					bind.push(self.upload_slot(tex, textures.get(&tex), chain.as_ref().map(|c| c.as_slice()), wrap, !ordered)); // [PC] texture packs
					slots.insert(slot_key, bind.len() - 1);
					bind.len() - 1
				}
			};
			draws.push(Draw { inst: 0, cull, texture, start: indices.len() as u32, count: idx.len() as u32 });
			indices.extend(idx);
		}
		if spare.0 > 0 { crate::loadlog::say(|| "bs: draws+indices".into()); }
		let vbuf = sized_buffer(&self.device, "vertices", (verts.len() + spare.0) * std::mem::size_of::<Vertex>(), wgpu::BufferUsages::VERTEX);
		self.wb(&vbuf, 0, bytemuck::cast_slice(&verts));
		if spare.0 > 0 { crate::loadlog::say(|| format!("bs: vbuf written ({} verts)", verts.len())); }
		let ibuf = sized_buffer(&self.device, "indices", (indices.len() + spare.1) * 4, wgpu::BufferUsages::INDEX);
		self.wb(&ibuf, 0, bytemuck::cast_slice(&indices));
		(Batch { vbuf, ibuf, draws }, lo, hi, verts)
	}

	/// A texture with its own mip chain (level 0 first, each level half the size of the previous): no resampling, the levels are
	/// exactly the ones given (REN-CULL-SHRUB-BB-MIP, the GS reads authored levels).
	pub(in crate::viewer) fn upload_chain(&self, chain: &[Texture], sampler: &wgpu::Sampler) -> wgpu::BindGroup {
		let t = &chain[0];
		let tex = self.device.create_texture(&wgpu::TextureDescriptor {
			label: None,
			size: wgpu::Extent3d { width: t.w as u32, height: t.h as u32, depth_or_array_layers: 1 },
			mip_level_count: chain.len() as u32,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::Rgba8Unorm,
			usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
			view_formats: &[],
		});
		for (k, m) in chain.iter().enumerate() {
			self.queue.write_texture(
				wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: k as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
				bytemuck::cast_slice(&m.rgba),
				wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(m.w as u32 * 4), rows_per_image: Some(m.h as u32) },
				wgpu::Extent3d { width: m.w as u32, height: m.h as u32, depth_or_array_layers: 1 },
			);
		}
		let view = tex.create_view(&Default::default());
		let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: None,
			layout: &self.tex_layout,
			entries: &[
				wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
				wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
			],
		});
		// bindless.rs: `load_scene` collects the views of the scene's slots (any sampler: the array shader picks the shrub wrap
		// sampler itself; a missing view turned the array path off on levels with clamped shrub materials).
		if let Some(log) = self.tex_log.borrow_mut().as_mut() {
			log.insert(bind.clone(), view);
		}
		bind
	}

	/// The instance-aware builder (BRIDGE-29, shared with the shrub batch of BRIDGE-27): like `batch` over unordered world
	/// meshes, but every mesh keeps its own draws, one per (mesh, texture) in ascending texture key, in mesh order. Returns
	/// the batch (its `draws` are that static list), the bounds, the vertices as uploaded, and per mesh its range in
	/// `draws` and its vertex range.
	pub(in crate::viewer) fn batch_instances(&self, meshes: &[&Mesh], textures: &HashMap<u32, Texture>, bind: &mut Vec<wgpu::BindGroup>, slots: &mut HashMap<u32, usize>, terrain: Option<&TerrainRule>, shrub_chains: &[Option<Vec<Texture>>], extra: wgpu::BufferUsages) -> (Batch, [f32; 3], [f32; 3], Vec<Vertex>, Vec<(std::ops::Range<usize>, std::ops::Range<usize>)>) {
		let mut verts: Vec<Vertex> = Vec::new();
		let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
		for (mi, m) in meshes.iter().enumerate() {
			push_mesh_vertices(m, mi, None, &mut verts, &mut lo, &mut hi);
		}
		let (indices, runs, ranges) = instance_layout(meshes);
		let mut draws = Vec::new();
		for (mi, (dr, _)) in ranges.iter().enumerate() {
			for &(key, start, count) in &runs[dr.clone()] {
				let cull = ((key >> 24) & 3) as u8;
				let mut wrap = (key >> crate::shrubs::WRAP_SHIFT) & 3;
				let shrub_k = crate::shrubs::key_k(key);
				let key = if shrub_k.is_some() { key & crate::mobies::KEY_MASK & !crate::shrubs::MATERIAL_BITS } else { key & crate::mobies::KEY_MASK };
				// A terrain face key is its texture index, with the material record index in bits 8..16 (`terrain_face_keys`).
				let (key, inst, chain) = match terrain {
					Some(t) => {
						let (tex, mat) = (key & 0xff, (key >> 8) & 0xff);
						let chain = t.chains.get(tex as usize).and_then(|c| c.as_ref());
						let material = t.materials[mi].get(mat as usize);
						let inst = material.map_or(0, |m| terrain_inst(m.k, chain.map_or(0, |c| c.len() - 1)));
						// The record's GS CLAMP_1 (REPEAT or CLAMP per axis): a REPEAT v at a fading cliff edge drew the top row of the
						// texture as a thin line there (Eudora cliff undersides, fx_census_late.md run 21).
						wrap = material.map_or(0, |m| m.wrap);
						(tex, inst, chain)
					}
					// A shrub mesh draw's first instance is its material word (`vs_shrub`): CLAMP wrap, TEX1 K and the top level of
					// its texture's authored chain (MXL = mips - 1); its slot has the wrap's sampler.
					None => match shrub_k {
						Some(k) => {
							let chain = shrub_chains.get((key & 0xff) as usize).and_then(|c| c.as_ref());
							// A tie (key group 2) without an authored chain keeps the plain sample (word = the wrap alone, `tie_kmx`).
							let word = if chain.is_none() && (key >> 16) == 2 { wrap } else { wrap | k << 2 | (chain.map_or(0, |c| c.len() as u32 - 1)) << 14 };
							(key, word, chain)
						}
						None => (key, wrap, None),
					},
				};
				let slot_key = key | wrap << crate::shrubs::WRAP_SHIFT;
				let texture = match slots.get(&slot_key) {
					Some(&i) => i,
					None => {
						bind.push(self.upload_slot(key, textures.get(&key), chain.map(|c| c.as_slice()), wrap, true)); // [PC] texture packs
						slots.insert(slot_key, bind.len() - 1);
						bind.len() - 1
					}
				};
				draws.push(Draw { inst, cull, texture, start, count });
			}
		}
		let vbuf = buffer(&self.device, "instance vertices", bytemuck::cast_slice(&verts), wgpu::BufferUsages::VERTEX | extra);
		self.wb(&vbuf, 0, bytemuck::cast_slice(&verts));
		let ibuf = buffer(&self.device, "instance indices", bytemuck::cast_slice(&indices), wgpu::BufferUsages::INDEX);
		self.wb(&ibuf, 0, bytemuck::cast_slice(&indices));
		(Batch { vbuf, ibuf, draws }, lo, hi, verts, ranges)
	}
}
