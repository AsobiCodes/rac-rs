//! The cloud noise, made on the GPU by a compute shader when the clouds first run (no file, no CPU work, ~9 MB of video memory),
//! spread over `STEPS` frames so there is no hitch: the shape volume (128^3: R = Perlin-Worley, G B A = Worley fbm at rising
//! frequencies; the big cauliflower forms), the detail volume (32^3: Worley fbm at three frequencies; erosion of the edges) and the
//! weather map (512^2, `Rgba16Float` so the coverage edges stay smooth), baked again whenever the cumulus coverage changes
//! (`Noise::step`): heaps of four size classes (giant, big, medium, small; each a cone of its own random radius, eased at its rim),
//! ranked by a cluster field (rows along the wind plus blobs: giants and bigs in the cores merge into banks, small puffs fringe
//! them), the coverage showing the lower ranks. R = the soft union of the shown heaps' cones at the near coverage; G = the union
//! (max) of those heaps' dome heights (flat-topped, wide: a big heap about a third as tall as its radius; weather tiles); B = a
//! broad noise field (the stratocumulus deck, the holes in the banks, the smooth warp, and the patches of the altocumulus band and
//! the high veil at other scales); A = the soft union at the far coverage (the band low in the sky).
//! Every texture tiles (lattice periods divide the size); the volumes are `Rgba8Unorm`; all are sampled with repeat and a linear
//! filter.

pub const SHAPE: u32 = 128;
pub const DETAIL: u32 = 32;
pub const WEATHER: u32 = 512;
/// Frames the shape volume is spread over (slabs of SHAPE / STEPS slices); the detail volume and the weather map go in the first.
pub const STEPS: u32 = 8;

const WGSL: &str = r#"
struct G { a: vec4<u32> };
@group(0) @binding(0) var<uniform> g: G;
@group(0) @binding(1) var shape: texture_storage_3d<rgba8unorm, write>;
@group(0) @binding(2) var detail: texture_storage_3d<rgba8unorm, write>;
@group(0) @binding(3) var weather: texture_storage_2d<rgba16float, write>;

fn hash3(p: vec3<u32>) -> vec3<f32> {
	var v = p * 1664525u + 1013904223u;
	v.x += v.y * v.z; v.y += v.z * v.x; v.z += v.x * v.y;
	v = v ^ (v >> vec3<u32>(16u));
	v.x += v.y * v.z; v.y += v.z * v.x; v.z += v.x * v.y;
	return vec3<f32>(v) * (1.0 / 4294967296.0);
}
fn cell(c: vec3<i32>, per: i32, seed: u32) -> vec3<f32> {
	let w = vec3<u32>(((c % per) + per) % per);
	return hash3(w + vec3<u32>(seed * 131u, seed * 71u, seed * 37u));
}
// Gradient noise of period `per` cells over the unit cube (about -1..1).
fn perlin(p: vec3<f32>, per: i32, seed: u32) -> f32 {
	let q = p * f32(per);
	let i = vec3<i32>(floor(q));
	let fr = q - floor(q);
	let u = fr * fr * fr * (fr * (fr * 6.0 - 15.0) + 10.0);
	var n: array<f32, 8>;
	for (var k = 0; k < 8; k++) {
		let o = vec3<i32>(k & 1, (k >> 1) & 1, (k >> 2) & 1);
		let gr = normalize(cell(i + o, per, seed) * 2.0 - 1.0 + vec3<f32>(1.0e-4));
		n[k] = dot(gr, fr - vec3<f32>(o));
	}
	let x0 = mix(mix(n[0], n[1], u.x), mix(n[2], n[3], u.x), u.y);
	let x1 = mix(mix(n[4], n[5], u.x), mix(n[6], n[7], u.x), u.y);
	return mix(x0, x1, u.z);
}
// Inverted cellular noise (1 at the feature points, 0 far from them) of period `per`.
fn worley(p: vec3<f32>, per: i32, seed: u32) -> f32 {
	let q = p * f32(per);
	let i = vec3<i32>(floor(q));
	let fr = q - floor(q);
	var d = 1.0e9;
	for (var z = -1; z <= 1; z++) {
		for (var y = -1; y <= 1; y++) {
			for (var x = -1; x <= 1; x++) {
				let o = vec3<i32>(x, y, z);
				let dv = vec3<f32>(o) + cell(i + o, per, seed) - fr;
				d = min(d, dot(dv, dv));
			}
		}
	}
	return 1.0 - clamp(sqrt(d), 0.0, 1.0);
}
fn worley_fbm(p: vec3<f32>, per: i32, seed: u32) -> f32 {
	return worley(p, per, seed) * 0.625 + worley(p, per * 2, seed + 1u) * 0.25 + worley(p, per * 4, seed + 2u) * 0.125;
}
fn perlin_fbm(p: vec3<f32>, per: i32, seed: u32, oct: i32) -> f32 {
	var s = 0.0;
	var a = 1.0;
	var f = per;
	var t = 0.0;
	for (var k = 0; k < oct; k++) {
		s += perlin(p, f, seed + u32(k)) * a;
		t += a;
		a *= 0.5;
		f *= 2;
	}
	return s / t;
}
fn remap(x: f32, a: f32, b: f32, c: f32, d: f32) -> f32 { return c + (x - a) / (b - a) * (d - c); }

