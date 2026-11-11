//! [PC] Frame generation (lane fgen, S1): our own Vulkan swapchain on the window (RAC_FG=null). A VkSurfaceKHR made from the window's
//! HWND on wgpu's instance, a swapchain made like wgpu-hal's own (min image count = frame latency + 1, the window's format, Opaque,
//! FIFO first and then the uncapped mode over it as gpu/setup.rs `configure_surface` does), every image wrapped as a wgpu texture
//! (`texture_from_raw`, memory not owned by wgpu) so the frame's last encoder copies our backbuffer onto it and moves it to the
//! present layout with wgpu's own barriers. All swapchain calls go through `SwapFns`, the table FidelityFX's replacement functions
//! fill in S2 (plain = the loader's).
//!
//! Sync: the image's acquire semaphore and its render-done semaphore ride on the frame's last wgpu submit (wgpu-hal `Queue::
//! add_wait_semaphore` / `add_signal_semaphore`, under the capture's submit lock so no other thread's submit takes them first);
//! `vkQueuePresentKHR` waits the render-done semaphore, on the present thread, under the same lock and before the presenter gate
//! opens (VkQueue needs external sync against every other submit). Acquire semaphores are recycled per image (the one an image
//! was last acquired with is free once that image comes back: its frame's submit has run). A frame dropped unpresented (a
//! skipped frame) marks the swapchain stale: it is made again before the next acquire.
//!
//! [S2] RAC_FG=fsr (`Core::fg`): the swapchain is FidelityFX's frame-generation swapchain, made through its context on the game
//! queue + the device's extra queues (queues.rs `made`), and `SwapFns` holds its replacement image / acquire / present functions.
//! Its game-queue submits go through `game_submit` (under the capture's submit lock), so our present does not take that lock.
//! Resize: its replacement create in place; no area: the whole swapchain goes (`ffx_teardown`) and is made again later.

use super::ffx_fg::Fg;
use super::ffx_fg_abi as fga;
use ash::vk;
use ash::vk::Handle;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// The swapchain entry points: the loader's (plain) or a vendor's replacements (S2: FidelityFX frame interpolation swapchain).
#[derive(Clone, Copy)]
pub(super) struct SwapFns {
	pub create: vk::PFN_vkCreateSwapchainKHR,
	pub destroy: vk::PFN_vkDestroySwapchainKHR,
	pub images: vk::PFN_vkGetSwapchainImagesKHR,
	pub acquire: vk::PFN_vkAcquireNextImageKHR,
	pub present: vk::PFN_vkQueuePresentKHR,
}

impl SwapFns {
	fn plain(instance: &ash::Instance, device: &ash::Device) -> SwapFns {
		let d = ash::khr::swapchain::Device::new(instance, device);
		let f = d.fp();
		SwapFns { create: f.create_swapchain_khr, destroy: f.destroy_swapchain_khr, images: f.get_swapchain_images_khr, acquire: f.acquire_next_image_khr, present: f.queue_present_khr }
	}
}

/// What the render thread and the present thread share.
struct Core {
	device: ash::Device,
	queue: vk::Queue,
	fns: std::sync::Mutex<SwapFns>,
	/// [S2] RAC_FG=fsr: FidelityFX frame generation owns the swapchain.
	fg: Option<Arc<Fg>>,
	/// A present asked for a new swapchain.
	stale: AtomicBool,
	/// A frame dropped unpresented (a skipped frame): its image is still acquired and its acquire semaphore still pending, so the
	/// next `acquire` hands out the same image again (swapchain, index) instead of acquiring another.
	held: std::sync::Mutex<Option<(vk::SwapchainKHR, u32)>>,
}

impl Core {
	fn fns(&self) -> SwapFns {
		*self.fns.lock().unwrap_or_else(|e| e.into_inner())
	}
}

/// [S2] The game queue and vkQueueSubmit for FidelityFX's submits (its QueueInfo submit function has no queue argument).
static GAME: std::sync::OnceLock<(u64, vk::PFN_vkQueueSubmit)> = std::sync::OnceLock::new();

