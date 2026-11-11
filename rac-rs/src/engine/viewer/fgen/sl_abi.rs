//! [PC] Frame generation S3 (lane fgdl): our own binding of the NVIDIA Streamline SDK 2.14.16 C ABI (sl_struct.h, sl_core_types.h,
//! sl_consts.h, sl_core_api.h, sl_dlss_g.h, sl_reflex.h, sl_pcl.h, sl_version.h). The headers are MIT (Copyright (c) 2022-2023
//! NVIDIA CORPORATION); this file restates their layouts in Rust, no NVIDIA code or binary is in tree. x64 layouts are checked by
//! fgen/tests.rs (`sl_abi_layouts`). Every SL structure starts with `Header` (sl::BaseStructure: next, structType GUID, version).

use std::ffi::{c_char, c_void};

/// sl::kSDKVersion of 2.14.16 (major << 48 | minor << 32 | patch << 16 | magic 0xfedc).
pub(super) const SDK_VERSION: u64 = (2u64 << 48) | (14u64 << 32) | (16u64 << 16) | 0xfedc;

pub(super) const FEATURE_REFLEX: u32 = 3;
pub(super) const FEATURE_PCL: u32 = 4;
pub(super) const FEATURE_DLSS_G: u32 = 1000;

pub(super) const RESULT_OK: i32 = 0;

pub(super) const BUFFER_DEPTH: u32 = 0;
pub(super) const BUFFER_MVEC: u32 = 1;
pub(super) const BUFFER_HUDLESS: u32 = 2;
/// kBufferTypeBackbuffer: tagged with a null resource, only to give SL the backbuffer extent (the whole window).
pub(super) const BUFFER_BACKBUFFER: u32 = 53;

/// sl::ResourceLifecycle::eValidUntilPresent.
pub(super) const VALID_UNTIL_PRESENT: u32 = 1;

/// sl::PreferenceFlags.
pub(super) const PREF_DISABLE_CL_STATE_TRACKING: u64 = 1 << 0;
pub(super) const PREF_USE_MANUAL_HOOKING: u64 = 1 << 2;
/// eUseFrameBasedResourceTagging: slSetTagForFrame (the frame-token tagging API) needs it.
pub(super) const PREF_FRAME_BASED_TAGGING: u64 = 1 << 7;

/// sl::RenderAPI.
pub(super) const API_D3D12: u32 = 1;
#[allow(dead_code)] // Vulkan stage (HANDOFF.md)
pub(super) const API_VULKAN: u32 = 2;

/// sl::LogLevel::eDefault.
pub(super) const LOG_DEFAULT: u32 = 1;

/// sl::DLSSGMode.
pub(super) const DLSSG_OFF: u32 = 0;
pub(super) const DLSSG_ON: u32 = 1;
/// sl::DLSSGFlags.
pub(super) const DLSSG_SHOW_ONLY_INTERPOLATED: u32 = 1 << 0;
pub(super) const DLSSG_RETAIN_RESOURCES_WHEN_OFF: u32 = 1 << 3;

/// sl::ReflexMode::eLowLatency.
pub(super) const REFLEX_LOW_LATENCY: u32 = 1;

/// sl::PCLMarker.
pub(super) const PCL_SIM_START: u32 = 0;
pub(super) const PCL_SIM_END: u32 = 1;
pub(super) const PCL_RENDER_START: u32 = 2;
pub(super) const PCL_RENDER_END: u32 = 3;
pub(super) const PCL_PRESENT_START: u32 = 4;
pub(super) const PCL_PRESENT_END: u32 = 5;

/// sl::Boolean.
pub(super) const FALSE: u8 = 0;
pub(super) const TRUE: u8 = 1;

/// sl::INVALID_FLOAT.
pub(super) const INVALID_FLOAT: f32 = f32::MAX;

/// sl::StructType (a GUID).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Guid(pub u32, pub u16, pub u16, pub [u8; 8]);

/// sl::BaseStructure.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct Header {
	pub next: *mut Header,
	pub ty: Guid,
	pub version: usize,
}

impl Header {
	pub(super) const fn of(ty: Guid, version: usize) -> Header {
		Header { next: std::ptr::null_mut(), ty, version }
	}
}

