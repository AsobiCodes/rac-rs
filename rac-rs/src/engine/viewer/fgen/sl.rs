//! [PC] Frame generation S3 (lane fgdl): NVIDIA DLSS frame generation through Streamline (RAC_FG=dlss2|dlss3|dlss4, options file
//! `framegen=dlss2..4`). Streamline runs with manual hooking: we load `sl.interposer.dll` from the exe's folder ourselves (after a
//! WinVerifyTrust check of its full path), call slInit before the swapchain exists, hand it the native D3D12 device
//! (slSetD3DDevice) and make the window's swapchain through its proxy DXGI factory (swap_dx12.rs). Per frame (index = our counter):
//! `begin_frame` (render thread, before the acquire): frame token, slReflexSleep, PCL simulation + render-submit-start markers;
//! fgen/sl_frame.rs: slSetConstants + slSetTagForFrame (depth, motion vectors, hudless; eValidUntilPresent); `present_begin` /
//! `present_end` (present thread, around the proxy's Present): DLSS-G options (on with N generated frames, or off for a frame
//! without inputs), render-submit-end and present markers, a stats line every 2 s.
//! Vulkan (sl_vk.rs): slInit (Vulkan API) before the window's instance, which is made through the interposer; its device proxy
//! adds what DLSS-G needs; the swapchain goes through its proxies. A run whose instance was not made that way refuses DLSS FG.
//! The menu's DLSS values above the card's maximum are withdrawn here (choice.rs `refuse_dlss`).

use super::sl_abi as abi;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The Streamline DLLs DLSS frame generation needs next to the exe (NVIDIA's; never in git).
pub(crate) const SL_DLLS: [&str; 6] = ["sl.interposer.dll", "sl.common.dll", "sl.dlss_g.dll", "sl.reflex.dll", "sl.pcl.dll", "nvngx_dlssg.dll"];

struct Fns {
	init: abi::PfnInit,
	shutdown: abi::PfnShutdown,
	supported: abi::PfnIsFeatureSupported,
	tag: abi::PfnSetTagForFrame,
	constants: abi::PfnSetConstants,
	feature_fn: abi::PfnGetFeatureFunction,
	token: abi::PfnGetNewFrameToken,
	set_d3d: abi::PfnSetD3DDevice,
	upgrade: abi::PfnUpgradeInterface,
}

struct FeatureFns {
	get_state: abi::PfnDlssgGetState,
	set_options: abi::PfnDlssgSetOptions,
	sleep: abi::PfnReflexSleep,
	reflex_options: abi::PfnReflexSetOptions,
	marker: abi::PfnPclSetMarker,
}

/// One frame's Streamline handle: our frame index and SL's token for it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SlFrame {
	pub(crate) index: u32,
	token: usize,
	/// Constants and tags were set for this frame (fgen/sl_frame.rs): generation runs for it.
	pub(crate) prepared: bool,
}

#[derive(Default)]
struct Stats {
	since: Option<(Instant, u64, u64)>,
	frames: u64,
	generated: u64,
	presented: u64,
}

/// The run's Streamline (one per process: slInit once).
pub(crate) struct Sl {
	f: &'static Fns,
	ff: FeatureFns,
	/// Frames generated per rendered frame (1 = 2x .. 3 = 4x), clamped to what the card offers; lane fglive: changed live by the
	/// menu (`want`), the next DLSS-G options carry it.
	pub(crate) frames: AtomicU32,
	/// The card's maximum generated frames (slDLSSGGetState at `open`; 1 when it did not answer).
	max: u32,
	only_gen: bool,
	next: AtomicU32,
	/// Whether DLSS-G is on in the last options set, with its generated frames (options change only when this changes).
	on: Mutex<Option<(bool, u32)>>,
	failed: AtomicBool,
	errors: AtomicU32,
	stats: Mutex<Stats>,
	/// The previous frame's camera (world -> view, view -> clip, fov; sl_frame.rs: clipToPrevClip).
	pub(super) prev: Mutex<Option<super::sl_frame::sl_math::Prev>>,
	/// "DirectX 12" or "Vulkan" (log lines).
	api: &'static str,
	/// The native D3D12 device (or VkDevice) SL was given (`shutdown` matches the window's Gpu by it).
	device: usize,
	/// slShutdown ran (`shutdown`): no SL call any more.
	down: AtomicBool,
	/// Vulkan: the hudless texture's view, made once per texture (`hudless_resource`).
	hudless: Mutex<HudlessViews>,
}

