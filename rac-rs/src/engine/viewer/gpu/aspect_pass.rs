//! [PC] The Aspect ratio box on the GPU (the shape and the option: viewer/aspect_box.rs, GitHub #45). With a shape other than the
//! window's, the frame is drawn at the BOX size: `Gpu::win_size` is the box (the camera aspect, HUD placement, UI scale, render
//! scale and upscaler all read it or `config`, which holds the box's render size), the swapchain keeps the window size
//! (`Gpu::real_size`). Everything that went onto the window image goes onto the box picture instead (the level and HUD passes or
//! the render-scale copy, the movie, the modern menu pass), and `abox_blit`, the last pass, clears the window image black (the
//! bars) and copies the box picture into its rectangle (`box_rect`, exact texel loads). The screenshot reads the window image, bars
//! included. Auto, or a window within 1 % of the shape (`aspect_box::fit`): no box picture, no pass, the frame takes the same path as
//! before. The copy pipe has its own pipe-guard bit (`Effect::AspectBox`): a refusal fills the window and touches nothing else.
//! `rscale_plan` calls `abox_plan` first, `rscale_sync` plans again when the option changed (`abox_changed`).
//! [PC] Resolution (viewer/resolution.rs, v0.5.1): fullscreen at a chosen size makes a box too: the picture has that size (or the
//! shape's box inside it), the rectangle is the largest one of the picture's shape on the screen (`aspect_box::picture`), and the
//! copy filters (`fs_scale`, bilinear) because the two sizes differ. Without a chosen size the copy stays the exact `fs_box`.

use super::*;
use crate::viewer::aspect_box::{self, Ratio, Rect};

const SHADER: &str = include_str!("aspect_pass.wgsl");

struct Pic {
	tex: wgpu::Texture,
	bind: wgpu::BindGroup,
}

struct Pipe {
	pipeline: wgpu::RenderPipeline,
	/// [PC] Resolution: the filtered copy of a picture whose size is not the rectangle's, built on first use under its own pipe-guard
	/// bit (`Effect::ResScale`, `ensure_scale`): a refusal drops the chosen size, never the plain box.
	scale: Option<wgpu::RenderPipeline>,
	layout: wgpu::BindGroupLayout,
	pl: wgpu::PipelineLayout,
	shader: wgpu::ShaderModule,
	format: wgpu::TextureFormat,
}

/// The box state on `RScale`: the window size, the shape asked for, the rectangle, and (only while the rectangle is not the whole
/// window) the box picture and the copy pipeline.
#[derive(Default)]
pub(in crate::viewer) struct ABox {
	real: (u32, u32),
	ratio: Ratio,
	rect: Rect,
	/// [PC] Resolution: the frame size the box was planned for (`resolution::frame_size`).
	frame: Option<(u32, u32)>,
	pic: Option<Pic>,
	pipe: Option<Pipe>,
	/// The pipe guard refused the copy shader once: no box for the run (the picture fills the window, one log line).
	refused: bool,
	/// [PC] Resolution: the pipe guard refused `fs_scale` once: the chosen fullscreen size is ignored for the run.
	scale_refused: bool,
}

fn make_pipe(device: &wgpu::Device, format: wgpu::TextureFormat) -> Pipe {
	let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
		label: Some("aspect box"),
		entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None }],
	});
	let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("aspect box"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
	let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("aspect box"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("aspect box", SHADER.into())) });
	let pipeline = make_copy(device, &pl, &shader, format, "fs_box");
	Pipe { pipeline, scale: None, layout, pl, shader, format }
}

