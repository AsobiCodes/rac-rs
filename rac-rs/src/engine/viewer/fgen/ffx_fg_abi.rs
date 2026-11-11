//! [PC] Frame generation (lane fgen, S2): the C ABI of AMD's FidelityFX frame generation (FSR SDK 2.4.0: frame generation 4.0.2 /
//! 3.1.x and its swapchains). Our own Rust declarations written from the SDK's MIT-licensed headers (Kits/FidelityFX/
//! framegeneration/include/ffx_framegeneration.h, vk/ffx_api_framegeneration_vk.h, dx12/ffx_api_framegeneration_dx12.h,
//! ffx_framegeneration_api_types.h; Copyright (c) Advanced Micro Devices, Inc., MIT licence). The shared pieces (header, resource,
//! dimensions, backend descs) are upscale/ffx_abi.rs's. No AMD code is linked: the loader DLL is opened at run time. Layouts are
//! pinned by unit tests (fgen/tests.rs) for 64-bit targets.
#![allow(dead_code)]

use crate::viewer::upscale::ffx_abi::{Coords, Dim, Header, Resource};
use std::ffi::c_void;

/// `FFX_API_EFFECT_ID_FRAMEGENERATION` and the two swapchain effects (DX12 = backend id 0, Vulkan its own effect id).
pub const EFFECT_FG: u64 = 0x0002_0000;
pub const EFFECT_FG_SWAPCHAIN: u64 = 0x0003_0000;
pub const EFFECT_FG_SWAPCHAIN_VK: u64 = 0x0004_0000;

pub const CREATE_FG: u64 = EFFECT_FG | 0x01;
pub const CONFIGURE_FG: u64 = EFFECT_FG | 0x02;
pub const DISPATCH_FG: u64 = EFFECT_FG | 0x03;
pub const CALLBACK_PRESENT: u64 = EFFECT_FG | 0x05;
pub const CREATE_FG_HUDLESS: u64 = EFFECT_FG | 0x09;
pub const PREPARE_V2: u64 = EFFECT_FG | 0x0c;
pub const CREATE_FG_VERSION: u64 = EFFECT_FG | 0x0e;

pub const CREATE_SWAPCHAIN_VK: u64 = EFFECT_FG_SWAPCHAIN_VK | 0x01;
pub const QUERY_SWAPCHAIN_FUNCTIONS_VK: u64 = EFFECT_FG_SWAPCHAIN_VK | 0x05;
pub const WAIT_FOR_PRESENTS_VK: u64 = EFFECT_FG_SWAPCHAIN_VK | 0x07;
pub const CREATE_SWAPCHAIN_VERSION_VK: u64 = EFFECT_FG_SWAPCHAIN_VK | 0x0a;

pub const CREATE_SWAPCHAIN_FOR_HWND_DX12: u64 = EFFECT_FG_SWAPCHAIN | 0x06;
pub const WAIT_FOR_PRESENTS_DX12: u64 = EFFECT_FG_SWAPCHAIN | 0x07;
pub const CREATE_SWAPCHAIN_VERSION_DX12: u64 = EFFECT_FG_SWAPCHAIN | 0x0b;

/// `FFX_FRAMEGENERATION_VERSION` (4.0.2) and the swapchains' API versions (4.0.0): major << 22 | minor << 12 | patch.
pub const FG_VERSION: u32 = (4 << 22) | 2;
pub const SWAPCHAIN_VERSION: u32 = 4 << 22;

/// `FfxApiCreateContextFramegenerationFlags`.
pub const FG_ASYNC_WORKLOAD_SUPPORT: u32 = 1 << 0;
pub const FG_DISPLAY_RES_MOTION_VECTORS: u32 = 1 << 1;
pub const FG_MV_JITTER_CANCELLATION: u32 = 1 << 2;
pub const FG_DEPTH_INVERTED: u32 = 1 << 3;
pub const FG_DEPTH_INFINITE: u32 = 1 << 4;
pub const FG_HDR: u32 = 1 << 5;
pub const FG_DEBUG_CHECKING: u32 = 1 << 6;

/// `FfxApiDispatchFramegenerationFlags` (configure / prepare `flags`).
pub const FG_DRAW_DEBUG_TEAR_LINES: u32 = 1 << 0;
pub const FG_DRAW_DEBUG_VIEW: u32 = 1 << 2;

/// `FfxApiRect2D`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
	pub left: i32,
	pub top: i32,
	pub width: i32,
	pub height: i32,
}

/// `ffxCreateContextDescFrameGeneration`.
#[repr(C)]
pub struct CreateFg {
	pub header: Header,
	pub flags: u32,
	pub display_size: Dim,
	pub max_render_size: Dim,
	pub back_buffer_format: u32,
}

/// `ffxCreateContextDescFrameGenerationVersion` / `...Hudless` / the swapchains' `...VersionVK` / `...VersionDX12`: a header and
/// one u32 (the API version, or the HUD-less back buffer format).
#[repr(C)]
pub struct HeaderU32 {
	pub header: Header,
	pub value: u32,
}

