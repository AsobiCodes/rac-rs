//! [PC] The Aspect ratio box on the GPU (the shape and the option: viewer/aspect_box.rs, GitHub #45). With a shape other than the
//! window's, the frame is drawn at the BOX size: `Gpu::win_size` is the box (the camera aspect, HUD placement, UI scale, render
//! scale and upscaler all read it or `config`, which holds the box's render size), the swapchain keeps the window size
//! (`Gpu::real_size`). Everything that went onto the window image goes onto the box picture instead (the level and HUD passes or
//! the render-scale copy, the movie, the modern menu pass), and `abox_blit`, the last pass, clears the window image black (the
//! bars) and copies the box picture into its rectangle (`box_rect`, exact texel loads). The screenshot reads the window image, bars
//! included. Auto, or a window within 1 % of the shape (`aspect_box::fit`): no box picture, no pass, the frame takes the same path as
//! before. The copy pipe has its own pipe-guard bit (`Effect::AspectBox`): a refusal fills the window and touches nothing else.
//! `rscale_plan` calls `abox_plan` first, `rscale_sync` plans again when the option changed (`abox_changed`).

use super::*;
use crate::viewer::aspect_box::{self, Ratio, Rect};

const SHADER: &str = include_str!("aspect_pass.wgsl");

struct Pic {
	tex: wgpu::Texture,
	bind: wgpu::BindGroup,
}

struct Pipe {
	pipeline: wgpu::RenderPipeline,
	layout: wgpu::BindGroupLayout,
}

/// The box state on `RScale`: the window size, the shape asked for, the rectangle, and (only while the rectangle is not the whole
/// window) the box picture and the copy pipeline.
#[derive(Default)]
pub(in crate::viewer) struct ABox {
	real: (u32, u32),
	ratio: Ratio,
	rect: Rect,
	pic: Option<Pic>,
	pipe: Option<Pipe>,
	/// The pipe guard refused the copy shader once: no box for the run (the picture fills the window, one log line).
	refused: bool,
}

fn make_pipe(device: &wgpu::Device, format: wgpu::TextureFormat) -> Pipe {
	let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
		label: Some("aspect box"),
		entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None }],
	});
	let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("aspect box"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
	let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("aspect box"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("aspect box", SHADER.into())) });
	let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
		label: Some("aspect box"),
		layout: Some(&pl),
		vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_box"), compilation_options: Default::default(), buffers: &[] },
		fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_box"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
		primitive: Default::default(),
		depth_stencil: None,
		multisample: Default::default(),
		multiview_mask: None,
		cache: None,
	});
	Pipe { pipeline, layout }
}

/// The box picture: the window image's format and uses (the modern menu pass copies from it for its blur) plus sampling.
fn make_pic(device: &wgpu::Device, pipe: &Pipe, format: wgpu::TextureFormat, size: (u32, u32)) -> Pic {
	let tex = device.create_texture(&wgpu::TextureDescriptor {
		label: Some("aspect box"),
		size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format,
		usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
		view_formats: &[],
	});
	let view = tex.create_view(&Default::default());
	let bind = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("aspect box"), layout: &pipe.layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }] });
	Pic { tex, bind }
}

/// The part of `r` inside a `w` x `h` image (a window image of another size for a frame while the window is resized): the scissor.
fn clip(r: Rect, w: u32, h: u32) -> Rect {
	let (x, y) = (r.x.min(w), r.y.min(h));
	Rect { x, y, w: r.w.min(w - x), h: r.h.min(h - y) }
}

impl Gpu {
	/// The window's own size in pixels (the swapchain, the screenshot); `win_size` is the box the frame is drawn at.
	pub fn real_size(&self) -> (u32, u32) {
		let r = self.rscale.abox.real;
		if r.0 == 0 { self.win_size() } else { r }
	}

	/// The picture's rectangle in window pixels (the whole window without a box): the host maps the mouse with it.
	pub fn box_rect(&self) -> Rect {
		if self.rscale.abox.pic.is_some() { self.rscale.abox.rect } else { Rect::full(self.real_size()) }
	}

	/// The frame is drawn into the box picture (the ReShade add-on stays off then: its picture and depth are the window's own).
	pub(super) fn abox_on(&self) -> bool {
		self.rscale.abox.pic.is_some()
	}

	/// The option differs from the one the sizes were planned for (`rscale_sync` plans again).
	pub(super) fn abox_changed(&self) -> bool {
		self.rscale.abox.ratio != aspect_box::current()
	}

