//! [PC] Frame generation (lane fgen, S1): who presents the window. `Present::Wgpu` = wgpu's surface (today's path, the default);
//! `Present::Own` = our own swapchain (RAC_FG=null; S2 / S3 put the vendors' swapchains here) with our own window-size backbuffer
//! that every pass draws into. `FrameImage` is the frame's window image either way (gpu/draw/prepare.rs `acquire_frame`): the
//! surface texture, or the backbuffer + the acquired swapchain image. At the frame's end (gpu/draw/finish.rs) `encode_out` copies
//! the backbuffer onto the image, the screenshot reads the image (`shot`), `encode_present` moves it to the present state; the
//! presenter (presenter.rs) submits the frame with `submit_last` and presents it with `present`.
//!
//! Switching (`Screen::set_own`): the presenter is flushed, the GPU idled, the old present dropped (one swapchain per window) and the
//! other made; a refused own swapchain falls back to wgpu's surface with one log line. Resize / minimize: `configure` (a window
//! without area keeps no own swapchain and skips frames until it has one); out of date / a dropped frame: made again before the
//! next acquire; a lost device: the frame is skipped and gpu_errors.rs reports the loss.
//!
//! [S2] RAC_FG=fsr: the own swapchain is FidelityFX's (`Screen::fg`); fgen/frame.rs leaves the frame's inputs in `pending`
//! (`set_pending`), gpu/draw/finish.rs attaches them to the frame (`take_pending` + `FrameImage::attach_fg`), and `present`
//! configures frame generation for that frame right before the present (DirectX 12: after running its prepare list).

#[cfg(windows)]
use super::ffx_fg::{Fg, FgFrame};
#[cfg(windows)]
use super::sl::{Sl, SlFrame};
use std::sync::Arc;
use winit::window::Window;

pub(crate) enum Present {
	Wgpu(wgpu::Surface<'static>),
	#[cfg(windows)]
	Own(Own),
	/// Only while one present is swapped for the other (`Screen::set_own`): nothing on the window.
	#[cfg(windows)]
	Gone,
}

#[cfg(windows)]
pub(crate) struct Own {
	swap: OwnSwap,
	/// Our window-size backbuffer (the window's format, the window image's usages).
	back: Option<wgpu::Texture>,
}

#[cfg(windows)]
enum OwnSwap {
	Vk(super::swap_vk::VkSwap),
	Dx(super::swap_dx12::DxSwap),
}

/// The window's present and what it needs to make the other one.
pub(crate) struct Screen {
	instance: wgpu::Instance,
	window: Arc<Window>,
	pub(crate) kind: Present,
	/// [S2] RAC_FG=fsr: FidelityFX frame generation on our own swapchain; this frame's inputs; the hudless picture.
	#[cfg(windows)]
	fg: Option<Arc<Fg>>,
	#[cfg(windows)]
	pending: Option<FgFrame>,
	#[cfg(windows)]
	hudless: Option<wgpu::Texture>,
	/// [S3] RAC_FG=dlss2..4: DLSS frame generation through Streamline (sl.rs) on our own swapchain; this frame's Streamline frame
	/// (`sl_begin` at the acquire, sl_frame.rs marks it prepared, `take_sl` hands it to the present).
	#[cfg(windows)]
	sl: Option<&'static Sl>,
	#[cfg(windows)]
	sl_cur: Option<SlFrame>,
}

/// The frame's window image.
pub(crate) enum FrameImage {
	Surface(wgpu::SurfaceTexture),
	#[cfg(windows)]
	Own(OwnFrame),
}

#[cfg(windows)]
pub(crate) struct OwnFrame {
	back: wgpu::Texture,
	image: wgpu::Texture,
	sync: FrameSync,
	/// [S2] Frame generation for this frame (configured right before its present).
	fg: Option<(Arc<Fg>, FgFrame)>,
	/// [S3] DLSS frame generation for this frame (its options and markers around the present).
	sl: Option<(&'static Sl, SlFrame)>,
}

#[cfg(windows)]
enum FrameSync {
	Vk(super::swap_vk::VkFrame),
	Dx(super::swap_dx12::DxFrame),
}

/// What `Screen::acquire` gave.
pub(crate) enum Acquired {
	Frame(FrameImage),
	/// No image this frame (minimized, timeout, a lost device).
	Skip,
	/// Configure again (`Screen::configure`), then skip the frame.
	Outdated,
}

/// [S6] The presented frame's generation inputs (pure): what the render thread left for this id (fgen/frame.rs records the prepare
/// with `Fg::peek_id`, the id `take_pending` then takes), else a frame without generation. Inputs of another id never carry
/// `prepared` over (their prepare was not this frame's).
#[cfg(windows)]
pub(super) fn pick(pending: Option<FgFrame>, id: u64) -> FgFrame {
	pending.filter(|f| f.id == id).unwrap_or_else(|| FgFrame { id, prepared: false, hudless: crate::viewer::upscale::ffx_abi::Resource::none(), dx: None })
}

/// The window's raw handles (Windows: HWND, HINSTANCE).
#[cfg(windows)]
fn hwnd_of(window: &Window) -> Option<(isize, isize)> {
	use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
	match window.window_handle().ok()?.as_raw() {
		RawWindowHandle::Win32(h) => Some((h.hwnd.get(), h.hinstance.map_or(0, |i| i.get()))),
		_ => None,
	}
}

impl Screen {
	pub(crate) fn new(instance: wgpu::Instance, window: Arc<Window>, surface: wgpu::Surface<'static>) -> Screen {
		#[cfg(windows)]
		return Screen { instance, window, kind: Present::Wgpu(surface), fg: None, pending: None, hudless: None, sl: None, sl_cur: None };
		#[cfg(not(windows))]
		Screen { instance, window, kind: Present::Wgpu(surface) }
	}

