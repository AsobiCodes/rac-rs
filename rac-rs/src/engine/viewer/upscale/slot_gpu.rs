//! [PC] The upscaler slot's GPU side (slot.rs; lane ndlss stage 2, FSR 1 removed in lane nfsr1). Per frame at the split point:
//! the scene's 4x MSAA colour resolved into a render-size picture (`resolve`: the input of FSR 3 / 4 and DLSS), their output in
//! the window-size picture `mid` (a storage texture for FidelityFX), then `copy` from `mid` into the window-size MSAA target the
//! HUD pass loads, with the scene depth upsampled (nearest) into the window-size depth target. `scale` = a plain bilinear
//! upscale into `mid` for a frame in which neither upscaler recorded (its runtime refused just now: the slot is off from the
//! next frame, slot.rs `active`). Shader slot_gpu.wgsl. Built only while an upscaler is chosen.

/// The window-size picture's format (gamma-space LDR, as the scene).
pub const MID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub(super) const SHADER: &str = include_str!("slot_gpu.wgsl");
/// [PC] Brightness / Contrast: `fs_copy_grade` (the copy with the curve; the uniform's second row `k`).
const GRADE_PART: &str = include_str!("slot_grade.wgsl");
/// The uniform buffer's size: `SlotUniforms` (the plain shader's `U`) and the graded shader's `k` row after it.
const UBUF_SIZE: u64 = std::mem::size_of::<SlotUniforms>() as u64 + 16;

/// The graded copy's shader: the plain one with `k` added to `U`, the shared curve and `fs_copy_grade` (the plain shader's text is
/// untouched, so the 50 / 50 copy is the copy of before).
pub(super) fn graded_shader() -> String {
	let s = SHADER.replacen("\tsize: vec4<f32>,\n};", "\tsize: vec4<f32>,\n\tk: vec4<f32>,\n};", 1);
	format!("{s}\n{}\n{GRADE_PART}", crate::viewer::picture_adjust::CURVE_WGSL)
}

/// The uniform block (slot_gpu.wgsl `U`, 16 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SlotUniforms {
	/// Render width, height; window width, height (at least 1 each).
	pub size: [f32; 4],
}

/// The constants for a `render` -> `win` slot.
pub fn uniforms(render: (u32, u32), win: (u32, u32)) -> SlotUniforms {
	SlotUniforms { size: [render.0.max(1) as f32, render.1.max(1) as f32, win.0.max(1) as f32, win.1.max(1) as f32] }
}

fn texture(device: &wgpu::Device, label: &str, format: wgpu::TextureFormat, size: (u32, u32), samples: u32, usage: wgpu::TextureUsages) -> wgpu::TextureView {
	device
		.create_texture(&wgpu::TextureDescriptor {
			label: Some(label),
			size: wgpu::Extent3d { width: size.0.max(1), height: size.1.max(1), depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: samples,
			dimension: wgpu::TextureDimension::D2,
			format,
			usage,
			view_formats: &[],
		})
		.create_view(&Default::default())
}

fn tex_entry(binding: u32, sample_type: wgpu::TextureSampleType, multisampled: bool) -> wgpu::BindGroupLayoutEntry {
	wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled }, count: None }
}

struct Targets {
	render: (u32, u32),
	win: (u32, u32),
	/// Render size, single sample: the resolved scene (also the level pass's resolve target in Classic).
	input: wgpu::TextureView,
	/// Window size: the upscaler's output (or the plain scale's).
	mid: wgpu::TextureView,
	/// `mid` was written by a wgpu pass once (wgpu clears a texture it thinks unwritten before a read: FFX's writes are not tracked).
	mid_init: std::cell::Cell<bool>,
	/// Window size, MSAA: what the HUD pass loads.
	msaa: wgpu::TextureView,
	depth: wgpu::TextureView,
	scale: wgpu::BindGroup,
}

