//! C ABI of `rac_ngx.dll`, the small helper that wraps NVIDIA NGX (DLSS Super Resolution).
//!
//! This crate holds NO NVIDIA code: only `repr(C)` structs, constants and (feature `loader`) a run-time loader
//! that opens `rac_ngx.dll` with libloading. The game links only this crate; the NVIDIA static library is linked
//! into `rac_ngx.dll` alone (crate `rac_ngx`, msvc), so the GNU player build never links NVIDIA code.
//!
//! Every export returns a `u32` NGX result: `RESULT_SUCCESS` (1) = ok, `failed(r)` for errors
//! (`0xBAD0_00xx` = NGX codes, `0xBAD0_F0xx` = errors of the helper itself).
//! Handles: `RacNgxCtx` = one NGX init on one device (Vulkan or D3D12); `RacNgxFeature` = one DLSS feature
//! (fixed render size, output size, quality and preset; create a new one when any of them changes).
//! Keep the `Lib` alive for the whole process: NGX keeps threads and the DLSS model inside it.
#![allow(non_camel_case_types)]

use core::ffi::{c_char, c_void};

/// Bumped on any change of a struct or export below; `Lib::load` refuses a DLL with another value.
pub const ABI_VERSION: u32 = 1;

// ---- results ----
pub const RESULT_SUCCESS: u32 = 0x1;
pub const RESULT_FAIL: u32 = 0xBAD0_0000;
pub const FAIL_FEATURE_NOT_SUPPORTED: u32 = RESULT_FAIL | 1;
pub const FAIL_PLATFORM_ERROR: u32 = RESULT_FAIL | 2;
pub const FAIL_INVALID_PARAMETER: u32 = RESULT_FAIL | 5;
pub const FAIL_NOT_INITIALIZED: u32 = RESULT_FAIL | 7;
pub const FAIL_UNABLE_TO_INITIALIZE_FEATURE: u32 = RESULT_FAIL | 11;
pub const FAIL_OUT_OF_DATE: u32 = RESULT_FAIL | 12;
pub const FAIL_DENIED: u32 = RESULT_FAIL | 17;
/// Helper errors (not NGX): a required pointer argument was NULL.
pub const ERR_NULL_ARG: u32 = 0xBAD0_F001;
/// Helper error: the caller's buffer is too small (`*len` holds the needed size).
pub const ERR_BUFFER_TOO_SMALL: u32 = 0xBAD0_F002;
/// Helper error: the context was made for the other graphics API.
pub const ERR_WRONG_API: u32 = 0xBAD0_F003;
/// Helper error: a Rust panic was caught inside the DLL.
pub const ERR_PANIC: u32 = 0xBAD0_F004;

/// True when `r` is an error code (the top 12 bits equal `RESULT_FAIL`'s).
pub const fn failed(r: u32) -> bool {
    (r & 0xFFF0_0000) == RESULT_FAIL
}

// ---- quality modes (the numbers NGX uses for them) ----
pub const QUALITY_MAX_PERF: u32 = 0; // "Performance", render 50 %
pub const QUALITY_BALANCED: u32 = 1; // 58 %
pub const QUALITY_MAX_QUALITY: u32 = 2; // "Quality", 66.7 %
pub const QUALITY_ULTRA_PERFORMANCE: u32 = 3; // 33.3 %
pub const QUALITY_ULTRA_QUALITY: u32 = 4; // not offered by current DLSS
pub const QUALITY_DLAA: u32 = 5; // render = output size
pub const QUALITY_NAMES: [(u32, &str); 6] = [
    (QUALITY_DLAA, "DLAA"),
    (QUALITY_ULTRA_QUALITY, "UltraQuality"),
    (QUALITY_MAX_QUALITY, "Quality"),
    (QUALITY_BALANCED, "Balanced"),
    (QUALITY_MAX_PERF, "Performance"),
    (QUALITY_ULTRA_PERFORMANCE, "UltraPerformance"),
];