	/// Gpu::new's screen: wgpu's surface, then our own swapchain when RAC_FG=null or FSR frame generation asks for it (`set_own`).
	pub(crate) fn open(instance: wgpu::Instance, window: Arc<Window>, surface: wgpu::Surface<'static>, adapter: &wgpu::Adapter, device: &wgpu::Device, queue: &wgpu::Queue, config: &wgpu::SurfaceConfiguration) -> Screen {
		let mut screen = Screen::new(instance, window, surface);
		// [S3] another card or driver: its DLSS memory goes (no Streamline call); the driver as DXGI reports it (sl_card.rs).
		let info = adapter.get_info();
		#[cfg(windows)]
		let driver = super::sl_card::driver_of(&info.name).unwrap_or(&info.driver_info);
		#[cfg(not(windows))]
		let driver = info.driver_info.as_str();
		super::choice::card_seen(&info.name, driver);
		if matches!(super::mode(), super::Mode::Null | super::Mode::Fsr | super::Mode::Dlss(_)) {
			screen.set_own(true, Some(adapter), device, queue, config);
		}
		screen
	}

	/// [S2] The window's FidelityFX frame generation while it presents.
	#[cfg(windows)]
	pub(crate) fn fg(&self) -> Option<&Arc<Fg>> {
		self.fg.as_ref().filter(|_| self.own())
	}

	/// [S3] The window's DLSS frame generation while it presents.
	#[cfg(windows)]
	pub(crate) fn sl(&self) -> Option<&'static Sl> {
		self.sl.filter(|_| self.own())
	}

	/// [S3] At the frame's start (gpu/draw/prepare.rs `acquire_frame`, before the acquire): the frame's Streamline token, Reflex's
	/// sleep and the simulation markers. Nothing without DLSS frame generation.
	pub(crate) fn sl_begin(&mut self) {
		#[cfg(windows)]
		{
			self.sl_cur = self.sl().filter(|s| s.usable()).and_then(|s| s.begin_frame());
		}
	}

	/// [S3] This frame's Streamline frame (sl_frame.rs).
	#[cfg(windows)]
	pub(crate) fn sl_frame(&self) -> Option<SlFrame> {
		self.sl_cur
	}

