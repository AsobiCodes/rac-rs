//! The fur shader module, bind group layout and pipeline.

use super::*;

pub(super) struct Pipes {
	/// Per fire-shadow set (lane dlit2-9: metalpipe.rs `Pipes::fls`, the lookup compiled out on the frames that read no fire map).
	pub pipeline: crate::viewer::metalpipe::Pipes<wgpu::RenderPipeline>,
}

/// Group 2: the part values (dynamic offset), the marker's vertex buffer (read as floats for the shared normals), the weld table, the
/// fur map of the texture (vertex and fragment), the sampler (trilinear, anisotropic, repeat) and the fur noise (noise.rs).
pub(super) fn layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
	let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry { binding, visibility: match binding { 0 | 3 | 4 => wgpu::ShaderStages::VERTEX_FRAGMENT, 5 => wgpu::ShaderStages::FRAGMENT, _ => wgpu::ShaderStages::VERTEX }, ty, count: None };
	let storage = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None };
	device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
		label: Some("fur"),
		entries: &[
			entry(0, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: wgpu::BufferSize::new(params::SIZE) }),
			entry(1, storage),
			entry(2, storage),
			entry(3, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
			entry(4, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
			entry(5, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
		],
	})
}

/// The shader source: the level shader with the fur entry points appended.
pub(super) fn source(device: &wgpu::Device) -> String {
	source_with(volfog::soft::early(device))
}

/// `source` for a given forced-early-depth choice (the tests have no device; metalpipe.rs `every_module_takes_the_fire_shadow_constants`).
pub(in crate::viewer) fn source_with(early: bool) -> String {
	format!("{}\n{}", volfog::soft::level_source(SHADER, early), include_str!("fur.wgsl"))
}

impl Pipes {
	/// `level` = the level pipeline (its group 0 and 1 layouts are reused), `fur` = group 2's layout (`layout`, made once per window).
	pub(super) fn new(device: &wgpu::Device, level: &wgpu::RenderPipeline, tex_layout: &wgpu::BindGroupLayout, fur: &wgpu::BindGroupLayout, format: wgpu::TextureFormat, depth_format: wgpu::TextureFormat) -> Pipes {
		let modules = crate::viewer::flitquad::Modules::new(device, "fur", crate::viewer::texfilter::frag_w_fix(source(device))); // [PC] lane fshade: plain + fire module (flitquad.rs); DX12 LOD fix once, before both
		let ulayout = level.get_bind_group_layout(0);
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("fur"), bind_group_layouts: &[Some(&ulayout), Some(tex_layout), Some(fur)], immediate_size: 0 });
		let (device, z) = (device.clone(), crate::gs_state::ztest());
		let pipeline = crate::viewer::metalpipe::Pipes::new(move |set| { let cache = crate::viewer::pipewarm::cache(); modules.with(set, |module| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("fur"),
			layout: Some(&pl),
			vertex: wgpu::VertexState {
				module,
				entry_point: Some("vs_fur"),
				compilation_options: Default::default(),
				buffers: &[Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x3, 3 => Float32x4, 4 => Float32x4] })],
			},
			fragment: Some(wgpu::FragmentState { module, entry_point: Some("fs_fur"), compilation_options: wgpu::PipelineCompilationOptions { constants: crate::viewer::metalpipe::fls_constants(set), ..Default::default() }, targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), write_mask: wgpu::ColorWrites::COLOR })] }),
			primitive: wgpu::PrimitiveState::default(),
			depth_stencil: Some(wgpu::DepthStencilState { format: depth_format, depth_write_enabled: Some(false), depth_compare: Some(z), stencil: Default::default(), bias: Default::default() }),
			multisample: wgpu::MultisampleState { count: targets::msaa(), ..Default::default() },
			multiview_mask: None,
			cache: cache.as_ref().map(|c| &c.cache),
		})) });
		Pipes { pipeline }
	}
}
