//! [PC] The new sky of Volumetric clouds Low / Medium / High: the retail sky captured and turned into a smooth model, and the
//! backdrop drawn from it in place of the painted shells.
//!
//! Each level (and every `EVERY` frames, as some retail skies turn) the retail sky shells are drawn into six small cube faces with
//! the retail sky pipeline (`FACES`), then reduced into the model map (`sky.rs` REDUCE: per elevation band and azimuth, the mean of
//! the most saturated samples, so the painted clouds vanish and every planet keeps its palette; plus the sun's colour and the
//! zenith / horizon / ground colours the clouds are lit with). In the level pass (and the water mirror and bed passes) the backdrop
//! (`sky.rs` BACKDROP: the model plus a sun disc where the painted sun was) goes first, then only the shells `params::sky_mask`
//! keeps (landmarks). Classic draws the retail sky alone: nothing here runs and the mask is 0.

use super::sky;
use super::*;
use std::cell::{Cell, OnceCell};

/// The model map: elevation columns and azimuth rows (absolute azimuth, a full turn; one more row holds the single values).
pub(in crate::viewer) const MAP_W: u32 = 48;
pub(in crate::viewer) const SECTORS: u32 = 24;
/// Texels across a captured cube face.
const FACE: u32 = 64;
/// Frames between two captures.
const EVERY: u32 = 30;