@compute @workgroup_size(4, 4, 4)
fn cs_shape(@builtin(global_invocation_id) id: vec3<u32>) {
	let v = vec3<u32>(id.x, id.y, id.z + g.a.x);
	if (v.x >= 128u || v.y >= 128u || v.z >= 128u) { return; }
	let p = (vec3<f32>(v) + 0.5) / 128.0;
	let pf = clamp(perlin_fbm(p, 4, 11u, 4) * 0.75 + 0.5, 0.0, 1.0);
	let w0 = worley_fbm(p, 4, 21u);
	let pw = clamp(remap(pf, 0.0, 1.0, w0, 1.0), 0.0, 1.0);
	let g1 = worley_fbm(p, 6, 31u);
	let g2 = worley_fbm(p, 12, 41u);
	let g3 = worley_fbm(p, 24, 51u);
	textureStore(shape, v, vec4<f32>(pw, g1, g2, g3));
}

@compute @workgroup_size(4, 4, 4)
fn cs_detail(@builtin(global_invocation_id) id: vec3<u32>) {
	if (id.x >= 32u || id.y >= 32u || id.z >= 32u) { return; }
	let p = (vec3<f32>(id) + 0.5) / 32.0;
	textureStore(detail, id, vec4<f32>(worley_fbm(p, 2, 61u), worley_fbm(p, 4, 71u), worley_fbm(p, 8, 81u), 1.0));
}

