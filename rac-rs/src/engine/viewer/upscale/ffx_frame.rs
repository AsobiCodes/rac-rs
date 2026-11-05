//! [PC] The per-frame FidelityFX upscale (lane nups2, upscaler stage 3b; no retail counterpart): FSR 3.1 / 4 through AMD's
//! loader DLLs (ffx.rs) in the upscaler slot (slot.rs). The newest version the runtime offers is pinned
//! (`pick_version`); the menu shows its short name (`short_label`, ui/live.rs). Per frame (slot.rs `slot_encode`): the scene
//! resolved to the render size (slot_gpu.rs `resolve`), the reactive mask (ffx_reactive.rs), wgpu transitions of every input to
//! RESOURCE and of the output (the slot's window-size `mid`) to STORAGE_READ_WRITE, then the FFX dispatch in a command buffer of
//! its own, never mixed with wgpu commands (wgpu 30 panics on that):
//! - Vulkan: a fresh wgpu encoder used only through `as_hal_mut`, put between the frame's wgpu parts (presenter.rs `Cuts::raw`).
//! - DX12: wgpu gives no access to its command list: our own lists (ffx_dx12.rs), executed on the queue between the two halves
//!   of the frame's submit (`Cuts::split` hook; the ReShade add-on, the only other hook, is off while the slot runs).
//! FFX restores the states it was given, so wgpu's tracking stays right. Then slot_gpu.rs `copy` copies `mid` into the HUD pass's
//! window-size MSAA target (no second sharpening: FFX sharpens, `sharpness_of_stops`). A failed create or dispatch switches FFX
//! off for the run (`failed`, one log line, `ffx_label(None)`): the FSR choice runs Off from then on (the frame in between gets
//! slot_gpu.rs `scale`, a plain bilinear stretch). The context is made for the exact
//! (render, window) sizes; the caller makes the GPU idle (presenter flush + device poll) before an old one is dropped.

use super::ffx::{self, Backend, Frame, Runtime, UpscaleContext};
use super::ffx_abi as abi;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// (render, window) sizes.
pub type Sizes = ((u32, u32), (u32, u32));

/// The first `major.minor[.patch]` in a version name ("FSR 3.1.5", "4.1.2 (driver)"); None without one.
pub fn parse_version(name: &str) -> Option<[u32; 3]> {
	name.split(|c: char| !(c.is_ascii_digit() || c == '.')).find_map(|tok| {
		let mut it = tok.trim_matches('.').split('.');
		let major = it.next()?.parse().ok()?;
		let minor = it.next()?.parse().ok()?;
		let patch = it.next().and_then(|p| p.parse().ok()).unwrap_or(0);
		Some([major, minor, patch])
	})
}

/// The version to pin: the highest parsed `major.minor.patch` (the loader's first one when no name parses); None for no version.
pub fn pick_version(v: &[(u64, String)]) -> Option<(u64, &str)> {
	let best = v.iter().filter_map(|(id, n)| parse_version(n).map(|p| (p, *id, n.as_str()))).fold(None::<([u32; 3], u64, &str)>, |b, x| match b {
		Some(b) if b.0 >= x.0 => Some(b),
		_ => Some(x),
	});
	best.map(|b| (b.1, b.2)).or_else(|| v.first().map(|(id, n)| (*id, n.as_str())))
}

/// The menu text of a version name: "FSR major.minor" ("FSR 3.1", "FSR 4.1"); "FSR" when it does not parse.
pub fn short_label(name: &str) -> String {
	parse_version(name).map_or_else(|| String::from("FSR"), |v| format!("FSR {}.{}", v[0], v[1]))
}

/// FFX's sharpness (0..1) from `RAC_UPSCALE`'s sharpening stops (0 = sharpest, 2 = none; the old RCAS scale: strength 2^-stops
/// matches 1 - stops / 2 at the ends); the row default `slot::SHARP` (0.2 stops) gives 0.9.
pub fn sharpness_of_stops(stops: f32) -> f32 {
	(1.0 - stops / 2.0).clamp(0.0, 1.0)
}

/// The history is dropped on a cut (upscale.rs `is_cut`), on a new context and after a frame without FFX (`prev` = the last
/// frame number FFX ran on; a fallback frame, Off or a pause in between).
pub fn needs_reset(cut: bool, ctx_new: bool, prev: Option<u32>, frame: u32) -> bool {
	cut || ctx_new || prev != Some(frame.wrapping_sub(1))
}

