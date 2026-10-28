//! The native view handles ReShade takes (`resource_view` = one u64): on D3D12 views made by ReShade's own device (see `dx12`),
//! on Vulkan the VkImageView handles of the wgpu views (which must stay alive as long as the handles are used).

use super::abi::{self, Device, Runtime, ViewDesc};
use super::targets::Targets;
use std::any::Any;
use std::ffi::c_void;
use std::sync::atomic::Ordering::Acquire;

/// The three handles of the add-on (render target plain / sRGB view of `rt`, shader resource view of the depth texture) and
/// whatever keeps them valid. Cloned into the submit hook as an `Arc`.
pub(super) struct Native {
	pub(super) rtv: u64,
	pub(super) rtv_srgb: u64,
	pub(super) depth_srv: u64,
	/// The views ReShade's device made for us (D3D12), destroyed with this object.
	owned: Option<Owned>,
	_keep: Box<dyn Any + Send + Sync>,
}

/// ReShade's device the views were made with: they are destroyed only while a live runtime reports this same device, because the
/// device and its heaps may be gone otherwise. The device outlives a runtime reset (a swapchain resize destroys and re-creates the
/// runtime), so the views of an earlier runtime are still destroyed through the same device.
pub(super) struct Under {
	pub(super) device: Device,
}

struct Owned {
	under: Under,
	views: [u64; 3],
}

// SAFETY: the raw device pointer is ReShade's device, whose view functions take their own lock.
unsafe impl Send for Owned {}
unsafe impl Sync for Owned {}

impl Drop for Native {
	fn drop(&mut self) {
		let Some(o) = &self.owned else { return };
		let live = super::LINK.runtime.load(Acquire);
		if live != 0 && unsafe { Runtime(live as *mut c_void).device() }.0 == o.under.device.0 {
			for v in o.views {
				unsafe { o.under.device.destroy_view(v) };
			}
		}
	}
}

/// Builds the handles for `backend`; Err says why this PC stays without the feature (logged once by the caller). `under` is
/// ReShade's device (live runs): D3D12 needs it.
pub(super) fn create(backend: wgpu::Backend, t: &Targets, format: wgpu::TextureFormat, under: Option<Under>) -> Result<Native, String> {
	match backend {
		#[cfg(windows)]
		wgpu::Backend::Dx12 => dx12(t, format, under.ok_or("no ReShade device to make the views with")?),
		#[cfg(not(target_vendor = "apple"))]
		wgpu::Backend::Vulkan => vulkan(t),
		other => {
			let _ = (t, format, under);
			Err(format!("graphics API {other:?} not supported"))
		}
	}
}

#[cfg(not(target_vendor = "apple"))]
fn vulkan(t: &Targets) -> Result<Native, String> {
	let handle = |v: &wgpu::TextureView| -> Result<u64, String> {
		let hal = unsafe { v.as_hal::<wgpu::hal::api::Vulkan>() }.ok_or("a texture view without a Vulkan image view")?;
		let raw = unsafe { hal.raw_handle() };
		Ok(unsafe { std::mem::transmute_copy::<_, u64>(&raw) })
	};
	let keep = (t.rt_view.clone(), t.rt_srgb.clone(), t.depth1_view.clone());
	Ok(Native { rtv: handle(&t.rt_view)?, rtv_srgb: handle(&t.rt_srgb)?, depth_srv: handle(&t.depth1_view)?, owned: None, _keep: Box::new(keep) })
}