	/// [S3] sl_frame.rs set this frame's constants and tags (`ok`): generation runs for it.
	#[cfg(windows)]
	pub(crate) fn sl_prepared(&mut self, ok: bool) {
		if let Some(f) = self.sl_cur.as_mut() {
			f.prepared = ok;
		}
	}

	/// [S3] This frame's Streamline frame for the present (gpu/draw/finish.rs + `FrameImage::attach_sl`).
	#[cfg(windows)]
	pub(crate) fn take_sl(&mut self) -> Option<(&'static Sl, SlFrame)> {
		let f = self.sl_cur.take()?;
		Some((self.sl()?, f))
	}

	/// [S2] Whether the frame's generation inputs are wanted (the upscaler inputs then run without an upscaler too).
	pub(crate) fn fg_wants(&self) -> bool {
		#[cfg(windows)]
		return self.fg().is_some_and(|f| f.usable()) || self.sl().is_some_and(|s| s.usable());
		#[cfg(not(windows))]
		false
	}

	/// [S2] This frame's generation inputs (fgen/frame.rs).
	#[cfg(windows)]
	pub(crate) fn set_pending(&mut self, f: FgFrame) {
		self.pending = Some(f);
	}

	/// [S2] This frame's generation inputs for the present: one frame id per presented frame, taken here (`pick`).
	#[cfg(windows)]
	pub(crate) fn take_pending(&mut self) -> Option<(Arc<Fg>, FgFrame)> {
		let pending = self.pending.take();
		let fg = self.fg()?.clone();
		let id = fg.take_id();
		Some((fg, pick(pending, id)))
	}

	/// [S6] At a frame's acquire: inputs a dropped frame left (its prepare never submitted) go; the frame makes its own.
	pub(crate) fn clear_pending(&mut self) {
		#[cfg(windows)]
		{
			self.pending = None;
		}
	}

	/// [S2] The window-size hudless picture (`format`), made again when the size or format changes.
	#[cfg(windows)]
	pub(crate) fn hudless(&mut self, device: &wgpu::Device, size: (u32, u32), format: wgpu::TextureFormat) -> wgpu::Texture {
		if let Some(t) = self.hudless.as_ref().filter(|t| (t.width(), t.height()) == size && t.format() == format) {
			return t.clone();
		}
		let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
		let t = device.create_texture(&wgpu::TextureDescriptor { label: Some("fgen hudless"), size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] });
		self.hudless = Some(t.clone());
		t
	}

	/// True while our own swapchain presents.
	pub(crate) fn own(&self) -> bool {
		#[cfg(windows)]
		return matches!(self.kind, Present::Own(_));
		#[cfg(not(windows))]
		false
	}

	/// The swapchain for `config` (window size, format, present mode, latency). wgpu: setup.rs `configure_surface` as always.
	pub(crate) fn configure(&mut self, device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) {
		match &mut self.kind {
			Present::Wgpu(s) => crate::viewer::gpu::configure_surface(s, device, config),
			#[cfg(windows)]
			Present::Own(o) => {
				let want = (config.width, config.height);
				let r = match &mut o.swap {
					OwnSwap::Vk(v) => v.configure(device, config.format, want, config.desired_maximum_frame_latency, config.present_mode),
					OwnSwap::Dx(d) => d.configure(device, config.format, want, config.desired_maximum_frame_latency, config.present_mode),
				};
				match r {
					Ok(_) => o.back = Some(backbuffer(device, config)),
					Err(e) => self.fall_back(device, config, &format!("own swapchain configure failed ({e})")),
				}
			}
			#[cfg(windows)]
			Present::Gone => {}
		}
	}