/// The frame time FFX gets, ms: with the test clock (`clock` = (field seconds, display frames per field)) exactly one display
/// frame of it (pictures repeat); else the wall time since the last dispatch, 1..100 ms (16.7 on the first frame).
pub fn frame_ms(clock: Option<(f32, u32)>, wall_ms: Option<f32>) -> f32 {
	match clock {
		Some((dt, sub)) => dt * 1000.0 / sub.max(1) as f32,
		None => wall_ms.unwrap_or(1000.0 / 60.0).clamp(1.0, 100.0),
	}
}

/// What the dispatch reads and writes this frame (all render size but `output`, window size).
pub(super) struct Io<'a> {
	pub color: &'a wgpu::Texture,
	pub depth: &'a wgpu::Texture,
	pub motion: &'a wgpu::Texture,
	pub reactive: Option<&'a wgpu::Texture>,
	pub output: &'a wgpu::Texture,
}

/// The frame values from upscale.rs (`Prep`): jitter (render pixels), cut, frame number, camera far plane and vertical fov.
#[derive(Clone, Copy, Debug)]
pub(super) struct FrameIn {
	pub jitter: [f32; 2],
	pub cut: bool,
	pub frame: u32,
	pub far: f32,
	pub fov_y: f32,
	pub sharp: f32,
}

/// The FidelityFX side of the slot: the runtime, the pinned version, the context and the failure flag.
pub(in crate::viewer) struct FfxGpu {
	rt: &'static Runtime,
	/// For the GPU wait before the context goes (`Drop`, review R16 F4).
	device: wgpu::Device,
	pin: Option<u64>,
	/// The pinned version's name ("FSR 3.1.5").
	pub(super) name: String,
	ctx: Option<(Sizes, Arc<Mutex<UpscaleContext>>)>,
	/// The context was made since the last dispatch (reset).
	fresh: bool,
	failed: Arc<AtomicBool>,
	last: Option<u32>,
	clock: Option<Instant>,
	pub(super) reactive: super::ffx_reactive::ReactiveGpu,
	#[cfg(windows)]
	lists: Option<Arc<Mutex<super::ffx_dx12::Lists>>>,
	#[cfg(windows)]
	queue: wgpu::Queue,
}

impl FfxGpu {
	pub(super) fn new(device: &wgpu::Device, queue: &wgpu::Queue, rt: &'static Runtime) -> FfxGpu {
		let (pin, name) = pick_version(&rt.versions).map_or((None, String::from("FSR")), |(id, n)| (Some(id), n.to_string()));
		let _ = queue;
		FfxGpu {
			rt,
			device: device.clone(),
			pin,
			name,
			ctx: None,
			fresh: false,
			failed: Arc::new(AtomicBool::new(false)),
			last: None,
			clock: None,
			reactive: super::ffx_reactive::ReactiveGpu::new(device),
			#[cfg(windows)]
			lists: None,
			#[cfg(windows)]
			queue: queue.clone(),
		}
	}

	/// FFX may run (no failed create or dispatch so far).
	pub(super) fn usable(&self) -> bool {
		!self.failed.load(Ordering::Relaxed)
	}

	/// Whether a context exists that the frame with `want` (None: no FFX this frame) would drop: the caller makes the GPU idle first.
	/// A refused run's context goes too (review R16 F5: its VRAM is not held for the run).
	pub(super) fn needs_idle(&self, want: Option<Sizes>) -> bool {
		self.ctx.as_ref().is_some_and(|(s, _)| want != Some(*s) || !self.usable())
	}

	/// Drops the context (the GPU must be idle: `needs_idle`).
	pub(super) fn drop_ctx(&mut self) {
		self.ctx = None;
	}

	/// The context for `sizes` (made when missing or for other sizes; the old one must be idle). False = refused (logged, FFX off).
	pub(super) fn prepare(&mut self, sizes: Sizes) -> bool {
		if !self.usable() {
			self.ctx = None; // the caller made the GPU idle (`needs_idle` is true for a refused context)
			return false;
		}
		if self.ctx.as_ref().is_some_and(|(s, _)| *s == sizes) {
			return true;
		}
		self.ctx = None;
		match UpscaleContext::create(self.rt, sizes.0, sizes.1, self.pin) {
			Ok(mut c) => {
				let backend = if self.rt.backend == Backend::Vulkan { "Vulkan" } else { "Dx12" };
				eprintln!("upscale: {} (FidelityFX, {backend}), render {}x{} -> {}x{}", c.version(), sizes.0 .0, sizes.0 .1, sizes.1 .0, sizes.1 .1);
				self.ctx = Some((sizes, Arc::new(Mutex::new(c))));
				self.fresh = true;
				true
			}
			Err(e) => {
				self.fail(&e);
				false
			}
		}
	}

