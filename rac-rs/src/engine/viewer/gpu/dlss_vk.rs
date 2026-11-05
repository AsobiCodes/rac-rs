//! [PC] The window's instance and device for DLSS (lane dlss2; upscale/dlss.rs). NGX needs extensions on the VkInstance and the
//! VkDevice that wgpu does not enable itself, so with `rac_ngx.dll` + `nvngx_dlss.dll` next to the exe (`dlss::lib`):
//! - `instance`: the Vulkan-only instance (the first try of renderer.rs `open`, never wgpu's fallback set) is made through
//!   wgpu-hal's `init_with_callback` with NGX's instance extensions added where the loader has them (wgpu usually has them all:
//!   then the instance is the one `Instance::new` makes), same name, flags and options as wgpu's own path; only while the flags
//!   are wgpu's default (`hal_keeps_flags`: wgpu-core takes the default set on this path, so under Wine, GitHub #49, or with a
//!   WGPU_* flag override, wgpu's own instance is made and DLSS stays off on Vulkan).
//! - `request_device`: on an NVIDIA Vulkan adapter whose NGX requirements say "supported" and that has every device extension NGX
//!   names, the device is opened through `open_with_callback` with them (+ the buffer-device-address feature) and wrapped by
//!   `create_device_from_hal` with the same descriptor. DX12 needs no device change: an NVIDIA adapter's requirements are asked.
//! Every other case (no DLLs: every public install; AMD / Intel / Steam Deck; a refusal) takes the plain wgpu calls, so device
//! creation there is byte-identical to a build without DLSS. The outcome goes to `dlss::device_made` (the slot's DLSS value).

use crate::viewer::upscale::dlss;
use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};

/// The window's instance was made through `instance` (its devices may get NGX's extensions).
static NGX_INSTANCE: AtomicBool = AtomicBool::new(false);

fn leak(s: &str) -> Option<&'static CStr> {
	Some(Box::leak(CString::new(s).ok()?.into_boxed_c_str()))
}

/// A '\n'-joined name list of the helper: asked for its size, then filled (None: the call failed).
pub(super) fn ext_list(f: impl Fn(*mut c_char, u32, *mut u32) -> u32) -> Option<Vec<String>> {
	let mut len = 0u32;
	if rac_ngx_abi::failed(f(std::ptr::null_mut(), 0, &mut len)) {
		return None;
	}
	let mut buf = vec![0u8; len.max(1) as usize];
	if rac_ngx_abi::failed(f(buf.as_mut_ptr() as *mut c_char, buf.len() as u32, &mut len)) {
		return None;
	}
	Some(split_names(&buf))
}

/// The names of a NUL-terminated '\n'-joined list (empty names dropped).
pub(super) fn split_names(buf: &[u8]) -> Vec<String> {
	let s = CStr::from_bytes_until_nul(buf).map(|c| c.to_string_lossy().into_owned()).unwrap_or_default();
	s.split('\n').filter(|e| !e.is_empty()).map(str::to_string).collect()
}

/// The device extensions to add: NGX's list without VK_EXT_buffer_device_address when the KHR one is in it (both together are
/// invalid), and whether a buffer-device-address extension (KHR, else EXT) is wanted.
pub(super) fn device_ext_plan(exts: Vec<String>) -> (Vec<String>, Option<bool>) {
	let khr = exts.iter().any(|e| e == "VK_KHR_buffer_device_address");
	let exts: Vec<String> = exts.into_iter().filter(|e| !(khr && e == "VK_EXT_buffer_device_address")).collect();
	let ext = exts.iter().any(|e| e == "VK_EXT_buffer_device_address");
	let bda = if khr { Some(true) } else if ext { Some(false) } else { None };
	(exts, bda)
}

/// Whether an instance made through wgpu-hal keeps `flags`: `wgpu::Instance::from_hal` gives wgpu-core the default flags, not the
/// hal instance's (wgpu-core uses its own copy for the adapter filter, indirect validation and every device), so any other set
/// would be lost there: the Wine flag that keeps the Vulkan adapter visible (wine.rs, GitHub #49) or a WGPU_* override (review
/// R21 M1). Those runs take wgpu's own `Instance::new` (no DLSS on Vulkan; DX12 still asks NGX).
#[cfg_attr(target_vendor = "apple", allow(dead_code))]
pub(super) fn hal_keeps_flags(flags: wgpu::InstanceFlags) -> bool {
	flags == wgpu::InstanceFlags::default()
}

