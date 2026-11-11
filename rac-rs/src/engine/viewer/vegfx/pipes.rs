//! [PC] The vegetation pipelines (blade, mesh, leaf; moved out of vegfx.rs `VegFx::new`) per fire-shadow set (lane dlit2-9, GPU
//! cost: metalpipe.rs `Pipes::fls`, the fire-shadow lookup compiled out on the frames that read no fire shadow map).

use super::*;

/// Blade, mesh, leaf.
pub(super) type Set = [wgpu::RenderPipeline; 3];

/// The vegetation shader source: the level shader with the vegetation entry points appended, for a forced-early-depth choice
/// (metalpipe.rs test `every_module_takes_the_fire_shadow_constants`).
pub(in crate::viewer) fn source_with(early: bool) -> String {
	format!("{}{}", crate::viewer::volfog::soft::level_source(crate::viewer::SHADER, early), super::shader::WGSL)
}

/// The lazily built sets over the vegetation `layout` and `shader` (each set's fragment constants: metalpipe.rs `fls_constants`; its
/// module: flitquad.rs `Modules::of`).
pub(super) fn pipes(device: &wgpu::Device, layout: wgpu::PipelineLayout, shader: crate::viewer::flitquad::Modules, format: wgpu::TextureFormat, depth_format: wgpu::TextureFormat) -> metalpipe::Pipes<Set> {
	let device = device.clone();
	metalpipe::Pipes::new(move |set| {
		let cache = crate::viewer::pipewarm::cache();
		shader.with(set, |shader| {
		let make = |vs: &str, fs: &str, a2c: bool| {
			device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
				label: Some(vs),
				layout: Some(&layout),
				vertex: wgpu::VertexState {
					module: shader,
					entry_point: Some(vs),
					compilation_options: Default::default(),
					buffers: &[
						Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<VegVertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x2, 3 => Float32x4, 4 => Float32x2] }),
						Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<VegInst>() as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &wgpu::vertex_attr_array![5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4] }),
					],
				},
				fragment: Some(wgpu::FragmentState { module: shader, entry_point: Some(fs), compilation_options: wgpu::PipelineCompilationOptions { constants: metalpipe::fls_constants(set), ..Default::default() }, targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
				primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
				depth_stencil: Some(wgpu::DepthStencilState { format: depth_format, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::LessEqual), stencil: Default::default(), bias: Default::default() }),
				multisample: wgpu::MultisampleState { count: msaa(), mask: !0, alpha_to_coverage_enabled: a2c },
				multiview_mask: None,
				cache: cache.as_ref().map(|c| &c.cache),
			})
		};
		[make("vs_veg_blade", "fs_veg_blade", false), make("vs_veg_mesh", "fs_veg_mesh", true), make("vs_veg_leaf", "fs_veg_blade", false)]
		})
	})
}

impl VegFx {
	/// `op` on the fire-shadow sets `metalpipe::group(true, fire)` (metalpipe.rs `Gpu::fls_sets`: queue their background build, build
	/// them now, or ask if built).
	pub(in crate::viewer) fn fls_op(&self, op: metalpipe::Op, fire: bool) -> bool {
		self.pipes.op(op, true, fire)
	}
}