pub(super) const T_PREFERENCES: Guid = Guid(0x1ca10965, 0xbf8e, 0x432b, [0x8d, 0xa1, 0x67, 0x16, 0xd8, 0x79, 0xfb, 0x14]);
pub(super) const T_RESOURCE: Guid = Guid(0x3a9d70cf, 0x2418, 0x4b72, [0x83, 0x91, 0x13, 0xf8, 0x72, 0x1c, 0x72, 0x61]);
pub(super) const T_RESOURCE_TAG: Guid = Guid(0x4c6a5aad, 0xb445, 0x496c, [0x87, 0xff, 0x1a, 0xf3, 0x84, 0x5b, 0xe6, 0x53]);
pub(super) const T_VIEWPORT: Guid = Guid(0x171b6435, 0x9b3c, 0x4fc8, [0x99, 0x94, 0xfb, 0xe5, 0x25, 0x69, 0xaa, 0xa4]);
pub(super) const T_ADAPTER_INFO: Guid = Guid(0x0677315f, 0xa746, 0x4492, [0x9f, 0x42, 0xcb, 0x61, 0x42, 0xc9, 0xc3, 0xd4]);
pub(super) const T_CONSTANTS: Guid = Guid(0xdcd35ad7, 0x4e4a, 0x4bad, [0xa9, 0x0c, 0xe0, 0xc4, 0x9e, 0xb2, 0x3a, 0xfe]);
pub(super) const T_DLSSG_OPTIONS: Guid = Guid(0xfac5f1cb, 0x2dfd, 0x4f36, [0xa1, 0xe6, 0x3a, 0x9e, 0x86, 0x52, 0x56, 0xc5]);
pub(super) const T_DLSSG_STATE: Guid = Guid(0xcc8ac8e1, 0xa179, 0x44f5, [0x97, 0xfa, 0xe7, 0x41, 0x12, 0xf9, 0xbc, 0x61]);
pub(super) const T_REFLEX_OPTIONS: Guid = Guid(0xf03af81a, 0x6d0b, 0x4902, [0xa6, 0x51, 0xc4, 0x96, 0x5e, 0x21, 0x54, 0x34]);

pub(super) type LogCb = unsafe extern "C" fn(ty: u32, msg: *const c_char);

/// sl::Preferences (kStructVersion1).
#[repr(C)]
pub(super) struct Preferences {
	pub header: Header,
	pub show_console: bool,
	pub log_level: u32,
	pub paths_to_plugins: *const *const u16,
	pub num_paths_to_plugins: u32,
	pub path_to_logs_and_data: *const u16,
	pub allocate_callback: *const c_void,
	pub release_callback: *const c_void,
	pub log_message_callback: Option<LogCb>,
	pub flags: u64,
	pub features_to_load: *const u32,
	pub num_features_to_load: u32,
	pub application_id: u32,
	pub engine: u32,
	pub engine_version: *const c_char,
	pub project_id: *const c_char,
	pub render_api: u32,
}

/// sl::Resource (kStructVersion1). `ty` 0 = eTex2d.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct Resource {
	pub header: Header,
	pub ty: u8,
	pub native: *mut c_void,
	pub memory: *mut c_void,
	pub view: *mut c_void,
	pub state: u32,
	pub width: u32,
	pub height: u32,
	pub native_format: u32,
	pub mip_levels: u32,
	pub array_layers: u32,
	pub gpu_virtual_address: u64,
	pub flags: u32,
	pub usage: u32,
	pub internal_flags: u16,
	pub reserved: u16,
}

impl Resource {
	pub(super) fn tex2d(native: *mut c_void, state: u32) -> Resource {
		Resource { header: Header::of(T_RESOURCE, 1), ty: 0, native, memory: std::ptr::null_mut(), view: std::ptr::null_mut(), state, width: 0, height: 0, native_format: 0, mip_levels: 0, array_layers: 0, gpu_virtual_address: 0, flags: 0, usage: 0, internal_flags: 0, reserved: 0 }
	}
}

/// sl::Extent.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Extent {
	pub top: u32,
	pub left: u32,
	pub width: u32,
	pub height: u32,
}

/// sl::ResourceTag (kStructVersion1). A null `resource` = the tag is cleared.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct ResourceTag {
	pub header: Header,
	pub resource: *mut Resource,
	pub ty: u32,
	pub lifecycle: u32,
	pub extent: Extent,
}

impl ResourceTag {
	pub(super) fn new(resource: *mut Resource, ty: u32, extent: Extent) -> ResourceTag {
		ResourceTag { header: Header::of(T_RESOURCE_TAG, 1), resource, ty, lifecycle: VALID_UNTIL_PRESENT, extent }
	}
}

/// sl::ViewportHandle (kStructVersion1).
#[repr(C)]
pub(super) struct Viewport {
	pub header: Header,
	pub value: u32,
}

