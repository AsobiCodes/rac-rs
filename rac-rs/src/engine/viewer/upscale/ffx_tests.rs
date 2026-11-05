// ffx_abi.rs layouts (64-bit, the C headers' order and padding) and ffx.rs pure helpers; no DLL needed.
use super::super::ffx_abi as abi;
use super::*;
use std::mem::{offset_of, size_of};

#[test]
fn abi_layouts_match_the_c_headers() {
	assert_eq!(size_of::<abi::Header>(), 16);
	assert_eq!(size_of::<abi::ResourceDesc>(), 32);
	assert_eq!(size_of::<abi::Resource>(), 48);
	assert_eq!(offset_of!(abi::Resource, state), 40);
	assert_eq!(size_of::<abi::BackendDx12>(), 24);
	assert_eq!(size_of::<abi::BackendVk>(), 40);
	assert_eq!(offset_of!(abi::BackendVk, device_context), 16);
	assert_eq!(size_of::<abi::VkDeviceContext>(), 24);
	assert_eq!(offset_of!(abi::VkDeviceContext, vk_physical_device), 8);
	assert_eq!(offset_of!(abi::VkDeviceContext, vk_device_proc_addr), 16);
	assert_eq!(offset_of!(abi::CreateUpscaleVersion, version), 16);
	assert_eq!(size_of::<abi::CreateUpscaleVersion>(), 24);
	assert_eq!(offset_of!(abi::QueryVersions, device), 24);
	assert_eq!(size_of::<abi::QueryVersions>(), 56);
	assert_eq!(size_of::<abi::OverrideVersion>(), 24);
	assert_eq!(size_of::<abi::QueryProviderVersion>(), 32);
	assert_eq!(offset_of!(abi::CreateUpscale, flags), 16);
	assert_eq!(offset_of!(abi::CreateUpscale, max_render_size), 20);
	assert_eq!(offset_of!(abi::CreateUpscale, max_upscale_size), 28);
	assert_eq!(offset_of!(abi::CreateUpscale, message), 40);
	assert_eq!(size_of::<abi::CreateUpscale>(), 48);
	assert_eq!(offset_of!(abi::QueryRenderSize, out_width), 32);
	assert_eq!(offset_of!(abi::QueryJitterPhases, out_phase_count), 24);
}

#[test]
fn dispatch_desc_layout_matches_ffx_upscale_h() {
	use abi::DispatchUpscale as D;
	assert_eq!(offset_of!(D, command_list), 16);
	assert_eq!(offset_of!(D, color), 24);
	assert_eq!(offset_of!(D, output), 312);
	assert_eq!(offset_of!(D, jitter_offset), 360);
	assert_eq!(offset_of!(D, motion_vector_scale), 368);
	assert_eq!(offset_of!(D, render_size), 376);
	assert_eq!(offset_of!(D, upscale_size), 384);
	assert_eq!(offset_of!(D, enable_sharpening), 392);
	assert_eq!(offset_of!(D, sharpness), 396);
	assert_eq!(offset_of!(D, reset), 408);
	assert_eq!(offset_of!(D, camera_near), 412);
	assert_eq!(offset_of!(D, view_space_to_meters_factor), 424);
	assert_eq!(offset_of!(D, flags), 428);
	assert_eq!(size_of::<D>(), 432);
}

#[test]
fn desc_type_ids() {
	assert_eq!(abi::CREATE_UPSCALE, 0x0001_0000);
	assert_eq!(abi::DISPATCH_UPSCALE, 0x0001_0001);
	assert_eq!(abi::QUERY_JITTER_OFFSET, 0x0001_0005);
	assert_eq!(abi::CREATE_BACKEND_VK, 0x0100_0003);
	assert_eq!(abi::CREATE_BACKEND_DX12, 0x0000_0002);
	assert_eq!(abi::CREATE_UPSCALE_VERSION, 0x0001_000b);
	assert_eq!(abi::UPSCALER_VERSION, 0x0100_1002); // 4.1.2
}