/// [S3] Vulkan: the view Streamline's hudless tag carries, one per hudless texture (it changes only with the window's size). The
/// view of the texture before stays alive until the next change: SL's present-time work of the frames tagged with it (2x..4x,
/// GPU-bound) may still read it after the resize's `off_now`.
#[derive(Default)]
struct HudlessViews {
	cur: Option<(wgpu::Texture, wgpu::TextureView)>,
	prev: Option<(wgpu::Texture, wgpu::TextureView)>,
}

/// Vulkan: the window's VkDevice when Streamline refused DLSS-G on it (`open`), for `shutdown` (0: none).
static VK_REFUSED_DEV: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

// Streamline's API is thread-safe for these calls (render thread: token/constants/tags; present thread: options/markers).
unsafe impl Send for Sl {}
unsafe impl Sync for Sl {}

unsafe extern "C" fn log_cb(ty: u32, msg: *const std::ffi::c_char) {
	// eInfo = 0 stays quiet; warnings and errors go to stderr (err.txt), the first 40 of the run.
	static N: AtomicU32 = AtomicU32::new(0);
	let verbose = verbose();
	if (ty == 0 && !verbose) || msg.is_null() || N.fetch_add(1, Ordering::Relaxed) >= if verbose { 3000 } else { 40 } {
		return;
	}
	let s = unsafe { std::ffi::CStr::from_ptr(msg) }.to_string_lossy();
	eprintln!("fgen: Streamline {}: {}", if ty == 1 { "warning" } else { "error" }, s.trim_end());
}

/// [Debug] `RAC_SL_VERBOSE=1`: Streamline's verbose log (info lines too, up to 3000) into stderr.
fn verbose() -> bool {
	static V: OnceLock<bool> = OnceLock::new();
	*V.get_or_init(|| std::env::var("RAC_SL_VERBOSE").is_ok_and(|v| v == "1"))
}

static SL: OnceLock<Option<Sl>> = OnceLock::new();

/// The Streamline of this run (made once, on the first DirectX 12 device that asks; None: not available, logged once, or shut
/// down already by `shutdown`).
pub(crate) fn get(adapter: &wgpu::Adapter, device: &wgpu::Device, frames: u32) -> Option<&'static Sl> {
	SL.get_or_init(|| match Sl::open(adapter, device, frames) {
		Ok(s) => Some(s),
		Err(e) => {
			eprintln!("fgen: DLSS frame generation not available ({e}): not offered");
			None
		}
	})
	.as_ref()
	.filter(|s| !s.down.load(Ordering::Acquire))
}

/// The native D3D12 device or VkDevice of `device` (0: neither).
fn raw_dev(device: &wgpu::Device) -> usize {
	use ash::vk::Handle;
	use windows::core::Interface;
	if let Some(d) = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() } {
		return d.raw_device().handle().as_raw() as usize;
	}
	unsafe { device.as_hal::<wgpu::hal::api::Dx12>() }.map_or(0, |d| d.raw_device().as_raw() as usize)
}

/// The loaded interposer after slInit (one per process, for one render API).
pub(super) struct Boot {
	lib: libloading::Library,
	f: Fns,
	api: u32,
}

static BOOT: OnceLock<Result<Boot, String>> = OnceLock::new();

/// Loads `sl.interposer.dll` (signature checked) and runs slInit for `api` once per process: DirectX 12 when the swapchain is
/// made (`Sl::open`), Vulkan before the window's instance (sl_vk.rs). Err: why not (the same answer for the rest of the run).
pub(super) fn boot(api: u32) -> Result<&'static Boot, String> {
	let b = BOOT.get_or_init(|| Boot::load(api)).as_ref().map_err(|e| e.clone())?;
	if b.api != api {
		return Err(format!("Streamline was started for render API {}, not {api}", b.api));
	}
	Ok(b)
}

impl Boot {
	pub(super) fn sym<T: Copy>(&self, name: &str) -> Result<T, String> {
		sym(&self.lib, name)
	}

