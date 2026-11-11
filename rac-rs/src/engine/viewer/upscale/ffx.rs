//! [PC] FSR 4.1 / 3.1 through AMD's FidelityFX API (lane ndlss stage 3; `RAC_UPSCALE=fsr[:quality]`, dev only). The loader DLL
//! of the wgpu backend (`amd_fidelityfx_loader_vk.dll` / `_dx12.dll`, FSR SDK 2.4.0, MIT) is opened at run time from the folder
//! of the exe; it finds `amd_fidelityfx_upscaler_vk.dll` / `_dx12.dll` there and offers every upscaler version the PC runs
//! (`versions`: FSR 4.1.2 on Radeon RX 6000+ / RDNA 3.5 APUs, else 3.1.x; a newer driver-provided one wins automatically).
//! Nothing is linked: no DLL (Linux, a missing file, macOS) = `runtime` gives None: FSR cannot run, a FSR choice runs Off (choice.rs `set_runs`).
//! The C structs are in ffx_abi.rs. Inputs (upscale.rs / motion.rs): the render-size colour (gamma-space 8-bit, LDR: the
//! non-linear flags, exposure 1), the R32 depth copy (0 near .. 1 far), motion vectors in UV (scale = render size, current ->
//! previous), the Halton jitter in render pixels (same sign convention as FSR's samples), reset on cuts.

#![allow(dead_code)] // a few ABI helpers (configure, the other queries) are kept for later stages

use super::ffx_abi as abi;
use std::ffi::{c_void, CStr};
use std::path::Path;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
	Vulkan,
	Dx12,
}

/// The loader DLL (exports ffxCreateContext ..) and the upscaler DLL it opens, per backend (FSR SDK 2.x names).
pub fn dll_names(b: Backend) -> (&'static str, &'static str) {
	match b {
		Backend::Vulkan => ("amd_fidelityfx_loader_vk.dll", "amd_fidelityfx_upscaler_vk.dll"),
		Backend::Dx12 => ("amd_fidelityfx_loader_dx12.dll", "amd_fidelityfx_upscaler_dx12.dll"),
	}
}

/// The native device of the wgpu device: VkDevice + VkPhysicalDevice + vkGetDeviceProcAddr, or the ID3D12Device.
#[derive(Clone, Copy, Debug)]
pub struct DeviceHandles {
	pub device: *mut c_void,
	pub physical: *mut c_void,
	pub proc_addr: *const c_void,
}

/// The five entry points of the loader.
#[derive(Clone, Copy)]
pub struct Fns {
	pub create: abi::PfnCreateContext,
	pub destroy: abi::PfnDestroyContext,
	pub configure: abi::PfnConfigure,
	pub query: abi::PfnQuery,
	pub dispatch: abi::PfnDispatch,
}

/// The opened loader of this run, its device and the upscaler versions it offers (id, name; the loader's order).
pub struct Runtime {
	_lib: libloading::Library,
	pub fns: Fns,
	pub backend: Backend,
	pub dev: DeviceHandles,
	pub versions: Vec<(u64, String)>,
}

// The handles are the wgpu device's, alive for the whole run (the device outlives every frame); the loader is thread-safe.
unsafe impl Send for Runtime {}
unsafe impl Sync for Runtime {}

/// Opens `dir`'s loader DLL for `b` and resolves its entry points; Err says what is missing.
pub fn load(dir: &Path, b: Backend) -> Result<(libloading::Library, Fns), String> {
	let (loader, upscaler) = dll_names(b);
	load_with(dir, loader, upscaler)
}