impl Viewport {
	pub(super) fn new(value: u32) -> Viewport {
		Viewport { header: Header::of(T_VIEWPORT, 1), value }
	}
}

/// sl::AdapterInfo (kStructVersion1).
#[repr(C)]
pub(super) struct AdapterInfo {
	pub header: Header,
	pub device_luid: *const u8,
	pub device_luid_size: u32,
	pub vk_physical_device: *mut c_void,
}

/// sl::float4x4: four rows (row-major).
pub(super) type Mat = [[f32; 4]; 4];

/// sl::Constants (kStructVersion2).
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct Constants {
	pub header: Header,
	pub camera_view_to_clip: Mat,
	pub clip_to_camera_view: Mat,
	pub clip_to_lens_clip: Mat,
	pub clip_to_prev_clip: Mat,
	pub prev_clip_to_clip: Mat,
	pub jitter_offset: [f32; 2],
	pub mvec_scale: [f32; 2],
	pub camera_pinhole_offset: [f32; 2],
	pub camera_pos: [f32; 3],
	pub camera_up: [f32; 3],
	pub camera_right: [f32; 3],
	pub camera_fwd: [f32; 3],
	pub camera_near: f32,
	pub camera_far: f32,
	pub camera_fov: f32,
	pub camera_aspect_ratio: f32,
	pub motion_vectors_invalid_value: f32,
	pub depth_inverted: u8,
	pub camera_motion_included: u8,
	pub motion_vectors_3d: u8,
	pub reset: u8,
	pub orthographic_projection: u8,
	pub motion_vectors_dilated: u8,
	pub motion_vectors_jittered: u8,
	pub min_relative_linear_depth_object_separation: f32,
}

/// sl::DLSSGOptions (kStructVersion5).
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct DlssgOptions {
	pub header: Header,
	pub mode: u32,
	pub num_frames_to_generate: u32,
	pub flags: u32,
	pub dynamic_res_width: u32,
	pub dynamic_res_height: u32,
	pub num_back_buffers: u32,
	pub mvec_depth_width: u32,
	pub mvec_depth_height: u32,
	pub color_width: u32,
	pub color_height: u32,
	pub color_buffer_format: u32,
	pub mvec_buffer_format: u32,
	pub depth_buffer_format: u32,
	pub hud_less_buffer_format: u32,
	pub ui_buffer_format: u32,
	pub on_error_callback: *const c_void,
	pub b_reserved15: u8,
	pub queue_parallelism_mode: u32,
	pub enable_user_interface_recomposition: u8,
	pub dynamic_target_frame_rate: f32,
}

impl DlssgOptions {
	pub(super) fn new(mode: u32, frames: u32, flags: u32) -> DlssgOptions {
		DlssgOptions { header: Header::of(T_DLSSG_OPTIONS, 5), mode, num_frames_to_generate: frames, flags, dynamic_res_width: 0, dynamic_res_height: 0, num_back_buffers: 0, mvec_depth_width: 0, mvec_depth_height: 0, color_width: 0, color_height: 0, color_buffer_format: 0, mvec_buffer_format: 0, depth_buffer_format: 0, hud_less_buffer_format: 0, ui_buffer_format: 0, on_error_callback: std::ptr::null(), b_reserved15: 2, queue_parallelism_mode: 0, enable_user_interface_recomposition: FALSE, dynamic_target_frame_rate: 0.0 }
	}
}

/// sl::DLSSGState (kStructVersion4).
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct DlssgState {
	pub header: Header,
	pub estimated_vram_usage_in_bytes: u64,
	pub status: u32,
	pub min_width_or_height: u32,
	pub num_frames_actually_presented: u32,
	pub num_frames_to_generate_max: u32,
	pub b_reserved4: u8,
	pub b_is_vsync_support_available: u8,
	pub inputs_processing_completion_fence: *mut c_void,
	pub last_present_inputs_processing_completion_fence_value: u64,
	pub b_is_dynamic_mfg_supported: u8,
}

impl DlssgState {
	pub(super) fn new() -> DlssgState {
		DlssgState { header: Header::of(T_DLSSG_STATE, 4), estimated_vram_usage_in_bytes: 0, status: 0, min_width_or_height: 0, num_frames_actually_presented: 0, num_frames_to_generate_max: 0, b_reserved4: 0, b_is_vsync_support_available: 0, inputs_processing_completion_fence: std::ptr::null_mut(), last_present_inputs_processing_completion_fence_value: 0, b_is_dynamic_mfg_supported: 0 }
	}
}

