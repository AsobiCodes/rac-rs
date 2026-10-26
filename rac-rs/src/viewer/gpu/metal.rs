//! Metal (chrome) mobys: view and palette rows, packet to vertex conversion, upload and the per-frame metal props.

use super::*;

impl Gpu {
	/// NOV-35b: the three light rows (world to-light direction, colour) a metal vertex of a moby sums (FUN_00210358 vf25..vf30, the
	/// moby's own record): its light pair blended as REN-5d (`slot_blend`; colours and directions lerp by t, directions renormalised,
	/// t = 0 uses A as is) and its dynamic third light (`dyn_id` id, 0 = none). The wrap lanes are not used.
	fn metal_rows(&self, placed: Option<([u8; 3], u32)>, id: f32) -> [([f32; 3], [f32; 3]); 3] {
		let lights = self.light_rows();
		let (_, word) = placed.unwrap_or((gameplay::SPAWN_AMBIENT, 0));
		let (a, b, t) = gameplay::light_word(word);
		let (la, lb, t) = (lights[a.min(15)], lights[b.min(15)], t.clamp(0.0, 0.999));
		let xyz = |v: [f32; 4]| [v[0], v[1], v[2]];
		let mix = |x: [f32; 4], y: [f32; 4]| -> [f32; 3] { std::array::from_fn(|k| x[k] * (1.0 - t) + y[k] * t) };
		let pair = |j: usize| -> ([f32; 3], [f32; 3]) {
			if t == 0.0 {
				return (xyz(la[2 * j + 1]), xyz(la[2 * j]));
			}
			let d = mix(la[2 * j + 1], lb[2 * j + 1]);
			let l = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
			if l < 1e-12 {
				return ([0.0; 3], [0.0; 3]);
			}
			(d.map(|x| x / l.sqrt()), mix(la[2 * j], lb[2 * j]))
		};
		let third = match (id.round() as usize).checked_sub(1).and_then(|i| self.moby_dyn.get(i)) {
			Some((c, d)) => (d.map(|x| -x), *c),
			None => ([0.0; 3], [0.0; 3]),
		};
		[pair(0), pair(1), third]
	}

	/// BRIDGE-45 (NOV-35b): `load_scene`'s metal half: every class's metal packets and the chrome / glass textures (`CHROME_KEY`,
	/// `GLASS_KEY` in `textures`), uploaded with the clamp sampler (CLAMP_1 = 5, TEX1 MMAG linear; FUN_00253c78 OV:44617-44620).
	pub(in crate::viewer) fn load_metal(&mut self, classes: HashMap<u32, Vec<mobies::MetalPacket>>, textures: &HashMap<u32, Texture>) {
		let tex = match (textures.get(&crate::level::CHROME_KEY), textures.get(&crate::level::GLASS_KEY)) {
			(Some(c), Some(g)) => Some([self.upload(Some(c), &self.clamp_sampler), self.upload(Some(g), &self.clamp_sampler)]),
			_ => None,
		};
		self.metal = MetalDraw { classes, tex, props: None, cap: (0, 0), scratch: Vec::new() };
		self.orig_gpu.metal = crate::texpack::extra_hd::take_metal_originals().map(|[c, g]| [self.upload(Some(&c), &self.clamp_sampler), self.upload(Some(&g), &self.clamp_sampler)]); // [PC] split view with a pack
	}