/// Opens `dir`'s `loader` DLL when the effect DLL `effect` is next to it (fgen/ffx_fg.rs: frame generation's) and resolves its
/// entry points; Err says what is missing.
pub fn load_with(dir: &Path, loader: &str, effect: &str) -> Result<(libloading::Library, Fns), String> {
	for f in [loader, effect] {
		if !dir.join(f).is_file() {
			return Err(format!("{f} not found in {}", dir.display()));
		}
	}
	let lib = unsafe { libloading::Library::new(dir.join(loader)) }.map_err(|e| format!("{loader}: {e}"))?;
	let fns = unsafe {
		let get = |n: &[u8]| -> Result<*const c_void, String> {
			lib.get::<*const c_void>(n).map(|s| *s).map_err(|e| format!("{loader}: {e}"))
		};
		Fns {
			create: std::mem::transmute::<*const c_void, abi::PfnCreateContext>(get(b"ffxCreateContext\0")?),
			destroy: std::mem::transmute::<*const c_void, abi::PfnDestroyContext>(get(b"ffxDestroyContext\0")?),
			configure: std::mem::transmute::<*const c_void, abi::PfnConfigure>(get(b"ffxConfigure\0")?),
			query: std::mem::transmute::<*const c_void, abi::PfnQuery>(get(b"ffxQuery\0")?),
			dispatch: std::mem::transmute::<*const c_void, abi::PfnDispatch>(get(b"ffxDispatch\0")?),
		}
	};
	Ok((lib, fns))
}

/// The wgpu device's backend and native handles (None: another backend, e.g. Metal or GL).
pub fn device_handles(device: &wgpu::Device) -> Option<(Backend, DeviceHandles)> {
	#[cfg(not(target_vendor = "apple"))]
	if let Some(d) = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() } {
		let raw = d.raw_device();
		let proc_addr = d.shared_instance().raw_instance().fp_v1_0().get_device_proc_addr as *const c_void;
		let dev = DeviceHandles {
			device: unsafe { std::mem::transmute_copy::<_, *mut c_void>(&raw.handle()) },
			physical: unsafe { std::mem::transmute_copy::<_, *mut c_void>(&d.raw_physical_device()) },
			proc_addr,
		};
		return Some((Backend::Vulkan, dev));
	}
	#[cfg(windows)]
	if let Some(d) = unsafe { device.as_hal::<wgpu::hal::api::Dx12>() } {
		use windows::core::Interface;
		let dev = DeviceHandles { device: d.raw_device().as_raw(), physical: std::ptr::null_mut(), proc_addr: std::ptr::null() };
		return Some((Backend::Dx12, dev));
	}
	let _ = device;
	None
}

impl DeviceHandles {
	/// The handles as the SDK's `VkDeviceContext` (Vulkan backend desc and version query).
	pub fn vk_context(&self) -> abi::VkDeviceContext {
		abi::VkDeviceContext { vk_device: self.device, vk_physical_device: self.physical, vk_device_proc_addr: self.proc_addr }
	}
}

/// `ffxQueryDescGetVersions.device` for `b` (ffx_api.h): the ID3D12Device* on DX12, on Vulkan a pointer to `vk` (the caller's
/// `VkDeviceContext`, kept alive while the queries run), never the raw VkDevice.
pub fn query_device(b: Backend, d: &DeviceHandles, vk: &mut abi::VkDeviceContext) -> *mut c_void {
	match b {
		Backend::Vulkan => {
			*vk = d.vk_context();
			vk as *mut abi::VkDeviceContext as *mut c_void
		}
		Backend::Dx12 => d.device,
	}
}

/// The chained backend desc of a create / version query for `rt`'s device (kept alive by the caller while the call runs).
pub enum BackendDesc {
	Vk(abi::BackendVk),
	Dx12(abi::BackendDx12),
}

impl BackendDesc {
	pub fn of(b: Backend, d: &DeviceHandles) -> BackendDesc {
		match b {
			Backend::Vulkan => BackendDesc::Vk(abi::BackendVk { header: abi::Header::of(abi::CREATE_BACKEND_VK), device_context: d.vk_context() }),
			Backend::Dx12 => BackendDesc::Dx12(abi::BackendDx12 { header: abi::Header::of(abi::CREATE_BACKEND_DX12), device: d.device }),
		}
	}

