//! The OmniLoader C ABI v1 (omniloader.h) as this host sees it: the `#[repr(C)]` structs written from the header, and the DLL's
//! functions loaded with libloading. Nothing here uses the omniloader crate; the DLL is driven only through its C ABI.

use libloading::Library;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;

pub const ABI_VERSION: u32 = 1;

/// `OmniValue` (24 bytes): a number crossing the boundary; integers of every width in `i`, floats in `f`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Value {
	pub is_float: i32,
	pub reserved: i32,
	pub i: i64,
	pub f: f64,
}

impl Value {
	pub fn int(i: i64) -> Value {
		Value { is_float: 0, reserved: 0, i, f: 0.0 }
	}
	pub fn float(f: f64) -> Value {
		Value { is_float: 1, reserved: 0, i: 0, f }
	}
}

pub type PvarEmit = unsafe extern "C" fn(*mut c_void, *const c_char, f64);
pub type LogFn = unsafe extern "C" fn(*mut c_void, i32, *const c_char, *const c_char);
pub type ReadFn = unsafe extern "C" fn(*mut c_void, u32, *mut u8, u32) -> i32;
pub type WriteFn = unsafe extern "C" fn(*mut c_void, u32, *const u8, u32) -> i32;
pub type NameFn = unsafe extern "C" fn(*mut c_void, *const c_char) -> i32;
pub type BaseFn = unsafe extern "C" fn(*mut c_void, *const c_char, *mut u32) -> i32;
pub type PvarsFn = unsafe extern "C" fn(*mut c_void, u32, PvarEmit, *mut c_void) -> i32;
pub type InstallFn = unsafe extern "C" fn(*mut c_void, u32, *const c_char) -> i32;
pub type UninstallFn = unsafe extern "C" fn(*mut c_void, u32, *const c_char);
pub type OriginalFn = unsafe extern "C" fn(*mut c_void, u32, *const Value, u32, *mut Value);
pub type MouseFn = unsafe extern "C" fn(*mut c_void, *mut f64);
pub type WindowFn = unsafe extern "C" fn(*mut c_void, *mut i32);
pub type PointerFn = unsafe extern "C" fn(*mut c_void, *mut f64, *mut i32);
pub type TakeInputFn = unsafe extern "C" fn(*mut c_void, u32);
pub type CaptureFn = unsafe extern "C" fn(*mut c_void, i32);
pub type RamPtrFn = unsafe extern "C" fn(*mut c_void) -> *mut u8;
pub type CallGameFn = unsafe extern "C" fn(*mut c_void, u32, *const Value, u32, *mut Value) -> i32;
pub type RelocateFn = unsafe extern "C" fn(*mut c_void, u32, *mut u32) -> i32;
pub type RelocateLevelFn = unsafe extern "C" fn(*mut c_void, i32, u32, *mut u32) -> i32;
pub type LoadedLevelFn = unsafe extern "C" fn(*mut c_void) -> i32;
pub type StackPushFn = unsafe extern "C" fn(*mut c_void, u32, *mut u32) -> u32;
pub type StackPopFn = unsafe extern "C" fn(*mut c_void, u32);

