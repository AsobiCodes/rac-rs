//! [PC] The C ABI of AMD's FidelityFX API (ffx_api, FSR SDK 2.4.0: upscaler 4.1.2 / 3.1.x), the few structs and constants the
//! upscaler slot uses (lane ndlss stage 3). Our own Rust declarations written from the SDK's MIT-licensed headers
//! (Kits/FidelityFX/api/include/ffx_api.h, ffx_api_types.h, dx12/ffx_api_dx12.h, vk/ffx_api_vk.h and
//! Kits/FidelityFX/upscalers/include/ffx_upscale.h; Copyright (c) Advanced Micro Devices, Inc., MIT licence). No AMD code is
//! linked: the loader DLL is opened at run time (ffx.rs). Every struct starts with `Header` (type id + chain pointer); the
//! layouts are pinned by unit tests (ffx_tests.rs) for 64-bit targets.
#![allow(non_snake_case, dead_code)]

use std::ffi::{c_char, c_void};

pub type Context = *mut c_void;
pub type ReturnCode = u32;

/// `ffxApiHeader`: `type` = a `*_DESC_TYPE_*` id, `pNext` = the next struct of the chain (backend desc, version override).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Header {
	pub ty: u64,
	pub next: *mut Header,
}

impl Header {
	pub fn of(ty: u64) -> Header {
		Header { ty, next: std::ptr::null_mut() }
	}
}

pub type Message = Option<unsafe extern "C" fn(ty: u32, msg: *const u16)>;
pub type PfnCreateContext = unsafe extern "C" fn(ctx: *mut Context, desc: *mut Header, mem: *const c_void) -> ReturnCode;
pub type PfnDestroyContext = unsafe extern "C" fn(ctx: *mut Context, mem: *const c_void) -> ReturnCode;
pub type PfnConfigure = unsafe extern "C" fn(ctx: *mut Context, desc: *const Header) -> ReturnCode;
pub type PfnQuery = unsafe extern "C" fn(ctx: *mut Context, desc: *mut Header) -> ReturnCode;
pub type PfnDispatch = unsafe extern "C" fn(ctx: *mut Context, desc: *const Header) -> ReturnCode;

// ffx_api.h
pub const RETURN_OK: ReturnCode = 0;
pub const RETURN_NO_PROVIDER: ReturnCode = 4;
pub const QUERY_GET_VERSIONS: u64 = 4;
pub const DESC_OVERRIDE_VERSION: u64 = 5;
pub const QUERY_GET_PROVIDER_VERSION: u64 = 6;
pub const EFFECT_ID_UPSCALE: u64 = 0x0001_0000;
pub const BACKEND_ID_VK: u64 = 0x0100_0000;
// ffx_api_dx12.h / ffx_api_vk.h: FFX_API_MAKE_BACKEND_SUB_ID(backend, 0x02 / 0x03)
pub const CREATE_BACKEND_DX12: u64 = 0x0000_0002;
pub const CREATE_BACKEND_VK: u64 = BACKEND_ID_VK | 0x03;
// ffx_upscale.h: FFX_API_MAKE_EFFECT_SUB_ID(FFX_API_EFFECT_ID_UPSCALE, n)
pub const CREATE_UPSCALE: u64 = EFFECT_ID_UPSCALE;
pub const DISPATCH_UPSCALE: u64 = EFFECT_ID_UPSCALE | 0x01;
pub const QUERY_RENDER_SIZE_FROM_QUALITY: u64 = EFFECT_ID_UPSCALE | 0x03;
pub const QUERY_JITTER_PHASE_COUNT: u64 = EFFECT_ID_UPSCALE | 0x04;
pub const QUERY_JITTER_OFFSET: u64 = EFFECT_ID_UPSCALE | 0x05;
pub const DISPATCH_REACTIVE_MASK: u64 = EFFECT_ID_UPSCALE | 0x06;
pub const CREATE_UPSCALE_VERSION: u64 = EFFECT_ID_UPSCALE | 0x0b;
/// `FFX_UPSCALER_VERSION` of the headers these declarations follow: FFX_UPSCALER_MAKE_VERSION(4, 1, 2) = major << 22 |
/// minor << 12 | patch (`ffxCreateContextDescUpscaleVersion.version`).
pub const UPSCALER_VERSION: u32 = (4 << 22) | (1 << 12) | 2;

// FfxApiCreateContextUpscaleFlags / FfxApiDispatchFsrUpscaleFlags
pub const CREATE_ENABLE_HDR: u32 = 1 << 0;
pub const CREATE_ENABLE_DEPTH_INVERTED: u32 = 1 << 3;
pub const CREATE_ENABLE_AUTO_EXPOSURE: u32 = 1 << 5;
pub const CREATE_ENABLE_DEBUG_CHECKING: u32 = 1 << 7;
pub const CREATE_ENABLE_NON_LINEAR_COLORSPACE: u32 = 1 << 8;
pub const DISPATCH_NON_LINEAR_COLOR_SRGB: u32 = 1 << 1;