// The cluster field at weather point c (tile units): a coarse noise stretched along x (the wind, see genera.rs `wind_frame`) into
// rows (cloud streets), with wide gaps of clear sky between them.
fn cluster(c: vec2<f32>) -> f32 {
	let q = vec3<f32>(fract(c), 0.37);
	let rows = clamp(perlin_fbm(vec3<f32>(q.x, q.y * 3.0, q.z), 2, 91u, 3) * 0.9 + 0.5, 0.0, 1.0);
	let blobs = clamp(perlin_fbm(q, 3, 95u, 3) * 0.9 + 0.5, 0.0, 1.0);
	return rows * 0.55 + blobs * 0.45;
}
// A heap's rank (0 the first to appear as the coverage grows) from its size class k (0 giant .. 3 small), its own random number
// and the cluster field at its centre: the cluster cores take the giant and big heaps (merging into banks), medium heaps gather
// round them (fewer in the cores), small puffs fringe the clusters and a few stray into the gaps.
fn rank_of(k: i32, rnd: f32, cl: f32) -> f32 {
	let core = smoothstep(0.55, 0.63, cl);
	if (k == 0) { return rnd * 0.4 + (1.0 - smoothstep(0.52, 0.62, cl)) * 0.9; }
	if (k == 1) { return rnd * 0.45 + (1.0 - smoothstep(0.48, 0.57, cl)) * 0.7; }
	if (k == 2) { return rnd * 0.6 + (1.0 - smoothstep(0.48, 0.56, cl)) * 0.5 + 0.1 + core * 0.15; }
	return rnd * 0.8 + (1.0 - smoothstep(0.44, 0.52, cl)) * 0.4 + 0.1 + core * 0.3;
}
// Heaps of class k, period `per`: per cell one point (kept inside the middle of its cell) with a random radius (rmin .. rmax
// cells), shown when its rank is under the coverage. x = the union of the shown heaps' cones (1 at a point falling to 0 at its
// radius; soft union) at coverage cov, y = the same at cov2, z = the union (max) of the domes' heights (weather tiles) of the heaps
// shown at cov2, each from its own radius (a small heap never takes a big neighbour's height).
fn heaps(p: vec2<f32>, per: i32, seed: u32, rmin: f32, rmax: f32, k: i32, cov: f32, cov2: f32) -> vec4<f32> {
	let q = p * f32(per);
	let i = vec2<i32>(floor(q));
	let fr = q - floor(q);
	var o = vec4<f32>(0.0);
	for (var y = -1; y <= 1; y++) {
		for (var x = -1; x <= 1; x++) {
			let ofs = vec2<i32>(x, y);
			let h = cell(vec3<i32>(i + ofs, 0), per, seed);
			let r = mix(rmin, rmax, h.z * h.z);
			let at = vec2<f32>(ofs) + 0.28 + h.xy * 0.44;
			let v0 = 1.0 - length(at - fr) / r;
			if (v0 > 0.0) {
				// (Eased at its rim: a cone's kink where it starts would leave a crease in the soft union, a vertical line up the
				// cloud's side.)
				let v = v0 * v0 * (3.0 - 2.0 * v0);
				let rk = rank_of(k, cell(vec3<i32>(i + ofs, 1), per, seed + 7u).x, cluster((vec2<f32>(i) + at) / f32(per)));
				let a = v * (1.0 - smoothstep(cov - 0.05, cov, rk));
				let b = v * (1.0 - smoothstep(cov2 - 0.05, cov2, rk));
				// (A soft union: where two cones overlap they add up, so the valley between two heaps of a bank fills instead of
				// leaving a straight slit along the line half way between them.)
				o.x = o.x + a - o.x * a;
				o.y = o.y + b - o.y * b;
				// (Its dome: flat-topped with rounded shoulders (1 - (1 - a)^3) and wide: a big heap about a third as tall as its
				// radius, small puffs rounder; only the heaps shown near: a shown small heap never takes the height of a giant hidden
				// at the near coverage.)
				let rt = r / f32(per);
				let fa = 1.0 - a;
				o.z = max(o.z, (1.0 - fa * fa * fa) * rt * mix(0.9, 0.42, smoothstep(250.0, 2000.0, rt * 16800.0)));
			}
		}
	}
	return o;
}

@compute @workgroup_size(8, 8, 1)
fn cs_weather(@builtin(global_invocation_id) id: vec3<u32>) {
	if (id.x >= 512u || id.y >= 512u) { return; }
	let p = vec3<f32>((vec2<f32>(id.xy) + 0.5) / 512.0, 0.37);
	let cov = bitcast<f32>(g.a.y);
	let cov2 = bitcast<f32>(g.a.z);
	let gi = heaps(p.xy, 5, 131u, 0.45, 0.72, 0, cov, cov2);
	let b = heaps(p.xy, 10, 101u, 0.35, 0.6, 1, cov, cov2);
	let m = heaps(p.xy, 20, 111u, 0.24, 0.42, 2, cov, cov2);
	let s = heaps(p.xy, 40, 121u, 0.22, 0.4, 3, cov, cov2);
	let x = 1.0 - (1.0 - gi.x) * (1.0 - b.x) * (1.0 - m.x) * (1.0 - s.x);
	let y = 1.0 - (1.0 - gi.y) * (1.0 - b.y) * (1.0 - m.y) * (1.0 - s.y);
	let rad = max(max(gi.z, b.z), max(m.z, s.z));
	let deck = clamp(perlin_fbm(p, 2, 151u, 3) * 0.9 + 0.5, 0.0, 1.0);
	textureStore(weather, id.xy, vec4<f32>(x, rad, deck, y));
}
"#;

pub struct Noise {
	pub shape: wgpu::TextureView,
	pub detail: wgpu::TextureView,
	pub weather: wgpu::TextureView,
	pub sampler: wgpu::Sampler,
	job: Gen,
	step: u32,
	/// The cumulus coverages the weather map holds (near, far); NaN before the first.
	cov: [f32; 2],
}

struct Gen {
	ubuf: wgpu::Buffer,
	group: wgpu::BindGroup,
	shape: wgpu::ComputePipeline,
	detail: wgpu::ComputePipeline,
	weather: wgpu::ComputePipeline,
}

