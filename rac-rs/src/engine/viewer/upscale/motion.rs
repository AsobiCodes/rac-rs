//! [PC] The motion-vector pass of the upscaler inputs (upscale.rs): two render-size targets (motion Rg16Float, depth R32Float)
//! filled by one full-screen pass over the level's MSAA depth, and the `show` debug pass that draws the vectors into the MSAA
//! colour target under the HUD. Pure wgpu; built only while RAC_UPSCALE_PREP is on.

use super::MotionUniforms;

pub(super) const SHADER: &str = include_str!("motion.wgsl");
/// The motion target's format: two half floats in UV units (FSR and DLSS both take 16-bit float vectors).
pub const MOTION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;
/// The single-sample depth the upscalers read (0 near .. 1 far, sample 0 of the MSAA depth).
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

fn tex_entry(binding: u32, sample_type: wgpu::TextureSampleType, multisampled: bool) -> wgpu::BindGroupLayoutEntry {
	wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled }, count: None }
}

fn target(device: &wgpu::Device, label: &str, format: wgpu::TextureFormat, size: (u32, u32)) -> wgpu::TextureView {
	device
		.create_texture(&wgpu::TextureDescriptor {
			label: Some(label),
			size: wgpu::Extent3d { width: size.0.max(1), height: size.1.max(1), depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format,
			// STORAGE_BINDING is left out: Rg16Float storage is not core everywhere; the upscalers read them as sampled textures.
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
			view_formats: &[],
		})
		.create_view(&Default::default())
}

struct Targets {
	size: (u32, u32),
	motion: wgpu::TextureView,
	depth: wgpu::TextureView,
	show: wgpu::BindGroup,
}

/// The pipelines, the uniform buffer and (per render size) the targets.
pub(in crate::viewer) struct MotionGpu {
	ubuf: wgpu::Buffer,
	motion_pipe: wgpu::RenderPipeline,
	show_pipe: wgpu::RenderPipeline,
	l_frame: wgpu::BindGroupLayout,
	l_show: wgpu::BindGroupLayout,
	targets: Option<Targets>,
	frame: Option<wgpu::BindGroup>,
}

impl MotionGpu {
	/// `format`: the MSAA colour target's format (the `show` picture is drawn into it).
	pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> MotionGpu {
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("upscale motion"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("upscale motion", SHADER.into())) });
		let uniform = wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None };
		let l_frame = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("upscale motion"), entries: &[uniform, tex_entry(1, wgpu::TextureSampleType::Depth, true)] });
		let l_show = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("upscale show"), entries: &[tex_entry(0, wgpu::TextureSampleType::Float { filterable: false }, false)] });
		let pipe = |entry: &'static str, layouts: &[Option<&wgpu::BindGroupLayout>], targets: &[Option<wgpu::ColorTargetState>], count: u32| {
			let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(entry), bind_group_layouts: layouts, immediate_size: 0 });
			device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
				label: Some(entry),
				layout: Some(&pl),
				vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
				fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some(entry), compilation_options: Default::default(), targets }),
				primitive: Default::default(),
				depth_stencil: None,
				multisample: wgpu::MultisampleState { count, ..Default::default() },
				multiview_mask: None,
				cache: None,
			})
		};
		let plain = |format| Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL });
		let motion_pipe = pipe("fs_motion", &[Some(&l_frame)], &[plain(MOTION_FORMAT), plain(DEPTH_FORMAT)], 1);
		let show_pipe = pipe("fs_show", &[Some(&l_frame), Some(&l_show)], &[plain(format)], crate::viewer::targets::MSAA);
		let ubuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("upscale motion"), size: std::mem::size_of::<MotionUniforms>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		MotionGpu { ubuf, motion_pipe, show_pipe, l_frame, l_show, targets: None, frame: None }
	}

	/// The targets at the render size `size` (made again only when it changed) and this frame's bind group over the level's
	/// depth `depth` (rebuilt every frame: a resize makes a new depth texture).
	pub(super) fn prepare(&mut self, device: &wgpu::Device, depth: &wgpu::TextureView, size: (u32, u32)) {
		if self.targets.as_ref().map_or(true, |t| t.size != size) {
			let motion = target(device, "upscale motion", MOTION_FORMAT, size);
			let show = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("upscale show"), layout: &self.l_show, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&motion) }] });
			self.targets = Some(Targets { size, motion, depth: target(device, "upscale depth", DEPTH_FORMAT, size), show });
		}
		let depth_only = depth.texture().create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
		self.frame = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("upscale motion"),
			layout: &self.l_frame,
			entries: &[wgpu::BindGroupEntry { binding: 0, resource: self.ubuf.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&depth_only) }],
		}));
	}

	pub(super) fn write(&self, queue: &crate::viewer::presenter::GatedQueue, u: &MotionUniforms) {
		queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(u));
	}

	/// The motion pass (both targets fully overwritten), then `objects` (velocity.rs: the moving objects over the camera vectors),
	/// and with `show` the debug picture into the MSAA colour target `msaa`.
	pub(super) fn encode(&self, enc: &mut wgpu::CommandEncoder, msaa: &wgpu::TextureView, show: bool, objects: impl FnOnce(&mut wgpu::CommandEncoder, &MotionGpu)) {
		let (Some(t), Some(frame)) = (&self.targets, &self.frame) else { return };
		let attach = |view| Some(wgpu::RenderPassColorAttachment { view, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } });
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("upscale motion"), color_attachments: &[attach(&t.motion), attach(&t.depth)], depth_stencil_attachment: None, timestamp_writes: crate::viewer::passtime::rw("upscale motion"), occlusion_query_set: None, multiview_mask: None });
		pass.set_pipeline(&self.motion_pipe);
		pass.set_bind_group(0, frame, &[]);
		pass.draw(0..3, 0..1);
		drop(pass);
		objects(enc, self);
		if show {
			let load = Some(wgpu::RenderPassColorAttachment { view: msaa, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } });
			let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("upscale show"), color_attachments: &[load], depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
			pass.set_pipeline(&self.show_pipe);
			pass.set_bind_group(0, frame, &[]);
			pass.set_bind_group(1, &t.show, &[]);
			pass.draw(0..3, 0..1);
		}
	}

	pub(super) fn motion_view(&self) -> &wgpu::TextureView {
		&self.targets.as_ref().expect("prepared").motion
	}

	/// The uniform block (`MotionUniforms`), also read by the object pass.
	pub(super) fn ubuf(&self) -> &wgpu::Buffer {
		&self.ubuf
	}

	pub(super) fn depth_view(&self) -> &wgpu::TextureView {
		&self.targets.as_ref().expect("prepared").depth
	}
}