/// D3D12: the views are made by ReShade's device (`create_resource_view`), not by us in descriptor heaps of our own. In the
/// full add-on build ReShade wraps every CBV_SRV_UAV / sampler heap the game creates and hands out VIRTUAL CPU handles for
/// them, while the `resource_view` values it works with (the `update_texture_bindings` copy included) are the real handles
/// behind them: a shader resource view made with a game-created heap and passed as `DEPTH` made the driver crash at the first
/// present. Views made by ReShade itself are valid in every build.
#[cfg(windows)]
fn dx12(t: &Targets, format: wgpu::TextureFormat, under: Under) -> Result<Native, String> {
	use windows::core::Interface;
	let (plain, srgb) = match format {
		wgpu::TextureFormat::Bgra8Unorm => (abi::FMT_B8G8R8A8_UNORM, abi::FMT_B8G8R8A8_UNORM_SRGB),
		wgpu::TextureFormat::Rgba8Unorm => (abi::FMT_R8G8B8A8_UNORM, abi::FMT_R8G8B8A8_UNORM_SRGB),
		other => return Err(format!("colour format {other:?} not supported")),
	};
	let (rt, depth) = unsafe {
		let hal_rt = t.rt.as_hal::<wgpu::hal::api::Dx12>().ok_or("no D3D12 resource for the colour target")?;
		let hal_depth = t.depth1.as_hal::<wgpu::hal::api::Dx12>().ok_or("no D3D12 resource for the depth texture")?;
		(hal_rt.raw_resource().as_raw() as usize as u64, hal_depth.raw_resource().as_raw() as usize as u64)
	};
	let mut views = Vec::with_capacity(3);
	for (resource, usage, fmt) in [(rt, abi::USAGE_RENDER_TARGET, plain), (rt, abi::USAGE_RENDER_TARGET, srgb), (depth, abi::USAGE_SHADER_RESOURCE, abi::FMT_R32_FLOAT)] {
		match unsafe { under.device.create_view(resource, usage, &ViewDesc::texture_2d(fmt)) } {
			Some(v) => views.push(v),
			None => {
				for v in views {
					unsafe { under.device.destroy_view(v) };
				}
				return Err("ReShade's device refused to create a view".into());
			}
		}
	}
	let views = [views[0], views[1], views[2]];
	let keep = (t.rt.clone(), t.depth1.clone());
	Ok(Native { rtv: views[0], rtv_srgb: views[1], depth_srv: views[2], owned: Some(Owned { under, views }), _keep: Box::new(keep) })
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::ffi::c_void;

	/// The views are destroyed through the device they were made with while a live runtime reports that device, also after a runtime
	/// reset (swapchain resize: new runtime and epoch, same device); never for another device or with no runtime.
	#[test]
	fn views_are_destroyed_while_a_live_runtime_reports_their_device() {
		static DESTROYED: std::sync::Mutex<Vec<u64>> = std::sync::Mutex::new(Vec::new());
		unsafe extern "system" fn destroy(_: *mut c_void, view: u64) {
			DESTROYED.lock().unwrap().push(view);
		}
		let table = [0usize, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, destroy as *const () as usize];
		let dev = Box::new([table.as_ptr() as usize]);
		let dev_ptr = dev.as_ptr() as usize;
		let other = Box::new([table.as_ptr() as usize]);
		// A fake effect_runtime whose get_device (slot RT_GET_DEVICE) returns the first word after its vtable pointer.
		unsafe extern "system" fn get_device(this: *mut c_void) -> *mut c_void {
			unsafe { *(this as *const usize).add(1) as *mut c_void }
		}
		let rt_table = [0usize, 0, 0, get_device as *const () as usize];
		let runtime_on = |device: usize| Box::new([rt_table.as_ptr() as usize, device]);
		let mk = || Native {
			rtv: 1,
			rtv_srgb: 2,
			depth_srv: 3,
			owned: Some(Owned { under: Under { device: Device(dev_ptr as *mut c_void) }, views: [1, 2, 3] }),
			_keep: Box::new(()),
		};
		let link = &super::super::LINK;
		let first = runtime_on(dev_ptr);
		link.runtime.store(first.as_ptr() as usize, std::sync::atomic::Ordering::Release);
		drop(mk());
		assert_eq!(*DESTROYED.lock().unwrap(), [1, 2, 3]);
		// A runtime reset: another runtime object and epoch on the same device still destroys the old views (no leak per resize).
		let second = runtime_on(dev_ptr);
		link.runtime.store(second.as_ptr() as usize, std::sync::atomic::Ordering::Release);
		link.epoch.fetch_add(2, std::sync::atomic::Ordering::Release);
		drop(mk());
		assert_eq!(DESTROYED.lock().unwrap().len(), 6);
		let foreign = runtime_on(other.as_ptr() as usize);
		link.runtime.store(foreign.as_ptr() as usize, std::sync::atomic::Ordering::Release);
		drop(mk());
		link.runtime.store(0, std::sync::atomic::Ordering::Release);
		drop(mk());
		assert_eq!(DESTROYED.lock().unwrap().len(), 6, "a view of another device, or with no runtime, is left alone");
	}
}
