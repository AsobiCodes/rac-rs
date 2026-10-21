//! [PC] Particles Enhanced (Pause -> Options -> Graphics -> Particles, with Lighting Enhanced; retail/pcext.rs `graphics`; no
//! retail counterpart). The particles stay retail: the same pool, counts, textures, colours and blending, built by the same code.
//! Only how they meet the picture changes:
//! - soft particles: the level pass ends before the particle pool and they draw in their own pass that reads the level's depth
//!   (read-only), so each one fades out over `FADE` world units where it meets the ground or a wall instead of cutting a hard line;
//! - the additive ones (sparks, muzzle flashes, explosions, glows) also write their light into a glow target that the bloom adds
//!   (post.rs `fs_bright`), so bright effects bloom however dark their surroundings are.
//! - effect lights (`lights`): the additive particles are gathered into up to `FX_LIGHTS` point lights a frame, which light
//!   the scene around them (post.rs `fx_light`, with a heat shimmer above warm ones: `shimmer`) and the alpha-blended smoke
//!   that drifts through them (`fs_part_soft`); the particles' own pixels are left as drawn.
//! Shader entries `fs_part_soft` / `fs_part_soft_add` in viewer.rs SHADER. Particles Classic draws the pool in the level pass as
//! before.

use super::*;

/// Soft fade distance (world units) and glow gain.
const FADE: f32 = 0.6;
const GLOW: f32 = 1.0;

/// Effect lights a frame; gathering radius, cull distance from the eye, brightness and reach (world units) per light.
pub(super) const FX_LIGHTS: usize = 8;
/// Effect lights, smoke glow and heat shimmer on (tuned on the Pyrocitor, burning enemies and explosions).
pub(super) const FX_ON: bool = true;
const GATHER: f32 = 4.0;
const FX_FAR: f32 = 90.0;
const FX_GAIN: f32 = 0.6;
const FX_CAP: f32 = 1.6;
const FX_FLOOR: f32 = 0.15;
const FX_REACH: (f32, f32, f32) = (3.0, 1.4, 10.0);
/// How much the effect lights brighten the alpha-blended smoke (grey texels; coloured flame texels barely move).
const SMOKE: f32 = 1.2;

/// The frame's effect lights: per light (position, reach), (rgb, 0); `n` in use.
#[derive(Clone, Copy, Default)]
pub(super) struct FxLights {
	pub l: [[[f32; 4]; 2]; FX_LIGHTS],
	pub n: usize,
}

impl FxLights {
	pub(super) fn rows(&self) -> [[f32; 4]; 2 * FX_LIGHTS] {
		std::array::from_fn(|k| self.l[k / 2][k % 2])
	}
}

/// How many effect lights a frame gathers by Graphics level of Particles (1 Low, 2 Medium, otherwise High = `FX_LIGHTS`).
pub(super) const fn light_count(level: u8) -> usize {
	match level {
		1 => 2,
		2 => 4,
		_ => FX_LIGHTS,
	}
}

