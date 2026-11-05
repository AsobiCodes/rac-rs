//! [PC] DLSS Super Resolution through NVIDIA NGX (lane dlss2, upscaler stage 4; no retail counterpart: the game never sees it).
//! Nothing NVIDIA is linked: `rac_ngx.dll` (ours, a C ABI over NGX, built outside this tree; its structs in crate rac_ngx_abi) and
//! NVIDIA's `nvngx_dlss.dll` are opened at run time from the folder of the exe (`lib`). Missing files (every public install, Linux),
//! `RAC_DLSS=0`, another GPU or a refusal = no DLSS: the Upscaler row does not offer DLSS and a DLSS choice runs Off (choice.rs).
//! - Device: Vulkan needs NGX's extensions on the instance and device (gpu/dlss_vk.rs, made through wgpu-hal's callbacks only on an
//!   adapter whose NGX requirements say "supported"); DX12 needs nothing. `device_made` notes what the window's device got.
//! - `runtime`: NGX init on the window's device once (the first frame DLSS is chosen), caps, the optimal render sizes (`optimal`,
//!   the rows' render size: rscale.rs plans with it). `Feature`: one DLSS feature (render size, window size, quality, preset).
//! - Inputs as FSR 3 (ffx.rs): gamma-space LDR colour, R32 depth (0 near), UV motion vectors current -> previous (scale = render
//!   size), the Halton jitter in render pixels (same sign as FSR's), auto exposure, reset on cuts. Preset M (L for Ultra
//!   performance) = DLSS 4.5, asked for in every mode (`abi::preset_dlss45`); NGX's report of the preset names the row's value.

use rac_ngx_abi as abi;
use std::ffi::{c_char, c_void, CStr};
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

/// Our helper DLL and NVIDIA's DLSS DLL, both next to the exe.
pub const DLL: &str = "rac_ngx.dll";
pub const MODEL_DLL: &str = "nvngx_dlss.dll";
/// PCI vendor id of NVIDIA.
pub const NVIDIA: u32 = 0x10DE;
/// The flags of every feature: motion vectors at the render size and not jittered (motion.rs), NGX's own exposure; depth 0 near
/// (not inverted), LDR colour (not HDR).
pub const FLAGS: u32 = abi::FLAG_MV_LOW_RES | abi::FLAG_AUTO_EXPOSURE;

/// The graphics API of the device NGX runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
	Vulkan,
	Dx12,
}

/// The opened helper and the app description NGX gets (the UTF-16 data folder kept alive here).
pub struct Lib {
	pub api: abi::Lib,
	_data: Vec<u16>,
	pub app: abi::RacNgxApp,
}

// The helper is thread-safe (NGX calls are serialised by the caller where NGX needs it); `app` points into `_data` and statics.
unsafe impl Send for Lib {}
unsafe impl Sync for Lib {}

/// Dev switch `RAC_DLSS=0`: no DLL is opened, the device is made as without them (ENV.md).
pub fn switched_off(v: Option<&str>) -> bool {
	v.is_some_and(|v| v.trim() == "0")
}

/// `RAC_DLSS_LOG=1`: NGX's verbose log, every line into the run log (dev; default: info level, only its error lines).
fn verbose() -> bool {
	std::env::var("RAC_DLSS_LOG").is_ok_and(|v| v == "1")
}

unsafe extern "C" fn log_sink(_user: *mut c_void, _level: u32, msg: *const c_char) {
	if msg.is_null() {
		return;
	}
	let line = unsafe { CStr::from_ptr(msg) }.to_string_lossy();
	if verbose() || line.to_ascii_lowercase().contains("error") {
		eprintln!("ngx: {line}");
	}
}

/// `rac_ngx.dll` + `nvngx_dlss.dll` from the exe folder, opened once (None: missing, `RAC_DLSS=0`, a wrong ABI; logged once).
pub fn lib() -> Option<&'static Lib> {
	static LIB: OnceLock<Option<Lib>> = OnceLock::new();
	LIB.get_or_init(|| match open() {
		Ok(l) => Some(l),
		Err(e) => {
			eprintln!("upscale: DLSS not available ({e})");
			None
		}
	})
	.as_ref()
}