	fn fail(&self, why: &str) {
		if !self.failed.swap(true, Ordering::Relaxed) {
			eprintln!("upscale: FidelityFX refused ({why}): no upscaler instead");
			super::ffx_label(None); // the menu names what runs (review R16 F5)
		}
	}

	/// This frame's dispatch values.
	fn frame(&mut self, sizes: Sizes, f: &FrameIn) -> Frame {
		let now = Instant::now();
		let wall = self.clock.replace(now).map(|t| (now - t).as_secs_f32() * 1000.0);
		let clock = crate::viewer::test_clock::clock_on().then(|| (crate::field::field_dt(), crate::viewer::test_clock::sub()));
		let reset = needs_reset(f.cut, std::mem::take(&mut self.fresh), self.last, f.frame);
		self.last = Some(f.frame);
		Frame { render: sizes.0, win: sizes.1, jitter: f.jitter, sharpness: sharpness_of_stops(f.sharp), frame_ms: frame_ms(clock, wall), reset, near: crate::viewer::projection::RETAIL_NEAR, far: f.far, fov_y: f.fov_y }
	}

	/// The upscale of this frame from `io` into `io.output` (after the wgpu commands so far in `enc`). False = not recorded (no
	/// context, refused now, or the frame's submit hook is taken): the caller draws slot_gpu.rs `scale` (plain bilinear) instead.
	pub(super) fn dispatch(&mut self, device: &wgpu::Device, enc: &mut wgpu::CommandEncoder, cuts: &mut crate::viewer::presenter::Cuts, io: &Io<'_>, f: &FrameIn) -> bool {
		let Some((sizes, ctx)) = self.ctx.as_ref().filter(|_| self.usable()).map(|(s, c)| (*s, c.clone())) else { return false };
		if self.rt.backend == Backend::Dx12 && cuts.has_hook() {
			return false;
		}
		let read = wgpu::TextureUses::RESOURCE;
		let mut list = vec![(io.color, read), (io.depth, read), (io.motion, read), (io.output, wgpu::TextureUses::STORAGE_READ_WRITE)];
		list.extend(io.reactive.map(|r| (r, read)));
		enc.transition_resources(std::iter::empty(), list.iter().map(|&(texture, state)| wgpu::TextureTransition { texture, selector: None, state }));
		let frame = self.frame(sizes, f);
		let r = match self.rt.backend {
			Backend::Vulkan => self.record_vk(device, enc, cuts, io, &frame, &ctx),
			Backend::Dx12 => self.record_dx12(device, enc, cuts, io, &frame, ctx),
		};
		match r {
			Ok(()) => {
				if self.rt.backend == Backend::Dx12 {
					// Review R16 F3: the same states again in the fresh encoder after the split (a no-op barrier), so wgpu keeps the
					// textures' memory until the frame's second half ran, which the queue runs after our list.
					enc.transition_resources(std::iter::empty(), list.iter().map(|&(texture, state)| wgpu::TextureTransition { texture, selector: None, state }));
				}
				true
			}
			Err(e) => {
				self.fail(&e);
				false
			}
		}
	}

