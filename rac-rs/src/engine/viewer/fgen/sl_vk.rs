//! [PC] Frame generation S3 (lane fgdl): DLSS frame generation on Vulkan through Streamline (RAC_FG=dlss2..4 at start; the
//! row says "Applies after a restart"). Streamline must see the Vulkan instance and device being made, so on a run that starts
//! with a DLSS mode (and only then: Off, FSR and every other run make exactly today's instance) the window's instance is made
//! through `sl.interposer.dll`'s own `vkGetInstanceProcAddr` (`instance`, after slInit with the Vulkan API: sl.rs `boot`). Every
//! instance-level call wgpu-hal makes then goes through the interposer: its vkCreateDevice proxy adds DLSS-G's and Reflex's
//! device extensions, features and extra queues and tells Streamline the device (no slSetVulkanInfo of ours, manual-hooking
//! guide 5.2), its vkCreateWin32SurfaceKHR / vkDestroySurfaceKHR proxies take swap_vk.rs's surface. The swapchain calls go
//! through the interposer's `vkGetDeviceProcAddr` proxies (`swap_fns`: create / destroy / images / acquire / present).
//! Tags on Vulkan carry the VkImage, its VkImageView, layout, size, format and usage (`resource`, pure parts `vk_format`,
//! `vk_usage`, `vk_layout`).

use super::sl_abi as abi;
use ash::vk;
use ash::vk::Handle;
use std::ffi::{c_void, CStr};
use std::sync::atomic::{AtomicBool, Ordering};

/// The window's Vulkan instance was made through Streamline (sl.rs `open` refuses DLSS FG on Vulkan otherwise).
static MADE: AtomicBool = AtomicBool::new(false);

/// Also read by gscap's device pick: a second VkDevice on this instance would go through Streamline's vkCreateDevice proxy (SL
/// re-runs its plugin start on it: "Plugins already initialized but could be using the wrong device").
pub(crate) fn instance_made() -> bool {
	MADE.load(Ordering::Relaxed)
}

/// Whether this run makes its Vulkan instance through Streamline: a DLSS mode at start, all Streamline DLLs next to the exe, the
/// card the game will use is an NVIDIA card (DXGI, no device made: an AMD / Intel card never gets Streamline's instance and device
/// proxies) and no remembered refusal of that card on this driver (sl_card.rs `vk_gate`). One log line when the mode asks but
/// something is missing.
pub(crate) fn wanted() -> bool {
	if !matches!(super::mode(), super::Mode::Dlss(_)) {
		return false;
	}
	let dir = match std::env::current_exe() {
		Ok(e) => e.parent().map(|p| p.to_path_buf()),
		Err(_) => None,
	};
	let Some(dir) = dir else { return false };
	if let Some(m) = super::sl::SL_DLLS.iter().find(|d| !dir.join(d).is_file()) {
		eprintln!("fgen: DLSS frame generation on Vulkan: {m} is not next to the exe: today's Vulkan instance");
		return false;
	}
	if let Err(e) = super::sl_card::vk_gate(super::sl_card::preferred(), super::choice::card_memory().as_ref()) {
		eprintln!("fgen: DLSS frame generation on Vulkan: {e}: today's Vulkan instance");
		return false;
	}
	true
}