/// [S2] FidelityFX's game-queue submit: under the capture's submit lock like every other submit on that queue.
unsafe extern "system" fn game_submit(count: u32, submits: *const c_void, fence: u64) -> i32 {
	let Some(&(q, f)) = GAME.get() else { return vk::Result::ERROR_INITIALIZATION_FAILED.as_raw() };
	super::trace(format_args!("game_submit {count} fence {fence:#x}"));
	let _no_capture_submit = (crate::pack::get().capture_submit_lock)();
	let r = unsafe { f(vk::Queue::from_raw(q), count, submits as *const vk::SubmitInfo<'_>, vk::Fence::from_raw(fence)) }.as_raw();
	super::trace(format_args!("game_submit -> {r}"));
	r
}

/// One swapchain and its images.
struct Chain {
	raw: vk::SwapchainKHR,
	images: Vec<wgpu::Texture>,
	/// Per image: the render-done semaphore the present waits, and the acquire semaphore the image was last acquired with.
	done: Vec<vk::Semaphore>,
	acq: Vec<vk::Semaphore>,
	extent: (u32, u32),
}

pub(super) struct VkSwap {
	core: Arc<Core>,
	surface_fn: ash::khr::surface::Instance,
	surface: vk::SurfaceKHR,
	pd: vk::PhysicalDevice,
	vk_format: vk::Format,
	chain: Option<Chain>,
	spare: Vec<vk::Semaphore>,
	/// The game queue's family.
	qfam: u32,
	/// [S2] FidelityFX's replacement create (in place) and destroy, once its swapchain was made.
	ffx: Option<(fga::PfnCreateSwapchainFfx, fga::PfnDestroySwapchainFfx)>,
	/// [S3] DLSS frame generation (sl_vk.rs): the swapchain calls go through Streamline's proxies; DLSS-G off before a recreate.
	sl: Option<&'static super::sl::Sl>,
}

/// One acquired image, presented on the present thread.
pub(super) struct VkFrame {
	core: Arc<Core>,
	swapchain: vk::SwapchainKHR,
	index: u32,
	acquire: vk::Semaphore,
	done: vk::Semaphore,
	presented: bool,
}

/// What an acquire gave.
pub(super) enum Got {
	Frame(wgpu::Texture, VkFrame),
	/// No image this frame (minimized, timeout).
	Skip,
	/// The swapchain no longer fits the window: configure again.
	Outdated,
	Lost,
}

fn vk_format(f: wgpu::TextureFormat) -> Option<vk::Format> {
	match f {
		wgpu::TextureFormat::Bgra8Unorm => Some(vk::Format::B8G8R8A8_UNORM),
		wgpu::TextureFormat::Rgba8Unorm => Some(vk::Format::R8G8B8A8_UNORM),
		_ => None,
	}
}

pub(super) fn vk_mode(m: wgpu::PresentMode) -> vk::PresentModeKHR {
	match m {
		wgpu::PresentMode::Immediate => vk::PresentModeKHR::IMMEDIATE,
		wgpu::PresentMode::Mailbox => vk::PresentModeKHR::MAILBOX,
		wgpu::PresentMode::FifoRelaxed => vk::PresentModeKHR::FIFO_RELAXED,
		_ => vk::PresentModeKHR::FIFO,
	}
}

/// The swapchain's image count (pure): frame latency + 1 as wgpu-hal asks, inside the surface's range (max 0 = no limit).
pub(super) fn image_count(latency: u32, min: u32, max: u32) -> u32 {
	let n = (latency + 1).max(min);
	if max == 0 { n } else { n.min(max) }
}

/// The swapchain extent (pure): the surface's current extent when it has one, else the wanted size clamped into the range.
pub(super) fn extent_of(current: (u32, u32), want: (u32, u32), min: (u32, u32), max: (u32, u32)) -> (u32, u32) {
	if current.0 != u32::MAX {
		return current;
	}
	(want.0.clamp(min.0, max.0.max(min.0)), want.1.clamp(min.1, max.1.max(min.1)))
}

impl VkSwap {
	/// The surface for `hwnd` and the core; no swapchain yet (`configure`). Err: why the own path cannot run (wgpu's instead).
	pub(super) fn new(device: &wgpu::Device, queue_raw: Option<vk::Queue>, hwnd: isize, hinstance: isize, format: wgpu::TextureFormat, fg: Option<Arc<Fg>>, sl: Option<&'static super::sl::Sl>) -> Result<VkSwap, String> {
		let vk_format = vk_format(format).ok_or_else(|| format!("format {format:?}"))?;
		// [S2] FidelityFX presents on the device's present queue (never the game queue, fgen/queues.rs): that family must present.
		let roles = super::queues::made();
		if fg.is_some() && roles.is_none() {
			return Err("the Vulkan device has no frame-generation queues".into());
		}
		let hal = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }.ok_or("not a Vulkan device")?;
		let inst = hal.shared_instance();
		let (entry, instance) = (inst.entry(), inst.raw_instance());
		let w32 = ash::khr::win32_surface::Instance::new(entry, instance);
		let info = vk::Win32SurfaceCreateInfoKHR::default().hwnd(hwnd).hinstance(hinstance);
		let surface = unsafe { w32.create_win32_surface(&info, None) }.map_err(|e| format!("vkCreateWin32SurfaceKHR {e:?}"))?;
		let surface_fn = ash::khr::surface::Instance::new(entry, instance);
		let pd = hal.raw_physical_device();
		let (pfam, who) = match roles.filter(|_| fg.is_some()) {
			Some(r) => (r.present.0, "FidelityFX's present queue"),
			None => (hal.queue_family_index(), "the game queue"),
		};
		let ok = unsafe { surface_fn.get_physical_device_surface_support(pd, pfam, surface) }.unwrap_or(false);
		if !ok {
			unsafe { surface_fn.destroy_surface(surface, None) };
			return Err(format!("{who} (family {pfam}) cannot present to the window"));
		}
		let fns = match sl {
			Some(_) => super::sl_vk::swap_fns(hal.raw_device().handle()).inspect_err(|_| unsafe { surface_fn.destroy_surface(surface, None) })?,
			None => SwapFns::plain(instance, hal.raw_device()),
		};
		let core = Core { device: hal.raw_device().clone(), queue: queue_raw.unwrap_or_else(|| hal.raw_queue()), fns: std::sync::Mutex::new(fns), fg, stale: AtomicBool::new(false), held: Default::default() };
		Ok(VkSwap { core: Arc::new(core), surface_fn, surface, pd, vk_format, chain: None, spare: Vec::new(), qfam: hal.queue_family_index(), ffx: None, sl })
	}

	/// Waits until the device is idle (every queue: under the capture's submit lock).
	fn idle(&self) {
		let _no_capture_submit = (crate::pack::get().capture_submit_lock)();
		let _ = unsafe { self.core.device.device_wait_idle() };
	}

	/// Drops the swapchain (device idle first). The wrapped images go first; wgpu never owned the VkImages.
	fn drop_chain(&mut self, keep_raw: bool) -> vk::SwapchainKHR {
		*self.core.held.lock().unwrap_or_else(|e| e.into_inner()) = None;
		let Some(c) = self.chain.take() else { return vk::SwapchainKHR::null() };
		self.idle();
		drop(c.images);
		for s in c.done.into_iter().chain(c.acq).chain(self.spare.drain(..)) {
			if s != vk::Semaphore::null() {
				unsafe { self.core.device.destroy_semaphore(s, None) };
			}
		}
		if keep_raw {
			return c.raw;
		}
		unsafe { (self.core.fns().destroy)(self.core.device.handle(), c.raw, std::ptr::null()) };
		vk::SwapchainKHR::null()
	}

	fn info(&self, extent: (u32, u32), count: u32, mode: vk::PresentModeKHR, old: vk::SwapchainKHR) -> vk::SwapchainCreateInfoKHR<'static> {
		vk::SwapchainCreateInfoKHR::default()
			.surface(self.surface)
			.min_image_count(count)
			.image_format(self.vk_format)
			.image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
			.image_extent(vk::Extent2D { width: extent.0, height: extent.1 })
			.image_array_layers(1)
			.image_usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::TRANSFER_SRC | vk::ImageUsageFlags::COLOR_ATTACHMENT)
			.image_sharing_mode(vk::SharingMode::EXCLUSIVE)
			.pre_transform(vk::SurfaceTransformFlagsKHR::IDENTITY)
			.composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
			.present_mode(mode)
			.clipped(true)
			.old_swapchain(old)
	}

	fn create_raw(&self, extent: (u32, u32), count: u32, mode: vk::PresentModeKHR, old: vk::SwapchainKHR) -> Result<vk::SwapchainKHR, vk::Result> {
		let info = self.info(extent, count, mode, old);
		let mut sc = vk::SwapchainKHR::null();
		let r = unsafe { (self.core.fns().create)(self.core.device.handle(), &info, std::ptr::null(), &mut sc) };
		if r == vk::Result::SUCCESS { Ok(sc) } else { Err(r) }
	}

	/// (Re)makes the swapchain for the window size `want`, `latency` frames in flight and `mode`. Ok(false): the window has no area
	/// (minimized): no swapchain until the next configure.
	pub(super) fn configure(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat, want: (u32, u32), latency: u32, mode: wgpu::PresentMode) -> Result<bool, String> {
		if self.core.fg.is_some() {
			return self.configure_ffx(device, format, want, latency, mode);
		}
		if let Some(s) = self.sl {
			s.off_now(); // [S3] SL guide 12.0: DLSS-G off before the swapchain changes
		}
		let old = self.drop_chain(true);
		self.core.stale.store(false, Ordering::Relaxed);
		let caps = unsafe { self.surface_fn.get_physical_device_surface_capabilities(self.pd, self.surface) }.map_err(|e| format!("surface caps {e:?}"))?;
		let ce = caps.current_extent;
		let extent = extent_of((ce.width, ce.height), want, (caps.min_image_extent.width, caps.min_image_extent.height), (caps.max_image_extent.width, caps.max_image_extent.height));
		let destroy = |sc: vk::SwapchainKHR| {
			if sc != vk::SwapchainKHR::null() {
				unsafe { (self.core.fns().destroy)(self.core.device.handle(), sc, std::ptr::null()) };
			}
		};
		if extent.0 == 0 || extent.1 == 0 {
			destroy(old);
			return Ok(false);
		}
		let count = image_count(latency, caps.min_image_count, caps.max_image_count);
		let mode = vk_mode(mode);
		// gpu/setup.rs `configure_surface`: an uncapped mode is made over a FIFO swapchain (less CPU per present on NVIDIA).
		let mut prev = old;
		if mode != vk::PresentModeKHR::FIFO {
			let fifo = self.create_raw(extent, count, vk::PresentModeKHR::FIFO, prev);
			destroy(prev);
			prev = fifo.map_err(|e| format!("vkCreateSwapchainKHR (FIFO) {e:?}"))?;
		}
		let sc = self.create_raw(extent, count, mode, prev);
		destroy(prev);
		let sc = sc.map_err(|e| format!("vkCreateSwapchainKHR {e:?}"))?;
		let r = self.finish_chain(device, format, sc, extent, mode, "own Vulkan swapchain");
		if r.is_err() {
			unsafe { (self.core.fns().destroy)(self.core.device.handle(), sc, std::ptr::null()) };
		}
		r
	}

	/// The images of the new swapchain `sc` wrapped, their semaphores made, the chain kept.
	fn finish_chain(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat, sc: vk::SwapchainKHR, extent: (u32, u32), mode: vk::PresentModeKHR, what: &str) -> Result<bool, String> {
		let raw_images = {
			let (mut n, d, images) = (0u32, self.core.device.handle(), self.core.fns().images);
			let r0 = unsafe { images(d, sc, &mut n, std::ptr::null_mut()) };
			let mut v = vec![vk::Image::null(); n as usize];
			let r1 = unsafe { images(d, sc, &mut n, v.as_mut_ptr()) };
			v.truncate(if r0 == vk::Result::SUCCESS && r1 == vk::Result::SUCCESS { n as usize } else { 0 });
			v
		};
		let images = raw_images.iter().map(|&img| wrap(device, img, format, extent)).collect::<Option<Vec<_>>>();
		let Some(images) = images.filter(|i| !i.is_empty()) else {
			return Err("wrapping the swapchain images failed".into());
		};
		let sem = || unsafe { self.core.device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None) }.unwrap_or_default();
		let done = (0..images.len()).map(|_| sem()).collect();
		let acq = vec![vk::Semaphore::null(); images.len()];
		eprintln!("fgen: {what} {}x{}, {} images, {:?}", extent.0, extent.1, images.len(), mode);
		self.chain = Some(Chain { raw: sc, images, done, acq, extent });
		Ok(true)
	}

	/// [S2] `configure` on FidelityFX's swapchain: the presents done and the frame-generation context gone first; made through
	/// its context the first time (and after a teardown), else its replacement create in place; then the context for the size.
	fn configure_ffx(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat, want: (u32, u32), latency: u32, mode: wgpu::PresentMode) -> Result<bool, String> {
		let fg = self.core.fg.clone().expect("FidelityFX path");
		fg.wait_presents();
		fg.drop_ctx();
		let old = self.drop_chain(true);
		self.core.stale.store(false, Ordering::Relaxed);
		let caps = unsafe { self.surface_fn.get_physical_device_surface_capabilities(self.pd, self.surface) }.map_err(|e| format!("surface caps {e:?}"))?;
		let ce = caps.current_extent;
		let extent = extent_of((ce.width, ce.height), want, (caps.min_image_extent.width, caps.min_image_extent.height), (caps.max_image_extent.width, caps.max_image_extent.height));
		if extent.0 == 0 || extent.1 == 0 {
			self.ffx_teardown(old);
			return Ok(false);
		}
		let count = image_count(latency, caps.min_image_count, caps.max_image_count);
		let mode = vk_mode(mode);
		// The in-place recreate gets no oldSwapchain: FidelityFX recreates its own swapchain inside the handle it hands back, and its
		// handle is not a driver swapchain (passed on as oldSwapchain it crashed in the NVIDIA driver, agent 7 trace).
		let info = self.info(extent, count, mode, vk::SwapchainKHR::null());
		let sc = match self.ffx.filter(|_| old != vk::SwapchainKHR::null()) {
			Some((create, _)) => {
				super::trace(format_args!("vk recreate {extent:?} old {:#x}", old.as_raw()));
				let mut sc = old.as_raw();
				let ctx = old.as_raw() as usize as *mut c_void;
				let r = unsafe { create(self.core.device.handle().as_raw() as usize as *mut c_void, &info as *const _ as *const c_void, std::ptr::null(), &mut sc, ctx) };
				if r != vk::Result::SUCCESS.as_raw() {
					self.ffx_teardown(old);
					return Err(format!("FidelityFX swapchain recreate {:?}", vk::Result::from_raw(r)));
				}
				super::trace(format_args!("vk recreated {sc:#x}"));
				vk::SwapchainKHR::from_raw(sc)
			}
			None => {
				self.ffx_teardown(old);
				let info = self.info(extent, count, mode, vk::SwapchainKHR::null());
				let (sc, f) = fg.create_swapchain_vk(info, self.queue_infos()?)?;
				let mut fns = self.core.fns.lock().unwrap_or_else(|e| e.into_inner());
				unsafe {
					fns.images = std::mem::transmute::<*const c_void, vk::PFN_vkGetSwapchainImagesKHR>(f.get_images);
					fns.acquire = std::mem::transmute::<*const c_void, vk::PFN_vkAcquireNextImageKHR>(f.acquire);
					fns.present = std::mem::transmute::<*const c_void, vk::PFN_vkQueuePresentKHR>(f.present);
				}
				self.ffx = f.create.zip(f.destroy);
				vk::SwapchainKHR::from_raw(sc)
			}
		};
		if let Err(e) = self.finish_chain(device, format, sc, extent, mode, "FidelityFX frame-generation Vulkan swapchain") {
			self.ffx_teardown(sc);
			return Err(e);
		}
		super::trace(format_args!("vk chain done, make_ctx"));
		// A refused context: Err, the caller (present.rs) goes back to wgpu's surface for the rest of the run.
		fg.make_ctx(extent, format)?;
		Ok(true)
	}

	/// [S2] The whole FidelityFX swapchain goes (no area, a failed create, exit): presents done, both contexts destroyed, then its
	/// replacement destroy (the header's order). The next `configure_ffx` makes it again.
	fn ffx_teardown(&mut self, sc: vk::SwapchainKHR) {
		let Some(fg) = self.core.fg.clone() else { return };
		fg.wait_presents();
		fg.drop_ctx();
		fg.destroy_swap_ctx();
		if let (Some((_, destroy)), false) = (self.ffx, sc == vk::SwapchainKHR::null()) {
			unsafe { destroy(self.core.device.handle().as_raw() as usize as *mut c_void, sc.as_raw(), std::ptr::null(), sc.as_raw() as usize as *mut c_void) };
		}
	}

	/// [S2] The four queues of FidelityFX's swapchain: the game queue (its submits through `game_submit`) and the device's extra
	/// queues (async compute, present, image acquire), which only FidelityFX uses.
	fn queue_infos(&self) -> Result<[fga::QueueInfo; 4], String> {
		let roles = super::queues::made().ok_or("the Vulkan device has no frame-generation queues")?;
		if [roles.compute, roles.present, roles.acquire].contains(&(self.qfam, 0)) {
			return Err("a frame-generation queue is the game queue".into()); // never (queues::plan); FidelityFX would race wgpu there
		}
		let d = &self.core.device;
		let _ = GAME.set((self.core.queue.as_raw(), d.fp_v1_0().queue_submit));
		let q = |(fam, i): (u32, u32)| fga::QueueInfo { queue: unsafe { d.get_device_queue(fam, i) }.as_raw() as usize as *mut c_void, family_index: fam, submit: None };
		let game = fga::QueueInfo { queue: self.core.queue.as_raw() as usize as *mut c_void, family_index: self.qfam, submit: Some(game_submit) };
		Ok([game, q(roles.compute), q(roles.present), q(roles.acquire)])
	}

	/// Whether a present or a dropped frame asked for a new swapchain, or there is none.
	pub(super) fn needs_configure(&self) -> bool {
		self.chain.is_none() || self.core.stale.load(Ordering::Relaxed)
	}

	#[allow(dead_code)] // stage S2: the vendors' swapchains read it
	pub(super) fn extent(&self) -> Option<(u32, u32)> {
		self.chain.as_ref().map(|c| c.extent)
	}

	/// The next image (1 s timeout).
	pub(super) fn acquire(&mut self) -> Got {
		super::trace(format_args!("vk acquire"));
		let Some(c) = &mut self.chain else { return Got::Skip };
		if let Some((sc, index)) = self.core.held.lock().unwrap_or_else(|e| e.into_inner()).take().filter(|h| h.0 == c.raw) {
			let i = index as usize;
			let f = VkFrame { core: self.core.clone(), swapchain: sc, index, acquire: c.acq[i], done: c.done[i], presented: false };
			return Got::Frame(c.images[i].clone(), f);
		}
		let s = self.spare.pop().unwrap_or_else(|| unsafe { self.core.device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None) }.unwrap_or_default());
		let mut index = 0u32;
		let r = unsafe { (self.core.fns().acquire)(self.core.device.handle(), c.raw, 1_000_000_000, s, vk::Fence::null(), &mut index) };
		match r {
			vk::Result::SUCCESS | vk::Result::SUBOPTIMAL_KHR => {}
			other => {
				self.spare.push(s);
				return match other {
					vk::Result::ERROR_OUT_OF_DATE_KHR => Got::Outdated,
					vk::Result::TIMEOUT | vk::Result::NOT_READY => Got::Skip,
					_ => {
						eprintln!("fgen: vkAcquireNextImageKHR {other:?}");
						Got::Lost
					}
				};
			}
		}
		let i = index as usize;
		let old = std::mem::replace(&mut c.acq[i], s);
		if old != vk::Semaphore::null() {
			self.spare.push(old);
		}
		let f = VkFrame { core: self.core.clone(), swapchain: c.raw, index, acquire: s, done: c.done[i], presented: false };
		Got::Frame(c.images[i].clone(), f)
	}
}

