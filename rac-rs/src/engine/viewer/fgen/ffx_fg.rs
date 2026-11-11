//! [PC] Frame generation (lane fgen, S2): AMD FidelityFX frame generation (FSR SDK 2.4.0) behind RAC_FG=fsr. The loader DLL and
//! the frame-generation DLL are opened at run time (`runtime`, probed once; missing or no version for this GPU = one log line,
//! wgpu's surface instead). `Fg` owns the two FidelityFX contexts of the window: the frame-generation swapchain context (made by
//! swap_vk.rs / swap_dx12.rs through `create_swapchain_vk` / `create_swapchain_dx12`) and the frame-generation context (`make_ctx`,
//! made again at every swapchain configure for the window size, after the swapchain).
//!
//! Per frame (one id each, `next_id`): the prepare dispatch (depth + motion vectors + camera; Vulkan: recorded on the render thread
//! into a raw-only command buffer between the frame's wgpu parts, frame.rs; DirectX 12: on the present thread in a list of our own
//! on wgpu's queue right before the present, `prepare_dx12`), then on the present thread right before the present `configure`
//! with that id, the hudless picture and our callback that dispatches the generation. Interpolation runs on the game queue in
//! order (no async workloads), so the inputs are what the frame left. The present thread is the only caller while a present is in
//! flight (gpu/draw/prepare.rs flushes the presenter before every acquire); `calls` guards the context anyway.

use super::ffx_fg_abi as fga;
use crate::viewer::upscale::ffx::{self, Backend, DeviceHandles, Fns};
use crate::viewer::upscale::ffx_abi as abi;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The loader DLL and the frame-generation DLL it opens, per backend (FSR SDK 2.4.0 names).
pub(super) fn dll_names(b: Backend) -> (&'static str, &'static str) {
	match b {
		Backend::Vulkan => ("amd_fidelityfx_loader_vk.dll", "amd_fidelityfx_framegeneration_vk.dll"),
		Backend::Dx12 => ("amd_fidelityfx_loader_dx12.dll", "amd_fidelityfx_framegeneration_dx12.dll"),
	}
}

/// The opened loader of this run and the device it was probed on.
pub(super) struct Runtime {
	_lib: libloading::Library,
	fns: Fns,
	pub(super) backend: Backend,
	dev: DeviceHandles,
}

// The handles are the window device's (alive for the run); the loader is thread-safe.
unsafe impl Send for Runtime {}
unsafe impl Sync for Runtime {}

/// The frame-generation runtime, probed once (log line either way). None: RAC_FG=fsr runs as off.
pub(super) fn runtime(device: &wgpu::Device) -> Option<&'static Runtime> {
	static RT: OnceLock<Option<Runtime>> = OnceLock::new();
	let rt = RT
		.get_or_init(|| match probe(device) {
			Ok((rt, v)) => {
				eprintln!("fgen: FSR frame generation {:?}: {v}", rt.backend);
				Some(rt)
			}
			Err(e) => {
				eprintln!("fgen: FSR frame generation not available ({e}): RAC_FG=fsr runs as off (wgpu's surface)");
				None
			}
		})
		.as_ref()?;
	ffx::device_handles(device).is_some_and(|(b, d)| ffx::same_device((rt.backend, &rt.dev), (b, &d))).then_some(rt)
}

fn probe(device: &wgpu::Device) -> Result<(Runtime, String), String> {
	let (backend, dev) = ffx::device_handles(device).ok_or("not a Vulkan or D3D12 device")?;
	let exe = std::env::current_exe().map_err(|e| e.to_string())?;
	let (loader, effect) = dll_names(backend);
	let (lib, fns) = ffx::load_with(exe.parent().ok_or("no exe folder")?, loader, effect)?;
	let v = ffx::versions_of(&fns, backend, &dev, fga::CREATE_FG);
	let name = v.first().map(|v| v.1.clone()).ok_or("the loader offers no frame generation for this GPU")?;
	Ok((Runtime { _lib: lib, fns, backend, dev }, name))
}