/// R9-1: on Vulkan the version query's `device` is a pointer to a VkDeviceContext {VkDevice, VkPhysicalDevice, proc addr}
/// (ffx_api.h), not the VkDevice; on DX12 the ID3D12Device*. And the runtime only serves the device it was probed on (R9-5).
#[test]
fn version_query_device_per_backend() {
	let d = DeviceHandles { device: 0x1000 as *mut c_void, physical: 0x2000 as *mut c_void, proc_addr: 0x3000 as *const c_void };
	let mut vk = abi::VkDeviceContext { vk_device: std::ptr::null_mut(), vk_physical_device: std::ptr::null_mut(), vk_device_proc_addr: std::ptr::null() };
	let p = query_device(Backend::Vulkan, &d, &mut vk);
	assert_eq!(p, &mut vk as *mut abi::VkDeviceContext as *mut c_void);
	let c = unsafe { *(p as *const abi::VkDeviceContext) };
	assert_eq!((c.vk_device, c.vk_physical_device, c.vk_device_proc_addr), (d.device, d.physical, d.proc_addr));
	assert_eq!(query_device(Backend::Dx12, &d, &mut vk), d.device);
	let BackendDesc::Vk(b) = BackendDesc::of(Backend::Vulkan, &d) else { panic!("vk desc") };
	assert_eq!((b.device_context.vk_device, b.device_context.vk_device_proc_addr), (d.device, d.proc_addr));
	let other = DeviceHandles { device: 0x4000 as *mut c_void, ..d };
	assert!(same_device((Backend::Vulkan, &d), (Backend::Vulkan, &d)));
	assert!(!same_device((Backend::Vulkan, &d), (Backend::Vulkan, &other)) && !same_device((Backend::Vulkan, &d), (Backend::Dx12, &d)));
}

#[test]
fn formats_and_textures() {
	assert_eq!(surface_format(wgpu::TextureFormat::Rgba8Unorm), 10);
	assert_eq!(surface_format(wgpu::TextureFormat::Bgra8Unorm), 14);
	assert_eq!(surface_format(wgpu::TextureFormat::Rg16Float), 18);
	assert_eq!(surface_format(wgpu::TextureFormat::R32Float), 28);
	assert_eq!(surface_format(wgpu::TextureFormat::R8Unorm), 25);
	assert_eq!(surface_format(wgpu::TextureFormat::Depth24PlusStencil8), abi::FORMAT_UNKNOWN);
	let t = texture(std::ptr::null_mut(), wgpu::TextureFormat::Rg16Float, (853, 480), abi::USAGE_READ_ONLY, abi::STATE_COMPUTE_READ);
	assert_eq!((t.description.ty, t.description.width, t.description.height, t.description.depth, t.description.mip_count), (2, 853, 480, 1, 1));
}

/// Review R16 F1: every format the slot hands to FidelityFX (colour as the swapchain may be, motion, depth, reactive mask, the
/// window-size output) has an FFX format; an unknown one would reach the driver as VK_FORMAT_UNDEFINED.
#[test]
fn every_format_handed_to_ffx_is_known() {
	use wgpu::TextureFormat as F;
	let handed = [F::Bgra8Unorm, F::Rgba8Unorm, F::Bgra8UnormSrgb, F::Rgba8UnormSrgb, super::super::motion::MOTION_FORMAT, super::super::motion::DEPTH_FORMAT, super::super::ffx_reactive::FORMAT, super::super::slot_gpu::MID_FORMAT];
	for f in handed {
		assert_ne!(surface_format(f), abi::FORMAT_UNKNOWN, "{f:?}");
	}
}

#[test]
fn dispatch_desc_scales_uv_vectors_to_render_pixels() {
	let f = Frame { render: (853, 480), win: (1280, 720), jitter: [0.25, -0.125], sharpness: 0.0, frame_ms: 16.7, reset: true, near: 0.03125, far: 728.0, fov_y: 1.0 };
	let n = abi::Resource::none;
	let d = dispatch_desc(std::ptr::null_mut(), n(), n(), n(), n(), &f);
	assert_eq!(d.header.ty, abi::DISPATCH_UPSCALE);
	assert_eq!((d.motion_vector_scale.x, d.motion_vector_scale.y), (853.0, 480.0));
	assert_eq!((d.jitter_offset.x, d.jitter_offset.y), (0.25, -0.125));
	assert_eq!((d.render_size.width, d.upscale_size.height), (853, 720));
	assert!(!d.enable_sharpening && d.reset && d.pre_exposure == 1.0);
	assert_eq!(d.flags, abi::DISPATCH_NON_LINEAR_COLOR_SRGB);
	assert!(d.exposure.resource.is_null() && d.reactive.resource.is_null());
}

#[test]
fn missing_loader_is_an_error_not_a_crash() {
	let dir = std::env::temp_dir().join("rac_ffx_test_empty_dir");
	let _ = std::fs::create_dir_all(&dir);
	for b in [Backend::Vulkan, Backend::Dx12] {
		let e = load(&dir, b).err().expect("no DLL there");
		assert!(e.contains(dll_names(b).0), "{e}");
	}
}