	pub fn header(&mut self) -> *mut abi::Header {
		match self {
			BackendDesc::Vk(v) => &mut v.header,
			BackendDesc::Dx12(v) => &mut v.header,
		}
	}
}

/// The upscaler versions the loader offers on this device: two calls (count, then ids + names).
pub fn versions(fns: &Fns, b: Backend, d: &DeviceHandles) -> Vec<(u64, String)> {
	versions_of(fns, b, d, abi::CREATE_UPSCALE)
}

/// The versions the loader offers on this device for the effect whose create desc type is `create_desc_type`.
pub fn versions_of(fns: &Fns, b: Backend, d: &DeviceHandles, create_desc_type: u64) -> Vec<(u64, String)> {
	let (mut count, mut vk) = (0u64, d.vk_context());
	let device = query_device(b, d, &mut vk); // R9-1: `vk` outlives both queries below
	let mut q = abi::QueryVersions { header: abi::Header::of(abi::QUERY_GET_VERSIONS), create_desc_type, device, output_count: &mut count, version_ids: std::ptr::null_mut(), version_names: std::ptr::null_mut() };
	if unsafe { (fns.query)(std::ptr::null_mut(), &mut q.header) } != abi::RETURN_OK || count == 0 || count > 64 {
		return Vec::new();
	}
	let mut ids = vec![0u64; count as usize];
	let mut names = vec![std::ptr::null(); count as usize];
	q.version_ids = ids.as_mut_ptr();
	q.version_names = names.as_mut_ptr();
	if unsafe { (fns.query)(std::ptr::null_mut(), &mut q.header) } != abi::RETURN_OK {
		return Vec::new();
	}
	let n = (count as usize).min(ids.len());
	(0..n).map(|i| (ids[i], if names[i].is_null() { String::new() } else { unsafe { CStr::from_ptr(names[i]) }.to_string_lossy().into_owned() })).collect()
}

/// The FidelityFX runtime of this run, probed once (loader next to the exe, the device's backend, at least one version);
/// None = no FSR (the slot stays off). Logs the outcome once. Keyed to the device it was probed on (R9-5): the window's device is
/// made once per process (`Gpu::new`, rac1 handler `resumed` / rac2 host; a lost device is logged, never recreated), and any other
/// device gets None (no FSR) instead of contexts on the first device's raw handles.
pub fn runtime(device: &wgpu::Device) -> Option<&'static Runtime> {
	static RT: OnceLock<Option<Runtime>> = OnceLock::new();
	let rt = RT
		.get_or_init(|| {
			let r = probe(device);
			match &r {
				Ok(rt) => eprintln!("upscale: FidelityFX {:?}: {}", rt.backend, rt.versions.iter().map(|v| v.1.as_str()).collect::<Vec<_>>().join(", ")),
				Err(e) => eprintln!("upscale: FidelityFX not available ({e}): no upscaler instead"),
			}
			r.ok()
		})
		.as_ref()?;
	device_handles(device).is_some_and(|(b, d)| same_device((rt.backend, &rt.dev), (b, &d))).then_some(rt)
}

/// Whether two (backend, handles) name the same native device.
pub fn same_device(a: (Backend, &DeviceHandles), b: (Backend, &DeviceHandles)) -> bool {
	a.0 == b.0 && a.1.device == b.1.device
}

fn probe(device: &wgpu::Device) -> Result<Runtime, String> {
	let (backend, dev) = device_handles(device).ok_or("not a Vulkan or D3D12 device")?;
	let exe = std::env::current_exe().map_err(|e| e.to_string())?;
	let (lib, fns) = load(exe.parent().ok_or("no exe folder")?, backend)?;
	let versions = versions(&fns, backend, &dev);
	if versions.is_empty() {
		return Err("the loader offers no upscaler for this GPU".into());
	}
	Ok(Runtime { _lib: lib, fns, backend, dev, versions })
}