/// One copy pipeline of the box shader: `fs_box` (exact) or `fs_scale` (filtered).
fn make_copy(device: &wgpu::Device, pl: &wgpu::PipelineLayout, shader: &wgpu::ShaderModule, format: wgpu::TextureFormat, fs: &str) -> wgpu::RenderPipeline {
	device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
		label: Some("aspect box"),
		layout: Some(pl),
		vertex: wgpu::VertexState { module: shader, entry_point: Some("vs_box"), compilation_options: Default::default(), buffers: &[] },
		fragment: Some(wgpu::FragmentState { module: shader, entry_point: Some(fs), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
		primitive: Default::default(),
		depth_stencil: None,
		multisample: Default::default(),
		multiview_mask: None,
		cache: None,
	})
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

	/// [PC] The box's rectangle in window pixels and its picture's size (they differ at a chosen Resolution in fullscreen: the host
	/// scales the mouse with them); None without a box.
	pub fn box_map(&self) -> Option<(Rect, (u32, u32))> {
		let a = &self.rscale.abox;
		a.pic.as_ref().map(|p| (a.rect, (p.tex.width(), p.tex.height())))
	}

	/// The frame is drawn into the box picture (the ReShade add-on stays off then: its picture and depth are the window's own).
	pub(super) fn abox_on(&self) -> bool {
		self.rscale.abox.pic.is_some()
	}

	/// The option differs from the one the sizes were planned for (`rscale_sync` plans again).
	pub(super) fn abox_changed(&self) -> bool {
		let a = &self.rscale.abox;
		a.ratio != aspect_box::current() || (!a.refused && !a.scale_refused && a.frame != crate::viewer::resolution::frame_size(a.real))
	}

	/// First step of `rscale_plan`: the box for the window `real` and the shape in force, with the box picture made, kept or dropped.
	/// Returns the box size (the window size without a box), which render scale and the upscaler then plan from.
	pub(super) fn abox_plan(&mut self, real: (u32, u32)) -> (u32, u32) {
		let want = aspect_box::current();
		let frame = crate::viewer::resolution::frame_size(real); // [PC] Resolution: fullscreen at a chosen size
		let (mut pic_size, mut rect) = aspect_box::picture(real, want, frame);
		let mut asked = want;
		let boxed = |rect: Rect, pic: (u32, u32)| !rect.is_full(real) || pic != rect.size();
		if boxed(rect, pic_size) && self.rscale.abox.pipe.is_none() && !self.rscale.abox.refused {
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
		let mut frame = frame;
		if self.rscale.abox.pipe.is_some() && pic_size != rect.size() && !self.ensure_scale() {
			// [PC] Resolution: no filtered copy (refused): plan as if no size was chosen.
			frame = None;
			(pic_size, rect) = aspect_box::picture(real, want, None);
		}
		if self.rscale.abox.pipe.is_none() {
			(pic_size, rect) = (real, Rect::full(real));
		}
		let format = self.config.format;
		let a = &mut self.rscale.abox;
		(a.real, a.ratio, a.rect, a.frame) = (real, asked, rect, frame);
		let Some(pipe) = a.pipe.as_ref().filter(|_| boxed(rect, pic_size)) else {
			a.pic = None;
			return real;
		};
		if a.pic.is_none() {
			eprintln!("aspect: box {}x{} at {},{} in the {}x{} window, picture {}x{} (ReShade effects are off while the box is on)", rect.w, rect.h, rect.x, rect.y, real.0, real.1, pic_size.0, pic_size.1);
		}
		if a.pic.as_ref().is_none_or(|p| (p.tex.width(), p.tex.height()) != pic_size) {
			a.pic = Some(make_pic(&self.device, pipe, format, pic_size));
		}
		pic_size
	}

	/// [PC] Resolution: the filtered copy (`fs_scale`) built once under `Effect::ResScale`; false when it is refused (then and for the
	/// rest of the run: one log line, the plain box keeps working).
	fn ensure_scale(&mut self) -> bool {
		let a = &mut self.rscale.abox;
		let Some(p) = a.pipe.as_mut() else { return false };
		if p.scale.is_none() && !a.scale_refused {
			let device = &self.device;
			p.scale = crate::viewer::pipe_guard::guard(device, crate::viewer::pipe_guard::Effect::ResScale, || make_copy(device, &p.pl, &p.shader, p.format, "fs_scale"));
			a.scale_refused = p.scale.is_none();
			if a.scale_refused {
				eprintln!("aspect: the resolution scaling shader was refused: the frame is drawn at the screen size");
			}
		}
		p.scale.is_some()
	}

	/// The box picture (None without a box: the window image itself).
	pub(in crate::viewer) fn abox_tex(&self) -> Option<wgpu::Texture> {
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
			timestamp_writes: crate::viewer::passtime::rw("aspect box"),
			occlusion_query_set: None,
			multiview_mask: None,
		});
		let r = clip(a.rect, dst.width(), dst.height());
		if r.w == 0 || r.h == 0 {
			return;
		}
		// [PC] Resolution: a picture of another size than the rectangle is filtered (`fs_scale`); the Aspect ratio box alone copies exactly.
		let exact = (pic.tex.width(), pic.tex.height()) == a.rect.size();
		pass.set_pipeline(match (exact, &p.scale) {
			(false, Some(scale)) => scale,
			_ => &p.pipeline,
		});
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