	/// First step of `rscale_plan`: the box for the window `real` and the shape in force, with the box picture made, kept or dropped.
	/// Returns the box size (the window size without a box), which render scale and the upscaler then plan from.
	pub(super) fn abox_plan(&mut self, real: (u32, u32)) -> (u32, u32) {
		let want = aspect_box::current();
		let mut rect = aspect_box::fit(real, want);
		let mut asked = want;
		if !rect.is_full(real) && self.rscale.abox.pipe.is_none() && !self.rscale.abox.refused {
			let format = self.config.format;
			// [PC] pipe_guard.rs: a driver that refuses the copy shader fills the window, no crash; `asked` = Auto makes the next
			// `rscale_sync` plan once more, so `config` (Gpu::new's `start_size` planned the box) follows the window size.
			self.rscale.abox.pipe = crate::viewer::pipe_guard::guard(&self.device, crate::viewer::pipe_guard::Effect::AspectBox, || make_pipe(&self.device, format));
			if self.rscale.abox.pipe.is_none() {
				self.rscale.abox.refused = true;
				asked = Ratio::Auto;
				eprintln!("aspect: the box copy shader was refused: the picture fills the window");
			}
		}
		if self.rscale.abox.pipe.is_none() {
			rect = Rect::full(real);
		}
		let format = self.config.format;
		let a = &mut self.rscale.abox;
		(a.real, a.ratio, a.rect) = (real, asked, rect);
		let Some(pipe) = a.pipe.as_ref().filter(|_| !rect.is_full(real)) else {
			a.pic = None;
			return real;
		};
		if a.pic.is_none() {
			eprintln!("aspect: box {}x{} at {},{} in the {}x{} window (ReShade effects are off while bars are shown)", rect.w, rect.h, rect.x, rect.y, real.0, real.1);
		}
		if a.pic.as_ref().is_none_or(|p| (p.tex.width(), p.tex.height()) != rect.size()) {
			a.pic = Some(make_pic(&self.device, pipe, format, rect.size()));
		}
		rect.size()
	}

	/// The box picture (None without a box: the window image itself).
	pub(super) fn abox_tex(&self) -> Option<wgpu::Texture> {
		self.rscale.abox.pic.as_ref().map(|p| p.tex.clone())
	}

	/// The view the frame is drawn into when there is a box and no render-scale picture.
	pub(super) fn abox_view(&self) -> Option<wgpu::TextureView> {
		self.rscale.abox.pic.as_ref().map(|p| p.tex.create_view(&Default::default()))
	}

	/// The last pass of a boxed frame: the window image `dst` cleared black, the box picture copied into its rectangle. Nothing
	/// without a box.
	pub(super) fn abox_blit(&self, enc: &mut wgpu::CommandEncoder, dst: &wgpu::Texture) {
		let a = &self.rscale.abox;
		let (Some(pic), Some(p)) = (&a.pic, &a.pipe) else { return };
		let view = dst.create_view(&Default::default());
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("aspect box"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
			depth_stencil_attachment: None,
			timestamp_writes: None,
			occlusion_query_set: None,
			multiview_mask: None,
		});
		let r = clip(a.rect, dst.width(), dst.height());
		if r.w == 0 || r.h == 0 {
			return;
		}
		pass.set_pipeline(&p.pipeline);
		pass.set_bind_group(0, &pic.bind, &[]);
		// The viewport is the whole rectangle (wgpu allows a viewport past the image), the scissor its part inside the image: a window
		// image smaller than the box (a frame during a resize) shows the box picture cropped, never squeezed. Both are the rectangle
		// whenever the image has the window size (every frame but such a one).
		pass.set_viewport(a.rect.x as f32, a.rect.y as f32, a.rect.w as f32, a.rect.h as f32, 0.0, 1.0);
		pass.set_scissor_rect(r.x, r.y, r.w, r.h);
		pass.draw(0..3, 0..1);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn clip_keeps_the_rectangle_inside_the_image() {
		let r = Rect { x: 160, y: 0, w: 960, h: 720 };
		assert_eq!(clip(r, 1280, 720), r);
		assert_eq!(clip(r, 1000, 700), Rect { x: 160, y: 0, w: 840, h: 700 });
		assert_eq!(clip(r, 100, 100), Rect { x: 100, y: 0, w: 0, h: 100 });
	}

	#[test]
	fn shader_validates() {
		let m = naga::front::wgsl::parse_str(SHADER).expect("wgsl parse");
		naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("wgsl validate");
	}
}
