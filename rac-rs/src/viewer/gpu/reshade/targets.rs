//! The textures and the three small passes of the ReShade add-on (reshade.rs): a 1x resolved colour target ReShade renders
//! into, the level's depth as a plain R32Float texture, and the pipelines that fill them and copy the result back into the MSAA
//! target for the HUD. Pure wgpu; the native handles ReShade needs come from native.rs.

use wgpu::TextureFormat as F;

pub(super) const SHADER: &str = include_str!("passes.wgsl");

/// The colour format pair ReShade is given (the plain view and the sRGB view of one resource): only the 8-bit swapchain formats.
pub(super) fn view_pair(format: F) -> Option<(F, F)> {
	matches!(format, F::Bgra8Unorm | F::Rgba8Unorm).then(|| (format, format.add_srgb_suffix()))
}

fn layout(device: &wgpu::Device, binding: u32, sample_type: wgpu::TextureSampleType, multisampled: bool) -> wgpu::BindGroupLayout {
	device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
		label: Some("reshade"),
		entries: &[wgpu::BindGroupLayoutEntry {
			binding,
			visibility: wgpu::ShaderStages::FRAGMENT,
			ty: wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled },
			count: None,
		}],
	})
}

/// The pipelines and their bind group layouts; built once per renderer (the formats never change in a run).
pub(super) struct Pipes {
	depth: wgpu::RenderPipeline,
	touch: wgpu::RenderPipeline,
	copy: wgpu::RenderPipeline,
	l_depth: wgpu::BindGroupLayout,
	l_touch: wgpu::BindGroupLayout,
	l_copy: wgpu::BindGroupLayout,
}

impl Pipes {
	pub(super) fn new(device: &wgpu::Device, format: F, depth_format: F) -> Pipes {
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("reshade"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
		let float = wgpu::TextureSampleType::Float { filterable: false };
		let (l_depth, l_touch, l_copy) = (layout(device, 0, wgpu::TextureSampleType::Depth, true), layout(device, 1, float, false), layout(device, 2, float, false));
		let make = |entry: &'static str, l: &wgpu::BindGroupLayout, target: F, ds: Option<wgpu::DepthStencilState>, count: u32| {
			let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("reshade"), bind_group_layouts: &[Some(l)], immediate_size: 0 });
			device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
				label: Some(entry),
				layout: Some(&pl),
				vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
				fragment: Some(wgpu::FragmentState {
					module: &shader,
					entry_point: Some(entry),
					compilation_options: Default::default(),
					targets: &[Some(wgpu::ColorTargetState { format: target, blend: None, write_mask: wgpu::ColorWrites::ALL })],
				}),
				primitive: Default::default(),
				depth_stencil: ds,
				multisample: wgpu::MultisampleState { count, ..Default::default() },
				multiview_mask: None,
				cache: None,
			})
		};
		// The copy-back draws into the HUD pass: same depth / stencil format as its attachment, nothing tested or written.
		let hud_ds = wgpu::DepthStencilState { format: depth_format, depth_write_enabled: Some(false), depth_compare: Some(wgpu::CompareFunction::Always), stencil: Default::default(), bias: Default::default() };
		Pipes {
			depth: make("fs_depth", &l_depth, F::R32Float, None, 1),
			touch: make("fs_touch", &l_touch, F::R8Unorm, None, 1),
			copy: make("fs_copy", &l_copy, format, Some(hud_ds), crate::viewer::MSAA),
			l_depth,
			l_touch,
			l_copy,
		}
	}
}

/// The per-size textures: `rt` (what ReShade renders into; plain view and sRGB view), `depth1` (the level's depth) and the
/// 1x1 target of the touch pass, with the bind groups of the three passes.
pub(super) struct Targets {
	pub(super) rt: wgpu::Texture,
	pub(super) rt_view: wgpu::TextureView,
	pub(super) rt_srgb: wgpu::TextureView,
	pub(super) depth1: wgpu::Texture,
	pub(super) depth1_view: wgpu::TextureView,
	touch: wgpu::TextureView,
	bg_depth: wgpu::BindGroup,
	bg_touch: wgpu::BindGroup,
	bg_copy: wgpu::BindGroup,
}

fn color_target(device: &wgpu::Device, label: &str, w: u32, h: u32, format: F, usage: wgpu::TextureUsages, view_formats: &[F]) -> wgpu::Texture {
	device.create_texture(&wgpu::TextureDescriptor {
		label: Some(label),
		size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format,
		usage,
		view_formats,
	})
}