fn open() -> Result<Lib, String> {
	if switched_off(std::env::var("RAC_DLSS").ok().as_deref()) {
		return Err(String::from("RAC_DLSS=0"));
	}
	let exe = std::env::current_exe().map_err(|e| e.to_string())?;
	let dir = exe.parent().ok_or("no exe folder")?;
	for f in [DLL, MODEL_DLL] {
		if !dir.join(f).is_file() {
			return Err(format!("{f} not found in {}", dir.display()));
		}
	}
	let api = unsafe { abi::Lib::load(&dir.join(DLL)) }?;
	let data_dir = std::env::temp_dir().join("rac-rs-ngx");
	let _ = std::fs::create_dir_all(&data_dir);
	let data: Vec<u16> = data_dir.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
	let level = if verbose() { abi::LOG_VERBOSE } else { abi::LOG_ON };
	unsafe { (api.rac_ngx_set_log)(Some(log_sink), std::ptr::null_mut(), level) };
	let app = abi::RacNgxApp { project_id: std::ptr::null(), engine_version: c"rac-rs".as_ptr(), data_path: data.as_ptr(), dll_dir: std::ptr::null(), log_level: level, _pad: 0 };
	Ok(Lib { api, _data: data, app })
}

impl Lib {
	pub fn name(&self, r: u32) -> String {
		self.api.result_name(r)
	}
}

/// What the window's device got (`device_made`): 0 not made yet / not DLSS-ready, 1 Vulkan with NGX's extensions, 2 DX12 on an
/// NVIDIA adapter whose NGX requirements passed.
static READY: AtomicU8 = AtomicU8::new(0);
/// The native device `READY` is about (VkDevice / ID3D12Device*), so another device (the GS capture's, tests) never gets NGX.
static READY_DEV: AtomicUsize = AtomicUsize::new(0);

/// gpu/dlss_vk.rs, after the window's device was made: whether DLSS can run on it.
pub fn device_made(device: &wgpu::Device, ready: bool) {
	let Some((b, raw)) = native(device).filter(|_| ready) else {
		READY.store(0, Ordering::Relaxed);
		return;
	};
	READY_DEV.store(raw as usize, Ordering::Relaxed);
	READY.store(if b == Backend::Vulkan { 1 } else { 2 }, Ordering::Relaxed);
}

/// Whether the window's device can run DLSS (the menu label; no NGX call).
pub fn device_ready() -> bool {
	READY.load(Ordering::Relaxed) != 0
}

/// The backend and native device of a wgpu device (None: another backend).
pub fn native(device: &wgpu::Device) -> Option<(Backend, *mut c_void)> {
	#[cfg(not(target_vendor = "apple"))]
	if let Some(d) = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() } {
		return Some((Backend::Vulkan, unsafe { std::mem::transmute_copy::<_, u64>(&d.raw_device().handle()) } as usize as *mut c_void));
	}
	#[cfg(windows)]
	if let Some(d) = unsafe { device.as_hal::<wgpu::hal::api::Dx12>() } {
		use windows::core::Interface;
		return Some((Backend::Dx12, d.raw_device().as_raw()));
	}
	let _ = device;
	None
}

/// NGX on the window's device (one init per run) and its DLSS caps.
pub struct Runtime {
	pub lib: &'static Lib,
	pub ctx: *mut abi::RacNgxCtx,
	pub backend: Backend,
	pub device: *mut c_void,
	/// ((window, quality) -> render size) asked so far (`optimal`).
	sizes: Mutex<Vec<((u32, u32), u32, Option<(u32, u32)>)>>,
}

// The context belongs to the window's device, alive for the whole run; NGX's calls on it are thread-safe for our use (create and
// release on the render thread with the GPU idle, evaluate recorded on one thread at a time under the feature's Mutex).
unsafe impl Send for Runtime {}
unsafe impl Sync for Runtime {}

/// NGX of the window's device: (the native device it was probed for, the result). One probe per window device, logged once;
/// another device (the GS capture's, tests) never gets NGX. `shutdown` ends NGX before its device goes and empties this, so a
/// new window device (a Gpu rebuild) probes again (review R21 m3). A runtime is leaked per init (one per window device).
static RT: Mutex<Option<(usize, Option<&'static Runtime>)>> = Mutex::new(None);

fn rt_lock() -> std::sync::MutexGuard<'static, Option<(usize, Option<&'static Runtime>)>> {
	RT.lock().unwrap_or_else(|e| e.into_inner())
}