// ---- DLSS render presets (preset letter A = 1, B = 2, ...) ----
pub const PRESET_DEFAULT: u32 = 0;
pub const PRESET_J: u32 = 10; // 1st-gen transformer (DLSS 4)
pub const PRESET_K: u32 = 11; // 1st-gen transformer (DLSS 4), NGX default for DLAA/Quality/Balanced
pub const PRESET_L: u32 = 12; // 2nd-gen transformer (DLSS 4.5), NGX default for Ultra Performance
pub const PRESET_M: u32 = 13; // 2nd-gen transformer (DLSS 4.5), NGX default for Performance

/// The "DLSS 4.5" choice for a quality mode: preset M everywhere, L for Ultra Performance (NVIDIA's pairing).
pub const fn preset_dlss45(quality: u32) -> u32 {
    if quality == QUALITY_ULTRA_PERFORMANCE {
        PRESET_L
    } else {
        PRESET_M
    }
}

/// Letter of a preset value (0 -> '-', 10 -> 'J' ...).
pub fn preset_letter(p: u32) -> char {
    if p == 0 {
        '-'
    } else if p <= 15 {
        (b'A' + (p as u8) - 1) as char
    } else {
        '?'
    }
}

// ---- DLSS create flags (bit values NGX uses) ----
pub const FLAG_IS_HDR: u32 = 1 << 0;
pub const FLAG_MV_LOW_RES: u32 = 1 << 1;
pub const FLAG_MV_JITTERED: u32 = 1 << 2;
pub const FLAG_DEPTH_INVERTED: u32 = 1 << 3;
pub const FLAG_AUTO_EXPOSURE: u32 = 1 << 6;

// ---- adapter support bits returned by rac_ngx_*_requirements (0 = supported) ----
pub const SUPPORT_OK: u32 = 0;
pub const SUPPORT_CHECK_NOT_PRESENT: u32 = 1;
pub const SUPPORT_DRIVER_UNSUPPORTED: u32 = 2;
pub const SUPPORT_ADAPTER_UNSUPPORTED: u32 = 4;
pub const SUPPORT_OS_TOO_OLD: u32 = 8;
pub const SUPPORT_NOT_IMPLEMENTED: u32 = 16;

// ---- logging ----
pub const LOG_OFF: u32 = 0;
pub const LOG_ON: u32 = 1;
pub const LOG_VERBOSE: u32 = 2;

/// rac-rs project id for `Init_with_ProjectID` (CUSTOM engine). Used when `RacNgxApp::project_id` is NULL.
pub const RAC_PROJECT_ID: &str = "817216f4-ae4f-4026-8f9f-f4d03b812d69";

/// Opaque NGX context (one init on one device).
#[repr(C)]
pub struct RacNgxCtx {
    _p: [u8; 0],
}
/// Opaque DLSS feature.
#[repr(C)]
pub struct RacNgxFeature {
    _p: [u8; 0],
}

/// Log sink: `level` 1 = info, 2 = verbose; `msg` is NUL-terminated UTF-8 (no trailing newline).
pub type RacNgxLogFn = Option<unsafe extern "C" fn(user: *mut c_void, level: u32, msg: *const c_char)>;

/// Who calls NGX. All strings NUL-terminated; the DLL copies what it keeps.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RacNgxApp {
    /// NULL = `RAC_PROJECT_ID`.
    pub project_id: *const c_char,
    /// NULL = "1.0".
    pub engine_version: *const c_char,
    /// Writable folder for NGX's data (UTF-16); NULL = ".".
    pub data_path: *const u16,
    /// Extra folder searched for nvngx_dlss.dll (UTF-16) before the exe folder; NULL = exe folder only.
    pub dll_dir: *const u16,
    /// NGX log level (`LOG_*`); lines go to the `rac_ngx_set_log` sink only (other NGX sinks are disabled).
    pub log_level: u32,
    pub _pad: u32,
}

