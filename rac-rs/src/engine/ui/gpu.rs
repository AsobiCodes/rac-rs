//! The GPU pass of the modern UI, drawn once per display frame over the finished window image (`viewer/gpu/ui_pass.rs`, right
//! after the render-scale copy). The game side hands in ONE frame's draw list with `set_scene` (valid for the next `draw` only,
//! so a menu that closes simply stops calling it); with `RAC_UI_DEMO` the built-in demo screen is drawn instead.
//!
//! What stays and what goes. The shader and its four pipelines are built ONCE (`warm` at start-up, else the first `draw`): a
//! rebuild is a WGSL parse plus a driver compile and four pipeline builds on the render thread, a visible hitch on DX12 and on the
//! Deck, so a menu that closes and opens again, or a hand-over between two pages, never pays it. They hold no notable GPU memory.
//! A frame without a scene (`sleep`) frees what scales with the window or the page: the blur chain, the instance buffer and the
//! glyph atlas TEXTURE (a few MB), and restarts the backdrop ease. The CPU glyph cache stays (1 to 4 MB, drops itself when the UI
//! scale changes): it holds every rasterised glyph, so a reopened page uploads one bitmap instead of rasterising its text again.
//! `release` frees everything, the pipelines included. A driver that refuses the UI shader switches `Effect::Ui` off (`refused()`),
//! which `warm` already knows before any menu can open: nothing is drawn and the game side keeps the classic menu from the start.
use super::backdrop::{self, Ease};
use super::backdrop_gpu::BackRes;
use super::font::{FontMeasure, FontSet};
use super::glyph_cache::GlyphCache;
use super::gpu_pipes::{self, Params, Pipes};
use super::images;
use super::instances::{self, Inst, INST_BYTES};
use super::kit::Scale;
use super::prim::Prim;
use super::{demo, Ui};
use crate::fmv::present::QueueWrites;
use crate::viewer::pipe_guard::{self, Effect};

const INST_START: u64 = 512;

#[derive(Default)]
pub struct UiGpu {
	scene: Option<Vec<Prim>>,
	demo: Option<Ui>,
	/// Shader and pipelines for one target format, kept once built (see the module doc).
	pipes: Option<(wgpu::TextureFormat, Pipes)>,
	/// The glyph cache and what a page needs while it is up.
	live: Option<Live>,
	/// How often the shader and pipelines were built (tests: open, close, open must build once).
	builds: u32,
}

struct Live {
	cache: GlyphCache,
	params: wgpu::Buffer,
	insts: Option<wgpu::Buffer>,
	cap: u64,
	atlas: Option<(wgpu::Texture, wgpu::BindGroup)>,
	/// The picture atlas (`images.rs`) with the generation of the set it holds.
	pics: Option<(u64, wgpu::Texture, wgpu::BindGroup)>,
	back: Option<BackRes>,
	ease: Ease,
}

/// What a `UiGpu` holds (the lifecycle tests read it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Held {
	/// Shader and pipelines.
	pub pipes: bool,
	/// The glyph cache (CPU) and the small uniform buffer.
	pub cache: bool,
	/// The glyph atlas texture.
	pub atlas: bool,
	/// The picture atlas texture (only while a frame draws a picture).
	pub pics: bool,
	/// The blur chain (window-size textures).
	pub backdrop: bool,
	/// The instance buffer.
	pub insts: bool,
}

/// The driver refused the UI shader or pipelines (`Effect::Ui`): the modern menu cannot be drawn, use the classic one.
pub fn refused() -> bool {
	pipe_guard::off(Effect::Ui)
}

impl UiGpu {
	/// The draw list of the next frame (design units, from `Ui::frame()`); drawn by the next `draw` only.
	pub fn set_scene(&mut self, mut prims: Vec<Prim>) {
		for prim in &mut prims { if let Prim::Text(run) = prim { run.text = crate::localization::ui_text(&run.text).to_owned(); } }
		self.scene = Some(prims);
	}

	/// Whether anything is held (the off-state test): a scene, a demo screen or any GPU object, the kept pipelines included.
	pub fn is_allocated(&self) -> bool {
		self.scene.is_some() || self.demo.is_some() || self.pipes.is_some() || self.live.is_some()
	}