	fn load(api: u32) -> Result<Boot, String> {
		let dir = exe_dir()?;
		if let Some(m) = SL_DLLS.iter().find(|d| !dir.join(d).is_file()) {
			return Err(format!("{m} is not next to the exe"));
		}
		let path = dir.join(SL_DLLS[0]);
		verify_signature(&path)?;
		let lib = unsafe { libloading::Library::new(&path) }.map_err(|e| format!("{}: {e}", SL_DLLS[0]))?;
		let f = Fns {
			init: sym(&lib, "slInit")?,
			shutdown: sym(&lib, "slShutdown")?,
			supported: sym(&lib, "slIsFeatureSupported")?,
			tag: sym(&lib, "slSetTagForFrame")?,
			constants: sym(&lib, "slSetConstants")?,
			feature_fn: sym(&lib, "slGetFeatureFunction")?,
			token: sym(&lib, "slGetNewFrameToken")?,
			set_d3d: sym(&lib, "slSetD3DDevice")?,
			upgrade: sym(&lib, "slUpgradeInterface")?,
		};
		// slInit: manual hooking, our own plugin folder, no over-the-air or downloaded plugins.
		let plugins = wide(&dir);
		// Streamline's / NGX's logs and app data go into the player dir (never the exe folder; orchestrator 2026-11-07).
		let data_dir = crate::pcopts::player_dir().join("streamline");
		let _ = std::fs::create_dir_all(&data_dir);
		let data = wide(&data_dir);
		let paths = [plugins.as_ptr()];
		let features = [abi::FEATURE_DLSS_G, abi::FEATURE_REFLEX, abi::FEATURE_PCL];
		let pref = abi::Preferences {
			header: abi::Header::of(abi::T_PREFERENCES, 1),
			show_console: false,
			log_level: if verbose() { abi::LOG_DEFAULT + 1 } else { abi::LOG_DEFAULT },
			paths_to_plugins: paths.as_ptr(),
			num_paths_to_plugins: 1,
			path_to_logs_and_data: data.as_ptr(),
			allocate_callback: std::ptr::null(),
			release_callback: std::ptr::null(),
			log_message_callback: Some(log_cb),
			flags: abi::PREF_DISABLE_CL_STATE_TRACKING | abi::PREF_USE_MANUAL_HOOKING | abi::PREF_FRAME_BASED_TAGGING,
			features_to_load: features.as_ptr(),
			num_features_to_load: features.len() as u32,
			application_id: 0,
			engine: 0,
			engine_version: c"1.0".as_ptr(),
			project_id: c"8f3c5b2e-6a41-4d7c-9e0b-52a7c1d4e9f3".as_ptr(),
			render_api: api,
		};
		check("slInit", unsafe { (f.init)(&pref, abi::SDK_VERSION) })?;
		Ok(Boot { lib, f, api })
	}
}

/// The window's Gpu goes (upscale.rs `Drop for Gpu`, after its present thread's last present and a GPU wait, BEFORE NGX's
/// `dlss::shutdown`, the proxy swapchain and the device): DLSS-G off and slShutdown (SL guides: before the DXGI / D3D12 objects
/// go). NGX of rac_ngx shut down first freed DLSS-G's NGX feature under it: nvngx_dlssg.dll crashed at quit (0xC0000005).
/// Nothing when Streamline was never made or runs on another device. Every per-frame call is a no-op from here on.
pub(crate) fn shutdown(device: &wgpu::Device) {
	if matches!(SL.get(), Some(None)) {
		shutdown_refused(device);
		return;
	}
	let Some(Some(sl)) = SL.get() else { return };
	if sl.device != raw_dev(device) || sl.down.load(Ordering::Acquire) {
		return;
	}
	sl.off_now();
	sl.down.store(true, Ordering::Release);
	let r = unsafe { (sl.f.shutdown)() };
	eprintln!("fgen: Streamline shut down ({}, {})", abi::result_name(r), sl.api);
}

/// Vulkan, Streamline refused DLSS-G (`open`) but stays loaded: the window's instance and device run through its proxies for the
/// whole run. At quit (same place as `shutdown`, before the device goes) slShutdown once, as on a run with generation.
fn shutdown_refused(device: &wgpu::Device) {
	let d = VK_REFUSED_DEV.load(Ordering::Acquire);
	if d == 0 || d != raw_dev(device) || VK_REFUSED_DEV.compare_exchange(d, 0, Ordering::AcqRel, Ordering::Acquire).is_err() {
		return;
	}
	let Some(Ok(b)) = BOOT.get() else { return };
	let r = unsafe { (b.f.shutdown)() };
	eprintln!("fgen: Streamline shut down ({}, Vulkan, DLSS frame generation was refused)", abi::result_name(r));
}

