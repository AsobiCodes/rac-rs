//! Pipelines of the UI pass (all from `ui.wgsl`): backdrop downsample / blur / composite and the instanced prims. One bind group
//! layout (texture, sampler, `Params`) serves every pass. The shader text is the built-in file unless `RAC_UI_WGSL=<file>` names
//! one (dev: tune without a rebuild; a file that fails to compile is refused by the pipe guard and the UI just does not draw).
use super::instances::INST_BYTES;
use std::borrow::Cow;

pub const BUILT_IN: &str = include_str!("ui.wgsl");

/// The uniform block of every pass (`Params` in ui.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Params {
	pub a: [f32; 4],
	pub w0: [f32; 4],
	pub w1: [f32; 4],
	pub scrim: [f32; 4],
}

/// The shader source: `RAC_UI_WGSL` file, else the built-in text (then `RAC_WGSL_DIR` may still override it, devtune.rs).
pub fn source() -> Cow<'static, str> {
	let base: Cow<'static, str> = match std::env::var_os("RAC_UI_WGSL") {
		Some(p) => match std::fs::read_to_string(&p) {
			Ok(t) => {
				eprintln!("ui: shader from {}", std::path::Path::new(&p).display());
				Cow::Owned(t)
			}
			Err(e) => {
				eprintln!("ui: RAC_UI_WGSL {}: {e}; using the built-in shader", std::path::Path::new(&p).display());
				Cow::Borrowed(BUILT_IN)
			}
		},
		None => Cow::Borrowed(BUILT_IN),
	};
	crate::devtune::wgsl("modern ui", base)
}

pub struct Pipes {
	pub layout: wgpu::BindGroupLayout,
	pub sampler: wgpu::Sampler,
	pub down: wgpu::RenderPipeline,
	pub blur: wgpu::RenderPipeline,
	pub comp: wgpu::RenderPipeline,
	pub inst: wgpu::RenderPipeline,
}

fn entry(binding: u32, vis: wgpu::ShaderStages, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
	wgpu::BindGroupLayoutEntry { binding, visibility: vis, ty, count: None }
}

pub fn layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
	device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
		label: Some("ui"),
		entries: &[
			entry(0, wgpu::ShaderStages::FRAGMENT, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
			entry(1, wgpu::ShaderStages::FRAGMENT, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
			entry(2, wgpu::ShaderStages::VERTEX_FRAGMENT, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
		],
	})
}

/// Binds `view`, the sampler and `buf` (a `Params`).
pub fn bind(device: &wgpu::Device, p: &Pipes, view: &wgpu::TextureView, buf: &wgpu::Buffer) -> wgpu::BindGroup {
	device.create_bind_group(&wgpu::BindGroupDescriptor {
		label: Some("ui"),
		layout: &p.layout,
		entries: &[
			wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
			wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&p.sampler) },
			wgpu::BindGroupEntry { binding: 2, resource: buf.as_entire_binding() },
		],
	})
}

const PREMULT: wgpu::BlendComponent = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add };

pub fn make(device: &wgpu::Device, format: wgpu::TextureFormat) -> Pipes {
	let layout = layout(device);
	let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("ui"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
	let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("modern ui"), source: wgpu::ShaderSource::Wgsl(source()) });
	let attrs = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4];
	let inst_buf = wgpu::VertexBufferLayout { array_stride: INST_BYTES, step_mode: wgpu::VertexStepMode::Instance, attributes: &attrs };
	let make = |label: &str, vs: &str, fs: &str, buffers: &[Option<wgpu::VertexBufferLayout>], blend: Option<wgpu::BlendState>| {
		device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some(label),
			layout: Some(&pl),
			vertex: wgpu::VertexState { module: &shader, entry_point: Some(vs), compilation_options: Default::default(), buffers },
			fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some(fs), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })] }),
			primitive: Default::default(),
			depth_stencil: None,
			multisample: Default::default(),
			multiview_mask: None,
			cache: None,
		})
	};
	let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
		label: Some("ui"),
		address_mode_u: wgpu::AddressMode::ClampToEdge,
		address_mode_v: wgpu::AddressMode::ClampToEdge,
		mag_filter: wgpu::FilterMode::Linear,
		min_filter: wgpu::FilterMode::Linear,
		..Default::default()
	});
	Pipes {
		down: make("ui down", "vs_full", "fs_down", &[], None),
		blur: make("ui blur", "vs_full", "fs_blur", &[], None),
		comp: make("ui backdrop", "vs_full", "fs_comp", &[], None),
		inst: make("ui prims", "vs_inst", "fs_inst", &[Some(inst_buf)], Some(wgpu::BlendState { color: PREMULT, alpha: PREMULT })),
		layout,
		sampler,
	}
}