/// The Vulkan-only instance with NGX's instance extensions, when the DLSS DLLs are next to the exe, `desc` is the Vulkan-only
/// set and its flags survive the hal path (`hal_keeps_flags`); None: wgpu's own `Instance::new` instead.
#[cfg(not(target_vendor = "apple"))]
pub(super) fn instance(desc: &wgpu::InstanceDescriptor) -> Option<wgpu::Instance> {
	use wgpu::hal;
	if desc.backends != wgpu::Backends::VULKAN || desc.display.is_some() {
		return None;
	}
	let lib = dlss::lib()?;
	if !hal_keeps_flags(desc.flags) {
		eprintln!("gpu: DLSS: instance flags {:?} differ from wgpu's default (Wine or WGPU_* set): wgpu's own Vulkan instance, no DLSS on Vulkan", desc.flags);
		return None;
	}
	let wanted = ext_list(|b, c, l| unsafe { (lib.api.rac_ngx_vk_instance_extensions)(&lib.app, b, c, l) })?;
	let wanted: Vec<&'static CStr> = wanted.iter().filter_map(|e| leak(e)).collect();
	let cb: Box<hal::vulkan::CreateInstanceCallback<'static>> = Box::new(move |args: hal::vulkan::CreateInstanceCallbackArgs<'_, '_, 'static>| {
		let avail = unsafe { args.entry.enumerate_instance_extension_properties(None) }.unwrap_or_default();
		for e in &wanted {
			if avail.iter().any(|a| a.extension_name_as_c_str().ok() == Some(*e)) && !args.extensions.contains(e) {
				eprintln!("gpu: DLSS: instance extension {} added", e.to_string_lossy());
				args.extensions.push(e);
			}
		}
	});
	let hd = hal::InstanceDescriptor { name: "wgpu", flags: desc.flags, memory_budget_thresholds: desc.memory_budget_thresholds, backend_options: desc.backend_options.clone(), telemetry: None, display: None };
	match unsafe { hal::vulkan::Instance::init_with_callback(&hd, Some(cb)) } {
		Ok(i) => {
			NGX_INSTANCE.store(true, Ordering::Relaxed);
			Some(unsafe { wgpu::Instance::from_hal::<hal::api::Vulkan>(i) })
		}
		Err(e) => {
			eprintln!("gpu: DLSS: Vulkan instance with NGX's extensions failed ({e:?}): wgpu's own");
			None
		}
	}
}

#[cfg(target_vendor = "apple")]
pub(super) fn instance(_: &wgpu::InstanceDescriptor) -> Option<wgpu::Instance> {
	None
}

/// The window's device: through NGX's extensions on a Vulkan adapter that runs DLSS, else `request_device` as always.
pub(super) fn request_device(adapter: &wgpu::Adapter, desc: &wgpu::DeviceDescriptor<'_>) -> Result<(wgpu::Device, wgpu::Queue), wgpu::RequestDeviceError> {
	let info = adapter.get_info();
	let nvidia = info.vendor == dlss::NVIDIA;
	#[cfg(not(target_vendor = "apple"))]
	if nvidia && info.backend == wgpu::Backend::Vulkan && NGX_INSTANCE.load(Ordering::Relaxed) {
		if let Some(dq) = vk_device(adapter, desc) {
			dlss::device_made(&dq.0, true);
			return Ok(dq);
		}
	}
	let dq = pollster::block_on(adapter.request_device(desc))?;
	let ready = nvidia && info.backend == wgpu::Backend::Dx12 && dx12_ready(adapter);
	dlss::device_made(&dq.0, ready);
	Ok(dq)
}

/// NGX's requirements on a DX12 adapter (true: DLSS runs there).
#[cfg(windows)]
fn dx12_ready(adapter: &wgpu::Adapter) -> bool {
	use windows::core::Interface;
	let Some(lib) = dlss::lib() else { return false };
	let Some(a) = (unsafe { adapter.as_hal::<wgpu::hal::api::Dx12>() }) else { return false };
	let raw = a.as_raw().as_raw(); // IDXGIAdapter3 derives from IDXGIAdapter: the same pointer
	let (mut bits, mut arch) = (0u32, 0u32);
	let r = unsafe { (lib.api.rac_ngx_dx12_requirements)(&lib.app, raw, &mut bits, &mut arch) };
	let ok = !rac_ngx_abi::failed(r) && bits == rac_ngx_abi::SUPPORT_OK;
	eprintln!("gpu: DLSS on DirectX 12: requirements {} (bits {bits:#x}){}", lib.name(r), if ok { "" } else { ": not available" });
	ok
}

#[cfg(not(windows))]
fn dx12_ready(_: &wgpu::Adapter) -> bool {
	false
}

/// NGX's device extensions of a Vulkan adapter when its requirements say "supported" and it has all of them (None: no DLSS there).
#[cfg(not(target_vendor = "apple"))]
fn vk_adapter_exts(lib: &dlss::Lib, adapter: &wgpu::Adapter) -> Option<Vec<String>> {
	let a = unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }?;
	let raw_inst = a.shared_instance().raw_instance();
	let inst = unsafe { std::mem::transmute_copy::<_, u64>(&raw_inst.handle()) } as usize as *mut c_void;
	let pd_h = a.raw_physical_device();
	let pd = unsafe { std::mem::transmute_copy::<_, u64>(&pd_h) } as usize as *mut c_void;
	let (mut bits, mut arch) = (0u32, 0u32);
	let r = unsafe { (lib.api.rac_ngx_vk_requirements)(&lib.app, inst, pd, &mut bits, &mut arch) };
	if rac_ngx_abi::failed(r) || bits != rac_ngx_abi::SUPPORT_OK {
		eprintln!("gpu: DLSS on Vulkan: requirements {} (bits {bits:#x}): not available", lib.name(r));
		return None;
	}
	let exts = ext_list(|b, c, l| unsafe { (lib.api.rac_ngx_vk_device_extensions)(&lib.app, inst, pd, b, c, l) })?;
	let avail = unsafe { raw_inst.enumerate_device_extension_properties(pd_h) }.unwrap_or_default();
	let missing: Vec<&String> = exts.iter().filter(|e| !avail.iter().any(|x| x.extension_name_as_c_str().is_ok_and(|c| c.to_bytes() == e.as_bytes()))).collect();
	if !missing.is_empty() {
		eprintln!("gpu: DLSS on Vulkan: the device lacks {missing:?}: not available");
		return None;
	}
	Some(exts)
}