	/// What is held right now.
	pub fn held(&self) -> Held {
		let l = self.live.as_ref();
		Held { pipes: self.pipes.is_some(), cache: l.is_some(), atlas: l.is_some_and(|l| l.atlas.is_some()), pics: l.is_some_and(|l| l.pics.is_some()), backdrop: l.is_some_and(|l| l.back.is_some()), insts: l.is_some_and(|l| l.insts.is_some()) }
	}

	/// How many times the shader and pipelines were built.
	pub fn pipe_builds(&self) -> u32 {
		self.builds
	}

	/// Frees everything, the pipelines included.
	pub fn release(&mut self) {
		self.scene = None;
		self.demo = None;
		self.pipes = None;
		self.live = None;
	}

	/// A frame without a scene: drops the scene and what scales with the window or the page; the pipelines and the glyph cache stay.
	fn sleep(&mut self) {
		self.scene = None;
		self.demo = None;
		if let Some(l) = self.live.as_mut() {
			l.sleep();
		}
	}

	/// Builds the shader and pipelines for `format` unless they exist (another format drops the old ones and what was made with
	/// them). False when the driver refused them (`refused()` is then true for the run).
	fn ensure_pipes(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) -> bool {
		if self.pipes.as_ref().is_some_and(|(f, _)| *f == format) {
			return true;
		}
		self.live = None;
		self.pipes = pipe_guard::guard(device, Effect::Ui, || gpu_pipes::make(device, format)).map(|p| (format, p));
		if self.pipes.is_some() {
			self.builds += 1;
		}
		self.pipes.is_some()
	}

	/// Start-up: builds and so validates the pipelines before any menu can open, so `refused()` is known by the time the game side
	/// decides to hide the retail menu (a refusal found by the first draw would cost one frame with neither menu on screen).
	pub fn warm(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) {
		self.ensure_pipes(device, format);
	}

	fn take_prims(&mut self, win: (u32, u32)) -> Option<Vec<Prim>> {
		if let Some(p) = self.scene.take() {
			return Some(p);
		}
		let spec = demo::enabled()?;
		let ui = self.demo.get_or_insert_with(|| demo::new_ui(spec, Box::new(FontMeasure)));
		ui.set_window(win.0, win.1);
		Some(ui.frame().1)
	}

	/// Draws the frame's scene (or the demo) onto `target`; false when nothing was drawn (the window-size objects are freed then).
	pub fn draw(&mut self, device: &wgpu::Device, q: &dyn QueueWrites, enc: &mut wgpu::CommandEncoder, target: &wgpu::Texture, format: wgpu::TextureFormat, win: (u32, u32)) -> bool {
		if refused() {
			self.release();
			return false;
		}
		let prims = if win.0 == 0 || win.1 == 0 { None } else { self.take_prims(win) };
		let Some(prims) = prims else {
			self.sleep();
			return false;
		};
		if !self.ensure_pipes(device, format) {
			self.release();
			return false;
		}
		let Some((_, pipes)) = self.pipes.as_ref() else { return false };
		let live = self.live.get_or_insert_with(|| Live::new(device));
		live.draw(device, q, enc, target, pipes, format, win, &prims);
		true
	}
}