/// `OmniHost` (296 bytes on 64-bit, ABI 1.2; the first 256 are ABI 1.1, the first 200 ABI 1.0), field for field in the header's order. A NULL entry means
/// "not supported on this port".
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Host {
	pub size: u32,
	pub abi_version: u32,
	pub user: *mut c_void,
	pub game: *const c_char,
	pub disc: *const c_char,
	pub platform: *const c_char,
	pub port: *const c_char,
	pub api_major: u32,
	pub api_minor: u32,
	pub ram_size: u32,
	pub purist: i32,
	pub log: Option<LogFn>,
	pub read_mem: Option<ReadFn>,
	pub write_mem: Option<WriteFn>,
	pub supports_field: Option<NameFn>,
	pub base_address: Option<BaseFn>,
	pub pvars: Option<PvarsFn>,
	pub supports_hook: Option<NameFn>,
	pub install_hook: Option<InstallFn>,
	pub uninstall_hook: Option<UninstallFn>,
	pub call_original: Option<OriginalFn>,
	pub key: Option<NameFn>,
	pub pressed: Option<NameFn>,
	pub mouse: Option<MouseFn>,
	pub window: Option<WindowFn>,
	pub pointer: Option<PointerFn>,
	pub take_input: Option<TakeInputFn>,
	pub capture_mouse: Option<CaptureFn>,
	// ---- ABI 1.1: native mods (native_host.rs) ----
	pub ram_ptr: Option<RamPtrFn>,
	pub call_game: Option<CallGameFn>,
	pub relocate: Option<RelocateFn>,
	pub relocate_level: Option<RelocateLevelFn>,
	pub loaded_level: Option<LoadedLevelFn>,
	pub stack_push: Option<StackPushFn>,
	pub stack_pop: Option<StackPopFn>,
	// ---- ABI 1.2 (size 296): big gameplay mods. NULL, or beyond `size`: not supported.
	pub draw_submit: Option<unsafe extern "C" fn(user: *mut c_void, cmds: *const OmniDrawCmd, n: u32, text_pool: *const c_char, text_len: u32)>,
	pub world_to_screen: Option<unsafe extern "C" fn(user: *mut c_void, xyz: *const f32, out_xy: *mut f32, in_front: *mut i32) -> i32>,
	pub raycast: Option<unsafe extern "C" fn(user: *mut c_void, from3: *const f32, to3: *const f32, out6: *mut f32) -> i32>,
	pub icon_count: Option<unsafe extern "C" fn(user: *mut c_void) -> u32>,
	pub icon_name: Option<unsafe extern "C" fn(user: *mut c_void, i: u32) -> *const c_char>,
	// ---- ABI 1.3 (size 312): moby queries. NULL, or beyond `size`: not supported.
	pub moby_query: Option<unsafe extern "C" fn(user: *mut c_void, q: *const OmniMobyQuery, out: *mut u32, cap: u32) -> i32>,
	pub moby_valid: Option<unsafe extern "C" fn(user: *mut c_void, addr: u32) -> i32>,
	// ---- ABI 1.4 (size 352): pose overrides (pose.rs, pose_host.rs). NULL, or beyond `size`: not supported.
	pub pose_set: Option<unsafe extern "C" fn(user: *mut c_void, moby: u32, ops: *const OmniPoseOp, n: u32) -> i32>,
	pub pose_info: Option<unsafe extern "C" fn(user: *mut c_void, moby: u32, parent: *mut u16, partner: *mut u16, cap: u32) -> i32>,
	pub pose_read: Option<unsafe extern "C" fn(user: *mut c_void, moby: u32, joint: u32, flags: u32, out12: *mut f32) -> i32>,
	pub pose_snapshot: Option<unsafe extern "C" fn(user: *mut c_void, moby: u32, first: u32, last_excl: u32) -> i32>,
	pub pose_debug: Option<unsafe extern "C" fn(user: *mut c_void, moby: u32, on: i32) -> i32>,
	// ---- ABI 1.6 (size 360): follow poses (pose_follow.rs, viewer/follow.rs). NULL, or beyond `size`: not supported.
	pub pose_follow: Option<unsafe extern "C" fn(user: *mut c_void, item: u32, source: u32, from_joint: u32, to_joint: u32, flags: u32) -> i32>,
}

/// One joint override (ABI 1.4, 48 bytes); see include/omniloader.h.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OmniPoseOp {
	pub joint: u32,
	pub op: u32,
	pub source: u32,
	pub flags: u32,
	pub q: [f32; 4],
	pub weight: f32,
	pub blend_ticks: u32,
	pub duration_ticks: u32,
	pub reserved: u32,
}

/// A moby query (ABI 1.3, 104 bytes); see include/omniloader.h.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct OmniMobyQuery {
	pub flags: u32,
	pub n_classes: u32,
	pub classes: [i32; 16],
	pub near: [f32; 3],
	pub radius: f32,
	pub max: u32,
	pub reserved: [u32; 3],
}