	/// BRIDGE-45 (NOV-35b): this frame's metal faces of the drawn props. `jobs` = (prop index, placement, scale, dynamic-light id)
	/// of every prop `set_props` drew; per prop: metal alpha from the MOBY-FADE view depth of its sphere (None: no metal part),
	/// env matrix from the sphere centre and the camera, then the skinned metal vertices (the pose tail after the LOD vertices,
	/// else the stored ones). Without a view (menus) nothing is built.
	pub(super) fn set_metal_props(&mut self, props: &[Prop], jobs: &[(usize, [f32; 16], f32, f32)], posed: &HashMap<PoseKey, Posed>, fx_posed: &HashMap<usize, Posed>, centres: &[[f32; 3]], radii: &[Option<f32>]) {
		let (mut verts, mut runs) = (Vec::new(), BTreeMap::new());
		if let Some((view, ranges)) = self.scene.as_ref().and_then(|s| Some((s.clip_view?, &s.props_ranges))) {
			let mut todo = Vec::new();
			for &(pi, m, scale, id) in jobs {
				let p = &props[pi];
				let (Some(packets), Some(&c), Some(r)) = (self.metal.classes.get(&p.class_id), centres.get(pi), ranges.get(pi)) else { continue };
				// The MOBY-FADE z of `set_props` and the sphere radius (0 without one).
				let d: [f32; 3] = std::array::from_fn(|k| c[k] - view.e[k] * 1024.0);
				let z = view.f[0] * d[0] + view.f[1] * d[1] + view.f[2] * d[2];
				let Some(alpha) = metal_alpha(metal_byte(p.class_id), z, radii.get(pi).copied().flatten().unwrap_or(0.0)) else { continue };
				let total: usize = packets.iter().map(|q| q.positions.len()).sum();
				let tail = fx_posed.get(&pi).or_else(|| posed.get(&pose_key(p))).filter(|(ps, _)| ps.len() == r.len() + total).map(|(ps, ns)| (&ps[r.len()..], &ns[r.len()..]));
				let e = mobies::metal_env(view.e, metal_view_rows(&view), c, axes(&m));
				todo.push((packets.as_slice(), tail, self.metal_rows(p.light, id), e, m, scale, alpha));
			}
			// [PC] perf round 3: one reused vertex buffer; each prop fills its own slice (`fill_metal`) on the frame worker pool.
			let counts: Vec<usize> = todo.iter().map(|t| metal_count(t.0)).collect();
			verts = std::mem::take(&mut self.metal.scratch);
			verts.clear();
			verts.resize(counts.iter().sum(), Vertex::zeroed());
			let mut slices: Vec<(&mut [Vertex], u32)> = Vec::with_capacity(counts.len());
			let (mut rest, mut first) = (&mut verts[..], 0u32);
			for &c in &counts {
				let (a, b) = std::mem::take(&mut rest).split_at_mut(c);
				slices.push((a, first));
				(rest, first) = (b, first + c as u32);
			}
			let built: Vec<BTreeMap<(u8, usize), Vec<u32>>> = todo
				.par_iter()
				.zip(slices.into_par_iter())
				.map(|((packets, tail, rows, e, m, scale, alpha), (dst, first))| {
					let mut r = BTreeMap::new();
					fill_metal(dst, first, &mut r, packets, *tail, rows, *e, m, *scale, *alpha, true);
					r
				})
				.collect();
			for r in built {
				for (key, idx) in r {
					runs.entry(key).or_insert_with(Vec::new).extend(idx);
				}
			}
		}
		upload_metal(&self.device, &self.queue, &mut self.metal.props, &mut self.metal.cap, &verts, runs);
		self.metal.scratch = verts;
	}
}

/// Appends `m`'s vertices (normals from its faces, fog class tag and baked colours as `Gpu::batch` documents) and grows the
/// bounds; `mi` indexes `colours` when the caller supplies per-vertex colours.
/// NOV-35b: the world-to-view rotation rows of the retail camera (0x167080 +0x00/+0x10/+0x20) from the view basis: the rows are
/// (-row 1, -row 2, row 0) of the camera rows (forward, left, up) (BRIDGE-9 tags, camera.md NOV-5.2), i.e. (right, down, forward);
/// row 1 = left is the hero / moby Euler order the camera record shares [I].
pub(super) fn metal_view_rows(v: &super::clip::ClipView) -> [[f32; 3]; 3] {
	[v.s, v.u.map(|x| -x), v.f]
}

/// The three model axes in world space (columns 0..2 of a column-major placement), retail moby +0xc0 / +0xd0 / +0xe0.
pub(super) fn axes(m: &[f32; 16]) -> [[f32; 3]; 3] {
	[0, 1, 2].map(|k| [m[4 * k], m[4 * k + 1], m[4 * k + 2]])
}

/// NOV-35b metal vertex colour (bytes / 128; FUN_00210358 metal branch, VU0 prog0 0x0a18-0x0a30, EE ppacb): per channel
/// v = 0.875 + sum_k 0.25 col_k max(to_k . n, 0) (the 0x47800070 bias; no ambient, no wrap term, `n` not renormalised) and
/// byte = trunc(128 v) & 0xff.
fn metal_colour(rows: &[([f32; 3], [f32; 3]); 3], n: [f32; 3]) -> [f32; 3] {
	let l = rows.map(|(d, _)| (d[0] * n[0] + d[1] * n[1] + d[2] * n[2]).max(0.0));
	std::array::from_fn(|ch| {
		let mut acc = 0.0f32;
		for k in 0..3 {
			acc += rows[k].1[ch] * 0.25 * l[k];
		}
		(((0.875 + acc as f64) * 128.0).floor() as i64 & 0xff) as f32 / 128.0
	})
}