	/// Vulkan: the dispatch in a raw-only wgpu command buffer between the frame's wgpu parts.
	fn record_vk(&self, device: &wgpu::Device, enc: &mut wgpu::CommandEncoder, cuts: &mut crate::viewer::presenter::Cuts, io: &Io<'_>, frame: &Frame, ctx: &Arc<Mutex<UpscaleContext>>) -> Result<(), String> {
		let img = |t: &wgpu::Texture| -> Result<*mut c_void, String> {
			let h = unsafe { t.as_hal::<wgpu::hal::api::Vulkan>() }.ok_or("no Vulkan image")?;
			Ok(unsafe { std::mem::transmute_copy::<_, u64>(&h.raw_handle()) } as usize as *mut c_void)
		};
		let handles = Handles { color: img(io.color)?, depth: img(io.depth)?, motion: img(io.motion)?, reactive: io.reactive.map(img).transpose()?, output: img(io.output)? };
		let mut desc = handles.desc(io, frame);
		let mut raw = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("ffx upscale") });
		let rc = unsafe {
			raw.as_hal_mut::<wgpu::hal::api::Vulkan, _, _>(|e| {
				let e = e.ok_or(u32::MAX)?;
				desc.command_list = std::mem::transmute_copy::<_, *mut c_void>(&e.raw_handle());
				ctx.lock().map_err(|_| u32::MAX - 1)?.dispatch(&desc)
			})
		};
		rc.map_err(|rc| format!("ffxDispatch (upscale) failed: return code {rc:#x}"))?;
		cuts.raw(device, enc, raw.finish());
		Ok(())
	}

	/// DX12: the dispatch in our own command list, executed between the two halves of the frame's submit (ffx_dx12.rs).
	#[cfg(windows)]
	fn record_dx12(&mut self, device: &wgpu::Device, enc: &mut wgpu::CommandEncoder, cuts: &mut crate::viewer::presenter::Cuts, io: &Io<'_>, frame: &Frame, ctx: Arc<Mutex<UpscaleContext>>) -> Result<(), String> {
		use windows::core::Interface;
		if self.lists.is_none() {
			self.lists = Some(Arc::new(Mutex::new(super::ffx_dx12::Lists::new(self.rt.dev.device, &self.queue)?)));
		}
		let lists = self.lists.clone().expect("made above");
		let res = |t: &wgpu::Texture| -> Result<windows::Win32::Graphics::Direct3D12::ID3D12Resource, String> {
			let h = unsafe { t.as_hal::<wgpu::hal::api::Dx12>() }.ok_or("no D3D12 resource")?;
			Ok(unsafe { h.raw_resource() }.clone())
		};
		let keep = [Some(res(io.color)?), Some(res(io.depth)?), Some(res(io.motion)?), io.reactive.map(res).transpose()?, Some(res(io.output)?)];
		let ptr = |i: usize| keep[i].as_ref().map_or(std::ptr::null_mut(), |r| r.as_raw());
		let handles = Handles { color: ptr(0), depth: ptr(1), motion: ptr(2), reactive: keep[3].as_ref().map(|_| ptr(3)), output: ptr(4) };
		let job = super::ffx_dx12::Job { desc: handles.desc(io, frame), keep: keep.into_iter().flatten().collect() };
		let failed = self.failed.clone();
		cuts.split(
			device,
			enc,
			Box::new(move || {
				let job = job; // the whole Job (Send), not its fields (edition 2024 closures capture by path)
				let super::ffx_dx12::Job { mut desc, keep } = job;
				let record = |list: *mut c_void| {
					desc.command_list = list;
					let rc = unsafe { ctx.lock().map_err(|_| String::from("context lock"))?.dispatch(&desc) };
					rc.map_err(|rc| format!("ffxDispatch (upscale) failed: return code {rc:#x}"))
				};
				let r = lists.lock().map_err(|_| String::from("lists lock")).and_then(|mut l| l.run(record, keep));
				if let Err(e) = r {
					if !failed.swap(true, Ordering::Relaxed) {
						eprintln!("upscale: FidelityFX refused ({e}): no upscaler instead");
						super::ffx_label(None);
					}
				}
			}),
		);
		Ok(())
	}

	#[cfg(not(windows))]
	fn record_dx12(&mut self, _: &wgpu::Device, _: &mut wgpu::CommandEncoder, _: &mut crate::viewer::presenter::Cuts, _: &Io<'_>, _: &Frame, _: Arc<Mutex<UpscaleContext>>) -> Result<(), String> {
		Err(String::from("D3D12 only on Windows"))
	}
}

/// Review R16 F4: the GPU finishes with the context before ffxDestroyContext frees its images and pipelines (shutdown, a Gpu
/// rebuild); the normal size / choice changes idle first anyway (`needs_idle`).
impl Drop for FfxGpu {
	fn drop(&mut self) {
		if self.ctx.is_some() {
			let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
			self.ctx = None;
		}
	}
}

/// The native handles of one frame's textures (VkImage / ID3D12Resource*).
struct Handles {
	color: *mut c_void,
	depth: *mut c_void,
	motion: *mut c_void,
	reactive: Option<*mut c_void>,
	output: *mut c_void,
}

impl Handles {
	/// The dispatch desc (command list filled in by the recorder): inputs in PIXEL_COMPUTE_READ (wgpu RESOURCE), output in
	/// UNORDERED_ACCESS (STORAGE_READ_WRITE).
	fn desc(&self, io: &Io<'_>, frame: &Frame) -> abi::DispatchUpscale {
		let tex = |h: *mut c_void, t: &wgpu::Texture, usage: u32, state: u32| ffx::texture(h, t.format(), (t.width(), t.height()), usage, state);
		let read = |h, t| tex(h, t, abi::USAGE_READ_ONLY, abi::STATE_PIXEL_COMPUTE_READ);
		let mut d = ffx::dispatch_desc(std::ptr::null_mut(), read(self.color, io.color), read(self.depth, io.depth), read(self.motion, io.motion), tex(self.output, io.output, abi::USAGE_UAV, abi::STATE_UNORDERED_ACCESS), frame);
		if let (Some(h), Some(t)) = (self.reactive, io.reactive) {
			d.reactive = read(h, t);
		}
		d
	}
}

#[cfg(test)]
#[path = "ffx_frame_tests.rs"]
mod ffx_frame_tests;