/// One mod drawing for this frame (ABI 1.2, 64 bytes); see include/omniloader.h.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct OmniDrawCmd {
	pub kind: u32,
	pub layer: u32,
	pub align: u32,
	pub mod_index: u32,
	pub x: f32,
	pub y: f32,
	pub w: f32,
	pub h: f32,
	pub color: u32,
	pub color2: u32,
	pub value: f32,
	pub icon: u32,
	pub text_off: u32,
	pub text_len: u32,
	pub reserved0: u32,
	pub reserved1: u32,
}

/// `OmniConfig` (24 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Config {
	pub size: u32,
	pub budget_us: u32,
	pub memory_limit: u64,
	pub status_path: *const c_char,
}

/// ABI 1.0: through `capture_mouse`.
pub const HOST_SIZE_V1: usize = 200;
/// ABI 1.1: through `stack_pop`. The size a host sets when the DLL's `omni_abi_minor` is 1 or more.
pub const HOST_SIZE_V1_1: usize = 256;
/// ABI 1.2: through `icon_name`. The size a host sets when the DLL's `omni_abi_minor` is 2 or more.
pub const HOST_SIZE_V1_2: usize = 296;
/// ABI 1.3: through `moby_valid`. The size a host sets when the DLL's `omni_abi_minor` is 3 or more.
pub const HOST_SIZE_V1_3: usize = 312;
/// ABI 1.4: through `pose_debug` (five more entries: 312 + 5 x 8). The size a host sets when the DLL's `omni_abi_minor` is 4 or more.
pub const HOST_SIZE_V1_4: usize = 352;
/// ABI 1.6: through `pose_follow` (one more entry: 352 + 8; ABI 1.5 added no entry and kept 352). The size a host sets when the DLL's `omni_abi_minor` is 6 or more.
pub const HOST_SIZE_V1_6: usize = 360;
pub const VALUE_SIZE: usize = 24;
/// ABI 1.5: bit 0 of `OmniValue.reserved` on the value `call_original` returns = the original gave no result (no struct size change).
pub const OMNI_VALUE_NO_RESULT: i32 = 1;

/// The DLL's functions, valid while the library is held.
pub struct Dll {
	_lib: Library,
	/// `omni_abi_minor()`; 0 when the DLL has no such function (ABI 1.0).
	minor: u32,
	abi_version: unsafe extern "C" fn() -> u32,
	version: unsafe extern "C" fn() -> *const c_char,
	last_error: unsafe extern "C" fn() -> *const c_char,
	create: unsafe extern "C" fn(*const c_char, *const Host, *const Config, *mut *mut c_void) -> i32,
	load: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> i32,
	tick: unsafe extern "C" fn(*mut c_void),
	pad: unsafe extern "C" fn(*mut c_void, u32, *mut u8, u32),
	dispatch: unsafe extern "C" fn(*mut c_void, u32, *const Value, u32, *mut Value) -> i32,
	hook_count: unsafe extern "C" fn(*const c_void) -> u32,
	hook_id: unsafe extern "C" fn(*const c_void, u32) -> *const c_char,
	hook_signature: unsafe extern "C" fn(*const c_void, u32) -> *const c_char,
	hooked: unsafe extern "C" fn(*const c_void, *mut u32, u32) -> u32,
	status_json: unsafe extern "C" fn(*const c_void) -> *mut c_char,
	free_string: unsafe extern "C" fn(*mut c_char),
	destroy: unsafe extern "C" fn(*mut c_void),
	/// ABI 1.2 exports; `None` on a DLL that predates them.
	event: Option<EventFn>,
	storage_dir: Option<StorageDirFn>,
	set_setting: Option<SetSettingFn>,
	settings_json: Option<SettingsJsonFn>,
}