/// BRIDGE-45: appends one moby's metal faces. Per vertex: the position (`world`: placement `m` times the `scale`d skinned position;
/// else model space for a panel draw whose uniform carries `m`), the sphere-map UV `metal_uv(e, n')`, the CPU colour from `rows`
/// on the world normal (axes of `m` times n') and the metal alpha (vertex alpha / 128 like the LOD alpha). Faces go by (cull,
/// texture): slot -3 glass (1), other slots < -1 chrome (0) (every metal face on the disc has one, mobies test).
pub(super) fn push_metal(verts: &mut Vec<Vertex>, runs: &mut BTreeMap<(u8, usize), Vec<u32>>, packets: &[mobies::MetalPacket], posed: Option<(&[[f32; 3]], &[[f32; 3]])>, rows: &[([f32; 3], [f32; 3]); 3], e: [[f32; 3]; 3], m: &[f32; 16], scale: f32, alpha: u8, world: bool) {
	let b = verts.len();
	verts.resize(b + metal_count(packets), Vertex::zeroed());
	fill_metal(&mut verts[b..], b as u32, runs, packets, posed, rows, e, m, scale, alpha, world);
}

/// The vertices `push_metal` appends for `packets`.
fn metal_count(packets: &[mobies::MetalPacket]) -> usize {
	packets.iter().map(|pk| pk.positions.len().min(pk.normals.len())).sum()
}

/// `push_metal` into `dst` (`metal_count` vertices), whose first vertex is batch vertex `first` ([PC] perf round 3: the props'
/// metal batch fills disjoint slices on the frame worker pool).
fn fill_metal(dst: &mut [Vertex], first: u32, runs: &mut BTreeMap<(u8, usize), Vec<u32>>, packets: &[mobies::MetalPacket], posed: Option<(&[[f32; 3]], &[[f32; 3]])>, rows: &[([f32; 3], [f32; 3]); 3], e: [[f32; 3]; 3], m: &[f32; 16], scale: f32, alpha: u8, world: bool) {
	let (mut at, mut out) = (0, 0);
	for pk in packets {
		let base = first + out as u32;
		for (k, (p, n)) in pk.positions.iter().zip(&pk.normals).enumerate() {
			let (p, n) = posed.and_then(|(ps, ns)| Some((*ps.get(at + k)?, *ns.get(at + k)?))).unwrap_or((*p, *n));
			let a = p.map(|c| c * scale);
			let wn: [f32; 3] = std::array::from_fn(|i| m[i] * n[0] + m[4 + i] * n[1] + m[8 + i] * n[2]);
			let pos = if world { std::array::from_fn(|i| m[i] * a[0] + m[4 + i] * a[1] + m[8 + i] * a[2] + m[12 + i]) } else { a };
			let c = metal_colour(rows, wn);
			dst[out] = Vertex { pos, uv: mobies::metal_uv(e, n), normal: wn, colour: [1.0, 1.0, 1.0, moby_alpha_encoding(0.0, alpha).0], light: [c[0], c[1], c[2], METAL_LIGHT] };
			out += 1;
		}
		at += pk.positions.len();
		let cull = (mobies::cull_bits(pk.cull) >> 24) as u8;
		for &(slot, f) in &pk.faces {
			if slot >= -1 {
				continue;
			}
			runs.entry((cull, (slot == -3) as usize)).or_default().extend(f.map(|i| i + base));
		}
	}
}

/// Uploads a metal batch (growing its buffers to the next power of two) with one draw per (cull, texture) run.
pub(super) fn upload_metal(device: &wgpu::Device, queue: &wgpu::Queue, slot: &mut Option<Batch>, cap: &mut (usize, usize), verts: &[Vertex], runs: BTreeMap<(u8, usize), Vec<u32>>) {
	let (mut indices, mut draws) = (Vec::new(), Vec::new());
	for ((cull, texture), idx) in runs {
		draws.push(Draw { inst: 0, cull, texture, start: indices.len() as u32, count: idx.len() as u32 });
		indices.extend(idx);
	}
	if slot.is_none() || verts.len() > cap.0 || indices.len() > cap.1 {
		*cap = (verts.len().next_power_of_two().max(cap.0).max(64), indices.len().next_power_of_two().max(cap.1).max(192));
		*slot = Some(Batch { vbuf: buffer(device, "metal vertices", &vec![0u8; cap.0 * std::mem::size_of::<Vertex>()], wgpu::BufferUsages::VERTEX), ibuf: buffer(device, "metal indices", &vec![0u8; cap.1 * 4], wgpu::BufferUsages::INDEX), draws: Vec::new() });
	}
	let Some(batch) = slot else { return };
	if !verts.is_empty() {
		queue.write_buffer(&batch.vbuf, 0, bytemuck::cast_slice(verts));
		queue.write_buffer(&batch.ibuf, 0, bytemuck::cast_slice(&indices));
	}
	batch.draws = draws;
}
