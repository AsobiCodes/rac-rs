//! [PC] The reactive mask of the FidelityFX upscale (ffx_frame.rs; lane nups2): one full-screen render pass at the render size
//! into an R8Unorm target from the opaque-only picture (upscale.rs `opaque_copy`, Particles Enhanced path) and the finished
//! scene (the slot's resolved input), plus the heat haze's pixels (heathaze.rs `take_mask`, review S5). Shader ffx_reactive.wgsl. No opaque picture this frame = no mask (FFX then uses none).
//! Split view: the Classic half never reaches it (upscale.rs `pause` turns the slot off for that pass).

pub(super) const SHADER: &str = include_str!("ffx_reactive.wgsl");
/// The mask's format (`FFX_API_SURFACE_FORMAT_R8_UNORM`).
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

pub(in crate::viewer) struct ReactiveGpu {
	pipe: wgpu::RenderPipeline,
	layout: wgpu::BindGroupLayout,
	target: Option<((u32, u32), wgpu::Texture, wgpu::TextureView)>,
	/// The heat input of a frame without haze: 1 x 1, zero (wgpu clears new textures).
	no_heat: wgpu::TextureView,
}

impl ReactiveGpu {
	pub(super) fn new(device: &wgpu::Device) -> ReactiveGpu {
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("ffx reactive"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("ffx reactive", SHADER.into())) });
		let tex = |binding| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None };
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("ffx reactive"), entries: &[tex(0), tex(1), tex(2)] });
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("ffx reactive"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
		let pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("ffx reactive"),
			layout: Some(&pl),
			vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
			fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_reactive"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
			primitive: Default::default(),
			depth_stencil: None,
			multisample: Default::default(),
			multiview_mask: None,
			cache: None,
		});
		let one = wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 };
		let no_heat = device.create_texture(&wgpu::TextureDescriptor { label: Some("ffx reactive no heat"), size: one, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: FORMAT, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] }).create_view(&Default::default());
		ReactiveGpu { pipe, layout, target: None, no_heat }
	}

	/// The mask of this frame from `opaque`, `scene` and `heat` (all render size `size`); returns its texture for the dispatch.
	pub(super) fn encode(&mut self, device: &wgpu::Device, enc: &mut wgpu::CommandEncoder, (opaque, scene, heat): (&wgpu::TextureView, &wgpu::TextureView, Option<&wgpu::TextureView>), size: (u32, u32)) -> &wgpu::Texture {
		if self.target.as_ref().map_or(true, |t| t.0 != size) {
			let tex = device.create_texture(&wgpu::TextureDescriptor {
				label: Some("ffx reactive"),
				size: wgpu::Extent3d { width: size.0.max(1), height: size.1.max(1), depth_or_array_layers: 1 },
				mip_level_count: 1,
				sample_count: 1,
				dimension: wgpu::TextureDimension::D2,
				format: FORMAT,
				usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
				view_formats: &[],
			});
			let view = tex.create_view(&Default::default());
			self.target = Some((size, tex, view));
		}
		let (_, tex, view) = self.target.as_ref().expect("made above");
		let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("ffx reactive"),
			layout: &self.layout,
			entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(opaque) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(scene) }, wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(heat.unwrap_or(&self.no_heat)) }],
		});
		let att = wgpu::RenderPassColorAttachment { view, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } };
		let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("ffx reactive"), color_attachments: &[Some(att)], depth_stencil_attachment: None, timestamp_writes: crate::viewer::passtime::rw("ffx reactive"), occlusion_query_set: None, multiview_mask: None });
		p.set_pipeline(&self.pipe);
		p.set_bind_group(0, &bind, &[]);
		p.draw(0..3, 0..1);
		drop(p);
		tex
	}
}

#[cfg(test)]
mod tests {
	fn module() -> (naga::Module, naga::valid::ModuleInfo) {
		let m = naga::front::wgsl::parse_str(super::SHADER).expect("wgsl parse");
		let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("wgsl validate");
		(m, info)
	}

	#[test]
	fn the_reactive_shader_validates_and_writes_hlsl() {
		let (m, info) = module();
		for e in ["vs_full", "fs_reactive"] {
			assert!(m.entry_points.iter().any(|p| p.name == e), "{e}");
		}
		let (options, pipeline) = (naga::back::hlsl::Options::default(), naga::back::hlsl::PipelineOptions::default());
		let mut out = String::new();
		naga::back::hlsl::Writer::new(&mut out, &options, &pipeline).write(&m, &info, None).expect("HLSL writes");
		assert!(out.contains("fs_reactive"));
	}
}