impl Live {
	fn new(device: &wgpu::Device) -> Live {
		let params = device.create_buffer(&wgpu::BufferDescriptor { label: Some("ui params"), size: std::mem::size_of::<Params>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		Live { cache: GlyphCache::default(), params, insts: None, cap: 0, atlas: None, pics: None, back: None, ease: Ease::default() }
	}

	/// No scene this frame: frees the window-size and page-size GPU objects and restarts the backdrop ease (the next page takes its
	/// first target at once, as it always did after a gap). The glyph cache and the uniform buffer stay.
	fn sleep(&mut self) {
		self.back = None;
		self.atlas = None;
		self.pics = None;
		self.insts = None;
		self.cap = 0;
		self.ease.reset();
	}

	#[allow(clippy::too_many_arguments)]
	fn draw(&mut self, device: &wgpu::Device, q: &dyn QueueWrites, enc: &mut wgpu::CommandEncoder, target: &wgpu::Texture, pipes: &Pipes, format: wgpu::TextureFormat, win: (u32, u32), prims: &[Prim]) {
		let sc = Scale::fit(win.0, win.1);
		let knobs = backdrop::knobs();
		self.cache.begin_frame(sc.s);
		// ONE read of the picture set for the UVs and the texture: a level load registering a new set mid-frame cannot pair this
		// frame's UVs with the next set's atlas.
		let pics = images::current();
		let uv_of = |id: u32| pics.as_ref().and_then(|(_, set)| set.uv(id));
		let built = instances::build_with(prims, &sc, FontSet::get(), &mut self.cache, &uv_of);
		self.upload_atlas(device, q, pipes);
		self.upload_pics(device, q, pipes, built.insts.iter().any(|i| i.q[2] == instances::K_IMAGE), pics);
		self.upload_insts(device, q, &built.insts);
		q.write_buffer(&self.params, 0, bytemuck::bytes_of(&Params { a: [win.0 as f32, win.1 as f32, knobs.text_gamma, 0.0], ..Default::default() }));
		let blur_level = self.backdrop(device, q, enc, target, pipes, format, win, &sc, built.backdrop, &knobs);
		let view = target.create_view(&Default::default());
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("modern ui"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
			depth_stencil_attachment: None,
			timestamp_writes: None,
			occlusion_query_set: None,
			multiview_mask: None,
		});
		if let (Some(k), Some(back)) = (blur_level, &self.back) {
			pass.set_pipeline(&pipes.comp);
			pass.set_bind_group(0, back.comp_group(k), &[]);
			pass.draw(0..3, 0..1);
		}
		if let (false, Some((_, group)), Some(insts)) = (built.insts.is_empty(), &self.atlas, &self.insts) {
			pass.set_pipeline(&pipes.inst);
			pass.set_vertex_buffer(0, insts.slice(..));
			// One draw per run of instances that read the same texture (glyph atlas, or the picture atlas for `K_IMAGE`), in draw order.
			for (is_pic, a, b) in runs(&built.insts) {
				match (is_pic, &self.pics) {
					(false, _) => pass.set_bind_group(0, group, &[]),
					(true, Some((_, _, pg))) => pass.set_bind_group(0, pg, &[]),
					(true, None) => continue,
				}
				pass.draw(0..6, a..b);
			}
		}
	}

	/// Blurs the window image into the chain when the scene asks for a backdrop; returns the blur level the composite uses.
	#[allow(clippy::too_many_arguments)]
	fn backdrop(&mut self, device: &wgpu::Device, q: &dyn QueueWrites, enc: &mut wgpu::CommandEncoder, target: &wgpu::Texture, pipes: &Pipes, format: wgpu::TextureFormat, win: (u32, u32), sc: &Scale, request: Option<(f32, super::prim::Rgba)>, knobs: &backdrop::Knobs) -> Option<u32> {
		let Some((blur_px, scrim)) = request else {
			self.ease.reset();
			return None;
		};
		let now = backdrop::now_secs();
		self.ease.target([blur_px, scrim.a as f32 / 255.0], now, knobs.ease_s);
		let [blur, alpha] = self.ease.value(now, knobs.ease_s);
		if self.back.as_ref().map(|b| b.win) != Some(win) {
			self.back = BackRes::new(device, pipes, format, win);
		}
		let back = self.back.as_ref()?;
		let (k, sigma) = backdrop::plan(blur * knobs.blur * sc.s);
		let k = k.min(back.k_max);
		let colour = [scrim.r as f32 / 255.0, scrim.g as f32 / 255.0, scrim.b as f32 / 255.0, (alpha * knobs.scrim).clamp(0.0, 1.0)];
		back.encode(enc, q, pipes, target, k, sigma, colour, knobs.dither);
		Some(k)
	}

	fn upload_insts(&mut self, device: &wgpu::Device, q: &dyn QueueWrites, insts: &[Inst]) {
		if insts.is_empty() {
			return;
		}
		if self.insts.is_none() || insts.len() as u64 > self.cap {
			self.cap = (insts.len() as u64).next_power_of_two().max(INST_START);
			self.insts = Some(inst_buffer(device, self.cap));
		}
		if let Some(buf) = &self.insts {
			q.write_buffer(buf, 0, bytemuck::cast_slice(insts));
		}
	}

	/// Uploads the registered picture set (`images.rs`) when a frame draws a picture and the texture is missing or from an older set.
	fn upload_pics(&mut self, device: &wgpu::Device, q: &dyn QueueWrites, pipes: &Pipes, wanted: bool, pics: Option<(u64, std::sync::Arc<images::ImageSet>)>) {
		if !wanted {
			return;
		}
		let Some((generation, set)) = pics else { return };
		if self.pics.as_ref().is_some_and(|(g, ..)| *g == generation) {
			return;
		}
		let max = device.limits().max_texture_dimension_2d;
		if set.w > max || set.h > max {
			self.pics = None; // past the device's texture size: the pictures' runs are skipped (no validation error)
			return;
		}
		let tex = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("ui pictures"),
			size: wgpu::Extent3d { width: set.w, height: set.h, depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::Rgba8Unorm,
			usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
			view_formats: &[],
		});
		let info = wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All };
		let layout = wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(set.w * 4), rows_per_image: Some(set.h) };
		q.write_texture(info, &set.rgba, layout, wgpu::Extent3d { width: set.w, height: set.h, depth_or_array_layers: 1 });
		let group = gpu_pipes::bind(device, pipes, &tex.create_view(&Default::default()), &self.params);
		self.pics = Some((generation, tex, group));
	}

	/// Makes the atlas texture again when the cache grew or the texture was freed (uploaded whole), else uploads only the rows that changed.
	fn upload_atlas(&mut self, device: &wgpu::Device, q: &dyn QueueWrites, pipes: &Pipes) {
		let size = self.cache.size();
		if self.cache.take_resized() || self.atlas.is_none() {
			let tex = device.create_texture(&wgpu::TextureDescriptor {
				label: Some("ui glyphs"),
				size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
				mip_level_count: 1,
				sample_count: 1,
				dimension: wgpu::TextureDimension::D2,
				format: wgpu::TextureFormat::R8Unorm,
				usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
				view_formats: &[],
			});
			let group = gpu_pipes::bind(device, pipes, &tex.create_view(&Default::default()), &self.params);
			self.cache.take_dirty();
			write_rows(q, &tex, &self.cache.pixels, size, 0, size);
			self.atlas = Some((tex, group));
		} else if let (Some((a, b)), Some((tex, _))) = (self.cache.take_dirty(), &self.atlas) {
			write_rows(q, tex, &self.cache.pixels, size, a, b.min(size));
		}
	}
}