/// The FFX surface format of a wgpu texture format (the ones the slot hands over; else unknown).
pub fn surface_format(f: wgpu::TextureFormat) -> u32 {
	use wgpu::TextureFormat as F;
	match f {
		F::Rgba8Unorm => abi::FORMAT_R8G8B8A8_UNORM,
		F::Rgba8UnormSrgb => abi::FORMAT_R8G8B8A8_SRGB,
		F::Bgra8Unorm => abi::FORMAT_B8G8R8A8_UNORM,
		F::Bgra8UnormSrgb => abi::FORMAT_B8G8R8A8_SRGB,
		F::Rgba16Float => abi::FORMAT_R16G16B16A16_FLOAT,
		F::Rg16Float => abi::FORMAT_R16G16_FLOAT,
		F::R32Float | F::Depth32Float => abi::FORMAT_R32_FLOAT,
		F::R8Unorm => abi::FORMAT_R8_UNORM, // the reactive mask (ffx_reactive.rs `FORMAT`; review R16 F1)
		_ => abi::FORMAT_UNKNOWN,
	}
}

/// A 2D texture input / output: `handle` = VkImage or ID3D12Resource*, `state` = where the frame left it (ffx_abi STATE_*).
/// Every format handed over must be known to FFX: an unknown one becomes VK_FORMAT_UNDEFINED in its image view (review R16 F1).
pub fn texture(handle: *mut c_void, format: wgpu::TextureFormat, size: (u32, u32), usage: u32, state: u32) -> abi::Resource {
	debug_assert_ne!(surface_format(format), abi::FORMAT_UNKNOWN, "FFX gets a texture of format {format:?} it does not know");
	let description = abi::ResourceDesc { ty: abi::RESOURCE_TYPE_TEXTURE2D, format: surface_format(format), width: size.0, height: size.1, depth: 1, mip_count: 1, flags: 0, usage };
	abi::Resource { resource: handle, description, state }
}

/// One frame's values for the dispatch.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
	pub render: (u32, u32),
	pub win: (u32, u32),
	/// upscale.rs `Prep::jitter`: render pixels, the offset the projection was moved by.
	pub jitter: [f32; 2],
	/// FSR's own RCAS (0 = off; the slot sharpens nothing afterwards: slot_gpu.rs `copy` is an exact copy).
	pub sharpness: f32,
	pub frame_ms: f32,
	pub reset: bool,
	pub near: f32,
	pub far: f32,
	pub fov_y: f32,
}

/// The dispatch desc of one frame (inputs from `texture`; reactive (set by the caller), transparency-and-composition and
/// exposure none).
pub fn dispatch_desc(cmd: *mut c_void, color: abi::Resource, depth: abi::Resource, motion: abi::Resource, output: abi::Resource, f: &Frame) -> abi::DispatchUpscale {
	abi::DispatchUpscale {
		header: abi::Header::of(abi::DISPATCH_UPSCALE),
		command_list: cmd,
		color,
		depth,
		motion_vectors: motion,
		exposure: abi::Resource::none(),
		reactive: abi::Resource::none(),
		transparency_and_composition: abi::Resource::none(),
		output,
		jitter_offset: abi::Coords { x: f.jitter[0], y: f.jitter[1] },
		// Our vectors are in UV units of the render picture: times the render size = the pixels FSR expects.
		motion_vector_scale: abi::Coords { x: f.render.0 as f32, y: f.render.1 as f32 },
		render_size: abi::Dim { width: f.render.0, height: f.render.1 },
		upscale_size: abi::Dim { width: f.win.0, height: f.win.1 },
		enable_sharpening: f.sharpness > 0.0,
		sharpness: f.sharpness.clamp(0.0, 1.0),
		frame_time_delta: f.frame_ms,
		pre_exposure: 1.0,
		reset: f.reset,
		camera_near: f.near,
		camera_far: f.far,
		camera_fov_angle_vertical: f.fov_y,
		view_space_to_meters_factor: 1.0,
		flags: abi::DISPATCH_NON_LINEAR_COLOR_SRGB,
	}
}