fn raw_of(device: &wgpu::Device) -> usize {
	native(device).map_or(0, |(_, d)| d as usize)
}

/// NGX on `device`, initialised once (None: no DLSS on this run: missing DLLs, another GPU, a refusal; logged once). Only the
/// window's device (`device_made`) ever gets it.
pub fn runtime(device: &wgpu::Device) -> Option<&'static Runtime> {
	let raw = raw_of(device);
	let mut g = rt_lock();
	if let Some((d, rt)) = *g {
		return rt.filter(|_| d == raw);
	}
	let r = probe(device);
	match &r {
		Ok(rt) => eprintln!("upscale: DLSS available (NGX on {:?})", rt.backend),
		Err(e) => eprintln!("upscale: DLSS not available ({e}): no upscaler instead"),
	}
	let rt: Option<&'static Runtime> = r.ok().map(|rt| &*Box::leak(Box::new(rt)));
	*g = Some((raw, rt));
	rt
}

/// The window's Gpu goes (upscale.rs `Drop for Gpu`: the GPU is idle and the slot's feature released): NGX is shut down while its
/// device still lives (else the device is destroyed under NGX's objects), and the next window device probes again.
pub fn shutdown(device: &wgpu::Device) {
	let raw = raw_of(device);
	let mut g = rt_lock();
	if g.is_some_and(|(d, _)| d == raw) {
		if let Some((_, Some(rt))) = g.take() {
			let r = unsafe { (rt.lib.api.rac_ngx_shutdown)(rt.ctx) };
			eprintln!("upscale: DLSS: NGX shut down ({})", rt.lib.name(r));
		}
	}
}

/// Whether NGX was initialised on this run's device (None: not tried yet; never initialises it).
pub fn probed() -> Option<bool> {
	rt_lock().map(|(_, r)| r.is_some())
}

/// The runtime when it was made already (never initialises NGX).
pub fn made() -> Option<&'static Runtime> {
	rt_lock().and_then(|(_, r)| r)
}

fn probe(device: &wgpu::Device) -> Result<Runtime, String> {
	let lib = lib().ok_or("rac_ngx.dll / nvngx_dlss.dll missing or switched off")?;
	let (backend, raw) = native(device).ok_or("not a Vulkan or D3D12 device")?;
	if !device_ready() || READY_DEV.load(Ordering::Relaxed) != raw as usize {
		return Err(String::from("the GPU does not run DLSS (not NVIDIA RTX, or NGX's requirements failed)"));
	}
	let mut ctx: *mut abi::RacNgxCtx = std::ptr::null_mut();
	let r = match backend {
		Backend::Vulkan => init_vk(lib, device, &mut ctx),
		Backend::Dx12 => unsafe { (lib.api.rac_ngx_init_dx12)(&lib.app, raw, &mut ctx) },
	};
	if abi::failed(r) || ctx.is_null() {
		return Err(format!("NGX init: {}", lib.name(r)));
	}
	let mut caps = abi::RacNgxCaps::default();
	let r = unsafe { (lib.api.rac_ngx_caps)(ctx, &mut caps) };
	if abi::failed(r) || caps.available != 1 {
		unsafe { (lib.api.rac_ngx_shutdown)(ctx) };
		let driver = if caps.needs_driver != 0 { format!(", needs driver {}.{}", caps.min_driver_major, caps.min_driver_minor) } else { String::new() };
		return Err(format!("NGX caps: {}, available {}{driver}, init {}", lib.name(r), caps.available, lib.name(caps.init_result)));
	}
	Ok(Runtime { lib, ctx, backend, device: raw, sizes: Mutex::new(Vec::new()) })
}

#[cfg(not(target_vendor = "apple"))]
fn init_vk(lib: &Lib, device: &wgpu::Device, ctx: &mut *mut abi::RacNgxCtx) -> u32 {
	let Some(d) = (unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }) else { return abi::ERR_WRONG_API };
	let shared = d.shared_instance();
	let h = |v: u64| v as usize as *mut c_void;
	let init = abi::RacNgxVkInit {
		instance: h(unsafe { std::mem::transmute_copy::<_, u64>(&shared.raw_instance().handle()) }),
		physical: h(unsafe { std::mem::transmute_copy::<_, u64>(&d.raw_physical_device()) }),
		device: h(unsafe { std::mem::transmute_copy::<_, u64>(&d.raw_device().handle()) }),
		gipa: shared.entry().static_fn().get_instance_proc_addr as *const c_void,
		gdpa: shared.raw_instance().fp_v1_0().get_device_proc_addr as *const c_void,
	};
	unsafe { (lib.api.rac_ngx_init_vk)(&lib.app, &init, ctx) }
}