impl Drop for VkSwap {
	fn drop(&mut self) {
		if self.core.fg.is_some() {
			let old = self.drop_chain(true);
			self.ffx_teardown(old);
		} else {
			if let Some(s) = self.sl {
				s.off_now();
			}
			self.drop_chain(false);
		}
		unsafe { self.surface_fn.destroy_surface(self.surface, None) };
	}
}

/// A swapchain image as a wgpu texture (COPY_DST | COPY_SRC | RENDER_ATTACHMENT; first use from the undefined layout).
fn wrap(device: &wgpu::Device, img: vk::Image, format: wgpu::TextureFormat, extent: (u32, u32)) -> Option<wgpu::Texture> {
	use wgpu::hal;
	let size = wgpu::Extent3d { width: extent.0, height: extent.1, depth_or_array_layers: 1 };
	let hd = hal::TextureDescriptor {
		label: Some("fgen swapchain image"),
		size,
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format,
		usage: wgpu::TextureUses::COPY_DST | wgpu::TextureUses::COPY_SRC | wgpu::TextureUses::COLOR_TARGET,
		memory_flags: hal::MemoryFlags::empty(),
		view_formats: Vec::new(),
	};
	let raw = {
		let hal_dev = unsafe { device.as_hal::<hal::api::Vulkan>() }?;
		unsafe { hal_dev.texture_from_raw(img, &hd, Some(Box::new(|| {})), hal::vulkan::TextureMemory::External) }
	};
	let desc = wgpu::TextureDescriptor { label: Some("fgen swapchain image"), size, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::RENDER_ATTACHMENT, view_formats: &[] };
	Some(unsafe { device.create_texture_from_hal::<hal::api::Vulkan>(raw, &desc, wgpu::TextureUses::UNINITIALIZED) })
}