	/// Puts our own swapchain on the window (`on`) or gives it back to wgpu. The caller flushed the presenter; `config` is the window
	/// configuration. Returns whether our own swapchain presents now.
	pub(crate) fn set_own(&mut self, on: bool, adapter: Option<&wgpu::Adapter>, device: &wgpu::Device, queue: &wgpu::Queue, config: &wgpu::SurfaceConfiguration) -> bool {
		if on == self.own() {
			return on;
		}
		let _ = device.poll(wgpu::PollType::wait_indefinitely());
		if !on {
			self.kind = Present::Wgpu(self.wgpu_surface());
			self.configure(device, config);
			eprintln!("fgen: present: wgpu's surface");
			return false;
		}
		#[cfg(windows)]
		{
			// [S2] RAC_FG=fsr: FidelityFX frame generation (no runtime: today's path, logged once by ffx_fg.rs).
			let fsr = super::mode() == super::Mode::Fsr;
			self.fg = if fsr { Fg::new(device).map(Arc::new) } else { None };
			if fsr && self.fg.is_none() {
				super::choice::refuse(super::choice::FSR_VALUE); // [S4] the menus withdraw FSR 2x on this PC
				return false;
			}
			// [S3] RAC_FG=dlss2..4: Streamline (DirectX 12; Vulkan when the instance was made through it: sl_vk.rs; refused with one
			// log line otherwise: sl.rs `get`).
			let dlss = match super::mode() {
				super::Mode::Dlss(n) => Some(n),
				_ => None,
			};
			self.sl = match (dlss, adapter) {
				(Some(n), Some(a)) => super::sl::get(a, device, n),
				_ => None,
			};
			if dlss.is_some() && self.sl.is_none() {
				super::choice::refuse_dlss(0); // the menus withdraw every DLSS value on this PC
				return false;
			}
			// One swapchain per window: wgpu's surface goes first (DX12: one flip swapchain per HWND; Vulkan: native window in use).
			self.kind = Present::Gone;
			match make_own(&self.window, adapter, device, queue, config.format, self.fg.clone(), self.sl) {
				Ok(swap) => {
					self.kind = Present::Own(Own { swap, back: None });
					self.configure(device, config); // a refused swapchain or context: `fall_back` (wgpu's surface again)
					if self.own() {
						// Lane fglive: the maker whose values now apply at once from the menu (follow.rs).
						super::follow::set_running(if fsr { Some(super::follow::Maker::Fsr) } else if self.sl.is_some() { Some(super::follow::Maker::Dlss) } else { None });
						eprintln!("fgen: present: own swapchain ({})", if fsr { "RAC_FG=fsr: FSR frame generation" } else if self.sl.is_some() { "RAC_FG=dlss: DLSS frame generation" } else { "RAC_FG=null: no generation" });
					}
					return self.own();
				}
				Err(e) => self.fall_back(device, config, &format!("own swapchain refused ({e})")),
			}
		}
		let _ = (adapter, queue);
		false
	}

	/// [B3/S3] Our own swapchain could not be made (at start or on a recreate): what was made goes (VkSwap / DxSwap drop: FidelityFX's
	/// contexts and swapchain), wgpu's surface presents for the rest of the run, the menus withdraw FSR 2x. One log line.
	#[cfg(windows)]
	fn fall_back(&mut self, device: &wgpu::Device, config: &wgpu::SurfaceConfiguration, why: &str) {
		let fsr = self.fg.is_some() || super::mode() == super::Mode::Fsr;
		let dlss = self.sl.is_some() || matches!(super::mode(), super::Mode::Dlss(_));
		self.kind = Present::Gone; // drops the own swapchain before wgpu's surface takes the window
		self.fg = None;
		self.pending = None;
		self.sl = None;
		self.sl_cur = None;
		if dlss {
			super::choice::refuse_dlss(0);
		}
		self.kind = Present::Wgpu(self.wgpu_surface());
		self.configure(device, config);
		if fsr {
			super::choice::refuse(super::choice::FSR_VALUE);
		}
		super::follow::set_running(None);
		eprintln!("fgen: {why}: wgpu's surface for the rest of the run{}", if fsr { ", no FSR frame generation" } else if dlss { ", no DLSS frame generation" } else { "" });
	}