#[cfg(target_vendor = "apple")]
fn init_vk(_: &Lib, _: &wgpu::Device, _: &mut *mut abi::RacNgxCtx) -> u32 {
	abi::ERR_WRONG_API
}

impl Runtime {
	/// NGX's render size for `win` and `quality` (None: the mode is not offered, or the call failed), asked once per pair.
	pub fn optimal(&self, win: (u32, u32), quality: u32) -> Option<(u32, u32)> {
		let mut s = self.sizes.lock().unwrap_or_else(|e| e.into_inner());
		if let Some(e) = s.iter().find(|e| e.0 == win && e.1 == quality) {
			return e.2;
		}
		let mut o = abi::RacNgxOptimal::default();
		let r = unsafe { (self.lib.api.rac_ngx_optimal)(self.ctx, win.0, win.1, quality, &mut o) };
		let size = (!abi::failed(r) && o.render_w > 0 && o.render_h > 0).then_some((o.render_w, o.render_h));
		s.push((win, quality, size));
		size
	}
}

/// The NGX quality mode of a window / render ratio: the nearest of DLAA 1, Quality 1.5, Balanced 1.7 (NGX: 1 / 0.58), Performance
/// 2, Ultra performance 3 (slot.rs `QUALITY`; a percentage from RAC_UPSCALE gets the nearest mode).
pub fn ngx_quality(ratio: f32) -> u32 {
	const MODES: [(f32, u32); 5] = [(1.0, abi::QUALITY_DLAA), (1.5, abi::QUALITY_MAX_QUALITY), (1.7, abi::QUALITY_BALANCED), (2.0, abi::QUALITY_MAX_PERF), (3.0, abi::QUALITY_ULTRA_PERFORMANCE)];
	MODES.iter().fold((f32::INFINITY, abi::QUALITY_MAX_QUALITY), |b, &(r, q)| if (r - ratio).abs() < b.0 { ((r - ratio).abs(), q) } else { b }).1
}

/// The render size DLSS runs at for a window size and ratio: NGX's optimal one once NGX runs (`made`), else None (the ratio's).
pub fn optimal_size(win: (u32, u32), ratio: f32) -> Option<(u32, u32)> {
	made()?.optimal(win, ngx_quality(ratio))
}

/// The menu name of a reported preset letter: M / L (2nd-gen transformer) = "DLSS 4.5", J / K = "DLSS 4", else "DLSS".
pub fn label_of(preset: u32) -> String {
	match char::from_u32(preset).unwrap_or('-') {
		'M' | 'L' => String::from("DLSS 4.5"),
		'J' | 'K' => String::from("DLSS 4"),
		_ => String::from("DLSS"),
	}
}

/// The create desc of a feature: (render, window) sizes, quality mode, preset M (L for Ultra performance).
pub fn create_desc(sizes: ((u32, u32), (u32, u32)), quality: u32) -> abi::RacNgxCreate {
	abi::RacNgxCreate { render_w: sizes.0 .0, render_h: sizes.0 .1, out_w: sizes.1 .0, out_h: sizes.1 .1, quality, preset: abi::preset_dlss45(quality), flags: FLAGS, _pad: 0 }
}

/// The VkFormat of the formats the slot hands over (0 = VK_FORMAT_UNDEFINED for anything else).
pub fn vk_format(f: wgpu::TextureFormat) -> u32 {
	use wgpu::TextureFormat as F;
	match f {
		F::Rgba8Unorm => 37,
		F::Rgba8UnormSrgb => 43,
		F::Bgra8Unorm => 44,
		F::Bgra8UnormSrgb => 50,
		F::Rg16Float => 83,
		F::Rgba16Float => 97,
		F::R32Float => 100,
		_ => 0,
	}
}

