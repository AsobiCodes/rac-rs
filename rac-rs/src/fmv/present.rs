//! Movie presentation: Y/Cb/Cr planes as R8 textures, converted in the shader (BT.601 limited range),
//! drawn in its own pass over the surface, fitted by height (or width if too wide) at the movie's own
//! display aspect, black bars around. The surface format is non-sRGB, so values are written as is.

use super::mpeg2::Frame;
use wgpu::util::DeviceExt;

const SHADER: &str = "
struct U { rect: vec4<f32>, tint: vec4<f32> };
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var ty: texture_2d<f32>;
@group(0) @binding(2) var tcb: texture_2d<f32>;
@group(0) @binding(3) var tcr: texture_2d<f32>;
@group(0) @binding(4) var smp: sampler;
struct V { @builtin(position) p: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs(@builtin(vertex_index) i: u32) -> V {
	let c = vec2<f32>(f32(i & 1u), f32((i >> 1u) & 1u));
	var o: V;
	o.p = vec4<f32>(mix(u.rect.x, u.rect.z, c.x), mix(u.rect.y, u.rect.w, c.y), 0.0, 1.0);
	o.uv = c;
	return o;
}
@fragment fn fs(v: V) -> @location(0) vec4<f32> {
	let y = (textureSample(ty, smp, v.uv).r - 16.0 / 255.0) * 1.164383;
	let cb = textureSample(tcb, smp, v.uv).r - 128.0 / 255.0;
	let cr = textureSample(tcr, smp, v.uv).r - 128.0 / 255.0;
	let rgb = vec3<f32>(y + 1.596027 * cr, y - 0.391762 * cb - 0.812968 * cr, y + 2.017232 * cb);
	return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * u.tint.a + u.tint.rgb, 1.0);
}
";

struct Planes {
	size: (u32, u32),
	tex: [wgpu::Texture; 3],
	bind: wgpu::BindGroup,
}

pub struct Presenter {
	pipeline: wgpu::RenderPipeline,
	layout: wgpu::BindGroupLayout,
	sampler: wgpu::Sampler,
	ubuf: wgpu::Buffer,
	planes: Option<Planes>,
	/// Display aspect of the current frame; None = no movie shown (draw is a no-op).
	pub aspect: f32,
	pub active: bool,
	/// Retail mode: a uniform GS draw over the held last picture (the movie-end fade, `Retail::movie_tint`):
	/// out = picture * tint.a + tint.rgb. (0, 0, 0, 1) = the picture as decoded.
	pub tint: [f32; 4],
	/// UI-11c black ticks: `draw` clears the view to black without a picture.
	pub black: bool,
}

impl Presenter {
	pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Presenter {
		let tex_entry = |b| wgpu::BindGroupLayoutEntry {
			binding: b,
			visibility: wgpu::ShaderStages::FRAGMENT,
			ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
			count: None,
		};
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("movie"),
			entries: &[
				wgpu::BindGroupLayoutEntry {
					binding: 0,
					visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
					count: None,
				},
				tex_entry(1),
				tex_entry(2),
				tex_entry(3),
				wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
			],
		});
		let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("movie"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("movie", SHADER.into())) });
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("movie"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
		let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("movie"),
			layout: Some(&pl),
			vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
			fragment: Some(wgpu::FragmentState {
				module: &module,
				entry_point: Some("fs"),
				compilation_options: Default::default(),
				targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
			}),
			primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
			depth_stencil: None,
			multisample: wgpu::MultisampleState::default(),
			multiview_mask: None,
			cache: None,
		});
		let sampler = device.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
		let ubuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("movie rect"), contents: &[0u8; 32], usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST });
		Presenter { pipeline, layout, sampler, ubuf, planes: None, aspect: 4.0 / 3.0, active: false, black: false, tint: [0.0, 0.0, 0.0, 1.0] }
	}

	/// Uploads a decoded frame and shows it from the next `draw` on. A decoded frame is a playing movie's picture, never a
	/// held one, so it shows untinted: a movie-end fade tint left from the movie before (a second movie right after the
	/// first, Veldin's ending) does not darken it.
	pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, f: &Frame) {
		self.tint = [0.0, 0.0, 0.0, 1.0];
		let (w, h) = (f.width, f.height);
		if self.planes.as_ref().is_none_or(|p| p.size != (w, h)) {
			let mk = |w: u32, h: u32| {
				device.create_texture(&wgpu::TextureDescriptor {
					label: Some("movie plane"),
					size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
					mip_level_count: 1,
					sample_count: 1,
					dimension: wgpu::TextureDimension::D2,
					format: wgpu::TextureFormat::R8Unorm,
					usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
					view_formats: &[],
				})
			};
			let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
			let tex = [mk(w, h), mk(cw, ch), mk(cw, ch)];
			let v: Vec<_> = tex.iter().map(|t| t.create_view(&Default::default())).collect();
			let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
				label: Some("movie"),
				layout: &self.layout,
				entries: &[
					wgpu::BindGroupEntry { binding: 0, resource: self.ubuf.as_entire_binding() },
					wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&v[0]) },
					wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&v[1]) },
					wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&v[2]) },
					wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&self.sampler) },
				],
			});
			self.planes = Some(Planes { size: (w, h), tex, bind });
		}
		let p = self.planes.as_ref().unwrap();
		let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
		for (t, data, pw, ph) in [(&p.tex[0], &f.y, w, h), (&p.tex[1], &f.cb, cw, ch), (&p.tex[2], &f.cr, cw, ch)] {
			queue.write_texture(
				t.as_image_copy(),
				data,
				wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(pw), rows_per_image: Some(ph) },
				wgpu::Extent3d { width: pw, height: ph, depth_or_array_layers: 1 },
			);
		}
		self.aspect = super::player::display_aspect(f.aspect_ratio_information, w, h);
		self.active = true;
	}

	/// Stops showing the movie.
	pub fn clear(&mut self) {
		self.active = false;
		self.tint = [0.0, 0.0, 0.0, 1.0];
	}

	/// Draws black plus the picture into `view` (window size `sw` x `sh`); no-op when inactive or
	/// before the first frame.
	pub fn draw(&self, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, sw: f32, sh: f32) {
		if self.black {
			drop(enc.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("movie black"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
				depth_stencil_attachment: None,
				timestamp_writes: None,
				occlusion_query_set: None,
				multiview_mask: None,
			}));
			return;
		}
		let (true, Some(p)) = (self.active, &self.planes) else { return };
		let (mut pw, mut ph) = (sh * self.aspect, sh);
		if pw > sw {
			(pw, ph) = (sw, sw / self.aspect);
		}
		let (x, y) = ((sw - pw) / 2.0, (sh - ph) / 2.0);
		let rect = [x / sw * 2.0 - 1.0, 1.0 - y / sh * 2.0, (x + pw) / sw * 2.0 - 1.0, 1.0 - (y + ph) / sh * 2.0];
		queue.write_buffer(&self.ubuf, 0, bytemuck::cast_slice(&[rect, self.tint]));
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("movie"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment {
				view,
				depth_slice: None,
				resolve_target: None,
				ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
			})],
			depth_stencil_attachment: None,
			timestamp_writes: None,
			occlusion_query_set: None,
			multiview_mask: None,
		});
		pass.set_pipeline(&self.pipeline);
		pass.set_bind_group(0, &p.bind, &[]);
		pass.draw(0..4, 0..1);
	}
}