/// The window's Vulkan-only instance made through Streamline's interposer, with wgpu-hal's own extensions + `extra` (NGX's for
/// DLSS SR, where the loader has them). None (logged): wgpu's own path (DLSS FG then refuses at the swapchain: sl.rs `open`).
pub(crate) fn instance(desc: &wgpu::InstanceDescriptor, extra: &[&'static CStr]) -> Option<wgpu::Instance> {
	match make_instance(desc, extra) {
		Ok(i) => {
			MADE.store(true, Ordering::Relaxed);
			eprintln!("fgen: Vulkan instance made through Streamline (DLSS frame generation)");
			Some(i)
		}
		Err(e) => {
			eprintln!("fgen: DLSS frame generation on Vulkan: no Streamline instance ({e}): today's Vulkan instance");
			None
		}
	}
}

fn make_instance(desc: &wgpu::InstanceDescriptor, extra: &[&'static CStr]) -> Result<wgpu::Instance, String> {
	use wgpu::hal;
	let b = super::sl::boot(abi::API_VULKAN)?;
	let gipa: vk::PFN_vkGetInstanceProcAddr = b.sym("vkGetInstanceProcAddr")?;
	// SAFETY: the interposer's vkGetInstanceProcAddr has the Vulkan signature and returns the loader's functions where it does
	// not intercept; the library stays loaded for the run (sl.rs BOOT).
	let entry = unsafe { ash::Entry::from_static_fn(ash::StaticFn { get_instance_proc_addr: gipa }) };
	let version = match unsafe { entry.try_enumerate_instance_version() } {
		Ok(Some(v)) => v,
		Ok(None) => vk::API_VERSION_1_0,
		Err(e) => return Err(format!("vkEnumerateInstanceVersion {e:?}")),
	};
	if version < vk::API_VERSION_1_1 {
		return Err(format!("Vulkan {version:#x} < 1.1"));
	}
	let mut exts = hal::vulkan::Instance::desired_extensions(&entry, version, desc.flags).map_err(|e| format!("extensions {e:?}"))?;
	let avail = unsafe { entry.enumerate_instance_extension_properties(None) }.unwrap_or_default();
	for e in extra {
		if avail.iter().any(|a| a.extension_name_as_c_str().ok() == Some(*e)) && !exts.contains(e) {
			exts.push(e);
		}
	}
	let layers = unsafe { entry.enumerate_instance_layer_properties() }.unwrap_or_default();
	let has_nv_optimus = layers.iter().any(|l| l.layer_name_as_c_str() == Ok(c"VK_LAYER_NV_optimus"));
	// Like wgpu-hal's init_with_callback with wgpu's default flags (no validation layer, no debug messenger).
	let app = vk::ApplicationInfo::default().application_name(c"wgpu").application_version(1).engine_name(c"wgpu-hal").engine_version(2).api_version(vk::API_VERSION_1_3);
	let ptrs: Vec<*const std::ffi::c_char> = exts.iter().map(|e| e.as_ptr()).collect();
	let mut flags = vk::InstanceCreateFlags::empty();
	if exts.contains(&ash::khr::portability_enumeration::NAME) {
		flags |= vk::InstanceCreateFlags::ENUMERATE_PORTABILITY_KHR;
	}
	let ci = vk::InstanceCreateInfo::default().flags(flags).application_info(&app).enabled_extension_names(&ptrs);
	let raw = unsafe { entry.create_instance(&ci, None) }.map_err(|e| format!("vkCreateInstance (Streamline) {e:?}"))?;
	let i = unsafe { hal::vulkan::Instance::from_raw(entry, raw, version, 0, None, exts, desc.flags, desc.memory_budget_thresholds, has_nv_optimus, None) }.map_err(|e| format!("wgpu-hal instance {e:?}"))?;
	Ok(unsafe { wgpu::Instance::from_hal::<hal::api::Vulkan>(i) })
}

/// The swapchain entry points of `device` through the interposer's vkGetDeviceProcAddr (Streamline's proxies).
pub(super) fn swap_fns(device: vk::Device) -> Result<super::swap_vk::SwapFns, String> {
	let b = super::sl::boot(abi::API_VULKAN)?;
	let gdpa: vk::PFN_vkGetDeviceProcAddr = b.sym("vkGetDeviceProcAddr")?;
	let get = |n: &CStr| -> Result<*const c_void, String> {
		// SAFETY: a valid device and a NUL-terminated name.
		let p = unsafe { gdpa(device, n.as_ptr()) };
		p.map(|f| f as *const c_void).ok_or_else(|| format!("{} not found", n.to_string_lossy()))
	};
	// SAFETY: each pointer is the named Vulkan function (or Streamline's proxy of it) with that function's signature.
	unsafe {
		Ok(super::swap_vk::SwapFns {
			create: std::mem::transmute::<*const c_void, vk::PFN_vkCreateSwapchainKHR>(get(c"vkCreateSwapchainKHR")?),
			destroy: std::mem::transmute::<*const c_void, vk::PFN_vkDestroySwapchainKHR>(get(c"vkDestroySwapchainKHR")?),
			images: std::mem::transmute::<*const c_void, vk::PFN_vkGetSwapchainImagesKHR>(get(c"vkGetSwapchainImagesKHR")?),
			acquire: std::mem::transmute::<*const c_void, vk::PFN_vkAcquireNextImageKHR>(get(c"vkAcquireNextImageKHR")?),
			present: std::mem::transmute::<*const c_void, vk::PFN_vkQueuePresentKHR>(get(c"vkQueuePresentKHR")?),
		})
	}
}

/// The VkFormat of a texture format Streamline gets (pure; None: not one of ours).
pub(super) fn vk_format(f: wgpu::TextureFormat) -> Option<u32> {
	use wgpu::TextureFormat as F;
	let v = match f {
		F::Rgba8Unorm => vk::Format::R8G8B8A8_UNORM,
		F::Rgba8UnormSrgb => vk::Format::R8G8B8A8_SRGB,
		F::Bgra8Unorm => vk::Format::B8G8R8A8_UNORM,
		F::Bgra8UnormSrgb => vk::Format::B8G8R8A8_SRGB,
		F::Rgb10a2Unorm => vk::Format::A2B10G10R10_UNORM_PACK32,
		F::Rg11b10Ufloat => vk::Format::B10G11R11_UFLOAT_PACK32,
		F::R16Float => vk::Format::R16_SFLOAT,
		F::Rg16Float => vk::Format::R16G16_SFLOAT,
		F::Rgba16Float => vk::Format::R16G16B16A16_SFLOAT,
		F::R32Float => vk::Format::R32_SFLOAT,
		F::Rg32Float => vk::Format::R32G32_SFLOAT,
		F::Rgba32Float => vk::Format::R32G32B32A32_SFLOAT,
		F::Depth32Float => vk::Format::D32_SFLOAT,
		F::Depth24PlusStencil8 => vk::Format::D24_UNORM_S8_UINT,
		F::Depth32FloatStencil8 => vk::Format::D32_SFLOAT_S8_UINT,
		_ => return None,
	};
	Some(v.as_raw() as u32)
}

/// VkImageUsageFlags of a texture's wgpu usages (pure; wgpu-hal's mapping).
pub(super) fn vk_usage(u: wgpu::TextureUsages, depth: bool) -> u32 {
	use wgpu::TextureUsages as U;
	let mut f = vk::ImageUsageFlags::empty();
	if u.contains(U::COPY_SRC) {
		f |= vk::ImageUsageFlags::TRANSFER_SRC;
	}
	if u.contains(U::COPY_DST) {
		f |= vk::ImageUsageFlags::TRANSFER_DST;
	}
	if u.contains(U::TEXTURE_BINDING) {
		f |= vk::ImageUsageFlags::SAMPLED;
	}
	if u.contains(U::STORAGE_BINDING) {
		f |= vk::ImageUsageFlags::STORAGE;
	}
	if u.contains(U::RENDER_ATTACHMENT) {
		f |= if depth { vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT } else { vk::ImageUsageFlags::COLOR_ATTACHMENT };
	}
	f.as_raw()
}

/// The layout wgpu-hal leaves a texture in after a transition to RESOURCE (pure; conv.rs derive_image_layout).
pub(super) fn vk_layout(depth: bool) -> u32 {
	(if depth { vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL } else { vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL }).as_raw() as u32
}

/// The Streamline resource of a Vulkan texture and its view (None: not Vulkan or a format we do not map).
pub(super) fn resource(t: &wgpu::Texture, view: &wgpu::TextureView) -> Option<abi::Resource> {
	let img = unsafe { t.as_hal::<wgpu::hal::api::Vulkan>() }.map(|h| unsafe { h.raw_handle() }.as_raw())?;
	let vw = unsafe { view.as_hal::<wgpu::hal::api::Vulkan>() }.map(|h| unsafe { h.raw_handle() }.as_raw())?;
	let depth = t.format().is_depth_stencil_format();
	let mut r = abi::Resource::tex2d(img as usize as *mut c_void, vk_layout(depth));
	r.view = vw as usize as *mut c_void;
	r.width = t.width();
	r.height = t.height();
	r.native_format = vk_format(t.format())?;
	r.mip_levels = t.mip_level_count();
	r.array_layers = t.depth_or_array_layers();
	r.usage = vk_usage(t.usage(), depth);
	Some(r)
}