/// Raw Vulkan handles of the device wgpu made (with the NGX extensions enabled).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RacNgxVkInit {
    pub instance: *mut c_void,
    pub physical: *mut c_void,
    pub device: *mut c_void,
    /// PFN_vkGetInstanceProcAddr of the loader wgpu used.
    pub gipa: *const c_void,
    /// PFN_vkGetDeviceProcAddr.
    pub gdpa: *const c_void,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RacNgxCaps {
    /// 1 = DLSS super resolution is usable on this device.
    pub available: i32,
    /// 1 = the driver is too old for DLSS (see `min_driver_*`).
    pub needs_driver: i32,
    pub min_driver_major: u32,
    pub min_driver_minor: u32,
    /// Result code of NGX's own DLSS start-up check.
    pub init_result: u32,
    pub _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RacNgxOptimal {
    pub render_w: u32,
    pub render_h: u32,
    pub min_w: u32,
    pub min_h: u32,
    pub max_w: u32,
    pub max_h: u32,
    pub sharpness: f32,
    pub _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RacNgxCreate {
    pub render_w: u32,
    pub render_h: u32,
    pub out_w: u32,
    pub out_h: u32,
    /// `QUALITY_*`.
    pub quality: u32,
    /// `PRESET_*`, used for every quality mode (0 = NGX default).
    pub preset: u32,
    /// `FLAG_*`.
    pub flags: u32,
    pub _pad: u32,
}

/// One Vulkan image for DLSS: view + image of mip 0 / layer 0, size = the area DLSS reads or writes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RacNgxImageVk {
    pub view: u64,
    pub image: u64,
    /// VkFormat of the view.
    pub vk_format: u32,
    /// VkImageAspectFlags (1 = colour, 2 = depth).
    pub aspect: u32,
    pub width: u32,
    pub height: u32,
}

/// Vulkan evaluate inputs. Layouts at evaluate time: colour/depth/motion SHADER_READ_ONLY_OPTIMAL, output GENERAL.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RacNgxVkInputs {
    pub color: RacNgxImageVk,
    pub output: RacNgxImageVk,
    pub depth: RacNgxImageVk,
    pub motion: RacNgxImageVk,
}

/// D3D12 evaluate inputs (ID3D12Resource*). States: inputs PIXEL|NON_PIXEL_SHADER_RESOURCE, output UNORDERED_ACCESS.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RacNgxDx12Inputs {
    pub color: *mut c_void,
    pub output: *mut c_void,
    pub depth: *mut c_void,
    pub motion: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RacNgxEval {
    /// Sub-pixel jitter of this frame in render pixels.
    pub jitter_x: f32,
    pub jitter_y: f32,
    /// Motion vector scale (0 = 1).
    pub mv_scale_x: f32,
    pub mv_scale_y: f32,
    /// Area of the colour/depth/motion inputs that holds the frame.
    pub render_w: u32,
    pub render_h: u32,
    /// 1 = drop history (first frame, cuts).
    pub reset: u32,
    /// 0 = 1.
    pub pre_exposure: f32,
    /// 0 = 1.
    pub exposure_scale: f32,
    pub frame_ms: f32,
}

/// What a feature was made with, and the preset NGX said it uses (from its own log line).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RacNgxFeatureInfo {
    pub create: RacNgxCreate,
    /// Preset letter NGX reported (ASCII 'A'..'O'), 0 = no report seen (log level OFF, or no such line yet).
    pub preset_reported: u32,
    pub _pad: u32,
    /// The NGX log line the letter came from (NUL-terminated, may be cut).
    pub report: [c_char; 192],
}

impl Default for RacNgxFeatureInfo {
    fn default() -> Self {
        Self { create: RacNgxCreate::default(), preset_reported: 0, _pad: 0, report: [0; 192] }
    }
}