/// One upscaler context (created for the largest render / window size; recreated when they grow), destroyed on drop.
pub struct UpscaleContext {
	ctx: abi::Context,
	rt: &'static Runtime,
	pub max: ((u32, u32), (u32, u32)),
}

// The context is used under a Mutex (ffx_frame.rs), on the render thread (Vulkan) or the submitting thread (DX12 hook).
unsafe impl Send for UpscaleContext {}

impl UpscaleContext {
	/// `version`: None = the newest the loader offers (FSR 4.1 where it runs), Some(id) = pinned (`ffxOverrideVersion`).
	pub fn create(rt: &'static Runtime, max_render: (u32, u32), max_win: (u32, u32), version: Option<u64>) -> Result<UpscaleContext, String> {
		let mut backend = BackendDesc::of(rt.backend, &rt.dev);
		let mut api = abi::CreateUpscaleVersion { header: abi::Header::of(abi::CREATE_UPSCALE_VERSION), version: abi::UPSCALER_VERSION };
		let mut pin = abi::OverrideVersion { header: abi::Header::of(abi::DESC_OVERRIDE_VERSION), version_id: version.unwrap_or(0) };
		let mut desc = abi::CreateUpscale {
			header: abi::Header::of(abi::CREATE_UPSCALE),
			// LDR gamma-space colour (non-linear) and no exposure texture: FFX computes the exposure itself.
			flags: abi::CREATE_ENABLE_NON_LINEAR_COLORSPACE | abi::CREATE_ENABLE_AUTO_EXPOSURE,
			max_render_size: abi::Dim { width: max_render.0, height: max_render.1 },
			max_upscale_size: abi::Dim { width: max_win.0, height: max_win.1 },
			message: None,
		};
		// The chain as the SDK's FSR sample builds it: create desc -> backend -> upscaler API version (R9-3) [-> version pin].
		// Linked from the tail: no write through a binding after a raw pointer to it was taken.
		if version.is_some() {
			api.header.next = &mut pin.header;
		}
		unsafe { (*backend.header()).next = &mut api.header };
		desc.header.next = backend.header();
		let mut ctx: abi::Context = std::ptr::null_mut();
		let rc = unsafe { (rt.fns.create)(&mut ctx, &mut desc.header, std::ptr::null()) };
		if rc != abi::RETURN_OK || ctx.is_null() {
			return Err(format!("ffxCreateContext (upscale) failed: return code {rc}"));
		}
		Ok(UpscaleContext { ctx, rt, max: (max_render, max_win) })
	}

	/// The version this context runs ("FSR 4.1.2", "FSR 3.1.6", ..).
	pub fn version(&mut self) -> String {
		let mut q = abi::QueryProviderVersion { header: abi::Header::of(abi::QUERY_GET_PROVIDER_VERSION), version_id: 0, version_name: std::ptr::null() };
		if unsafe { (self.rt.fns.query)(&mut self.ctx, &mut q.header) } != abi::RETURN_OK || q.version_name.is_null() {
			return String::from("?");
		}
		unsafe { CStr::from_ptr(q.version_name) }.to_string_lossy().into_owned()
	}

	/// Records the upscale into the native command list / buffer of `desc` (its resources in the states it names).
	///
	/// # Safety
	/// `desc.command_list` must be an open command list / buffer of this context's device, every resource alive until it ran.
	pub unsafe fn dispatch(&mut self, desc: &abi::DispatchUpscale) -> Result<(), u32> {
		match unsafe { (self.rt.fns.dispatch)(&mut self.ctx, &desc.header) } {
			abi::RETURN_OK => Ok(()),
			rc => Err(rc),
		}
	}
}

impl Drop for UpscaleContext {
	fn drop(&mut self) {
		unsafe { (self.rt.fns.destroy)(&mut self.ctx, std::ptr::null()) };
	}
}

#[cfg(test)]
#[path = "ffx_tests.rs"]
mod ffx_tests;
