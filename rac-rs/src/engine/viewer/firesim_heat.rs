//! [PC] Fire simulation, the flames' heat buffer (lane nfire; the user 2026-11-03: the sim's flames read as "a spray of separate
//! specks" and the big Novalis fire as "just blob"). The flame particles (firesim.rs pool) are not shaded one sprite at a time:
//! - Pass A (`pass`, own render pass after the sim step): every flame particle draws a soft round blob of heat (its temperature x a
//!   gaussian) into a small Rgba16Float target, MAX-blended (r heat, g 1 / view depth, b/a the retail tint's chromaticity: overlap never
//!   saturates, a stream of hundreds of particles or a far fire keeps its shape and its hot core / cool tips), tested by hand
//!   against the scene's depth (the multisampled level depth read at full-resolution coordinates; no depth attachment).
//! - Pass B (`composite`, inside the particle pass after the smoke): one full-screen triangle reads the heat back through a rising,
//!   time-varying domain warp sized in world metres at the fire's depth (a big fire breaks into many tongues), stronger where the heat
//!   is thin (licking edges); compresses it (1 - exp(-k h): dense fire keeps structure, never a flat white disc) and maps it through a
//!   blackbody ramp (deep red, orange, yellow, near-white core). Premultiplied: the flames add light, a thin cover only at low heat.
//! Quality by the Fire level: Low quarter-resolution heat and no warp, Medium half and one warp octave, High half and two. Nothing is
//! made or run unless the fire is simulated (Fire Off, Classic, `fire_sim` 0: no target, no pass).
//! Dev knobs (`RAC_TUNE`, ENV.md): fire_heat_gain (heat per particle), fire_heat_k (compression), fire_heat_warp (warp reach m),
//! fire_heat_bright, fire_heat_tongue (tongue size m). GPU time: RAC_GPU_PASSES=1, rows "fire heat" and "lap fire heat comp".

use super::firefx::{tune, FireEnv};
use super::*;

/// The heat target's format.
const HEAT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// firesim.wgsl `U` (view_proj, sky_view_proj): the heat pass's own copy of the level block.
const U0_SIZE: u64 = 128;
/// firesim.wgsl `SoftU` (p, n, l[16]).
const SOFT_SIZE: u64 = 288;

/// firesim_heat.wgsl `HeatU`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct HeatU {
	pub dims: [f32; 4],
	pub k: [f32; 4],
	pub w: [f32; 4],
}

/// Full-resolution pixels per heat pixel and warp octaves by the Fire level (Low 4 and none, Medium 2 and one, High 2 and two).
pub(super) fn tier(level: u8) -> (u32, u32) {
	match level {
		0 | 1 => (4, 0),
		2 => (2, 1),
		_ => (2, 2),
	}
}

/// The heat target's size for a `w` x `h` frame at `div` (at least 1 x 1).
pub(super) fn heat_size(w: u32, h: u32, div: u32) -> (u32, u32) {
	(w.div_ceil(div).max(1), h.div_ceil(div).max(1))
}

struct Target {
	key: (u32, u32, u64, u32),
	view: wgpu::TextureView,
	soft_bind: wgpu::BindGroup,
	comp_bind: wgpu::BindGroup,
}

pub(super) struct HeatRes {
	heat: wgpu::RenderPipeline,
	comp: wgpu::RenderPipeline,
	u0: wgpu::Buffer,
	soft: wgpu::Buffer,
	hu: wgpu::Buffer,
	samp: wgpu::Sampler,
	u0_bind: wgpu::BindGroup,
	soft_layout: wgpu::BindGroupLayout,
	comp_layout: wgpu::BindGroupLayout,
	tgt: Option<Target>,
	/// The heat pass drew this frame (else the composite draws nothing).
	drawn: bool,
}

fn entry(binding: u32, visibility: wgpu::ShaderStages, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
	wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None }
}

const UNIFORM: wgpu::BindingType = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };

impl HeatRes {
	/// The pipelines (module: firesim.rs DRAW_WGSL, `draw_layout`: the sim's group 1). Targets are made at the first pass.
	pub(super) fn new(env: &FireEnv<'_>, module: &wgpu::ShaderModule, draw_layout: &wgpu::BindGroupLayout) -> HeatRes {
		let device = env.device;
		let (vs, fs, vf) = (wgpu::ShaderStages::VERTEX, wgpu::ShaderStages::FRAGMENT, wgpu::ShaderStages::VERTEX_FRAGMENT);
		let buf = |label: &str, size: u64| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		let (u0, soft, hu) = (buf("fire heat view", U0_SIZE), buf("fire heat soft", SOFT_SIZE), buf("fire heat comp", std::mem::size_of::<HeatU>() as u64));
		let u0_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("fire heat view"), entries: &[entry(0, vf, UNIFORM)] });
		let depth_ms = wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: true };
		let soft_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("fire heat soft"), entries: &[entry(0, vf, UNIFORM), entry(1, fs, depth_ms)] });
		let tex = wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
		let comp_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("fire heat comp"), entries: &[entry(0, fs, UNIFORM), entry(1, fs, tex), entry(2, fs, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering))] });
		let u0_bind = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("fire heat view"), layout: &u0_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: u0.as_entire_binding() }] });
		let clamp = wgpu::AddressMode::ClampToEdge;
		let samp = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("fire heat"), address_mode_u: clamp, address_mode_v: clamp, address_mode_w: clamp, mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
		let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
		let max = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Max };
		let pre = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add };
		let hpl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("fire heat"), bind_group_layouts: &[Some(&u0_layout), Some(draw_layout), Some(&soft_layout)], immediate_size: 0 });
		let heat = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("fire heat"),
			layout: Some(&hpl),
			vertex: wgpu::VertexState { module, entry_point: Some("vs_part"), compilation_options: Default::default(), buffers: &[] },
			fragment: Some(wgpu::FragmentState { module, entry_point: Some("fs_heat"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: HEAT_FORMAT, blend: Some(wgpu::BlendState { color: max, alpha: max }), write_mask: wgpu::ColorWrites::ALL })] }),
			primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
			depth_stencil: None,
			multisample: Default::default(),
			multiview_mask: None,
			cache: None,
		});
		// The composite runs inside the particle pass with the sim draw's groups 0..2 (bound by the caller and the smoke draw) plus its own
		// group 3: the same layouts, so nothing the caller bound is disturbed.
		let l0 = env.pipeline.get_bind_group_layout(0);
		let cpl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("fire heat comp"), bind_group_layouts: &[Some(&l0), Some(draw_layout), Some(env.partfx.soft_layout()), Some(&comp_layout)], immediate_size: 0 });
		let comp = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("fire heat comp"),
			layout: Some(&cpl),
			vertex: wgpu::VertexState { module, entry_point: Some("vs_comp"), compilation_options: Default::default(), buffers: &[] },
			fragment: Some(wgpu::FragmentState {
				module,
				entry_point: Some("fs_comp"),
				compilation_options: Default::default(),
				targets: &[Some(wgpu::ColorTargetState { format: env.format, blend: Some(wgpu::BlendState { color: pre, alpha: pre }), write_mask: wgpu::ColorWrites::ALL }), Some(wgpu::ColorTargetState { format: partfx::GLOW_FORMAT, blend: Some(wgpu::BlendState { color: add, alpha: add }), write_mask: wgpu::ColorWrites::ALL })],
			}),
			primitive: Default::default(),
			depth_stencil: Some(wgpu::DepthStencilState { format: env.depth_format, depth_write_enabled: Some(false), depth_compare: Some(wgpu::CompareFunction::Always), stencil: Default::default(), bias: Default::default() }),
			multisample: wgpu::MultisampleState { count: MSAA, ..Default::default() },
			multiview_mask: None,
			cache: None,
		});
		HeatRes { heat, comp, u0, soft, hu, samp, u0_bind, soft_layout, comp_layout, tgt: None, drawn: false }
	}

	/// The heat target for depth `depth` (`key`: width, height, generation) at `div`, remade when either changes.
	fn target(&mut self, device: &wgpu::Device, depth: &wgpu::TextureView, key: (u32, u32, u64), div: u32) {
		let k = (key.0, key.1, key.2, div);
		if self.tgt.as_ref().is_some_and(|t| t.key == k) {
			return;
		}
		let (w, h) = heat_size(key.0, key.1, div);
		let tex = device.create_texture(&wgpu::TextureDescriptor { label: Some("fire heat"), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: HEAT_FORMAT, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] });
		let view = tex.create_view(&Default::default());
		let d = depth.texture().create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
		let soft_bind = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("fire heat soft"), layout: &self.soft_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: self.soft.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&d) }] });
		let comp_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("fire heat comp"),
			layout: &self.comp_layout,
			entries: &[wgpu::BindGroupEntry { binding: 0, resource: self.hu.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) }, wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.samp) }],
		});
		self.tgt = Some(Target { key: k, view, soft_bind, comp_bind });
	}

	/// Pass A: the pool's flames (instances pool..2 pool of firesim.wgsl vs_part) into the heat target.
	#[allow(clippy::too_many_arguments)]
	pub(super) fn pass(&mut self, env: &FireEnv<'_>, enc: &mut wgpu::CommandEncoder, draw_bind: &wgpu::BindGroup, cam: &Camera, at: &super::firesim::StepIn<'_>, clock: f32, pool: u32) {
		self.drawn = false;
		let (div, oct) = tier(at.level);
		self.target(env.device, at.depth, at.key, div);
		let Some(t) = self.tgt.as_ref() else { return };
		let (w, h) = heat_size(at.key.0, at.key.1, div);
		let mut u0 = [0f32; 32];
		for (c, col) in env.view_proj.iter().enumerate() {
			u0[c * 4..c * 4 + 4].copy_from_slice(col);
		}
		env.queue.write_buffer(&self.u0, 0, bytemuck::cast_slice(&u0));
		let mut soft = [0f32; (SOFT_SIZE / 4) as usize];
		soft[..4].copy_from_slice(&[at.near_far[0], at.near_far[1], at.key.0 as f32 / w as f32, tune("fire_heat_gain", 1.0)]);
		env.queue.write_buffer(&self.soft, 0, bytemuck::cast_slice(&soft));
		let hu = HeatU {
			dims: [at.key.0 as f32, at.key.1 as f32, w as f32, h as f32],
			k: [cam.tan_half, oct as f32, clock, 0.0],
			w: [tune("fire_heat_k", 2.2), tune("fire_heat_warp", 0.5), tune("fire_heat_bright", 1.25), tune("fire_heat_tongue", 0.9)],
		};
		env.queue.write_buffer(&self.hu, 0, bytemuck::bytes_of(&hu));
		let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("fire heat"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &t.view, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
			depth_stencil_attachment: None,
			timestamp_writes: crate::viewer::passtime::rw("fire heat"),
			occlusion_query_set: None,
			multiview_mask: None,
		});
		rp.set_pipeline(&self.heat);
		rp.set_bind_group(0, &self.u0_bind, &[]);
		rp.set_bind_group(1, draw_bind, &[]);
		rp.set_bind_group(2, &t.soft_bind, &[]);
		rp.draw(0..6, pool..2 * pool);
		self.drawn = true;
	}

	/// Pass B, inside the particle pass (groups 0 and 2 bound by the caller): the flames from the heat target.
	pub(super) fn composite(&self, pass: &mut wgpu::RenderPass<'_>, draw_bind: &wgpu::BindGroup) {
		let (true, Some(t)) = (self.drawn, self.tgt.as_ref()) else { return };
		crate::viewer::passtime::lap(pass, "fire heat pre");
		pass.set_pipeline(&self.comp);
		pass.set_bind_group(1, draw_bind, &[]);
		pass.set_bind_group(3, &t.comp_bind, &[]);
		pass.draw(0..3, 0..1);
		crate::viewer::passtime::lap(pass, "fire heat comp");
	}
}