impl Noise {
	pub fn new(device: &wgpu::Device) -> Noise {
		let usage = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
		let tex = |label, n: u32, d3: bool| {
			device.create_texture(&wgpu::TextureDescriptor {
				label: Some(label),
				size: wgpu::Extent3d { width: n, height: n, depth_or_array_layers: if d3 { n } else { 1 } },
				mip_level_count: 1,
				sample_count: 1,
				dimension: if d3 { wgpu::TextureDimension::D3 } else { wgpu::TextureDimension::D2 },
				format: wgpu::TextureFormat::Rgba8Unorm,
				usage,
				view_formats: &[],
			})
		};
		let (st, dt) = (tex("cloud shape", SHAPE, true), tex("cloud detail", DETAIL, true));
		let wt = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("cloud weather"),
			size: wgpu::Extent3d { width: WEATHER, height: WEATHER, depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::Rgba16Float,
			usage,
			view_formats: &[],
		});
		let (shape, detail, weather) = (st.create_view(&Default::default()), dt.create_view(&Default::default()), wt.create_view(&Default::default()));
		let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("cloud noise"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("cloud noise", WGSL.into())) });
		let storage = |binding: u32, dim: wgpu::TextureViewDimension| wgpu::BindGroupLayoutEntry {
			binding,
			visibility: wgpu::ShaderStages::COMPUTE,
			ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: if binding == 3 { wgpu::TextureFormat::Rgba16Float } else { wgpu::TextureFormat::Rgba8Unorm }, view_dimension: dim },
			count: None,
		};
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("cloud noise"),
			entries: &[
				wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
				storage(1, wgpu::TextureViewDimension::D3),
				storage(2, wgpu::TextureViewDimension::D3),
				storage(3, wgpu::TextureViewDimension::D2),
			],
		});
		let ubuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("cloud noise"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("cloud noise"),
			layout: &layout,
			entries: &[
				wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
				wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&shape) },
				wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&detail) },
				wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&weather) },
			],
		});
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("cloud noise"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
		let pipe = |entry: &str| device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("cloud noise"), layout: Some(&pl), module: &module, entry_point: Some(entry), compilation_options: Default::default(), cache: None });
		let job = Gen { shape: pipe("cs_shape"), detail: pipe("cs_detail"), weather: pipe("cs_weather"), ubuf, group };
		let rep = wgpu::AddressMode::Repeat;
		let sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("cloud noise"), address_mode_u: rep, address_mode_v: rep, address_mode_w: rep, mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
		Noise { shape, detail, weather, sampler, job, step: 0, cov: [f32::NAN; 2] }
	}

	/// One generation step into `enc` (call once per frame) for the cumulus coverages `cov` (near, far: the weather map is remade,
	/// in one dispatch, when they change); true once every texture is complete.
	pub fn step(&mut self, queue: &crate::viewer::presenter::GatedQueue, enc: &mut wgpu::CommandEncoder, cov: [f32; 2]) -> bool {
		let done = self.step >= STEPS;
		if done && cov == self.cov {
			return true;
		}
		let job = &self.job;
		let slab = SHAPE / STEPS;
		queue.write_buffer(&job.ubuf, 0, bytemuck::cast_slice(&[self.step.min(STEPS - 1) * slab, cov[0].to_bits(), cov[1].to_bits(), 0]));
		{
			let mut p = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("cloud noise"), timestamp_writes: crate::viewer::passtime::cw("cloud noise") });
			p.set_bind_group(0, &job.group, &[]);
			if self.step == 0 {
				p.set_pipeline(&job.detail);
				p.dispatch_workgroups(DETAIL / 4, DETAIL / 4, DETAIL / 4);
			}
			if cov != self.cov {
				p.set_pipeline(&job.weather);
				p.dispatch_workgroups(WEATHER / 8, WEATHER / 8, 1);
				self.cov = cov;
			}
			if !done {
				p.set_pipeline(&job.shape);
				p.dispatch_workgroups(SHAPE / 4, SHAPE / 4, slab / 4);
				self.step += 1;
			}
		}
		done
	}

	/// The WGSL (for the shader test).
	#[cfg(test)]
	pub fn wgsl() -> &'static str {
		WGSL
	}
}