/// The instances split into runs of the same texture: (reads the picture atlas, first, past the last).
fn runs(insts: &[Inst]) -> Vec<(bool, u32, u32)> {
	let mut out: Vec<(bool, u32, u32)> = Vec::new();
	for (i, inst) in insts.iter().enumerate() {
		let pic = inst.q[2] == instances::K_IMAGE;
		match out.last_mut() {
			Some(r) if r.0 == pic => r.2 = i as u32 + 1,
			_ => out.push((pic, i as u32, i as u32 + 1)),
		}
	}
	out
}

fn inst_buffer(device: &wgpu::Device, cap: u64) -> wgpu::Buffer {
	device.create_buffer(&wgpu::BufferDescriptor { label: Some("ui instances"), size: cap * INST_BYTES, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
}

fn write_rows(q: &dyn QueueWrites, tex: &wgpu::Texture, pixels: &[u8], size: u32, a: u32, b: u32) {
	if b <= a {
		return;
	}
	let info = wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d { x: 0, y: a, z: 0 }, aspect: wgpu::TextureAspect::All };
	let layout = wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size), rows_per_image: Some(b - a) };
	q.write_texture(info, &pixels[(a * size) as usize..(b * size) as usize], layout, wgpu::Extent3d { width: size, height: b - a, depth_or_array_layers: 1 });
}