/// The capture cameras: forward, right, up (the WGSL `cube` picks the face the same way).
const FACES: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
	([1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0]),
	([-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
	([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
	([0.0, -1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
	([0.0, 0.0, 1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
	([0.0, 0.0, -1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
];

/// The sky matrix of capture face `k` (column-major, 90 degrees square; clip w = forward distance, so `vs_sky`'s xyww keeps the
/// shells at the far plane).
pub(super) fn face_matrix(k: usize) -> [[f32; 4]; 4] {
	let (f, r, u) = FACES[k];
	let mut m = [[0.0; 4]; 4];
	for c in 0..3 {
		m[c] = [r[c], u[c], f[c] * 0.5, f[c]];
	}
	m
}

pub(super) struct Res {
	colour: wgpu::TextureView,
	depth: wgpu::TextureView,
	faces: Vec<wgpu::TextureView>,
	pub(super) rgroup: wgpu::BindGroup,
	ubufs: Vec<wgpu::Buffer>,
	pub(super) map: wgpu::TextureView,
	rbuf: wgpu::Buffer,
	reduce: wgpu::RenderPipeline,
	pub(super) latlong: wgpu::RenderPipeline,
	backdrop: wgpu::RenderPipeline,
	bgroup: wgpu::BindGroup,
}

#[derive(Default)]
pub(in crate::viewer) struct SkyModel {
	res: OnceCell<Res>,
	/// (level, frames since) of the last capture.
	captured: Cell<Option<(usize, u32)>>,
	/// This frame: Some(the shell draws dropped) while the new sky is on.
	on: Cell<Option<u64>>,
	sun: Cell<[f32; 3]>,
	pub(super) dumped: Cell<usize>,
	/// [PC] Gemlik's animated star (viewer/star.rs).
	pub(in crate::viewer) star: crate::viewer::star::StarFx,
	/// [PC] The starfield's pipeline (viewer/starfield.rs), made once per Gpu.
	pub(in crate::viewer) starfield: crate::viewer::starfield::StarPipe,
}

impl SkyModel {
	/// The sky shell draws to leave out this frame (0: every shell, as retail). RAC_TUNE=cl_skip=N ORs a mask in (dev).
	pub(in crate::viewer) fn skip(&self) -> u64 {
		self.live().unwrap_or(0) | crate::devtune::f32("cl_skip", 0.0) as u64
	}

	/// This frame's new-sky mask, None in the Classic pass of a split frame (retail/splitview.rs): that half has retail's sky.
	fn live(&self) -> Option<u64> {
		self.on.get().filter(|_| crate::retail::splitview::fx(true))
	}

	/// The sun of the new sky (unit vector toward it) and the model map, while the new sky is on.
	pub(in crate::viewer) fn model(&self) -> Option<([f32; 3], &wgpu::TextureView)> {
		self.live()?;
		self.res.get().map(|r| (self.sun.get(), &r.map))
	}

	/// Draws the backdrop while the new sky is on: first in the sky step of a pass whose group 0 holds level uniforms.
	pub(in crate::viewer) fn backdrop(&self, pass: &mut wgpu::RenderPass<'_>) {
		if let (Some(_), Some(r)) = (self.live(), self.res.get()) {
			pass.set_pipeline(&r.backdrop);
			pass.set_bind_group(1, &r.bgroup, &[]);
			pass.draw(0..3, 0..1);
		}
	}
}

impl Gpu {
	/// Every frame before the level pass: decides whether the new sky is on, recaptures the retail sky when due. `active` = the
	/// clouds' own gate (`volclouds::Input::new`: Enhanced pass with the modern renderer, not under water, no warp, no ship fight):
	/// where the clouds are not drawn the retail shells stay.
	pub(in crate::viewer) fn cloud_sky(&self, enc: &mut wgpu::CommandEncoder, scene: &Scene, u: &Uniforms, gfx: &crate::retail::pcext::Graphics, active: bool) {
		let sm = &self.volclouds.sky;
		let on = active && input::level(gfx) > 0 && params::look(self.level).is_some();
		let Some(sky) = scene.sky.as_ref().filter(|s| on && !s.draws.is_empty()) else {
			sm.on.set(None);
			sm.captured.set(None);
			return;
		};
		// [PC] pipe_guard.rs: a driver that refuses the sky model shaders (DX12 FXC) keeps retail's sky, it does not crash the game.
		let built = || crate::viewer::pipe_guard::guard(&self.device, crate::viewer::pipe_guard::Effect::VolClouds, || self.sky_res()).map(|r| sm.res.get_or_init(|| r));
		let Some(r) = sm.res.get().or_else(built) else {
			sm.on.set(None);
			sm.captured.set(None);
			return;
		};
		let sun = params::sun_dir(crate::viewer::post::sky_sun(self.level), params::key_to_light(&self.sun_rows()));
		sm.sun.set(sun);
		let due = match sm.captured.get() {
			Some((l, age)) if l == self.level && age < EVERY => {
				sm.captured.set(Some((l, age + 1)));
				false
			}
			_ => true,
		};
		if due {
			let disc = params::sun_disc(self.level) as u32 as f32;
			let pale = params::look(self.level).map_or(0.0, |l| crate::devtune::f32("cl_pale", l.pale));
			let rv = [sun[0], sun[1], sun[2], disc, MAP_W as f32, (SECTORS + 1) as f32, SECTORS as f32, pale];
			self.queue.write_buffer(&r.rbuf, 0, bytemuck::cast_slice(&rv));
			self.sky_faces(enc, scene, u, r, None);
			let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("sky model"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &r.map, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
				depth_stencil_attachment: None,
				timestamp_writes: crate::viewer::passtime::rw("sky model"),
				occlusion_query_set: None,
				multiview_mask: None,
			});
			p.set_pipeline(&r.reduce);
			p.set_bind_group(0, &r.rgroup, &[]);
			p.draw(0..3, 0..1);
			drop(p);
			sm.captured.set(Some((self.level, 0)));
			super::skydump::maybe(self, scene, u, r, sky.draws.len());
		}
		sm.on.set(Some(params::sky_mask(self.level)));
	}

	/// The retail sky shells (all, or only draw `only`) into the six capture faces.
	pub(super) fn sky_faces(&self, enc: &mut wgpu::CommandEncoder, scene: &Scene, u: &Uniforms, r: &Res, only: Option<usize>) {
		let Some(sky) = scene.sky.as_ref() else { return };
		for k in 0..6 {
			let mut mu = *u;
			mu.sky_view_proj = face_matrix(k);
			self.queue.write_buffer(&r.ubufs[k], 0, bytemuck::bytes_of(&mu));
			let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("sky capture"), layout: &self.pipeline.get_bind_group_layout(0), entries: &self.sun.entries(&r.ubufs[k]) });
			let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("sky capture"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &r.colour, resolve_target: Some(&r.faces[k]), depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(scene.clear), store: wgpu::StoreOp::Discard } })],
				depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
					view: &r.depth,
					depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
					stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(crate::viewer::shadowvol::STENCIL_CLEAR), store: wgpu::StoreOp::Discard }),
				}),
				timestamp_writes: crate::viewer::passtime::rw("sky capture"),
				occlusion_query_set: None,
				multiview_mask: None,
			});
			pass.set_bind_group(0, &bind, &[]);
			pass.set_pipeline(&self.sky_pipeline);
			pass.set_vertex_buffer(0, sky.vbuf.slice(..));
			pass.set_index_buffer(sky.ibuf.slice(..), wgpu::IndexFormat::Uint32);
			for (_, d) in sky.draws.iter().enumerate().filter(|(i, _)| only.is_none_or(|o| o == *i)) {
				pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
				pass.draw_indexed(d.start..d.start + d.count, 0, d.inst..d.inst + 1);
			}
		}
	}

	/// The capture targets, the model map and the pipelines (once).
	fn sky_res(&self) -> Res {
		let device = &self.device;
		let tex = |label, w: u32, h: u32, layers: u32, samples: u32, format: wgpu::TextureFormat, usage: wgpu::TextureUsages| {
			device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: layers }, mip_level_count: 1, sample_count: samples, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] })
		};
		let ra = wgpu::TextureUsages::RENDER_ATTACHMENT;
		let colour = tex("sky capture msaa", FACE, FACE, 1, crate::viewer::targets::MSAA, self.config.format, ra).create_view(&Default::default());
		let depth = tex("sky capture depth", FACE, FACE, 1, crate::viewer::targets::MSAA, self.depth_format, ra).create_view(&Default::default());
		let ft = tex("sky capture faces", FACE, FACE, 6, 1, self.config.format, ra | wgpu::TextureUsages::TEXTURE_BINDING);
		let faces = (0..6).map(|k| ft.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2), base_array_layer: k, array_layer_count: Some(1), ..Default::default() })).collect();
		let array = ft.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
		let map = tex("sky model", MAP_W, SECTORS + 1, 1, 1, wgpu::TextureFormat::Rgba16Float, ra | wgpu::TextureUsages::TEXTURE_BINDING).create_view(&Default::default());
		let ubufs = (0..6).map(|_| device.create_buffer(&wgpu::BufferDescriptor { label: Some("sky capture uniforms"), size: std::mem::size_of::<Uniforms>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })).collect();
		let rbuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("sky model"), size: 32, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		let samp = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("sky model"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
		// The reduction (and the dev lat-long view).
		let rmod = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("sky model"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("sky model", sky::REDUCE.into())) });
		let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty, count: None };
		let rlayout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("sky model"),
			entries: &[
				entry(0, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
				entry(1, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }),
				entry(2, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
			],
		});
		let rgroup = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("sky model"),
			layout: &rlayout,
			entries: &[
				wgpu::BindGroupEntry { binding: 0, resource: rbuf.as_entire_binding() },
				wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&array) },
				wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&samp) },
			],
		});
		let rpl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("sky model"), bind_group_layouts: &[Some(&rlayout)], immediate_size: 0 });
		let flat = |fs: &str, format: wgpu::TextureFormat| {
			device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
				label: Some("sky model"),
				layout: Some(&rpl),
				vertex: wgpu::VertexState { module: &rmod, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
				fragment: Some(wgpu::FragmentState { module: &rmod, entry_point: Some(fs), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
				primitive: Default::default(),
				depth_stencil: None,
				multisample: Default::default(),
				multiview_mask: None,
				cache: None,
			})
		};
		let reduce = flat("fs_reduce", wgpu::TextureFormat::Rgba16Float);
		let latlong = flat("fs_latlong", wgpu::TextureFormat::Rgba8Unorm);
		// The backdrop: the level shader's bindings (group 0 level uniforms, group 1 one texture) with the model map in group 1.
		let head = &crate::viewer::SHADER[..crate::viewer::SHADER.find("struct VOut").unwrap_or(0)];
		let src = format!("{head}{}{}", sky::LOOKUP.replace("SKYTEX", "tex").replace("SKYSAMP", "samp"), sky::BACKDROP);
		let bmod = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("sky backdrop"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("sky backdrop", src.into())) });
		let ulayout = self.pipeline.get_bind_group_layout(0);
		let bpl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("sky backdrop"), bind_group_layouts: &[Some(&ulayout), Some(&self.tex_layout)], immediate_size: 0 });
		let backdrop = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("sky backdrop"),
			layout: Some(&bpl),
			vertex: wgpu::VertexState { module: &bmod, entry_point: Some("vs_backdrop"), compilation_options: Default::default(), buffers: &[] },
			fragment: Some(wgpu::FragmentState { module: &bmod, entry_point: Some("fs_backdrop"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: self.config.format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
			primitive: Default::default(),
			depth_stencil: Some(wgpu::DepthStencilState { format: self.depth_format, depth_write_enabled: Some(false), depth_compare: Some(wgpu::CompareFunction::Always), stencil: Default::default(), bias: Default::default() }),
			multisample: wgpu::MultisampleState { count: crate::viewer::targets::MSAA, ..Default::default() },
			multiview_mask: None,
			cache: None,
		});
		let bgroup = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("sky backdrop"),
			layout: &self.tex_layout,
			entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&map) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&samp) }],
		});
		Res { colour, depth, faces, rgroup, ubufs, map, rbuf, reduce, latlong, backdrop, bgroup }
	}
}