type EventFn = unsafe extern "C" fn(*mut c_void, *const c_char, i64, *const c_char) -> i32;
type StorageDirFn = unsafe extern "C" fn(*mut c_void, *const c_char) -> i32;
type SetSettingFn = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, *const c_char) -> i32;
type SettingsJsonFn = unsafe extern "C" fn(*mut c_void, *mut *mut c_char) -> i32;

/// The host size to announce for a DLL that speaks minor ABI step `minor` (the DLL reads only what it knows, up to `size`).
pub fn host_size_for(minor: u32) -> usize {
	match minor {
		0 => HOST_SIZE_V1,
		1 => HOST_SIZE_V1_1,
		2 => HOST_SIZE_V1_2,
		3 => HOST_SIZE_V1_3,
		4 | 5 => HOST_SIZE_V1_4,
		_ => HOST_SIZE_V1_6,
	}
}

/// An optional export (absent in older DLLs).
macro_rules! opt_sym {
	($lib:expr, $name:literal, $ty:ty) => {
		// SAFETY: the declared type is the signature in omniloader.h.
		unsafe { $lib.get::<$ty>(concat!($name, "\0").as_bytes()) }.ok().map(|s| *s)
	};
}

fn text(p: *const c_char) -> String {
	if p.is_null() {
		return String::new();
	}
	// SAFETY: the DLL returns NUL-terminated strings that stay valid for the call that returned them.
	unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

macro_rules! sym {
	($lib:expr, $name:literal) => {
		// SAFETY: the declared type is the signature in omniloader.h.
		*unsafe { $lib.get(concat!($name, "\0").as_bytes()) }.map_err(|e| format!("{} is not exported: {e}", $name))?
	};
}

/// The structs of another ABI major are not these: refused (the minor steps are negotiated by `omni_abi_minor`).
pub(super) fn abi_check(v: u32) -> Result<(), String> {
	if v != ABI_VERSION {
		return Err(format!("the DLL speaks ABI {v}, this game needs {ABI_VERSION}"));
	}
	Ok(())
}

impl Dll {
	pub fn open(path: &Path) -> Result<Dll, String> {
		// SAFETY: loading OmniLoader's DLL; its initialisers only set up Rust statics.
		let lib = unsafe { Library::new(path) }.map_err(|e| format!("cannot load {}: {e}", path.display()))?;
		// SAFETY: the declared type is the signature in omniloader.h; an ABI 1.0 DLL does not export it.
		let minor_fn: Option<unsafe extern "C" fn() -> u32> = unsafe { lib.get(b"omni_abi_minor\0") }.ok().map(|s: libloading::Symbol<unsafe extern "C" fn() -> u32>| *s);
		let dll = Dll {
			// SAFETY: plain function without arguments.
			minor: minor_fn.map_or(0, |f| unsafe { f() }),
			abi_version: sym!(lib, "omni_abi_version"),
			version: sym!(lib, "omni_version"),
			last_error: sym!(lib, "omni_last_error"),
			create: sym!(lib, "omni_create"),
			load: sym!(lib, "omni_load"),
			tick: sym!(lib, "omni_tick"),
			pad: sym!(lib, "omni_pad"),
			dispatch: sym!(lib, "omni_dispatch"),
			hook_count: sym!(lib, "omni_hook_count"),
			hook_id: sym!(lib, "omni_hook_id"),
			hook_signature: sym!(lib, "omni_hook_signature"),
			hooked: sym!(lib, "omni_hooked"),
			status_json: sym!(lib, "omni_status_json"),
			free_string: sym!(lib, "omni_free_string"),
			destroy: sym!(lib, "omni_destroy"),
			event: opt_sym!(lib, "omni_event", EventFn),
			storage_dir: opt_sym!(lib, "omni_storage_dir", StorageDirFn),
			set_setting: opt_sym!(lib, "omni_set_setting", SetSettingFn),
			settings_json: opt_sym!(lib, "omni_settings_json", SettingsJsonFn),
			_lib: lib,
		};
		// SAFETY: plain function without arguments.
		abi_check(unsafe { (dll.abi_version)() })?;
		Ok(dll)
	}

	pub fn version(&self) -> String {
		// SAFETY: static NUL-terminated string.
		text(unsafe { (self.version)() })
	}

	/// The DLL's minor ABI step inside ABI 1 (1: it reads the native-mod host entries).
	pub fn minor(&self) -> u32 {
		self.minor
	}

	pub fn last_error(&self) -> String {
		// SAFETY: valid until the next failing call on this thread.
		text(unsafe { (self.last_error)() })
	}
}

/// A created loader. All calls happen one at a time (the game's worker, then its EE thread, never both): `Send + Sync` is
/// only about handing the pointer between those threads.
pub struct Loader {
	dll: Dll,
	ptr: *mut c_void,
}

// SAFETY: see the type's doc; the omni module calls it from one thread at a time.
unsafe impl Send for Loader {}
unsafe impl Sync for Loader {}

impl Loader {
	/// `omni_create`; the loader, or the DLL's error text.
	pub fn create(dll: Dll, binding: &str, host: &Host, config: &Config) -> Result<Loader, String> {
		let binding = CString::new(binding).map_err(|_| "the binding has a NUL byte".to_string())?;
		let mut out: *mut c_void = std::ptr::null_mut();
		// An ABI 1.0 DLL reads 200 bytes; one that speaks 1.1 reads the native-mod entries (up to `size`).
		let mut host = *host;
		host.size = host_size_for(dll.minor) as u32;
		// A loader that speaks ABI 1.4 (pose overrides) is told the port's mod API is 1.4, so mods that need it load; one that speaks 1.5
		// (`call_original` flags "no result" in `OmniValue.reserved` bit 0) is told 1.5, one that speaks 1.6 (`pose_follow`) 1.6; an older one keeps its default.
		if dll.minor >= 4 {
			(host.api_major, host.api_minor) = (1, dll.minor.clamp(4, 6));
		}
		// SAFETY: the pointers are valid for the call; the DLL copies what it keeps.
		let code = unsafe { (dll.create)(binding.as_ptr(), &host, config, &mut out) };
		if code != 0 || out.is_null() {
			return Err(format!("omni_create failed ({code}): {}", dll.last_error()));
		}
		Ok(Loader { dll, ptr: out })
	}

	/// `omni_load`; `plugins` is the plugins.json text.
	pub fn load(&self, mods_dir: &Path, plugins: &str) -> Result<(), String> {
		let dir = CString::new(mods_dir.to_string_lossy().as_bytes()).map_err(|_| "the mods folder name has a NUL byte".to_string())?;
		let pl = CString::new(plugins).map_err(|_| "plugins.json has a NUL byte".to_string())?;
		// SAFETY: valid loader and C strings.
		let code = unsafe { (self.dll.load)(self.ptr, dir.as_ptr(), pl.as_ptr()) };
		if code != 0 {
			return Err(format!("omni_load failed ({code}): {}", self.dll.last_error()));
		}
		Ok(())
	}

	pub fn tick(&self) {
		// SAFETY: valid loader.
		unsafe { (self.dll.tick)(self.ptr) }
	}

	pub fn pad(&self, port: u32, report: &mut [u8]) {
		// SAFETY: valid loader; `report` is writable for its length.
		unsafe { (self.dll.pad)(self.ptr, port, report.as_mut_ptr(), report.len() as u32) }
	}

	/// `omni_dispatch`: the result code and the value the mods' chain returned.
	pub fn dispatch(&self, index: u32, args: &[Value]) -> (i32, Value) {
		let mut ret = Value::default();
		// SAFETY: valid loader; `args` and `ret` are valid for the call.
		let code = unsafe { (self.dll.dispatch)(self.ptr, index, args.as_ptr(), args.len() as u32, &mut ret) };
		(code, ret)
	}

	pub fn hook_count(&self) -> u32 {
		// SAFETY: valid loader.
		unsafe { (self.dll.hook_count)(self.ptr) }
	}

	pub fn hook_id(&self, index: u32) -> String {
		// SAFETY: the string lives as long as the loader.
		text(unsafe { (self.dll.hook_id)(self.ptr, index) })
	}

	pub fn hook_signature(&self, index: u32) -> String {
		// SAFETY: as above.
		text(unsafe { (self.dll.hook_signature)(self.ptr, index) })
	}

	/// The indices of the hook points the loaded mods use.
	pub fn hooked(&self) -> Vec<u32> {
		let mut out = vec![u32::MAX; 256];
		// SAFETY: valid loader; `out` holds 256.
		let total = unsafe { (self.dll.hooked)(self.ptr, out.as_mut_ptr(), out.len() as u32) } as usize;
		out.truncate(total.min(256));
		out
	}

	/// status.json as text.
	pub fn status(&self) -> String {
		// SAFETY: valid loader; the string is ours until freed.
		unsafe {
			let p = (self.dll.status_json)(self.ptr);
			if p.is_null() {
				return String::new();
			}
			let s = text(p);
			(self.dll.free_string)(p);
			s
		}
	}

	pub fn version(&self) -> String {
		self.dll.version()
	}

	/// True when the DLL exports `omni_event` (ABI 1.2).
	pub fn has_events(&self) -> bool {
		self.dll.event.is_some()
	}

	/// `omni_event`: tell the mods about a game event. False when the DLL has no such export or the call failed.
	pub fn event(&self, name: &str, a: i64, text: &str) -> bool {
		let Some(f) = self.dll.event else { return false };
		let (Ok(n), Ok(t)) = (CString::new(name), CString::new(text)) else { return false };
		// SAFETY: valid loader and C strings.
		unsafe { f(self.ptr, n.as_ptr(), a, t.as_ptr()) == 0 }
	}

	/// `omni_storage_dir`: the folder of the per-save slot files; call it before `load`. False when the DLL lacks the export.
	pub fn storage_dir(&self, dir: &Path) -> bool {
		let Some(f) = self.dll.storage_dir else { return false };
		let Ok(d) = CString::new(dir.to_string_lossy().as_bytes()) else { return false };
		// SAFETY: valid loader and C string.
		unsafe { f(self.ptr, d.as_ptr()) == 0 }
	}

	/// True when the DLL exports `omni_set_setting` (ABI 1.2); used by the settings page stage.
	pub fn has_settings(&self) -> bool {
		self.dll.set_setting.is_some() && self.dll.settings_json.is_some()
	}

	/// `omni_set_setting` (ABI 1.2): `value_json` is the JSON text of the new value. What `omni.setting(key)` returns changes at once and
	/// the mod's `"setting"` callbacks run. Err = no such export, or the loader's reason.
	pub fn set_setting(&self, mod_id: &str, key: &str, value_json: &str) -> Result<(), String> {
		let Some(f) = self.dll.set_setting else { return Err("this omniloader.dll has no omni_set_setting".to_string()) };
		let (Ok(m), Ok(k), Ok(v)) = (CString::new(mod_id), CString::new(key), CString::new(value_json)) else { return Err("a NUL byte in the setting".to_string()) };
		// SAFETY: valid loader and C strings.
		match unsafe { f(self.ptr, m.as_ptr(), k.as_ptr(), v.as_ptr()) } {
			0 => Ok(()),
			code => Err(format!("omni_set_setting failed ({code}): {}", self.dll.last_error())),
		}
	}

	/// `omni_settings_json` (ABI 1.2): `{"mods":[{"id","name","version","loaded","settings":[..],"presets":[..]}]}` in load order; None when
	/// the DLL lacks the export or the call failed.
	pub fn settings_json(&self) -> Option<String> {
		let f = self.dll.settings_json?;
		let mut out: *mut c_char = std::ptr::null_mut();
		// SAFETY: valid loader; the DLL allocates the string and `free_string` releases it.
		unsafe {
			if f(self.ptr, &mut out) != 0 || out.is_null() {
				return None;
			}
			let s = text(out);
			(self.dll.free_string)(out);
			Some(s)
		}
	}
}