impl VkFrame {
	/// Right before the frame's last wgpu submit, under the capture's submit lock: that submit waits the acquire semaphore and
	/// signals the render-done one.
	pub(super) fn arm(&self, q: &wgpu::Queue) {
		if let Some(h) = unsafe { q.as_hal::<wgpu::hal::api::Vulkan>() } {
			h.add_wait_semaphore(self.acquire, None, vk::PipelineStageFlags::TOP_OF_PIPE);
			h.add_signal_semaphore(self.done, None);
		}
	}

	/// [S2] The swapchain this frame was acquired from, and FidelityFX's present count so far (0 without its counter).
	pub(super) fn swapchain_raw(&self) -> u64 {
		self.swapchain.as_raw()
	}

	pub(super) fn present_count(&self) -> u64 {
		let f = self.core.fg.as_ref().and_then(|fg| *fg.vk_count.lock().unwrap_or_else(|e| e.into_inner()));
		f.map_or(0, |f| unsafe { f(self.swapchain.as_raw()) })
	}

	/// vkQueuePresentKHR (present thread), under the capture's submit lock. Out of date / suboptimal: a new swapchain next frame.
	pub(super) fn present(mut self) {
		self.presented = true;
		let (sc, idx, wait) = ([self.swapchain], [self.index], [self.done]);
		let info = vk::PresentInfoKHR::default().wait_semaphores(&wait).swapchains(&sc).image_indices(&idx);
		let present = self.core.fns().present;
		let r = if self.core.fg.is_some() {
			// [S2] FidelityFX's present submits through `game_submit`, which takes the capture's submit lock itself.
			unsafe { present(self.core.queue, &info) }
		} else {
			let _no_capture_submit = (crate::pack::get().capture_submit_lock)();
			unsafe { present(self.core.queue, &info) }
		};
		match r {
			// Suboptimal: presented; kept like wgpu's surface does (a resize reconfigures anyway).
			vk::Result::SUCCESS | vk::Result::SUBOPTIMAL_KHR => {}
			vk::Result::ERROR_OUT_OF_DATE_KHR => self.core.stale.store(true, Ordering::Relaxed),
			other => {
				self.core.stale.store(true, Ordering::Relaxed);
				eprintln!("fgen: vkQueuePresentKHR {other:?}");
			}
		}
	}
}

impl Drop for VkFrame {
	fn drop(&mut self) {
		if !self.presented {
			// Acquired and never presented (a skipped frame, never armed): the next acquire hands this image out again.
			*self.core.held.lock().unwrap_or_else(|e| e.into_inner()) = Some((self.swapchain, self.index));
		}
	}
}