	fn wgpu_surface(&self) -> wgpu::Surface<'static> {
		self.instance.create_surface(self.window.clone()).expect("surface")
	}

	/// The next window image.
	pub(crate) fn acquire(&mut self) -> Acquired {
		match &mut self.kind {
			Present::Wgpu(s) => match s.get_current_texture() {
				wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => Acquired::Frame(FrameImage::Surface(f)),
				wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => Acquired::Outdated,
				_ => Acquired::Skip,
			},
			#[cfg(windows)]
			Present::Own(o) => o.acquire(),
			#[cfg(windows)]
			Present::Gone => Acquired::Skip,
		}
	}
}

#[cfg(windows)]
fn make_own(window: &Window, adapter: Option<&wgpu::Adapter>, device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat, fg: Option<Arc<Fg>>, sl: Option<&'static Sl>) -> Result<OwnSwap, String> {
	let (hwnd, hinst) = hwnd_of(window).ok_or("no Win32 window handle")?;
	if unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }.is_some() {
		return super::swap_vk::VkSwap::new(device, None, hwnd, hinst, format, fg, sl).map(OwnSwap::Vk);
	}
	let adapter = adapter.ok_or("DirectX 12 needs the adapter")?;
	super::swap_dx12::DxSwap::new(adapter, queue, hwnd, format, fg, sl).map(OwnSwap::Dx)
}