/// The exe's folder as a wide path (no trailing NUL) and the full path of `name` in it.
fn exe_dir() -> Result<std::path::PathBuf, String> {
	let exe = std::env::current_exe().map_err(|e| e.to_string())?;
	Ok(exe.parent().ok_or("no exe folder")?.to_path_buf())
}

fn wide(p: &std::path::Path) -> Vec<u16> {
	use std::os::windows::ffi::OsStrExt;
	p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
}

/// WinVerifyTrust (WINTRUST_ACTION_GENERIC_VERIFY_V2, no UI, no revocation) of the file's embedded signature: Ok = signed and
/// the chain is trusted. wintrust.dll is loaded from System32 at run time.
fn verify_signature(path: &std::path::Path) -> Result<(), String> {
	#[repr(C)]
	struct FileInfo {
		cb: u32,
		path: *const u16,
		file: *mut c_void,
		known_subject: *const c_void,
	}
	#[repr(C)]
	struct Data {
		cb: u32,
		policy_callback_data: *mut c_void,
		sip_client_data: *mut c_void,
		ui_choice: u32,
		revocation_checks: u32,
		union_choice: u32,
		file: *mut FileInfo,
		state_action: u32,
		state_data: *mut c_void,
		url_reference: *const u16,
		prov_flags: u32,
		ui_context: u32,
		signature_settings: *mut c_void,
	}
	type Pfn = unsafe extern "system" fn(hwnd: *mut c_void, action: *const abi::Guid, data: *mut Data) -> i32;
	let sys = std::env::var_os("SystemRoot").map(|r| std::path::PathBuf::from(r).join("System32").join("wintrust.dll")).ok_or("no SystemRoot")?;
	let lib = unsafe { libloading::Library::new(&sys) }.map_err(|e| format!("wintrust.dll: {e}"))?;
	let f: libloading::Symbol<Pfn> = unsafe { lib.get(b"WinVerifyTrust\0") }.map_err(|e| e.to_string())?;
	let action = abi::Guid(0x00aac56b, 0xcd44, 0x11d0, [0x8c, 0xc2, 0x00, 0xc0, 0x4f, 0xc2, 0x95, 0xee]);
	let w = wide(path);
	let mut fi = FileInfo { cb: std::mem::size_of::<FileInfo>() as u32, path: w.as_ptr(), file: std::ptr::null_mut(), known_subject: std::ptr::null() };
	let mut d = Data { cb: std::mem::size_of::<Data>() as u32, policy_callback_data: std::ptr::null_mut(), sip_client_data: std::ptr::null_mut(), ui_choice: 2, revocation_checks: 0, union_choice: 1, file: &mut fi, state_action: 1, state_data: std::ptr::null_mut(), url_reference: std::ptr::null(), prov_flags: 0, ui_context: 0, signature_settings: std::ptr::null_mut() };
	let rc = unsafe { f(std::ptr::null_mut(), &action, &mut d) };
	d.state_action = 2; // WTD_STATEACTION_CLOSE
	unsafe { f(std::ptr::null_mut(), &action, &mut d) };
	if rc == 0 { Ok(()) } else { Err(format!("{} has no valid signature (WinVerifyTrust {rc:#x})", path.display())) }
}

fn sym<T: Copy>(lib: &libloading::Library, name: &str) -> Result<T, String> {
	let n = format!("{name}\0");
	unsafe { lib.get::<T>(n.as_bytes()) }.map(|s| *s).map_err(|e| format!("{name}: {e}"))
}

fn check(what: &str, r: i32) -> Result<(), String> {
	if r == abi::RESULT_OK { Ok(()) } else { Err(format!("{what}: {}", abi::result_name(r))) }
}