// ffx_api_types.h
pub const RESOURCE_TYPE_TEXTURE2D: u32 = 2;
pub const USAGE_READ_ONLY: u32 = 0;
pub const USAGE_RENDERTARGET: u32 = 1 << 0;
pub const USAGE_UAV: u32 = 1 << 1;
pub const USAGE_DEPTHTARGET: u32 = 1 << 2;
pub const STATE_UNORDERED_ACCESS: u32 = 1 << 1;
pub const STATE_COMPUTE_READ: u32 = 1 << 2;
pub const STATE_PIXEL_COMPUTE_READ: u32 = (1 << 3) | (1 << 2);
pub const STATE_RENDER_TARGET: u32 = 1 << 8;
pub const FORMAT_UNKNOWN: u32 = 0;
pub const FORMAT_R16G16B16A16_FLOAT: u32 = 4;
pub const FORMAT_R8G8B8A8_UNORM: u32 = 10;
pub const FORMAT_R8G8B8A8_SRGB: u32 = 12;
pub const FORMAT_B8G8R8A8_UNORM: u32 = 14;
pub const FORMAT_B8G8R8A8_SRGB: u32 = 15;
pub const FORMAT_R16G16_FLOAT: u32 = 18;
pub const FORMAT_R8_UNORM: u32 = 25;
pub const FORMAT_R32_FLOAT: u32 = 28;

/// `FfxApiDimensions2D` / `FfxApiFloatCoords2D`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Dim {
	pub width: u32,
	pub height: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Coords {
	pub x: f32,
	pub y: f32,
}

/// `FfxApiResourceDescription` (the unions width/size, height/stride, depth/alignment as their texture names).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ResourceDesc {
	pub ty: u32,
	pub format: u32,
	pub width: u32,
	pub height: u32,
	pub depth: u32,
	pub mip_count: u32,
	pub flags: u32,
	pub usage: u32,
}

/// `FfxApiResource`: the native handle (VkImage / ID3D12Resource*), its description and the state it is in at the dispatch.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Resource {
	pub resource: *mut c_void,
	pub description: ResourceDesc,
	pub state: u32,
}

impl Resource {
	/// No resource (optional inputs: exposure, reactive, transparency and composition).
	pub fn none() -> Resource {
		Resource { resource: std::ptr::null_mut(), description: ResourceDesc::default(), state: 0 }
	}
}

/// `ffxCreateBackendDX12Desc`.
#[repr(C)]
pub struct BackendDx12 {
	pub header: Header,
	pub device: *mut c_void,
}

/// `VkDeviceContext` (vk/ffx_api_vk.h): the Vulkan device of a backend desc, and what `QueryVersions.device` points at on Vulkan.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VkDeviceContext {
	pub vk_device: *mut c_void,
	pub vk_physical_device: *mut c_void,
	/// PFN_vkGetDeviceProcAddr.
	pub vk_device_proc_addr: *const c_void,
}

/// `ffxCreateBackendVKDesc`.
#[repr(C)]
pub struct BackendVk {
	pub header: Header,
	pub device_context: VkDeviceContext,
}

/// `ffxCreateContextDescUpscaleVersion` (chained after the backend desc, as the SDK's FSR sample does): the upscaler API
/// version the caller was built against, `UPSCALER_VERSION`.
#[repr(C)]
pub struct CreateUpscaleVersion {
	pub header: Header,
	pub version: u32,
}

/// `ffxQueryDescGetVersions` (context may be null): call once for the count, then with the two arrays.
#[repr(C)]
pub struct QueryVersions {
	pub header: Header,
	pub create_desc_type: u64,
	/// The ID3D12Device* on DX12; on Vulkan a POINTER to a `VkDeviceContext` (not the VkDevice itself; ffx_api.h).
	pub device: *mut c_void,
	pub output_count: *mut u64,
	pub version_ids: *mut u64,
	pub version_names: *mut *const c_char,
}

/// `ffxOverrideVersion` (chained after the create desc: pins one version, no automatic choice).
#[repr(C)]
pub struct OverrideVersion {
	pub header: Header,
	pub version_id: u64,
}

/// `ffxQueryGetProviderVersion` (the version a context runs).
#[repr(C)]
pub struct QueryProviderVersion {
	pub header: Header,
	pub version_id: u64,
	pub version_name: *const c_char,
}

/// `ffxCreateContextDescUpscale`.
#[repr(C)]
pub struct CreateUpscale {
	pub header: Header,
	pub flags: u32,
	pub max_render_size: Dim,
	pub max_upscale_size: Dim,
	pub message: Message,
}

/// `ffxDispatchDescUpscale` (motion vectors with `motion_vector_scale` give pixels, current -> previous; `frame_time_delta` ms).
#[repr(C)]
pub struct DispatchUpscale {
	pub header: Header,
	pub command_list: *mut c_void,
	pub color: Resource,
	pub depth: Resource,
	pub motion_vectors: Resource,
	pub exposure: Resource,
	pub reactive: Resource,
	pub transparency_and_composition: Resource,
	pub output: Resource,
	pub jitter_offset: Coords,
	pub motion_vector_scale: Coords,
	pub render_size: Dim,
	pub upscale_size: Dim,
	pub enable_sharpening: bool,
	pub sharpness: f32,
	pub frame_time_delta: f32,
	pub pre_exposure: f32,
	pub reset: bool,
	pub camera_near: f32,
	pub camera_far: f32,
	pub camera_fov_angle_vertical: f32,
	pub view_space_to_meters_factor: f32,
	pub flags: u32,
}

/// `ffxQueryDescUpscaleGetRenderResolutionFromQualityMode`.
#[repr(C)]
pub struct QueryRenderSize {
	pub header: Header,
	pub display_width: u32,
	pub display_height: u32,
	pub quality_mode: u32,
	pub out_width: *mut u32,
	pub out_height: *mut u32,
}

/// `ffxQueryDescUpscaleGetJitterPhaseCount`.
#[repr(C)]
pub struct QueryJitterPhases {
	pub header: Header,
	pub render_width: u32,
	pub display_width: u32,
	pub out_phase_count: *mut i32,
}