/// sl::ReflexOptions (kStructVersion1).
#[repr(C)]
pub(super) struct ReflexOptions {
	pub header: Header,
	pub mode: u32,
	pub frame_limit_us: u32,
	pub use_markers_to_optimize: bool,
	pub virtual_key: u16,
	pub id_thread: u32,
}

/// sl::FrameToken: SL-owned, opaque (a C++ object with a vtable); only its address is passed around.
#[repr(C)]
pub(super) struct FrameToken {
	_opaque: [u8; 0],
}

pub(super) type PfnInit = unsafe extern "C" fn(pref: *const Preferences, sdk_version: u64) -> i32;
pub(super) type PfnShutdown = unsafe extern "C" fn() -> i32;
pub(super) type PfnIsFeatureSupported = unsafe extern "C" fn(feature: u32, adapter: *const AdapterInfo) -> i32;
pub(super) type PfnSetTagForFrame = unsafe extern "C" fn(frame: *const FrameToken, viewport: *const Viewport, tags: *const ResourceTag, n: u32, cmd: *mut c_void) -> i32;
pub(super) type PfnSetConstants = unsafe extern "C" fn(values: *const Constants, frame: *const FrameToken, viewport: *const Viewport) -> i32;
pub(super) type PfnGetFeatureFunction = unsafe extern "C" fn(feature: u32, name: *const c_char, function: *mut *mut c_void) -> i32;
pub(super) type PfnGetNewFrameToken = unsafe extern "C" fn(token: *mut *mut FrameToken, index: *const u32) -> i32;
pub(super) type PfnSetD3DDevice = unsafe extern "C" fn(device: *mut c_void) -> i32;
pub(super) type PfnUpgradeInterface = unsafe extern "C" fn(base: *mut *mut c_void) -> i32;
pub(super) type PfnDlssgGetState = unsafe extern "C" fn(viewport: *const Viewport, state: *mut DlssgState, options: *const DlssgOptions) -> i32;
pub(super) type PfnDlssgSetOptions = unsafe extern "C" fn(viewport: *const Viewport, options: *const DlssgOptions) -> i32;
pub(super) type PfnReflexSleep = unsafe extern "C" fn(frame: *const FrameToken) -> i32;
pub(super) type PfnReflexSetOptions = unsafe extern "C" fn(options: *const ReflexOptions) -> i32;
pub(super) type PfnPclSetMarker = unsafe extern "C" fn(marker: u32, frame: *const FrameToken) -> i32;

/// sl::Result names (sl_result.h order) for log lines.
/// sl::Result values the card check tells apart (sl_result.h order).
pub(super) const RESULT_NO_SUPPORTED_ADAPTER: i32 = 6;
pub(super) const RESULT_ADAPTER_NOT_SUPPORTED: i32 = 7;
pub(super) const RESULT_FEATURE_NOT_SUPPORTED: i32 = 32;

pub(super) fn result_name(r: i32) -> &'static str {
	const N: [&str; 40] = [
		"eOk", "eErrorIO", "eErrorDriverOutOfDate", "eErrorOSOutOfDate", "eErrorOSDisabledHWS", "eErrorDeviceNotCreated", "eErrorNoSupportedAdapterFound", "eErrorAdapterNotSupported", "eErrorNoPlugins", "eErrorVulkanAPI", "eErrorDXGIAPI", "eErrorD3DAPI", "eErrorNRDAPI", "eErrorNVAPI", "eErrorReflexAPI", "eErrorNGXFailed", "eErrorJSONParsing", "eErrorMissingProxy", "eErrorMissingResourceState", "eErrorInvalidIntegration", "eErrorMissingInputParameter", "eErrorNotInitialized", "eErrorComputeFailed", "eErrorInitNotCalled", "eErrorExceptionHandler", "eErrorInvalidParameter", "eErrorMissingConstants", "eErrorDuplicatedConstants", "eErrorMissingOrInvalidAPI", "eErrorCommonConstantsMissing", "eErrorUnsupportedInterface", "eErrorFeatureMissing", "eErrorFeatureNotSupported", "eErrorFeatureMissingHooks", "eErrorFeatureFailedToLoad", "eErrorFeatureWrongPriority", "eErrorFeatureMissingDependency", "eErrorFeatureManagerInvalidState", "eErrorInvalidState", "eWarnOutOfVRAM",
	];
	N.get(r as usize).copied().unwrap_or("eError(other)")
}