impl RacNgxFeatureInfo {
    pub fn report_str(&self) -> String {
        let bytes: Vec<u8> = self.report.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// One table drives the export names, their types (`pfn::*`, used by `rac_ngx` to check its exports at compile
/// time) and the loader.
macro_rules! exports {
    ($( $(#[$doc:meta])* fn $name:ident ( $($arg:ident : $ty:ty),* $(,)? ) -> $ret:ty ; )*) => {
        /// Function pointer types of every export.
        pub mod pfn {
            #[allow(unused_imports)]
            use super::*;
            $( $(#[$doc])* pub type $name = unsafe extern "C" fn($($ty),*) -> $ret; )*
        }
        /// Every export name, NUL-terminated.
        pub const EXPORT_NAMES: &[&str] = &[$( concat!(stringify!($name), "\0") ),*];

        /// `rac_ngx.dll` opened at run time.
        #[cfg(feature = "loader")]
        pub struct Lib {
            $( pub $name: pfn::$name, )*
            _lib: libloading::Library,
        }

        #[cfg(feature = "loader")]
        impl Lib {
            /// Opens the DLL, resolves every export and checks `ABI_VERSION`.
            ///
            /// # Safety
            /// Runs the DLL's initialisers; `path` must be a `rac_ngx.dll` built from this repository.
            pub unsafe fn load(path: &std::path::Path) -> Result<Self, String> {
                let lib = unsafe { libloading::Library::new(path) }
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                $(
                    let $name: pfn::$name = unsafe {
                        *lib.get::<pfn::$name>(concat!(stringify!($name), "\0").as_bytes())
                            .map_err(|e| format!("{}: {}: {e}", path.display(), stringify!($name)))?
                    };
                )*
                let me = Self { $($name,)* _lib: lib };
                let v = unsafe { (me.rac_ngx_abi_version)() };
                if v != ABI_VERSION {
                    return Err(format!("{}: ABI {v}, expected {ABI_VERSION}", path.display()));
                }
                Ok(me)
            }

            /// Readable name of a result code, as the DLL reports it.
            pub fn result_name(&self, r: u32) -> String {
                let p = unsafe { (self.rac_ngx_result_name)(r) };
                if p.is_null() {
                    return format!("0x{r:08X}");
                }
                unsafe { core::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned()
            }
        }
    };
}

exports! {
    /// == ABI_VERSION.
    fn rac_ngx_abi_version() -> u32;
    /// Static NUL-terminated name of a result; never NULL.
    fn rac_ngx_result_name(r: u32) -> *const c_char;
    /// Sets the log sink (NULL = none) and the highest level forwarded to it. Process-wide.
    fn rac_ngx_set_log(cb: RacNgxLogFn, user: *mut c_void, level: u32) -> u32;
    /// Vulkan instance extensions NGX needs, '\n'-joined into `buf` (NUL-terminated). `buf` may be NULL to ask
    /// the size; `*len` = bytes needed incl. the NUL. Call before creating the VkInstance.
    fn rac_ngx_vk_instance_extensions(app: *const RacNgxApp, buf: *mut c_char, cap: u32, len: *mut u32) -> u32;
    /// Vulkan device extensions NGX needs on `physical` (same buffer rules).
    fn rac_ngx_vk_device_extensions(app: *const RacNgxApp, instance: *mut c_void, physical: *mut c_void,
        buf: *mut c_char, cap: u32, len: *mut u32) -> u32;
    /// DLSS support of a Vulkan adapter: `*bits` = `SUPPORT_*` (0 = supported), `*min_arch` = NV arch needed.
    fn rac_ngx_vk_requirements(app: *const RacNgxApp, instance: *mut c_void, physical: *mut c_void,
        bits: *mut u32, min_arch: *mut u32) -> u32;
    /// DLSS support of a DXGI adapter (IDXGIAdapter*).
    fn rac_ngx_dx12_requirements(app: *const RacNgxApp, adapter: *mut c_void, bits: *mut u32, min_arch: *mut u32) -> u32;
    /// NGX init on a Vulkan device; `*ctx` gets the context.
    fn rac_ngx_init_vk(app: *const RacNgxApp, init: *const RacNgxVkInit, ctx: *mut *mut RacNgxCtx) -> u32;
    /// NGX init on a D3D12 device (ID3D12Device*).
    fn rac_ngx_init_dx12(app: *const RacNgxApp, device: *mut c_void, ctx: *mut *mut RacNgxCtx) -> u32;
    /// DLSS availability on the context's device.
    fn rac_ngx_caps(ctx: *mut RacNgxCtx, caps: *mut RacNgxCaps) -> u32;
    /// Render size NGX recommends for an output size and quality (0 x 0 = mode not offered).
    fn rac_ngx_optimal(ctx: *mut RacNgxCtx, out_w: u32, out_h: u32, quality: u32, opt: *mut RacNgxOptimal) -> u32;
    /// Creates a DLSS feature; records init work into `cmd` (VkCommandBuffer or ID3D12GraphicsCommandList*, open),
    /// which the caller submits and waits for before the first evaluate.
    fn rac_ngx_create(ctx: *mut RacNgxCtx, cmd: *mut c_void, create: *const RacNgxCreate,
        feat: *mut *mut RacNgxFeature) -> u32;
    /// What the feature was created with + the preset NGX reported.
    fn rac_ngx_feature_info(ctx: *mut RacNgxCtx, feat: *mut RacNgxFeature, info: *mut RacNgxFeatureInfo) -> u32;
    /// Records DLSS into an open VkCommandBuffer.
    fn rac_ngx_evaluate_vk(ctx: *mut RacNgxCtx, feat: *mut RacNgxFeature, cmd: *mut c_void,
        inputs: *const RacNgxVkInputs, eval: *const RacNgxEval) -> u32;
    /// Records DLSS into an open ID3D12GraphicsCommandList.
    fn rac_ngx_evaluate_dx12(ctx: *mut RacNgxCtx, feat: *mut RacNgxFeature, list: *mut c_void,
        inputs: *const RacNgxDx12Inputs, eval: *const RacNgxEval) -> u32;
    /// Releases a feature (the GPU must be done with it).
    fn rac_ngx_release(ctx: *mut RacNgxCtx, feat: *mut RacNgxFeature) -> u32;
    /// Shuts NGX down on the context's device and frees the context (release every feature first).
    fn rac_ngx_shutdown(ctx: *mut RacNgxCtx) -> u32;
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{align_of, size_of};

    #[test]
    fn struct_sizes() {
        assert_eq!(size_of::<RacNgxApp>(), 40);
        assert_eq!(size_of::<RacNgxVkInit>(), 40);
        assert_eq!(size_of::<RacNgxCaps>(), 24);
        assert_eq!(size_of::<RacNgxOptimal>(), 32);
        assert_eq!(size_of::<RacNgxCreate>(), 32);
        assert_eq!(size_of::<RacNgxImageVk>(), 32);
        assert_eq!(size_of::<RacNgxVkInputs>(), 128);
        assert_eq!(size_of::<RacNgxDx12Inputs>(), 32);
        assert_eq!(size_of::<RacNgxEval>(), 40);
        assert_eq!(size_of::<RacNgxFeatureInfo>(), 32 + 8 + 192);
        assert_eq!(align_of::<RacNgxImageVk>(), 8);
    }

    #[test]
    fn presets() {
        assert_eq!(preset_dlss45(QUALITY_ULTRA_PERFORMANCE), PRESET_L);
        for q in [QUALITY_DLAA, QUALITY_MAX_QUALITY, QUALITY_BALANCED, QUALITY_MAX_PERF, QUALITY_ULTRA_QUALITY] {
            assert_eq!(preset_dlss45(q), PRESET_M);
        }
        assert_eq!(preset_letter(PRESET_M), 'M');
        assert_eq!(preset_letter(PRESET_L), 'L');
        assert_eq!(preset_letter(PRESET_K), 'K');
        assert_eq!(preset_letter(0), '-');
    }

    #[test]
    fn results() {
        assert!(!failed(RESULT_SUCCESS));
        assert!(failed(FAIL_FEATURE_NOT_SUPPORTED));
        assert!(failed(ERR_NULL_ARG) && failed(ERR_BUFFER_TOO_SMALL) && failed(ERR_PANIC));
        assert_eq!(EXPORT_NAMES.len(), 17);
        assert!(EXPORT_NAMES.iter().all(|n| n.starts_with("rac_ngx_") && n.ends_with('\0')));
    }
}