impl Targets {
	/// None: the format has no sRGB pair (the feature stays off).
	pub(super) fn new(device: &wgpu::Device, pipes: &Pipes, w: u32, h: u32, format: F, depth: &wgpu::TextureView) -> Option<Targets> {
		let (plain, srgb) = view_pair(format)?;
		use wgpu::TextureUsages as U;
		let rt = color_target(device, "reshade rt", w, h, plain, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING | U::COPY_SRC | U::COPY_DST, &[srgb]);
		let view_as = |f: F| rt.create_view(&wgpu::TextureViewDescriptor { format: Some(f), ..Default::default() });
		let (rt_view, rt_srgb) = (view_as(plain), view_as(srgb));
		let depth1 = color_target(device, "reshade depth", w, h, F::R32Float, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING, &[]);
		let depth1_view = depth1.create_view(&Default::default());
		let touch = color_target(device, "reshade touch", 1, 1, F::R8Unorm, U::RENDER_ATTACHMENT, &[]).create_view(&Default::default());
		let depth_only = depth.texture().create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
		let group = |l: &wgpu::BindGroupLayout, binding: u32, v: &wgpu::TextureView| {
			device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("reshade"), layout: l, entries: &[wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView(v) }] })
		};
		Some(Targets {
			bg_depth: group(&pipes.l_depth, 0, &depth_only),
			bg_touch: group(&pipes.l_touch, 1, &depth1_view),
			bg_copy: group(&pipes.l_copy, 2, &rt_view),
			rt,
			rt_view,
			rt_srgb,
			depth1,
			depth1_view,
			touch,
		})
	}

	/// Part A, no pass open: the level's depth into `depth1`, the MSAA picture resolved into `rt`, `rt` left as a colour
	/// attachment (the render_target state ReShade asks for), `depth1` left as a shader resource (read by the touch pass).
	pub(super) fn encode(&self, pipes: &Pipes, enc: &mut wgpu::CommandEncoder, msaa: &wgpu::TextureView) {
		fn attach<'a>(view: &'a wgpu::TextureView, resolve: Option<&'a wgpu::TextureView>, load: wgpu::LoadOp<wgpu::Color>, store: wgpu::StoreOp) -> [Option<wgpu::RenderPassColorAttachment<'a>>; 1] {
			[Some(wgpu::RenderPassColorAttachment { view, resolve_target: resolve, depth_slice: None, ops: wgpu::Operations { load, store } })]
		}
		fn pass<'e>(enc: &'e mut wgpu::CommandEncoder, label: &'static str, colors: &[Option<wgpu::RenderPassColorAttachment>]) -> wgpu::RenderPass<'e> {
			enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some(label), color_attachments: colors, depth_stencil_attachment: None, timestamp_writes: crate::viewer::passtime::rw(label), occlusion_query_set: None, multiview_mask: None })
		}
		let clear = wgpu::LoadOp::Clear(wgpu::Color::BLACK);
		let mut p = pass(enc, "rs depth", &attach(&self.depth1_view, None, clear, wgpu::StoreOp::Store));
		p.set_pipeline(&pipes.depth);
		p.set_bind_group(0, &self.bg_depth, &[]);
		p.draw(0..3, 0..1);
		drop(p);
		drop(pass(enc, "rs resolve", &attach(msaa, Some(&self.rt_view), wgpu::LoadOp::Load, wgpu::StoreOp::Discard)));
		drop(pass(enc, "rs rt", &attach(&self.rt_view, None, wgpu::LoadOp::Load, wgpu::StoreOp::Store)));
		let mut p = pass(enc, "rs touch", &attach(&self.touch, None, clear, wgpu::StoreOp::Discard));
		p.set_pipeline(&pipes.touch);
		p.set_bind_group(0, &self.bg_touch, &[]);
		p.draw(0..3, 0..1);
	}

	/// Part B, inside the HUD pass: the picture ReShade returned into every sample of the MSAA target.
	pub(super) fn copy_back(&self, pipes: &Pipes, pass: &mut wgpu::RenderPass<'_>) {
		pass.set_pipeline(&pipes.copy);
		pass.set_bind_group(0, &self.bg_copy, &[]);
		pass.draw(0..3, 0..1);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn shader_validates() {
		let m = naga::front::wgsl::parse_str(SHADER).expect("wgsl parse");
		naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("wgsl validate");
	}

	#[test]
	fn the_depth_pass_reads_every_msaa_sample() {
		assert!(SHADER.contains(&format!("const SAMPLES: i32 = {};", crate::viewer::MSAA)), "passes.wgsl SAMPLES must equal the renderer's MSAA");
	}

	#[test]
	fn only_the_8_bit_swapchain_formats_have_a_pair() {
		assert_eq!(view_pair(F::Bgra8Unorm), Some((F::Bgra8Unorm, F::Bgra8UnormSrgb)));
		assert_eq!(view_pair(F::Rgba8Unorm), Some((F::Rgba8Unorm, F::Rgba8UnormSrgb)));
		assert_eq!(view_pair(F::Bgra8UnormSrgb), None);
		assert_eq!(view_pair(F::Rgba16Float), None);
		assert_eq!(view_pair(F::Rgb10a2Unorm), None);
	}
}