/// Gathers the additive particle quads (4 vertices each, as built by gpu.rs `set_particles`) within `FX_FAR` of the eye into
/// effect lights: brightest first, each quad joins the first light within `GATHER` of it (or starts one while fewer than
/// `FX_LIGHTS` exist). A quad's colour is its vertex colour x its texture's mean alpha-weighted colour (`tints`; the flame
/// and spark textures carry the hue, their vertex colours are mostly grey). A quad weighs its alpha x area; a light takes the weighted mean position and hue, brightness
/// `FX_GAIN` x sqrt(sum of weight x luminance) (capped, faded out below `FX_FLOOR`..2 x `FX_FLOOR`) and reach `FX_REACH.0 + FX_REACH.1` x sqrt(sum of weights), at most `FX_REACH.2`.
pub(super) fn lights<'a>(quads: impl Iterator<Item = (u32, &'a [Vertex])>, tints: &HashMap<u32, [f32; 3]>, eye: [f32; 3], max: usize) -> FxLights {
	struct E {
		p: [f32; 3],
		c: [f32; 3],
		w: f32,
	}
	let mut es: Vec<E> = Vec::new();
	for (key, verts) in quads {
		let tint = tints.get(&(key & !((crate::particles::FX_CLAMP | crate::particles::FX_NOMIP) as u32))).copied().unwrap_or([0.4; 3]);
		for q in verts.chunks_exact(4) {
			let p = std::array::from_fn(|k| (q[0].pos[k] + q[1].pos[k] + q[2].pos[k] + q[3].pos[k]) * 0.25);
			if (0..3).map(|k| (p[k] - eye[k]) * (p[k] - eye[k])).sum::<f32>() > FX_FAR * FX_FAR {
				continue;
			}
			let d2 = |a: [f32; 3], b: [f32; 3]| (0..3).map(|k| (a[k] - b[k]) * (a[k] - b[k])).sum::<f32>();
			let area = (d2(q[0].pos, q[2].pos) * d2(q[1].pos, q[3].pos)).sqrt() * 0.5;
			let col = q[0].colour;
			let w = col[3].max(0.0) * area;
			let c = std::array::from_fn(|k| col[k].max(0.0) * tint[k]);
			if w * (c[0] + c[1] + c[2]) > 1e-4 {
				es.push(E { p, c, w });
			}
		}
	}
	let luma = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
	es.sort_by(|a, b| (b.w * luma(b.c)).total_cmp(&(a.w * luma(a.c))));
	// Per light: weighted position sum, weighted colour sum, weight sum, weight x luminance sum.
	let mut acc: Vec<([f32; 3], [f32; 3], f32, f32)> = Vec::new();
	for e in &es {
		let near = acc.iter().position(|a| {
			let c: [f32; 3] = std::array::from_fn(|k| a.0[k] / a.2);
			(0..3).map(|k| (c[k] - e.p[k]) * (c[k] - e.p[k])).sum::<f32>() < GATHER * GATHER
		});
		let a = match near {
			Some(i) => &mut acc[i],
			None if acc.len() < max.min(FX_LIGHTS) => {
				acc.push(([0.0; 3], [0.0; 3], 0.0, 0.0));
				acc.last_mut().unwrap()
			}
			None => continue,
		};
		for k in 0..3 {
			a.0[k] += e.p[k] * e.w;
			a.1[k] += e.c[k] * e.w;
		}
		a.2 += e.w;
		a.3 += e.w * luma(e.c);
	}
	let mut out = FxLights { n: acc.len(), ..Default::default() };
	for (o, a) in out.l.iter_mut().zip(&acc) {
		let hue: [f32; 3] = std::array::from_fn(|k| a.1[k] / a.2);
		let top = hue[0].max(hue[1]).max(hue[2]).max(1e-4);
		let b = (FX_GAIN * a.3.sqrt()).min(FX_CAP);
		// Faint clusters (water splash and ripple sprites, distant sparks) light nothing; the floor fades in, so no popping.
		let b = b * ((b - FX_FLOOR) / FX_FLOOR).clamp(0.0, 1.0);
		*o = [[a.0[0] / a.2, a.0[1] / a.2, a.0[2] / a.2, (FX_REACH.0 + FX_REACH.1 * a.2.sqrt()).min(FX_REACH.2)], [hue[0] / top * b, hue[1] / top * b, hue[2] / top * b, 0.0]];
	}
	out
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SoftU {
	p: [f32; 4],
	/// (effect lights in use, smoke gain, 0, 0), then the lights as `FxLights::rows`.
	n: [f32; 4],
	l: [[f32; 4]; 2 * FX_LIGHTS],
}

struct Targets {
	key: (u32, u32, u64),
	glow_ms: wgpu::TextureView,
	bind: wgpu::BindGroup,
}

pub(super) struct PartFx {
	layout: wgpu::BindGroupLayout,
	ubuf: wgpu::Buffer,
	/// Alpha-blended, additive.
	pub soft: [wgpu::RenderPipeline; 2],
	/// Water Enhanced with Particles Classic: cover only (glow alpha; colour masked off), alpha-blended and additive.
	pub cover: [wgpu::RenderPipeline; 2],
	/// The pool lines (KER-92), unchanged but in this pass.
	pub line: wgpu::RenderPipeline,
	t: Option<Targets>,
	/// Per particle texture key: its mean alpha-weighted colour (`set_tints`), for `lights`.
	pub tints: HashMap<u32, [f32; 3]>,
}

/// The glow target's format (post.rs binds the resolved copy).
pub(super) const GLOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl PartFx {
	/// `shader` / `ulayout` / `tex_layout`: the level shader and its groups 0 and 1.
	pub(super) fn new(device: &wgpu::Device, shader: &wgpu::ShaderModule, ulayout: &wgpu::BindGroupLayout, tex_layout: &wgpu::BindGroupLayout, format: wgpu::TextureFormat, depth_format: wgpu::TextureFormat) -> PartFx {
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("soft particles"),
			entries: &[
				wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
				wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: true }, count: None },
			],
		});
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("soft particles"), bind_group_layouts: &[Some(ulayout), Some(tex_layout), Some(&layout)], immediate_size: 0 });
		let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
		let make = |fs: &str, blend: wgpu::BlendState, glow: wgpu::ColorWrites, topology: wgpu::PrimitiveTopology, colour: wgpu::ColorWrites| {
			device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
				label: Some(fs),
				layout: Some(&pl),
				vertex: wgpu::VertexState {
					module: shader,
					entry_point: Some("vs"),
					compilation_options: Default::default(),
					buffers: &[Some(wgpu::VertexBufferLayout {
						array_stride: std::mem::size_of::<Vertex>() as u64,
						step_mode: wgpu::VertexStepMode::Vertex,
						attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x3, 3 => Float32x4, 4 => Float32x4],
					})],
				},
				fragment: Some(wgpu::FragmentState {
					module: shader,
					entry_point: Some(fs),
					compilation_options: Default::default(),
					targets: &[Some(wgpu::ColorTargetState { format, blend: Some(blend), write_mask: colour }), Some(wgpu::ColorTargetState { format: GLOW_FORMAT, blend: Some(wgpu::BlendState { color: add, alpha: add }), write_mask: glow })],
				}),
				primitive: wgpu::PrimitiveState { topology, ..Default::default() },
				// As `part_pipelines` / `line_pipeline`: depth-tested, no writes.
				depth_stencil: Some(wgpu::DepthStencilState { format: depth_format, depth_write_enabled: Some(false), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: Default::default() }),
				multisample: wgpu::MultisampleState { count: MSAA, ..Default::default() },
				multiview_mask: None,
				cache: None,
			})
		};
		// GS draw flags 0x44 (additive), as gpu.rs `additive`.
		let additive = wgpu::BlendState { color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add }, alpha: wgpu::BlendComponent::OVER };
		let tri = wgpu::PrimitiveTopology::TriangleList;
		let (all, none) = (wgpu::ColorWrites::ALL, wgpu::ColorWrites::empty());
		PartFx {
			soft: [make("fs_part_soft", wgpu::BlendState::ALPHA_BLENDING, wgpu::ColorWrites::ALPHA, tri, all), make("fs_part_soft_add", additive, wgpu::ColorWrites::ALL, tri, all)],
			cover: [make("fs_part_cover", wgpu::BlendState::ALPHA_BLENDING, wgpu::ColorWrites::ALPHA, tri, none), make("fs_part_cover_add", wgpu::BlendState::ALPHA_BLENDING, wgpu::ColorWrites::ALPHA, tri, none)],
			line: make("fs_line", wgpu::BlendState::ALPHA_BLENDING, wgpu::ColorWrites::empty(), wgpu::PrimitiveTopology::LineList, all),
			ubuf: device.create_buffer(&wgpu::BufferDescriptor { label: Some("soft particles"), size: std::mem::size_of::<SoftU>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
			layout,
			t: None,
			tints: HashMap::new(),
		}
	}

	/// The level's particle textures (fx_textures table: key, texture) -> `tints`.
	pub(super) fn set_tints(&mut self, textures: &[(usize, Texture)]) {
		self.tints = textures
			.iter()
			.map(|(k, t)| {
				let n = t.rgba.len().max(1) as f32;
				let c = std::array::from_fn(|c| t.rgba.iter().map(|p| p[c] as f32 * p[3] as f32).sum::<f32>() / (n * 255.0 * 255.0));
				(*k as u32, c)
			})
			.collect();
	}

	/// Ends nothing itself: the caller drops the level pass, then this begins the particle pass on the same MSAA target
	/// (`msaa`, loaded) and depth (`depth`, read-only), resolving into the post scene (`resolve`) and the post glow target
	/// (`glow`). `generation` = post targets' generation (they are remade with the depth target).
	#[allow(clippy::too_many_arguments)]
	/// `fx`: the effect lights that light the smoke (none in the cover pass). `far`: the world far plane the depth was drawn with.
	pub(super) fn begin<'e>(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, enc: &'e mut wgpu::CommandEncoder, msaa: &wgpu::TextureView, depth: &wgpu::TextureView, resolve: &wgpu::TextureView, glow: &wgpu::TextureView, generation: u64, fx: &FxLights, far: f32) -> wgpu::RenderPass<'e> {
		let tex = resolve.texture();
		let key = (tex.width(), tex.height(), generation);
		if self.t.as_ref().is_none_or(|t| t.key != key) {
			let glow_ms = device
				.create_texture(&wgpu::TextureDescriptor { label: Some("particle glow"), size: wgpu::Extent3d { width: key.0, height: key.1, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: MSAA, dimension: wgpu::TextureDimension::D2, format: GLOW_FORMAT, usage: wgpu::TextureUsages::RENDER_ATTACHMENT, view_formats: &[] })
				.create_view(&Default::default());
			let depth_only = depth.texture().create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
			let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
				label: Some("soft particles"),
				layout: &self.layout,
				entries: &[wgpu::BindGroupEntry { binding: 0, resource: self.ubuf.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&depth_only) }],
			});
			self.t = Some(Targets { key, glow_ms, bind });
		}
		let t = self.t.as_ref().expect("particle targets");
		queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(&SoftU { p: [RETAIL_NEAR, far, FADE, GLOW], n: [fx.n as f32, SMOKE, 0.0, 0.0], l: fx.rows() }));
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("particles"),
			color_attachments: &[
				Some(wgpu::RenderPassColorAttachment { view: msaa, resolve_target: Some(resolve), depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Discard } }),
				Some(wgpu::RenderPassColorAttachment { view: &t.glow_ms, resolve_target: Some(glow), depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Discard } }),
			],
			depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: depth, depth_ops: None, stencil_ops: None }),
			timestamp_writes: None,
			occlusion_query_set: None,
			multiview_mask: None,
		});
		pass.set_bind_group(2, &t.bind, &[]);
		pass
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn quad(p: [f32; 3], size: f32, colour: [f32; 4]) -> [Vertex; 4] {
		let h = size / 2.0;
		[[-h, -h], [h, -h], [h, h], [-h, h]].map(|[x, z]| Vertex { pos: [p[0] + x, p[1], p[2] + z], uv: [0.0; 2], normal: [0.0; 3], colour, light: [0.0; 4] })
	}

	#[test]
	fn lights_gather_nearby_quads_and_drop_far_ones() {
		let mut v = Vec::new();
		v.extend(quad([10.0, 0.0, 0.0], 2.0, [1.0, 0.5, 0.0, 1.0]));
		v.extend(quad([11.0, 0.0, 0.0], 2.0, [1.0, 0.5, 0.0, 1.0]));
		v.extend(quad([30.0, 0.0, 0.0], 1.0, [0.0, 0.0, 1.0, 1.0]));
		v.extend(quad([500.0, 0.0, 0.0], 4.0, [1.0, 1.0, 1.0, 1.0]));
		let fx = lights([(0, &v[..])].into_iter(), &HashMap::from([(0, [1.0; 3])]), [0.0; 3], FX_LIGHTS);
		assert_eq!(fx.n, 2);
		let [a, b] = [fx.l[0], fx.l[1]];
		assert!((a[0][0] - 10.5).abs() < 1e-4 && a[0][3] > FX_REACH.0);
		// Hue kept (orange: top channel = brightness), the blue one on its own.
		assert!(a[1][0] > a[1][1] && a[1][2] == 0.0);
		assert!((b[0][0] - 30.0).abs() < 1e-4 && b[1][2] > 0.0 && b[1][0] == 0.0);
		assert!(lights([(0, &[][..])].into_iter(), &HashMap::new(), [0.0; 3], FX_LIGHTS).n == 0);
		// The texture's hue wins over a grey vertex colour.
		let g = quad([10.0, 0.0, 0.0], 2.0, [0.5, 0.5, 0.5, 1.0]);
		let o = lights([(7 | crate::particles::FX_CLAMP as u32, &g[..])].into_iter(), &HashMap::from([(7, [0.8, 0.3, 0.0])]), [0.0; 3], FX_LIGHTS);
		assert!(o.l[0][1][0] > o.l[0][1][1] && o.l[0][1][2] == 0.0);
	}

	/// Graphics levels: High gathers all `FX_LIGHTS` (today), Medium and Low fewer; a lower cap keeps only the brightest clusters.
	#[test]
	fn light_count_by_level_caps_the_gather() {
		assert_eq!((light_count(3), light_count(2), light_count(1)), (FX_LIGHTS, 4, 2));
		let mut v = Vec::new();
		for k in 0..6 {
			v.extend(quad([10.0 + 12.0 * k as f32, 0.0, 0.0], 2.0, [1.0, 0.5, 0.0, 1.0]));
		}
		let tints = HashMap::from([(0, [1.0; 3])]);
		let n = |level| lights([(0, &v[..])].into_iter(), &tints, [0.0; 3], light_count(level)).n;
		assert_eq!((n(3), n(2), n(1)), (6, 4, 2));
	}
}