/// One frame's frame-generation inputs, from frame.rs `fg_encode` to the present.
pub(crate) struct FgFrame {
	pub(super) id: u64,
	/// The prepare dispatch was recorded (Vulkan) or is in `dx` (DirectX 12): generation on for this frame.
	pub(super) prepared: bool,
	pub(super) hudless: abi::Resource,
	pub(super) dx: Option<DxPrep>,
}

/// DirectX 12: the prepare dispatch the present thread records (command list filled in there) and the resources it names.
pub(super) struct DxPrep {
	pub(super) desc: fga::PrepareV2,
	pub(super) keep: Vec<windows::Win32::Graphics::Direct3D12::ID3D12Resource>,
}

// Raw pointers of the window device's resources, kept alive by `keep` / the frame's wgpu textures; used on the present thread.
unsafe impl Send for FgFrame {}
unsafe impl Send for DxPrep {}

/// The camera and sizes of a prepare dispatch.
pub(super) struct PrepIn {
	pub render: (u32, u32),
	pub jitter: [f32; 2],
	pub reset: bool,
	pub far: f32,
	pub fov_y: f32,
	pub eye: [f32; 3],
	pub fwd: [f32; 3],
	pub up: [f32; 3],
}

#[derive(Default)]
struct Stats {
	/// (when, frames, presents) at the last log line.
	since: Option<(Instant, u64, u64)>,
	frames: u64,
	generated: u64,
}

/// The window's FidelityFX frame generation: its swapchain context and frame-generation context.
pub(crate) struct Fg {
	rt: &'static Runtime,
	/// The frame-generation context (null: none). Its address is what the dispatch callback hands to ffxDispatch.
	ctx: AtomicPtr<c_void>,
	/// The swapchain context (null: none).
	swap: AtomicPtr<c_void>,
	calls: Mutex<()>,
	next_id: AtomicU64,
	/// RAC_FG_ONLYGEN=1: only the generated frames are shown (FidelityFX's check of the generation).
	only_gen: bool,
	/// Refused for the rest of the run (too many errors): every frame without generation.
	failed: AtomicBool,
	errors: AtomicU32,
	/// The context's display size.
	size: Mutex<(u32, u32)>,
	stats: Mutex<Stats>,
	clock: Mutex<Option<Instant>>,
	lists: Mutex<Option<crate::viewer::upscale::ffx_dx12::Lists>>,
	/// Vulkan: the swapchain's present counter (FidelityFX's `getLastPresentCount`).
	pub(super) vk_count: Mutex<Option<fga::PfnLastPresentCount>>,
}

// Raw contexts used under `calls` or by the present thread only (see the module doc).
unsafe impl Send for Fg {}
unsafe impl Sync for Fg {}

/// FidelityFX's frame-generation dispatch callback: our context's ffxDispatch (`user` = the `Fg`).
unsafe extern "C" fn dispatch_cb(params: *mut abi::Header, user: *mut c_void) -> u32 {
	let fg = unsafe { &*(user as *const Fg) };
	super::trace(format_args!("dispatch_cb type {:#x}", unsafe { (*params).ty }));
	let r = unsafe { (fg.rt.fns.dispatch)(fg.ctx.as_ptr(), params) };
	super::trace(format_args!("dispatch_cb -> {r}"));
	r
}

/// Links `b` after `a` in a desc chain.
fn chain(a: &mut abi::Header, b: *mut abi::Header) {
	a.next = b;
}

impl Fg {
	/// The window's frame generation on `device` (None: no runtime; logged once by `runtime`).
	pub(super) fn new(device: &wgpu::Device) -> Option<Fg> {
		let rt = runtime(device)?;
		let only_gen = std::env::var("RAC_FG_ONLYGEN").is_ok_and(|v| v == "1");
		Some(Fg { rt, ctx: AtomicPtr::default(), swap: AtomicPtr::default(), calls: Mutex::new(()), next_id: AtomicU64::new(1), only_gen, failed: AtomicBool::new(false), errors: AtomicU32::new(0), size: Mutex::new((0, 0)), stats: Default::default(), clock: Mutex::new(None), lists: Mutex::new(None), vk_count: Mutex::new(None) })
	}

	fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
		self.calls.lock().unwrap_or_else(|e| e.into_inner())
	}

	/// [S6] The id the next presented frame takes (the render thread records its prepare with it; nothing is taken).
	pub(super) fn peek_id(&self) -> u64 {
		self.next_id.load(Ordering::Relaxed)
	}

	/// [S6] The presented frame's id (present.rs `take_pending`, once per presented frame: ids grow by exactly one).
	pub(super) fn take_id(&self) -> u64 {
		self.next_id.fetch_add(1, Ordering::Relaxed)
	}

	/// Whether this frame should prepare (a context, not refused).
	pub(super) fn usable(&self) -> bool {
		!self.failed.load(Ordering::Relaxed) && !self.ctx.load(Ordering::Relaxed).is_null()
	}

	/// The context's display size.
	pub(super) fn size(&self) -> (u32, u32) {
		*self.size.lock().unwrap_or_else(|e| e.into_inner())
	}

	/// An error of a per-frame call: logged the first time; 60 in a row = no generation for the rest of the run.
	fn error(&self, what: &str, rc: u32) {
		let n = self.errors.fetch_add(1, Ordering::Relaxed);
		if n == 0 {
			eprintln!("fgen: FSR FG {what} failed: return code {rc:#x}");
		}
		if n == 59 && !self.failed.swap(true, Ordering::Relaxed) {
			eprintln!("fgen: FSR FG refused 60 times in a row: no generation for the rest of the run");
		}
	}

	fn ok(&self) {
		self.errors.store(0, Ordering::Relaxed);
	}

	/// (Re)makes the frame-generation context for `display` (after the swapchain; the caller waited for the presents).
	pub(super) fn make_ctx(&self, display: (u32, u32), format: wgpu::TextureFormat) -> Result<(), String> {
		let _c = self.lock();
		self.drop_ctx_locked();
		let fmt = ffx::surface_format(format);
		let dim = abi::Dim { width: display.0, height: display.1 };
		let mut create = fga::CreateFg { header: abi::Header::of(fga::CREATE_FG), flags: 0, display_size: dim, max_render_size: dim, back_buffer_format: fmt };
		let mut be = ffx::BackendDesc::of(self.rt.backend, &self.rt.dev);
		let mut ver = fga::HeaderU32 { header: abi::Header::of(fga::CREATE_FG_VERSION), value: fga::FG_VERSION };
		let mut hud = fga::HeaderU32 { header: abi::Header::of(fga::CREATE_FG_HUDLESS), value: fmt };
		chain(&mut create.header, be.header());
		chain(unsafe { &mut *be.header() }, &mut ver.header);
		chain(&mut ver.header, &mut hud.header);
		let rc = unsafe { (self.rt.fns.create)(self.ctx.as_ptr(), &mut create.header, std::ptr::null()) };
		if rc != abi::RETURN_OK {
			self.ctx.store(std::ptr::null_mut(), Ordering::Relaxed);
			return Err(format!("ffxCreateContext (frame generation) return code {rc:#x}"));
		}
		*self.size.lock().unwrap_or_else(|e| e.into_inner()) = display;
		eprintln!("fgen: FSR frame generation context {}x{}", display.0, display.1);
		Ok(())
	}

	fn drop_ctx_locked(&self) {
		if !self.ctx.load(Ordering::Relaxed).is_null() {
			unsafe { (self.rt.fns.destroy)(self.ctx.as_ptr(), std::ptr::null()) };
			self.ctx.store(std::ptr::null_mut(), Ordering::Relaxed);
		}
	}

	/// Drops the frame-generation context (the caller waited for the presents).
	pub(super) fn drop_ctx(&self) {
		let _c = self.lock();
		self.drop_ctx_locked();
	}

	/// Vulkan: the frame-generation swapchain for `info` on the four queues (game, async compute, present, image acquire) and its
	/// replacement functions. The swapchain context is ours from here (`destroy_swap_ctx`).
	pub(super) fn create_swapchain_vk(&self, info: ash::vk::SwapchainCreateInfoKHR<'static>, q: [fga::QueueInfo; 4]) -> Result<(u64, fga::SwapchainFunctionsVk), String> {
		super::failtest("Vulkan")?;
		let _c = self.lock();
		let mut sc = 0u64;
		let mut d = fga::CreateSwapchainVk { header: abi::Header::of(fga::CREATE_SWAPCHAIN_VK), physical_device: self.rt.dev.physical, device: self.rt.dev.device, swapchain: &mut sc, allocator: std::ptr::null(), create_info: info, game_queue: q[0], async_compute_queue: q[1], present_queue: q[2], image_acquire_queue: q[3] };
		let mut ver = fga::HeaderU32 { header: abi::Header::of(fga::CREATE_SWAPCHAIN_VERSION_VK), value: fga::SWAPCHAIN_VERSION };
		chain(&mut d.header, &mut ver.header);
		let rc = unsafe { (self.rt.fns.create)(self.swap.as_ptr(), &mut d.header, std::ptr::null()) };
		if rc != abi::RETURN_OK {
			self.swap.store(std::ptr::null_mut(), Ordering::Relaxed);
			return Err(format!("ffxCreateContext (Vulkan frame-generation swapchain) return code {rc:#x}"));
		}
		let mut f = fga::SwapchainFunctionsVk { header: abi::Header::of(fga::QUERY_SWAPCHAIN_FUNCTIONS_VK), create: None, destroy: None, get_images: std::ptr::null(), acquire: std::ptr::null(), present: std::ptr::null(), set_hdr_metadata: std::ptr::null(), last_present_count: None };
		let rc = unsafe { (self.rt.fns.query)(self.swap.as_ptr(), &mut f.header) };
		if rc != abi::RETURN_OK || f.create.is_none() || f.destroy.is_none() || f.get_images.is_null() || f.acquire.is_null() || f.present.is_null() {
			// Nothing of it stays: the swapchain context first, then its replacement destroy when it was handed out (VkSwap's order).
			unsafe { (self.rt.fns.destroy)(self.swap.as_ptr(), std::ptr::null()) };
			self.swap.store(std::ptr::null_mut(), Ordering::Relaxed);
			if let Some(destroy) = f.destroy.filter(|_| sc != 0) {
				unsafe { destroy(self.rt.dev.device, sc, std::ptr::null(), sc as usize as *mut c_void) };
			}
			return Err(format!("ffxQuery (swapchain functions) return code {rc:#x}"));
		}
		*self.vk_count.lock().unwrap_or_else(|e| e.into_inner()) = f.last_present_count;
		Ok((sc, f))
	}

	/// DirectX 12: the frame-generation proxy swapchain (an IDXGISwapChain4*) for `hwnd` on wgpu's queue.
	pub(super) fn create_swapchain_dx12(&self, desc: *const c_void, hwnd: *mut c_void, factory: *mut c_void, queue: *mut c_void) -> Result<*mut c_void, String> {
		super::failtest("DirectX 12")?;
		let _c = self.lock();
		let mut out: *mut c_void = std::ptr::null_mut();
		let mut d = fga::CreateSwapchainForHwndDx12 { header: abi::Header::of(fga::CREATE_SWAPCHAIN_FOR_HWND_DX12), swapchain: &mut out, hwnd, desc, fullscreen_desc: std::ptr::null(), dxgi_factory: factory, game_queue: queue };
		let mut ver = fga::HeaderU32 { header: abi::Header::of(fga::CREATE_SWAPCHAIN_VERSION_DX12), value: fga::SWAPCHAIN_VERSION };
		chain(&mut d.header, &mut ver.header);
		let rc = unsafe { (self.rt.fns.create)(self.swap.as_ptr(), &mut d.header, std::ptr::null()) };
		if rc != abi::RETURN_OK || out.is_null() {
			self.swap.store(std::ptr::null_mut(), Ordering::Relaxed);
			return Err(format!("ffxCreateContext (DirectX 12 frame-generation swapchain) return code {rc:#x}"));
		}
		Ok(out)
	}

	/// Blocks until the swapchain's presents (generated ones included) are done: before any resize, idle or destroy.
	pub(super) fn wait_presents(&self) {
		if self.swap.load(Ordering::Relaxed).is_null() {
			return;
		}
		let ty = if self.rt.backend == Backend::Vulkan { fga::WAIT_FOR_PRESENTS_VK } else { fga::WAIT_FOR_PRESENTS_DX12 };
		let h = abi::Header::of(ty);
		unsafe { (self.rt.fns.dispatch)(self.swap.as_ptr(), &h) };
	}

	/// Destroys the swapchain context (after `wait_presents` and `drop_ctx`).
	pub(super) fn destroy_swap_ctx(&self) {
		let _c = self.lock();
		if !self.swap.load(Ordering::Relaxed).is_null() {
			unsafe { (self.rt.fns.destroy)(self.swap.as_ptr(), std::ptr::null()) };
			self.swap.store(std::ptr::null_mut(), Ordering::Relaxed);
		}
	}

	/// The frame's milliseconds since the last prepare (wall clock; 16.7 the first time).
	fn frame_ms(&self) -> f32 {
		let now = Instant::now();
		let last = self.clock.lock().unwrap_or_else(|e| e.into_inner()).replace(now);
		last.map_or(1000.0 / 60.0, |t| ((now - t).as_secs_f32() * 1000.0).clamp(0.1, 200.0))
	}

	/// The prepare desc of frame `id` (command list filled in by the caller).
	pub(super) fn prepare_desc(&self, id: u64, depth: abi::Resource, motion: abi::Resource, p: &PrepIn) -> fga::PrepareV2 {
		let right = norm(cross(p.fwd, p.up));
		let up = norm(cross(right, p.fwd));
		fga::PrepareV2 {
			header: abi::Header::of(fga::PREPARE_V2),
			frame_id: id,
			flags: 0,
			command_list: std::ptr::null_mut(),
			render_size: abi::Dim { width: p.render.0, height: p.render.1 },
			jitter_offset: abi::Coords { x: p.jitter[0], y: p.jitter[1] },
			// Our vectors are UV units of the render picture, current -> previous: times the render size = pixels.
			motion_vector_scale: abi::Coords { x: p.render.0 as f32, y: p.render.1 as f32 },
			frame_time_delta: self.frame_ms(),
			reset: p.reset,
			camera_near: crate::viewer::projection::RETAIL_NEAR,
			camera_far: p.far,
			camera_fov_angle_vertical: p.fov_y,
			view_space_to_meters_factor: 1.0,
			depth,
			motion_vectors: motion,
			camera_position: p.eye,
			camera_up: up,
			camera_right: right,
			camera_forward: norm(p.fwd),
		}
	}

	/// Vulkan, render thread: records the prepare dispatch into `cmd` (a VkCommandBuffer). False: not recorded (error logged).
	pub(super) fn prepare_vk(&self, desc: &mut fga::PrepareV2, cmd: *mut c_void) -> bool {
		let _c = self.lock();
		if self.ctx.load(Ordering::Relaxed).is_null() {
			return false;
		}
		desc.command_list = cmd;
		let rc = unsafe { (self.rt.fns.dispatch)(self.ctx.as_ptr(), &desc.header) };
		if rc != abi::RETURN_OK {
			self.error("prepare", rc);
			return false;
		}
		true
	}

	/// DirectX 12, present thread, before `configure`: the frame's prepare dispatch in a list of ours on wgpu's queue (after the
	/// frame's submits). False: not run (error logged): the frame goes without generation.
	pub(super) fn prepare_dx12(&self, q: &wgpu::Queue, job: DxPrep) -> bool {
		let _c = self.lock();
		if self.ctx.load(Ordering::Relaxed).is_null() {
			return false;
		}
		let mut lists = self.lists.lock().unwrap_or_else(|e| e.into_inner());
		if lists.is_none() {
			match crate::viewer::upscale::ffx_dx12::Lists::new(self.rt.dev.device, q) {
				Ok(l) => *lists = Some(l),
				Err(e) => {
					eprintln!("fgen: FSR FG command lists ({e})");
					self.failed.store(true, Ordering::Relaxed);
					return false;
				}
			}
		}
		let DxPrep { mut desc, keep } = job;
		let ctx = self.ctx.as_ptr();
		let fns = self.rt.fns;
		let r = lists.as_mut().expect("made above").run(
			|list| {
				desc.command_list = list;
				let rc = unsafe { (fns.dispatch)(ctx, &desc.header) };
				if rc == abi::RETURN_OK { Ok(()) } else { Err(format!("{rc:#x}")) }
			},
			keep,
		);
		match r {
			Ok(()) => true,
			Err(e) => {
				self.error(&format!("prepare ({e})"), 0);
				false
			}
		}
	}

	/// Present thread, right before the present of frame `f` on `swap_chain` (VkSwapchainKHR as a pointer, or the IDXGISwapChain4*).
	pub(super) fn configure(&self, f: &FgFrame, swap_chain: *mut c_void) {
		let _c = self.lock();
		if self.ctx.load(Ordering::Relaxed).is_null() {
			return;
		}
		let enabled = f.prepared && !self.failed.load(Ordering::Relaxed);
		let (w, h) = self.size();
		let c = fga::ConfigureFg {
			header: abi::Header::of(fga::CONFIGURE_FG),
			swap_chain,
			present_callback: None,
			present_callback_user: std::ptr::null_mut(),
			fg_callback: Some(dispatch_cb),
			fg_callback_user: self as *const Fg as *mut c_void,
			enabled,
			allow_async_workloads: false,
			hudless_color: if enabled { f.hudless } else { abi::Resource::none() },
			flags: 0,
			only_present_generated: self.only_gen && enabled,
			generation_rect: fga::Rect { left: 0, top: 0, width: w as i32, height: h as i32 },
			frame_id: f.id,
		};
		let rc = unsafe { (self.rt.fns.configure)(self.ctx.as_ptr(), &c.header) };
		if rc != abi::RETURN_OK {
			self.error("configure", rc);
		} else if enabled {
			self.ok();
		}
		let mut s = self.stats.lock().unwrap_or_else(|e| e.into_inner());
		s.frames += 1;
		s.generated += enabled as u64;
	}

	/// After a present: `presents` = the swapchain's present count (generated frames included). One log line every 2 s.
	pub(super) fn after_present(&self, presents: u64) {
		let now = Instant::now();
		let mut s = self.stats.lock().unwrap_or_else(|e| e.into_inner());
		let frames = s.frames;
		crate::diag::presents(presents); // [PC] RAC_DIAG perf.csv fg_presented_fps
		let (t0, f0, p0) = *s.since.get_or_insert((now, frames, presents));
		let dt = now - t0;
		if dt >= Duration::from_secs(2) {
			let (df, dp, secs) = (s.frames - f0, presents.saturating_sub(p0), dt.as_secs_f32());
			eprintln!("fgen: FSR FG: {df} rendered, {dp} presented in {secs:.1} s ({:.1} -> {:.1} fps), {} of {} frames with generation", df as f32 / secs, dp as f32 / secs, s.generated, s.frames);
			s.since = Some((now, s.frames, presents));
		}
	}
}

impl Drop for Fg {
	fn drop(&mut self) {
		self.wait_presents();
		self.drop_ctx();
		*self.lists.lock().unwrap_or_else(|e| e.into_inner()) = None;
		self.destroy_swap_ctx();
	}
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn norm(v: [f32; 3]) -> [f32; 3] {
	let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
	if l > 1e-9 { [v[0] / l, v[1] / l, v[2] / l] } else { [0.0, 0.0, 1.0] }
}