/// The pipelines, the uniform buffer and (per render / window size) the targets.
pub(in crate::viewer) struct SlotGpu {
	formats: (wgpu::TextureFormat, wgpu::TextureFormat),
	ubuf: wgpu::Buffer,
	scale_pipe: wgpu::RenderPipeline,
	copy_pipe: wgpu::RenderPipeline,
	/// [PC] Brightness / Contrast: `fs_copy_grade`, built the first time a slider is off 50 (None inside = the guard refused it).
	copy_grade: std::sync::OnceLock<Option<wgpu::RenderPipeline>>,
	l_scale: wgpu::BindGroupLayout,
	l_copy: wgpu::BindGroupLayout,
	targets: Option<Targets>,
	copy: Option<wgpu::BindGroup>,
}

impl SlotGpu {
	/// `format`: the scene's colour format (the MSAA targets); `depth_format`: the level's depth-stencil format.
	pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat, depth_format: wgpu::TextureFormat) -> SlotGpu {
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("upscale slot"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("slot_gpu", SHADER.into())) });
		let uniform = wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None };
		let src = tex_entry(1, wgpu::TextureSampleType::Float { filterable: false }, false);
		let l_scale = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("upscale scale"), entries: &[uniform, src] });
		let l_copy = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("upscale copy"), entries: &[uniform, src, tex_entry(2, wgpu::TextureSampleType::Depth, crate::viewer::scene_msaa::msaa() > 1)] });
		let pipe = |entry: &'static str, layout: &wgpu::BindGroupLayout, target: wgpu::TextureFormat, depth: Option<wgpu::DepthStencilState>, count: u32| make_pipe(device, &shader, entry, layout, (target, depth, count));
		let scale_pipe = pipe("fs_scale", &l_scale, MID_FORMAT, None, 1);
		let copy_pipe = pipe("fs_copy", &l_copy, format, Some(copy_depth(depth_format)), crate::viewer::targets::msaa());
		let ubuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("upscale slot"), size: UBUF_SIZE, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		SlotGpu { formats: (format, depth_format), ubuf, scale_pipe, copy_pipe, copy_grade: std::sync::OnceLock::new(), l_scale, l_copy, targets: None, copy: None }
	}

	/// [PC] Brightness / Contrast: the graded copy's pipeline, built on first use (pipe-guarded: None = the copy stays plain).
	fn grade_pipe(&self, device: &wgpu::Device) -> Option<&wgpu::RenderPipeline> {
		self.copy_grade
			.get_or_init(|| {
				let (format, depth_format) = self.formats;
				crate::viewer::pipe_guard::guard(device, crate::viewer::pipe_guard::Effect::Grade, || {
					let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("upscale copy grade"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("slot_grade", graded_shader().into())) });
					make_pipe(device, &shader, "fs_copy_grade", &self.l_copy, (format, Some(copy_depth(depth_format)), crate::viewer::targets::msaa()))
				})
			})
			.as_ref()
	}

	/// [PC] Brightness / Contrast: the curve's uniform row (`k`) for this frame's graded copy.
	pub(super) fn write_grade(&self, queue: &crate::viewer::presenter::GatedQueue, k: [f32; 4]) {
		queue.write_buffer(&self.ubuf, std::mem::size_of::<SlotUniforms>() as u64, bytemuck::cast_slice(&k));
	}

	/// The targets at `sizes` = (render, window) (made again only when one changed), the constants, and this frame's copy bind
	/// group over the level's MSAA depth `depth` (rebuilt every frame: a resize makes a new depth texture).
	pub(super) fn prepare(&mut self, device: &wgpu::Device, queue: &crate::viewer::presenter::GatedQueue, depth: &wgpu::TextureView, sizes: ((u32, u32), (u32, u32))) {
		let (render, win) = sizes;
		if self.targets.as_ref().map_or(true, |t| (t.render, t.win) != sizes) {
			let (format, depth_format) = self.formats;
			let ra = wgpu::TextureUsages::RENDER_ATTACHMENT;
			let input = texture(device, "upscale input", format, render, 1, ra | wgpu::TextureUsages::TEXTURE_BINDING | crate::viewer::scene_msaa::copy_usage());
			let scale = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("upscale scale"), layout: &self.l_scale, entries: &[wgpu::BindGroupEntry { binding: 0, resource: self.ubuf.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&input) }] });
			// STORAGE_BINDING: the FidelityFX output (ffx_frame.rs).
			let mid = texture(device, "upscale mid", MID_FORMAT, win, 1, ra | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING);
			let msaa = texture(device, "upscale hud colour", format, win, crate::viewer::targets::msaa(), ra | crate::viewer::scene_msaa::copy_usage());
			let hud_depth = texture(device, "upscale hud depth", depth_format, win, crate::viewer::targets::msaa(), ra);
			self.targets = Some(Targets { render, win, input, mid, mid_init: std::cell::Cell::new(false), msaa, depth: hud_depth, scale });
		}
		queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(&uniforms(render, win)));
		let Some(t) = &self.targets else { return };
		let depth_only = depth.texture().create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
		self.copy = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("upscale copy"),
			layout: &self.l_copy,
			entries: &[
				wgpu::BindGroupEntry { binding: 0, resource: self.ubuf.as_entire_binding() },
				wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&t.mid) },
				wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&depth_only) },
			],
		}));
	}

	pub(super) fn sizes(&self) -> ((u32, u32), (u32, u32)) {
		self.targets.as_ref().map_or(((1, 1), (1, 1)), |t| (t.render, t.win))
	}

	pub(super) fn input(&self) -> &wgpu::TextureView {
		&self.targets.as_ref().expect("prepared").input
	}

	pub(super) fn hud_targets(&self) -> (&wgpu::TextureView, &wgpu::TextureView) {
		let t = self.targets.as_ref().expect("prepared");
		(&t.msaa, &t.depth)
	}

	/// The render-size resolved scene and the window-size picture, as views (DLSS's colour and output, dlss_frame.rs).
	pub(super) fn views(&self) -> (&wgpu::TextureView, &wgpu::TextureView) {
		let t = self.targets.as_ref().expect("prepared");
		(&t.input, &t.mid)
	}

	/// The render-size resolved scene and the window-size picture, as textures (the FidelityFX dispatch's colour and output).
	pub(super) fn ffx_textures(&self) -> Option<(&wgpu::Texture, &wgpu::Texture)> {
		self.targets.as_ref().map(|t| (t.input.texture(), t.mid.texture()))
	}

	/// No upscaler recorded this frame: resolve, the plain scale and the copy (every target fully overwritten). `msaa` = the
	/// scene's MSAA colour at the render size.
	pub(super) fn encode_plain(&self, device: &wgpu::Device, enc: &mut wgpu::CommandEncoder, msaa: &wgpu::TextureView, grade: bool) {
		self.resolve(enc, msaa);
		self.scale(enc);
		self.copy(device, enc, grade);
	}

	/// The scene's samples averaged into the render-size picture (an empty pass that only resolves).
	pub(super) fn resolve(&self, enc: &mut wgpu::CommandEncoder, msaa: &wgpu::TextureView) {
		let Some(t) = &self.targets else { return };
		drop(pass(enc, "upscale resolve", colour(msaa, Some(&t.input), wgpu::LoadOp::Load), None));
		crate::viewer::scene_msaa::flush(enc);
	}

	/// The plain bilinear scale from the render-size picture into `mid`.
	pub(super) fn scale(&self, enc: &mut wgpu::CommandEncoder) {
		let Some(t) = &self.targets else { return };
		let mut p = pass(enc, "upscale scale", colour(&t.mid, None, wgpu::LoadOp::Clear(wgpu::Color::BLACK)), None);
		p.set_pipeline(&self.scale_pipe);
		p.set_bind_group(0, &t.scale, &[]);
		p.draw(0..3, 0..1);
		t.mid_init.set(true);
	}

	/// Before `mid`'s first FidelityFX write: one clear pass, so wgpu counts it as written (else it clears it before `copy` reads).
	pub(super) fn mid_init(&self, enc: &mut wgpu::CommandEncoder) {
		let Some(t) = self.targets.as_ref().filter(|t| !t.mid_init.get()) else { return };
		drop(pass(enc, "upscale mid init", colour(&t.mid, None, wgpu::LoadOp::Clear(wgpu::Color::BLACK)), None));
		t.mid_init.set(true);
	}

	/// `mid` into the window-size MSAA target the HUD pass loads, with the scene depth upsampled (nearest); `grade` = with the
	/// Brightness / Contrast curve (`write_grade` set it this frame).
	pub(super) fn copy(&self, device: &wgpu::Device, enc: &mut wgpu::CommandEncoder, grade: bool) {
		let (Some(t), Some(copy)) = (&self.targets, &self.copy) else { return };
		let depth = wgpu::RenderPassDepthStencilAttachment {
			view: &t.depth,
			depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
			stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(crate::viewer::shadowvol::STENCIL_CLEAR), store: wgpu::StoreOp::Discard }),
		};
		let mut p = pass(enc, "upscale copy", colour(&t.msaa, None, wgpu::LoadOp::Clear(wgpu::Color::BLACK)), Some(depth));
		p.set_pipeline(if grade { self.grade_pipe(device).unwrap_or(&self.copy_pipe) } else { &self.copy_pipe });
		p.set_bind_group(0, copy, &[]);
		p.draw(0..3, 0..1);
	}
}