/// The device opened with NGX's extensions (None: not available or the open failed: the caller makes the plain one).
#[cfg(not(target_vendor = "apple"))]
fn vk_device(adapter: &wgpu::Adapter, desc: &wgpu::DeviceDescriptor<'_>) -> Option<(wgpu::Device, wgpu::Queue)> {
	use ash::vk;
	use wgpu::hal;
	let lib = dlss::lib()?;
	let (exts, bda) = device_ext_plan(vk_adapter_exts(lib, adapter)?);
	let dev_exts: Vec<&'static CStr> = exts.iter().filter_map(|e| leak(e)).collect();
	let cb: Box<hal::vulkan::CreateDeviceCallback<'static>> = Box::new(move |args: hal::vulkan::CreateDeviceCallbackArgs<'_, '_, 'static>| {
		// wgpu adds its own buffer-device-address feature struct only when it asked for the extension itself.
		let wgpu_bda = args.extensions.iter().any(|e| e.to_bytes().ends_with(b"_buffer_device_address"));
		for e in dev_exts {
			if !args.extensions.contains(&e) {
				args.extensions.push(e);
			}
		}
		match bda.filter(|_| !wgpu_bda) {
			Some(true) => {
				let f: &'static mut vk::PhysicalDeviceBufferDeviceAddressFeatures<'static> = Box::leak(Box::new(vk::PhysicalDeviceBufferDeviceAddressFeatures::default().buffer_device_address(true)));
				*args.create_info = args.create_info.push_next(f);
			}
			Some(false) => {
				let f: &'static mut vk::PhysicalDeviceBufferDeviceAddressFeaturesEXT<'static> = Box::leak(Box::new(vk::PhysicalDeviceBufferDeviceAddressFeaturesEXT::default().buffer_device_address(true)));
				*args.create_info = args.create_info.push_next(f);
			}
			None => {}
		}
	});
	let open = {
		let a = unsafe { adapter.as_hal::<hal::api::Vulkan>() }?;
		match unsafe { a.open_with_callback(desc.required_features, &desc.required_limits, &desc.memory_hints, Some(cb)) } {
			Ok(o) => o,
			Err(e) => {
				eprintln!("gpu: DLSS: the Vulkan device with NGX's extensions failed ({e:?}): the plain device, no DLSS");
				return None;
			}
		}
	};
	match unsafe { adapter.create_device_from_hal(open, desc) } {
		Ok(dq) => {
			eprintln!("gpu: DLSS: Vulkan device with NGX's extensions {exts:?}");
			Some(dq)
		}
		Err(e) => {
			eprintln!("gpu: DLSS: create_device_from_hal failed ({e}): the plain device, no DLSS");
			None
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn names_split_and_bda_is_kept_once() {
		assert_eq!(split_names(b"VK_A\nVK_B\n\0junk"), vec!["VK_A", "VK_B"]);
		assert!(split_names(b"\0").is_empty());
		let (e, bda) = device_ext_plan(vec!["VK_NVX_binary_import".into(), "VK_KHR_buffer_device_address".into(), "VK_EXT_buffer_device_address".into()]);
		assert_eq!((e.len(), bda), (2, Some(true)), "KHR wins, EXT dropped");
		let (e, bda) = device_ext_plan(vec!["VK_EXT_buffer_device_address".into()]);
		assert_eq!((e.len(), bda), (1, Some(false)));
		assert_eq!(device_ext_plan(vec!["VK_KHR_push_descriptor".into()]).1, None);
	}

	#[test]
	fn hal_instance_only_with_wgpus_default_flags() {
		let def = wgpu::InstanceFlags::default();
		assert!(hal_keeps_flags(def));
		assert!(!hal_keeps_flags(def | crate::wine::instance_flags_for(true)), "the Wine flag (GitHub #49) needs wgpu's own instance");
		assert!(hal_keeps_flags(def | crate::wine::instance_flags_for(false)), "plain Windows: the default set");
		assert!(!hal_keeps_flags(def | wgpu::InstanceFlags::GPU_BASED_VALIDATION));
		assert!(!hal_keeps_flags(def.difference(wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL)), "a flag taken away is lost too");
	}
}