/// `RAC_DLSS_JITTER=-1`: the jitter goes to NGX negated (dev A/B of the sign convention; ENV.md). Default +1: the offset the
/// projection was moved by, as FSR takes it.
pub fn jitter_sign(v: Option<&str>) -> f32 {
	if v.is_some_and(|v| v.trim() == "-1") { -1.0 } else { 1.0 }
}

/// One frame's evaluate values: jitter (render pixels, `sign` applied), UV vectors times the render size, reset, frame time.
pub fn eval_desc(render: (u32, u32), jitter: [f32; 2], sign: f32, reset: bool, frame_ms: f32) -> abi::RacNgxEval {
	abi::RacNgxEval {
		jitter_x: jitter[0] * sign,
		jitter_y: jitter[1] * sign,
		mv_scale_x: render.0 as f32,
		mv_scale_y: render.1 as f32,
		render_w: render.0,
		render_h: render.1,
		reset: reset as u32,
		pre_exposure: 1.0,
		exposure_scale: 1.0,
		frame_ms,
	}
}

/// One DLSS feature on the runtime's device (released on drop: the GPU must be done with it).
pub struct Feature {
	rt: &'static Runtime,
	feat: *mut abi::RacNgxFeature,
	/// The preset NGX reported (ASCII letter; 0 = no report).
	pub preset: u32,
}

// Used under a Mutex (dlss_frame.rs) on the render thread (Vulkan) or the submitting thread (DX12 hook).
unsafe impl Send for Feature {}

impl Feature {
	/// Creates the feature, its init work recorded into `cmd` (an open VkCommandBuffer / ID3D12GraphicsCommandList*), which the
	/// caller executes before the first evaluate.
	///
	/// # Safety
	/// `cmd` must be an open command buffer / list of the runtime's device.
	pub unsafe fn create(rt: &'static Runtime, cmd: *mut c_void, cr: &abi::RacNgxCreate) -> Result<Feature, String> {
		let mut feat: *mut abi::RacNgxFeature = std::ptr::null_mut();
		let r = unsafe { (rt.lib.api.rac_ngx_create)(rt.ctx, cmd, cr, &mut feat) };
		if abi::failed(r) || feat.is_null() {
			return Err(format!("NGX create: {}", rt.lib.name(r)));
		}
		let mut info = abi::RacNgxFeatureInfo::default();
		let r = unsafe { (rt.lib.api.rac_ngx_feature_info)(rt.ctx, feat, &mut info) };
		let preset = if abi::failed(r) { 0 } else { info.preset_reported };
		Ok(Feature { rt, feat, preset })
	}

	/// Records the upscale into an open VkCommandBuffer.
	///
	/// # Safety
	/// `cmd` open on the runtime's device; every image alive and in the layouts `RacNgxVkInputs` names until it ran.
	pub unsafe fn eval_vk(&mut self, cmd: *mut c_void, inputs: &abi::RacNgxVkInputs, eval: &abi::RacNgxEval) -> Result<(), String> {
		let r = unsafe { (self.rt.lib.api.rac_ngx_evaluate_vk)(self.rt.ctx, self.feat, cmd, inputs, eval) };
		if abi::failed(r) { Err(format!("NGX evaluate: {}", self.rt.lib.name(r))) } else { Ok(()) }
	}

	/// Records the upscale into an open ID3D12GraphicsCommandList.
	///
	/// # Safety
	/// `list` open on the runtime's device; every resource alive and in the states `RacNgxDx12Inputs` names until it ran.
	pub unsafe fn eval_dx12(&mut self, list: *mut c_void, inputs: &abi::RacNgxDx12Inputs, eval: &abi::RacNgxEval) -> Result<(), String> {
		let r = unsafe { (self.rt.lib.api.rac_ngx_evaluate_dx12)(self.rt.ctx, self.feat, list, inputs, eval) };
		if abi::failed(r) { Err(format!("NGX evaluate: {}", self.rt.lib.name(r))) } else { Ok(()) }
	}
}

impl Drop for Feature {
	fn drop(&mut self) {
		let r = unsafe { (self.rt.lib.api.rac_ngx_release)(self.rt.ctx, self.feat) };
		if abi::failed(r) {
			eprintln!("upscale: DLSS release: {}", self.rt.lib.name(r));
		}
	}
}

#[cfg(test)]
#[path = "dlss_tests.rs"]
mod dlss_tests;