impl Sl {
	fn open(adapter: &wgpu::Adapter, device: &wgpu::Device, frames: u32) -> Result<Sl, String> {
		let vk_pd = unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }.map(|a| {
			use ash::vk::Handle;
			a.raw_physical_device().as_raw() as usize as *mut c_void
		});
		if vk_pd.is_some() && !super::sl_vk::instance_made() {
			return Err("Vulkan: the window's instance was not made through Streamline (applies after a restart)".into());
		}
		let b = boot(if vk_pd.is_some() { abi::API_VULKAN } else { abi::API_D3D12 })?;
		let f = &b.f;
		let native_dev = match vk_pd {
			Some(_) => None,
			None => Some(unsafe { device.as_hal::<wgpu::hal::api::Dx12>() }.ok_or("neither a DirectX 12 nor a Vulkan device")?.raw_device().clone()),
		};
		let luid = match vk_pd {
			Some(_) => Default::default(),
			None => {
				use windows::Win32::Graphics::Dxgi::IDXGIAdapter1;
				use windows::core::Interface;
				let a = unsafe { adapter.as_hal::<wgpu::hal::api::Dx12>() }.ok_or("not a DirectX 12 adapter")?;
				let a1: IDXGIAdapter1 = a.as_raw().cast().map_err(|e| format!("IDXGIAdapter1 {e}"))?;
				unsafe { a1.GetDesc1() }.map_err(|e| format!("GetDesc1 {e}"))?.AdapterLuid
			}
		};
		let info = adapter.get_info();
		// The card memory's driver: DXGI's user-mode driver version (the same string on both backends: sl_card.rs).
		let driver = super::sl_card::driver_of(&info.name).unwrap_or(&info.driver_info).to_string();
		let card = info.name;
		let up = (|| -> Result<FeatureFns, String> {
			use windows::core::Interface;
			// Vulkan: the interposer's vkCreateDevice proxy told Streamline the device (manual-hooking guide 5.2).
			if let Some(d) = &native_dev {
				check("slSetD3DDevice", unsafe { (f.set_d3d)(d.as_raw()) })?;
			}
			let ai = match vk_pd {
				Some(pd) => abi::AdapterInfo { header: abi::Header::of(abi::T_ADAPTER_INFO, 1), device_luid: std::ptr::null(), device_luid_size: 0, vk_physical_device: pd },
				None => abi::AdapterInfo { header: abi::Header::of(abi::T_ADAPTER_INFO, 1), device_luid: &luid as *const _ as *const u8, device_luid_size: 8, vk_physical_device: std::ptr::null_mut() },
			};
			let r = unsafe { (f.supported)(abi::FEATURE_DLSS_G, &ai) };
			if card_refusal(r) {
				// Next starts with this driver: the DLSS values stay hidden, the row says why (a new driver asks again).
				super::choice::remember_card(&card, &driver, 0);
			}
			check("DLSS frame generation on this GPU", r)?;
			check("Reflex on this GPU", unsafe { (f.supported)(abi::FEATURE_REFLEX, &ai) })?;
			let get = |feature: u32, name: &str| -> Result<*mut c_void, String> {
				let n = format!("{name}\0");
				let mut p = std::ptr::null_mut();
				check(name, unsafe { (f.feature_fn)(feature, n.as_ptr() as *const _, &mut p) })?;
				if p.is_null() { Err(format!("{name}: null")) } else { Ok(p) }
			};
			// SAFETY: Streamline hands out the functions of these names with the sl_dlss_g.h / sl_reflex.h / sl_pcl.h signatures.
			unsafe {
				Ok(FeatureFns {
					get_state: std::mem::transmute::<*mut c_void, abi::PfnDlssgGetState>(get(abi::FEATURE_DLSS_G, "slDLSSGGetState")?),
					set_options: std::mem::transmute::<*mut c_void, abi::PfnDlssgSetOptions>(get(abi::FEATURE_DLSS_G, "slDLSSGSetOptions")?),
					sleep: std::mem::transmute::<*mut c_void, abi::PfnReflexSleep>(get(abi::FEATURE_REFLEX, "slReflexSleep")?),
					reflex_options: std::mem::transmute::<*mut c_void, abi::PfnReflexSetOptions>(get(abi::FEATURE_REFLEX, "slReflexSetOptions")?),
					marker: std::mem::transmute::<*mut c_void, abi::PfnPclSetMarker>(get(abi::FEATURE_PCL, "slPCLSetMarker")?),
				})
			}
		})();
		let ff = match up {
			Ok(ff) => ff,
			Err(e) => {
				// Vulkan keeps Streamline up: the instance and device run through its proxies for the rest of the run; slShutdown
				// at quit (`shutdown_refused`).
				if vk_pd.is_none() {
					unsafe { (f.shutdown)() };
				} else {
					VK_REFUSED_DEV.store(raw_dev(device), Ordering::Release);
				}
				return Err(e);
			}
		};
		// Reflex low latency (required by DLSS-G), no frame limit of its own (pace.rs keeps capping the rendered frames).
		let ro = abi::ReflexOptions { header: abi::Header::of(abi::T_REFLEX_OPTIONS, 1), mode: abi::REFLEX_LOW_LATENCY, frame_limit_us: 0, use_markers_to_optimize: false, virtual_key: 0, id_thread: 0 };
		let r = unsafe { (ff.reflex_options)(&ro) };
		if r != abi::RESULT_OK {
			eprintln!("fgen: slReflexSetOptions: {}", abi::result_name(r));
		}
		let vp = abi::Viewport::new(0);
		let mut st = abi::DlssgState::new();
		let gs = unsafe { (ff.get_state)(&vp, &mut st, std::ptr::null()) };
		let max = if gs == abi::RESULT_OK { st.num_frames_to_generate_max } else { 1 };
		let frames = offer_frames(frames, max);
		match remembered_max(gs, max) {
			// The menus withdraw the multipliers this card cannot run (also next starts with this driver); the row then shows the
			// multiplier that runs (choice.rs `shown`).
			Some(m) => super::choice::remember_card(&card, &driver, m),
			None => {
				eprintln!("fgen: slDLSSGGetState {}: running 2x, nothing remembered", abi::result_name(gs));
				super::choice::refuse_dlss(frames); // this run only
			}
		}
		let api = if vk_pd.is_some() { "Vulkan" } else { "DirectX 12" };
		eprintln!("fgen: DLSS frame generation (Streamline 2.14.16, {api}): card offers up to {}x, running {}x", max.max(1) + 1, frames + 1);
		let only_gen = std::env::var("RAC_FG_ONLYGEN").is_ok_and(|v| v == "1");
		Ok(Sl { f, ff, api, frames: AtomicU32::new(frames), max, only_gen, next: AtomicU32::new(1), on: Mutex::new(None), failed: AtomicBool::new(false), errors: AtomicU32::new(0), stats: Default::default(), prev: Mutex::new(None), device: raw_dev(device), down: AtomicBool::new(false), hudless: Default::default() })
	}

	/// Our DXGI factory's Streamline proxy (swap_dx12.rs makes the window's swapchain through it). `native` keeps its reference.
	pub(super) fn proxy_factory(&self, native: &windows::Win32::Graphics::Dxgi::IDXGIFactory2) -> Result<windows::Win32::Graphics::Dxgi::IDXGIFactory2, String> {
		use windows::core::Interface;
		let mut p = native.clone().into_raw();
		check("slUpgradeInterface", unsafe { (self.f.upgrade)(&mut p) })?;
		// SAFETY: slUpgradeInterface hands back an IDXGIFactory proxy carrying the reference we gave it.
		let any = unsafe { windows::core::IUnknown::from_raw(p) };
		any.cast().map_err(|e| format!("proxy IDXGIFactory2 {e}"))
	}

	/// Vulkan: the Streamline resource of the hudless texture `t` with its cached view (`HudlessViews`; a new texture makes a new
	/// view, the one before is kept until the next change).
	pub(super) fn hudless_resource(&self, t: &wgpu::Texture) -> Option<abi::Resource> {
		let mut h = self.hudless.lock().unwrap_or_else(|e| e.into_inner());
		if h.cur.as_ref().is_none_or(|(c, _)| c != t) {
			let v = t.create_view(&wgpu::TextureViewDescriptor::default());
			h.prev = h.cur.replace((t.clone(), v));
		}
		h.cur.as_ref().and_then(|(t, v)| super::sl_vk::resource(t, v))
	}

	/// An error of a per-frame call: logged the first time; 60 in a row = no generation for the rest of the run.
	fn error(&self, what: &str, r: i32) {
		let n = self.errors.fetch_add(1, Ordering::Relaxed);
		if n == 0 {
			eprintln!("fgen: DLSS FG {what}: {}", abi::result_name(r));
		}
		if n == 59 && !self.failed.swap(true, Ordering::Relaxed) {
			eprintln!("fgen: DLSS FG refused 60 times in a row: no generation for the rest of the run");
		}
	}

	pub(super) fn usable(&self) -> bool {
		!self.failed.load(Ordering::Relaxed)
	}

	fn is_down(&self) -> bool {
		self.down.load(Ordering::Acquire)
	}

	fn marker(&self, m: u32, fr: &SlFrame) {
		unsafe { (self.ff.marker)(m, fr.token as *const abi::FrameToken) };
	}

	/// Render thread, at the frame's start (before the acquire): the frame's token, Reflex's sleep, the simulation markers.
	pub(crate) fn begin_frame(&self) -> Option<SlFrame> {
		if self.is_down() {
			return None;
		}
		let index = self.next.fetch_add(1, Ordering::Relaxed);
		let mut t = std::ptr::null_mut();
		let r = unsafe { (self.f.token)(&mut t, &index) };
		if r != abi::RESULT_OK || t.is_null() {
			self.error("slGetNewFrameToken", r);
			return None;
		}
		let fr = SlFrame { index, token: t as usize, prepared: false };
		unsafe { (self.ff.sleep)(t) };
		self.marker(abi::PCL_SIM_START, &fr);
		self.marker(abi::PCL_SIM_END, &fr);
		self.marker(abi::PCL_RENDER_START, &fr);
		Some(fr)
	}

	/// Render thread: the frame's constants (fgen/sl_frame.rs).
	pub(super) fn set_constants(&self, fr: &SlFrame, c: &abi::Constants) -> bool {
		if self.is_down() {
			return false;
		}
		let vp = abi::Viewport::new(0);
		let r = unsafe { (self.f.constants)(c, fr.token as *const abi::FrameToken, &vp) };
		if r != abi::RESULT_OK {
			self.error("slSetConstants", r);
		}
		r == abi::RESULT_OK
	}

	/// Render thread: the frame's input tags (null resources = cleared).
	pub(super) fn set_tags(&self, fr: &SlFrame, tags: &[abi::ResourceTag]) -> bool {
		if self.is_down() {
			return false;
		}
		let vp = abi::Viewport::new(0);
		let r = unsafe { (self.f.tag)(fr.token as *const abi::FrameToken, &vp, tags.as_ptr(), tags.len() as u32, std::ptr::null_mut()) };
		if r != abi::RESULT_OK {
			self.error("slSetTagForFrame", r);
		}
		r == abi::RESULT_OK
	}

	/// Lane fglive: the menu's DLSS multiplier (`n` generated frames, clamped to the card), carried by the next DLSS-G options.
	/// [C] Streamline 2.14.16 ProgrammingGuideDLSS_G.md 6.2 + sl_dlss_g.h: numFramesToGenerate (1..DLSSGState's
	/// numFramesToGenerateMax) is a DLSSGOptions field set through slDLSSGSetOptions with mode eOn, the call that already flips
	/// the mode per frame here. [I] No restart needed: the guide asks for none, and its Dynamic MFG (6.3) changes the count per
	/// frame inside SL; proven live in lane fglive's runs (HANDOFF.md).
	pub(super) fn want(&self, n: u32) {
		let n = offer_frames(n, self.max);
		if self.frames.swap(n, Ordering::Relaxed) != n {
			eprintln!("fgen: DLSS frame generation {}x from now on (menu)", n + 1);
		}
	}

	/// DLSS-G on (this frame has its inputs) or off, set only when it (or, while on, the generated frames) changes; `force` sets
	/// it always (before a resize). `frame`: the SL frame of the present (None: a resize / swapchain change). One log line per
	/// call (lane fglive: which frame each slDLSSGSetOptions lands in, for SL's "Repeated slDLSSGSetOptions" warning).
	fn set_on(&self, on: bool, force: bool, frame: Option<u32>) {
		if self.is_down() {
			return;
		}
		let frames = self.frames.load(Ordering::Relaxed);
		let key = (on, if on { frames } else { 0 });
		let mut last = self.on.lock().unwrap_or_else(|e| e.into_inner());
		if !force && *last == Some(key) {
			return;
		}
		let mut flags = abi::DLSSG_RETAIN_RESOURCES_WHEN_OFF;
		if self.only_gen {
			flags |= abi::DLSSG_SHOW_ONLY_INTERPOLATED;
		}
		let o = abi::DlssgOptions::new(if on { abi::DLSSG_ON } else { abi::DLSSG_OFF }, frames, flags);
		let r = unsafe { (self.ff.set_options)(&abi::Viewport::new(0), &o) };
		if r != abi::RESULT_OK {
			self.error("slDLSSGSetOptions", r);
		}
		*last = Some(key);
		let at = frame.map_or_else(|| "a swapchain change".to_string(), |i| format!("frame {i}"));
		eprintln!("fgen: DLSS-G options {} ({}x) at {at}", if on { "on" } else { "off" }, frames + 1);
	}

	/// Before the swapchain's buffers change (resize) or go: DLSS-G off (SL guide 12.0: a present could deadlock otherwise).
	pub(super) fn off_now(&self) {
		// Never set yet (the first configure) or already off: nothing to switch (a second call per frame makes SL warn).
		if matches!(*self.on.lock().unwrap_or_else(|e| e.into_inner()), Some((true, _))) {
			self.set_on(false, true, None);
		}
	}

	/// Present thread, right before the proxy's Present of `fr`.
	pub(crate) fn present_begin(&self, fr: &SlFrame) {
		if self.is_down() {
			return;
		}
		self.set_on(fr.prepared && self.usable(), false, Some(fr.index));
		self.marker(abi::PCL_RENDER_END, fr);
		self.marker(abi::PCL_PRESENT_START, fr);
	}

	/// Present thread, after the proxy's Present of `fr`: the present marker and a stats line every 2 s.
	pub(crate) fn present_end(&self, fr: &SlFrame) {
		if self.is_down() {
			return;
		}
		self.marker(abi::PCL_PRESENT_END, fr);
		let mut st = abi::DlssgState::new();
		let presented = if unsafe { (self.ff.get_state)(&abi::Viewport::new(0), &mut st, std::ptr::null()) } == abi::RESULT_OK { st.num_frames_actually_presented.max(1) } else { 1 };
		let now = Instant::now();
		let mut s = self.stats.lock().unwrap_or_else(|e| e.into_inner());
		s.frames += 1;
		s.generated += fr.prepared as u64;
		s.presented += presented as u64;
		let (frames, pres) = (s.frames, s.presented);
		crate::diag::presents(pres); // [PC] RAC_DIAG perf.csv fg_presented_fps
		let (t0, f0, p0) = *s.since.get_or_insert((now, frames, pres));
		let dt = now - t0;
		if dt >= Duration::from_secs(2) {
			let (df, dp, secs) = (frames - f0, pres - p0, dt.as_secs_f32());
			eprintln!("fgen: DLSS FG {}x: {df} rendered, {dp} presented in {secs:.1} s ({:.1} -> {:.1} fps), {} of {} frames with generation, status {:#x}", self.frames.load(Ordering::Relaxed) + 1, df as f32 / secs, dp as f32 / secs, s.generated, s.frames, st.status);
			s.since = Some((now, frames, pres));
		}
	}
}

/// Whether slIsFeatureSupported's answer means the CARD runs no DLSS frame generation (remembered across runs: choice.rs
/// `remember_card`), not a driver, OS or HAGS setting the player can change (pure).
pub(super) fn card_refusal(r: i32) -> bool {
	matches!(r, abi::RESULT_NO_SUPPORTED_ADAPTER | abi::RESULT_ADAPTER_NOT_SUPPORTED | abi::RESULT_FEATURE_NOT_SUPPORTED)
}

/// What the card memory keeps from slDLSSGGetState's answer `r` and its maximum (pure): the maximum (1..3) only when the call
/// succeeded; a failed call says nothing about the card (None: nothing remembered, the run makes 2x).
pub(super) fn remembered_max(r: i32, max: u32) -> Option<u32> {
	(r == abi::RESULT_OK).then(|| max.clamp(1, 3))
}

/// Generated frames per rendered frame for a wanted count (1..3 = 2x..4x) and the card's maximum (pure): never above the card's
/// maximum nor above 3 (4x, user 2026-11-06), at least 1.
pub(super) fn offer_frames(want: u32, card_max: u32) -> u32 {
	want.clamp(1, 3).min(card_max.max(1))
}