/// The copy's depth state: the upsampled scene depth written as is.
fn copy_depth(format: wgpu::TextureFormat) -> wgpu::DepthStencilState {
	wgpu::DepthStencilState { format, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Always), stencil: Default::default(), bias: Default::default() }
}

/// One full-screen pipeline of the slot: fragment `entry` of `shader`, `out` = (target format, depth state, sample count).
fn make_pipe(device: &wgpu::Device, shader: &wgpu::ShaderModule, entry: &str, layout: &wgpu::BindGroupLayout, out: (wgpu::TextureFormat, Option<wgpu::DepthStencilState>, u32)) -> wgpu::RenderPipeline {
	let (target, depth, count) = out;
	let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(entry), bind_group_layouts: &[Some(layout)], immediate_size: 0 });
	device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
		label: Some(entry),
		layout: Some(&pl),
		vertex: wgpu::VertexState { module: shader, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
		fragment: Some(wgpu::FragmentState { module: shader, entry_point: Some(entry), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: target, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
		primitive: Default::default(),
		depth_stencil: depth,
		multisample: wgpu::MultisampleState { count, ..Default::default() },
		multiview_mask: None,
		cache: None,
	})
}

fn colour<'a>(view: &'a wgpu::TextureView, resolve_target: Option<&'a wgpu::TextureView>, load: wgpu::LoadOp<wgpu::Color>) -> Option<wgpu::RenderPassColorAttachment<'a>> {
	Some(crate::viewer::scene_msaa::att(view, resolve_target, wgpu::Operations { load, store: wgpu::StoreOp::Store })) // lane noaa: 1 sample = copy
}

fn pass(enc: &mut wgpu::CommandEncoder, label: &'static str, c: Option<wgpu::RenderPassColorAttachment<'_>>, d: Option<wgpu::RenderPassDepthStencilAttachment<'_>>) -> wgpu::RenderPass<'static> {
	enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some(label), color_attachments: &[c], depth_stencil_attachment: d, timestamp_writes: crate::viewer::passtime::rw(label), occlusion_query_set: None, multiview_mask: None }).forget_lifetime()
}