/// Our backbuffer: the window image's size, format and usages (+ COPY_SRC for the copy onto the swapchain image).
#[cfg(windows)]
fn backbuffer(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> wgpu::Texture {
	device.create_texture(&wgpu::TextureDescriptor {
		label: Some("fgen backbuffer"),
		size: wgpu::Extent3d { width: config.width.max(1), height: config.height.max(1), depth_or_array_layers: 1 },
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format: config.format,
		usage: config.usage | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
		view_formats: &[],
	})
}

#[cfg(windows)]
impl Own {
	fn acquire(&mut self) -> Acquired {
		let stale = match &self.swap {
			OwnSwap::Vk(v) => v.needs_configure(),
			OwnSwap::Dx(d) => d.needs_configure(),
		};
		if stale {
			return Acquired::Outdated;
		}
		let Some(back) = self.back.clone() else { return Acquired::Outdated };
		match &mut self.swap {
			OwnSwap::Vk(v) => match v.acquire() {
				super::swap_vk::Got::Frame(image, f) => Acquired::Frame(FrameImage::Own(OwnFrame { back, image, sync: FrameSync::Vk(f), fg: None, sl: None })),
				super::swap_vk::Got::Outdated => Acquired::Outdated,
				super::swap_vk::Got::Skip | super::swap_vk::Got::Lost => Acquired::Skip,
			},
			OwnSwap::Dx(d) => match d.acquire() {
				super::swap_dx12::Got::Frame(image, f) => Acquired::Frame(FrameImage::Own(OwnFrame { back, image, sync: FrameSync::Dx(f), fg: None, sl: None })),
				super::swap_dx12::Got::Skip | super::swap_dx12::Got::Lost => Acquired::Skip,
			},
		}
	}
}

impl FrameImage {
	/// [S2] Frame generation for this frame (gpu/draw/finish.rs, from `Screen::take_pending`).
	#[cfg(windows)]
	pub(crate) fn attach_fg(&mut self, fg: Option<(Arc<Fg>, FgFrame)>) {
		if let FrameImage::Own(o) = self {
			o.fg = fg;
		}
	}

	/// [S3] DLSS frame generation for this frame (gpu/draw/finish.rs, from `Screen::take_sl`).
	#[cfg(windows)]
	pub(crate) fn attach_sl(&mut self, sl: Option<(&'static Sl, SlFrame)>) {
		if let FrameImage::Own(o) = self {
			o.sl = sl;
		}
	}

	/// The texture the frame draws into: the surface texture, or our backbuffer.
	pub(crate) fn texture(&self) -> &wgpu::Texture {
		match self {
			FrameImage::Surface(f) => &f.texture,
			#[cfg(windows)]
			FrameImage::Own(o) => &o.back,
		}
	}

	/// End of the frame: our backbuffer onto the swapchain image (the overlapping part when a resize is under way).
	pub(crate) fn encode_out(&self, enc: &mut wgpu::CommandEncoder) {
		#[cfg(windows)]
		if let FrameImage::Own(o) = self {
			let w = o.back.width().min(o.image.width());
			let h = o.back.height().min(o.image.height());
			enc.copy_texture_to_texture(o.back.as_image_copy(), o.image.as_image_copy(), wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 });
		}
		let _ = enc;
	}

	/// What a screenshot reads: the image the window shows (after `encode_out`).
	pub(crate) fn shot(&self) -> &wgpu::Texture {
		match self {
			FrameImage::Surface(f) => &f.texture,
			#[cfg(windows)]
			FrameImage::Own(o) => &o.image,
		}
	}

	/// The swapchain image to the present state (wgpu's barrier; next frame's copy moves it back).
	pub(crate) fn encode_present(&self, enc: &mut wgpu::CommandEncoder) {
		#[cfg(windows)]
		if let FrameImage::Own(o) = self {
			enc.transition_resources(std::iter::empty(), std::iter::once(wgpu::TextureTransition { texture: &o.image, selector: None, state: wgpu::TextureUses::PRESENT }));
		}
		let _ = enc;
	}

	/// The frame's last submit (`cmds` hold the copy): on our Vulkan swapchain it waits the acquire semaphore and signals the
	/// render-done one, armed under the capture's submit lock so no other thread's submit takes them.
	pub(crate) fn submit_last(&self, q: &wgpu::Queue, cmds: Vec<wgpu::CommandBuffer>) {
		#[cfg(windows)]
		if let FrameImage::Own(OwnFrame { sync: FrameSync::Vk(f), .. }) = self {
			let _no_capture_submit = (crate::pack::get().capture_submit_lock)();
			f.arm(q);
			q.submit(cmds);
			return;
		}
		q.submit(cmds);
	}

	/// Whether the presenter gate stays shut until the present returned (our Vulkan present is a raw queue call that must not
	/// overlap a render-thread submit).
	pub(crate) fn holds_gate(&self) -> bool {
		#[cfg(windows)]
		if let FrameImage::Own(OwnFrame { sync: FrameSync::Vk(_), .. } | OwnFrame { fg: Some(_), .. } | OwnFrame { sl: Some(_), .. }) = self {
			return true; // [S2] also any frame-generation frame: its prepare list (DX12) and configure run on the queue here
		}
		false
	}

	/// Presents the frame (wgpu's present, or ours).
	pub(crate) fn present(self, q: &wgpu::Queue) {
		match self {
			FrameImage::Surface(f) => q.present(f),
			#[cfg(windows)]
			FrameImage::Own(o) => {
				let OwnFrame { sync, fg, sl, .. } = o;
				match sync {
					FrameSync::Vk(f) => {
						let count = fg.as_ref().map(|(g, ff)| {
							super::trace(format_args!("vk configure id {} prepared {}", ff.id, ff.prepared));
							g.configure(ff, f.swapchain_raw() as usize as *mut std::ffi::c_void);
							super::trace(format_args!("vk configured"));
							(g.clone(), f.present_count())
						});
						super::trace(format_args!("vk present"));
						if let Some((s, fr)) = &sl {
							s.present_begin(fr); // [S3] as the DirectX 12 branch below
						}
						f.present();
						if let Some((s, fr)) = &sl {
							s.present_end(fr);
						}
						super::trace(format_args!("vk presented"));
						if let Some((g, n)) = count {
							g.after_present(n);
						}
					}
					FrameSync::Dx(f) => {
						let count = fg.map(|(g, mut ff)| {
							if let Some(job) = ff.dx.take() {
								ff.prepared &= g.prepare_dx12(q, job);
							}
							super::trace(format_args!("dx configure id {} prepared {}", ff.id, ff.prepared));
							g.configure(&ff, f.raw());
							(g, f.present_count())
						});
						if let Some((s, fr)) = &sl {
							s.present_begin(fr); // [S3] DLSS-G options, render-submit-end and present-start markers
						}
						f.present();
						if let Some((s, fr)) = &sl {
							s.present_end(fr);
						}
						if let Some((g, n)) = count {
							g.after_present(n);
						}
					}
				}
			}
		}
	}
}