/// `ffxCallbackDescFrameGenerationPresent` (what the present callback gets).
#[repr(C)]
pub struct CallbackPresent {
	pub header: Header,
	pub device: *mut c_void,
	pub command_list: *mut c_void,
	pub current_back_buffer: Resource,
	pub current_ui: Resource,
	pub output_swap_chain_buffer: Resource,
	pub is_generated_frame: bool,
	pub frame_id: u64,
}

pub type PresentCallback = Option<unsafe extern "C" fn(params: *mut CallbackPresent, user: *mut c_void) -> u32>;
/// `FfxApiFrameGenerationDispatchFunc`: `params` is an `ffxDispatchDescFrameGeneration` the swapchain filled in; it goes to
/// `ffxDispatch` on the frame generation context unchanged.
pub type FgCallback = Option<unsafe extern "C" fn(params: *mut Header, user: *mut c_void) -> u32>;

/// `ffxConfigureDescFrameGeneration`.
#[repr(C)]
pub struct ConfigureFg {
	pub header: Header,
	pub swap_chain: *mut c_void,
	pub present_callback: PresentCallback,
	pub present_callback_user: *mut c_void,
	pub fg_callback: FgCallback,
	pub fg_callback_user: *mut c_void,
	pub enabled: bool,
	pub allow_async_workloads: bool,
	pub hudless_color: Resource,
	pub flags: u32,
	pub only_present_generated: bool,
	pub generation_rect: Rect,
	pub frame_id: u64,
}

/// `ffxDispatchDescFrameGenerationPrepareV2`.
#[repr(C)]
pub struct PrepareV2 {
	pub header: Header,
	pub frame_id: u64,
	pub flags: u32,
	pub command_list: *mut c_void,
	pub render_size: Dim,
	pub jitter_offset: Coords,
	pub motion_vector_scale: Coords,
	pub frame_time_delta: f32,
	pub reset: bool,
	pub camera_near: f32,
	pub camera_far: f32,
	pub camera_fov_angle_vertical: f32,
	pub view_space_to_meters_factor: f32,
	pub depth: Resource,
	pub motion_vectors: Resource,
	pub camera_position: [f32; 3],
	pub camera_up: [f32; 3],
	pub camera_right: [f32; 3],
	pub camera_forward: [f32; 3],
}

/// `VkQueueInfoFFXAPI`: the queue, its family and an optional submit function (`PFN_vkQueueSubmitFFXAPI`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct QueueInfo {
	pub queue: *mut c_void,
	pub family_index: u32,
	pub submit: Option<unsafe extern "system" fn(count: u32, submits: *const c_void, fence: u64) -> i32>,
}

/// `ffxCreateContextDescFrameGenerationSwapChainVK`.
#[cfg(not(target_vendor = "apple"))]
#[repr(C)]
pub struct CreateSwapchainVk {
	pub header: Header,
	pub physical_device: *mut c_void,
	pub device: *mut c_void,
	pub swapchain: *mut u64,
	pub allocator: *const c_void,
	pub create_info: ash::vk::SwapchainCreateInfoKHR<'static>,
	pub game_queue: QueueInfo,
	pub async_compute_queue: QueueInfo,
	pub present_queue: QueueInfo,
	pub image_acquire_queue: QueueInfo,
}

pub type PfnCreateSwapchainFfx = unsafe extern "system" fn(device: *mut c_void, info: *const c_void, alloc: *const c_void, sc: *mut u64, ctx: *mut c_void) -> i32;
pub type PfnDestroySwapchainFfx = unsafe extern "system" fn(device: *mut c_void, sc: u64, alloc: *const c_void, ctx: *mut c_void);
pub type PfnLastPresentCount = unsafe extern "system" fn(sc: u64) -> u64;

/// `ffxQueryDescSwapchainReplacementFunctionsVK` (the vk* ones as raw pointers: transmuted to ash's PFN types by the caller).
#[repr(C)]
pub struct SwapchainFunctionsVk {
	pub header: Header,
	pub create: Option<PfnCreateSwapchainFfx>,
	pub destroy: Option<PfnDestroySwapchainFfx>,
	pub get_images: *const c_void,
	pub acquire: *const c_void,
	pub present: *const c_void,
	pub set_hdr_metadata: *const c_void,
	pub last_present_count: Option<PfnLastPresentCount>,
}

/// `ffxCreateContextDescFrameGenerationSwapChainForHwndDX12`.
#[repr(C)]
pub struct CreateSwapchainForHwndDx12 {
	pub header: Header,
	pub swapchain: *mut *mut c_void,
	pub hwnd: *mut c_void,
	pub desc: *const c_void,
	pub fullscreen_desc: *const c_void,
	pub dxgi_factory: *mut c_void,
	pub game_queue: *mut c_void,
}
