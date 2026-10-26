//! ReShade add-on ABI as far as the renderer uses it: the exported functions, the event ids and the virtual-table slots of
//! `effect_runtime` and `command_queue`. Facts only (no header text), verified against crosire/reshade v6.8.0 `include/` (API
//! version 20) and layout-identical for v6.0.0 (API 10) .. 6.7.3; v5.9.2 (API 8) differs, so 10 is the oldest API tried.
//!
//! MSVC single inheritance: the base classes' virtual functions come first. `api_object` has 3 (get_native, get_private_data,
//! set_private_data: reshade_api_device.hpp:275/280/287) and `device_object` 1 (get_device: :664), so an own index n of
//! `effect_runtime` or `command_queue` is slot `BASE + n` of the table (`device` derives from `api_object` only: 3 + n). Calls go through `extern "system"` function pointers
//! (the Win64 calling convention; `this` first). Nothing here touches a real ReShade: the tests drive a fake table.

use std::ffi::{c_char, c_void, CStr, CString};

/// reshade.hpp:13 RESHADE_API_VERSION; the newest API this build speaks.
pub const API_NEWEST: u32 = 20;
/// The oldest API with the slot layout below (v6.0.0 = API 10).
pub const API_OLDEST: u32 = 10;
/// reshade_events.hpp:197 init_effect_runtime, callback `void(effect_runtime*)`.
pub const EV_INIT_RUNTIME: u32 = 9;
/// reshade_events.hpp:203 destroy_effect_runtime, callback `void(effect_runtime*)`.
pub const EV_DESTROY_RUNTIME: u32 = 10;
/// reshade_events.hpp:1707 reshade_begin_effects, callback `void(effect_runtime*, command_list*, resource_view rtv, resource_view rtv_srgb)`.
pub const EV_BEGIN_EFFECTS: u32 = 76;

/// Slots taken by `api_object` (3) and `device_object` (1) before an `effect_runtime` / `command_queue` own function.
const BASE: usize = 4;
/// reshade_api_device.hpp:664 device_object::get_device (own index 0 after api_object's 3), the device of an `effect_runtime`.
pub const RT_GET_DEVICE: usize = 3;
/// reshade_api_device.hpp:384 device::create_resource_view(resource, usage, desc, out_view) (own index 8 after api_object's 3).
pub const D_CREATE_VIEW: usize = 3 + 8;
/// reshade_api_device.hpp:388 device::destroy_resource_view(view) (own index 9).
pub const D_DESTROY_VIEW: usize = 3 + 9;
/// reshade_api.hpp:81 effect_runtime::get_command_queue (own index 4).
pub const RT_GET_COMMAND_QUEUE: usize = BASE + 4;
/// reshade_api.hpp:96 effect_runtime::render_effects(cmd_list, rtv, rtv_srgb) (own index 5).
pub const RT_RENDER_EFFECTS: usize = BASE + 5;
/// reshade_api.hpp:523 effect_runtime::update_texture_bindings(semantic, srv, srv_srgb) (own index 42).
pub const RT_UPDATE_TEXTURE_BINDINGS: usize = BASE + 42;
/// reshade_api.hpp:644 effect_runtime::get_preprocessor_definition(name, value, value_size) (own index 53).
pub const RT_GET_DEFINITION: usize = BASE + 53;
/// reshade_api.hpp:656 effect_runtime::set_preprocessor_definition(name, value) (own index 54).
pub const RT_SET_DEFINITION: usize = BASE + 54;
/// reshade_api_device.hpp:1230 command_queue::flush_immediate_command_list (own index 2).
pub const Q_FLUSH_IMMEDIATE: usize = BASE + 2;
/// reshade_api_device.hpp:1236 command_queue::get_immediate_command_list (own index 3).
pub const Q_GET_IMMEDIATE: usize = BASE + 3;

/// reshade_api_resource.hpp:254 resource_usage::render_target.
pub const USAGE_RENDER_TARGET: u32 = 0x4;
/// reshade_api_resource.hpp:268 resource_usage::shader_resource (pixel | non-pixel).
pub const USAGE_SHADER_RESOURCE: u32 = 0xC0;
/// reshade_api_format.hpp `format` values the views use.
pub const FMT_R8G8B8A8_UNORM: u32 = 28;
pub const FMT_R8G8B8A8_UNORM_SRGB: u32 = 29;
pub const FMT_R32_FLOAT: u32 = 41;
pub const FMT_B8G8R8A8_UNORM: u32 = 87;
pub const FMT_B8G8R8A8_UNORM_SRGB: u32 = 91;
/// reshade_api_resource.hpp:395 resource_view_type::texture_2d.
const VIEW_TEXTURE_2D: u32 = 4;

/// `api::resource_view_desc` (reshade_api_resource.hpp:414) for a texture view: type, format, then the union's `texture`
/// member (the union is 16 bytes at offset 8, 24 bytes in all).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ViewDesc {
	ty: u32,
	format: u32,
	first_level: u32,
	levels: u32,
	first_layer: u32,
	layers: u32,
}

impl ViewDesc {
	/// A 2D view of the first mip level and layer in `format`.
	pub const fn texture_2d(format: u32) -> ViewDesc {
		ViewDesc { ty: VIEW_TEXTURE_2D, format, first_level: 0, levels: 1, first_layer: 0, layers: 1 }
	}
}

/// One slot of the object's virtual table, as a function pointer of type `F`.
///
/// # Safety
/// `obj` must point to a C++ object whose first word is a vtable pointer with more than `idx` entries, and `F` must be the
/// right signature for that slot.
unsafe fn slot<F: Copy>(obj: *mut c_void, idx: usize) -> F {
	unsafe {
		let vt = *(obj as *const *const usize);
		std::mem::transmute_copy::<usize, F>(&*vt.add(idx))
	}
}

/// An `effect_runtime*` (ReShade owns it; valid from init_effect_runtime to destroy_effect_runtime).
#[derive(Clone, Copy)]
pub struct Runtime(pub *mut c_void);

/// A `command_queue*`.
#[derive(Clone, Copy)]
pub struct Queue(pub *mut c_void);

impl Runtime {
	/// # Safety
	/// The runtime must be alive.
	pub unsafe fn command_queue(self) -> Queue {
		let f: unsafe extern "system" fn(*mut c_void) -> *mut c_void = unsafe { slot(self.0, RT_GET_COMMAND_QUEUE) };
		Queue(unsafe { f(self.0) })
	}

	/// # Safety
	/// The runtime must be alive; `cmd_list` a ReShade command list; the views of the swapchain's size, the resource in the render_target state.
	pub unsafe fn render_effects(self, cmd_list: *mut c_void, rtv: u64, rtv_srgb: u64) {
		let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, u64) = unsafe { slot(self.0, RT_RENDER_EFFECTS) };
		unsafe { f(self.0, cmd_list, rtv, rtv_srgb) }
	}

	/// # Safety
	/// The runtime must be alive; the views valid shader resource views of ReShade's device.
	pub unsafe fn update_texture_bindings(self, semantic: &CStr, srv: u64, srv_srgb: u64) {
		let f: unsafe extern "system" fn(*mut c_void, *const c_char, u64, u64) = unsafe { slot(self.0, RT_UPDATE_TEXTURE_BINDINGS) };
		unsafe { f(self.0, semantic.as_ptr(), srv, srv_srgb) }
	}

	/// The value of a preprocessor definition (None: not defined).
	///
	/// # Safety
	/// The runtime must be alive.
	pub unsafe fn definition(self, name: &CStr) -> Option<String> {
		let f: unsafe extern "system" fn(*mut c_void, *const c_char, *mut c_char, *mut usize) -> bool = unsafe { slot(self.0, RT_GET_DEFINITION) };
		let mut buf = [0 as c_char; 128];
		let mut size = buf.len();
		if !unsafe { f(self.0, name.as_ptr(), buf.as_mut_ptr(), &mut size) } {
			return None;
		}
		Some(unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned())
	}

	/// # Safety
	/// The runtime must be alive.
	pub unsafe fn set_definition(self, name: &CStr, value: &str) {
		let f: unsafe extern "system" fn(*mut c_void, *const c_char, *const c_char) = unsafe { slot(self.0, RT_SET_DEFINITION) };
		let value = CString::new(value).unwrap_or_default();
		unsafe { f(self.0, name.as_ptr(), value.as_ptr()) }
	}
}

impl Runtime {
	/// The `api::device*` the runtime renders with.
	///
	/// # Safety
	/// The runtime must be alive.
	pub unsafe fn device(self) -> Device {
		let f: unsafe extern "system" fn(*mut c_void) -> *mut c_void = unsafe { slot(self.0, RT_GET_DEVICE) };
		Device(unsafe { f(self.0) })
	}
}

/// An `api::device*` (ReShade's device: on D3D12 its views live in ReShade's own CPU descriptor heaps).
#[derive(Clone, Copy)]
pub struct Device(pub *mut c_void);

impl Device {
	/// A view of `resource` (the native resource pointer / handle) that ReShade knows (valid for `render_effects` and
	/// `update_texture_bindings` in every build of ReShade). None when ReShade refuses.
	///
	/// # Safety
	/// The device must be alive; `resource` a native resource of this device that outlives the view.
	pub unsafe fn create_view(self, resource: u64, usage: u32, desc: &ViewDesc) -> Option<u64> {
		let f: unsafe extern "system" fn(*mut c_void, u64, u32, *const ViewDesc, *mut u64) -> bool = unsafe { slot(self.0, D_CREATE_VIEW) };
		let mut view = 0u64;
		(unsafe { f(self.0, resource, usage, desc, &mut view) } && view != 0).then_some(view)
	}

	/// # Safety
	/// The device must be alive; `view` made by `create_view` and not in use by a recorded command list.
	pub unsafe fn destroy_view(self, view: u64) {
		let f: unsafe extern "system" fn(*mut c_void, u64) = unsafe { slot(self.0, D_DESTROY_VIEW) };
		unsafe { f(self.0, view) }
	}
}

impl Queue {
	/// The immediate command list (null on a queue that cannot draw).
	///
	/// # Safety
	/// The queue must be alive.
	pub unsafe fn immediate_list(self) -> *mut c_void {
		let f: unsafe extern "system" fn(*mut c_void) -> *mut c_void = unsafe { slot(self.0, Q_GET_IMMEDIATE) };
		unsafe { f(self.0) }
	}

	/// # Safety
	/// The queue must be alive.
	pub unsafe fn flush_immediate(self) {
		let f: unsafe extern "system" fn(*mut c_void) = unsafe { slot(self.0, Q_FLUSH_IMMEDIATE) };
		unsafe { f(self.0) }
	}
}

/// The exports of the ReShade module that the add-on registration uses.
#[derive(Clone, Copy)]
pub struct Exports {
	/// `bool ReShadeRegisterAddon(HMODULE addon, uint32_t api_version)`.
	pub register: unsafe extern "system" fn(*mut c_void, u32) -> bool,
	/// `void ReShadeRegisterEventForAddon(HMODULE addon, uint32_t event, void *callback)`.
	pub register_event: unsafe extern "system" fn(*mut c_void, u32, *mut c_void),
	/// `void ReShadeUnregisterAddon(HMODULE addon)`: also drops every event callback of the add-on.
	pub unregister: unsafe extern "system" fn(*mut c_void),
}

#[cfg(windows)]
mod win {
	use std::ffi::c_void;

	unsafe extern "system" {
		pub fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
		pub fn GetModuleHandleW(name: *const u16) -> *mut c_void;
		pub fn GetCurrentProcess() -> *mut c_void;
		pub fn K32EnumProcessModules(process: *mut c_void, modules: *mut *mut c_void, bytes: u32, needed: *mut u32) -> i32;
	}
}

/// Finds the loaded ReShade: a module of this process that exports both ReShadeRegisterAddon and ReShadeUnregisterAddon (the
/// rule of reshade.hpp `get_reshade_module_handle`), with the exports resolved. None on a normal start (and off Windows).
pub fn find_reshade() -> Option<Exports> {
	#[cfg(windows)]
	unsafe {
		let mut modules = [std::ptr::null_mut::<c_void>(); 1024];
		let mut needed = 0u32;
		if win::K32EnumProcessModules(win::GetCurrentProcess(), modules.as_mut_ptr(), std::mem::size_of_val(&modules) as u32, &mut needed) == 0 {
			return None;
		}
		let n = (needed as usize / std::mem::size_of::<*mut c_void>()).min(modules.len());
		for &m in &modules[..n] {
			let (reg, unreg) = (win::GetProcAddress(m, c"ReShadeRegisterAddon".as_ptr().cast()), win::GetProcAddress(m, c"ReShadeUnregisterAddon".as_ptr().cast()));
			let ev = win::GetProcAddress(m, c"ReShadeRegisterEventForAddon".as_ptr().cast());
			if !reg.is_null() && !unreg.is_null() && !ev.is_null() {
				return Some(Exports {
					register: std::mem::transmute::<*mut c_void, unsafe extern "system" fn(*mut c_void, u32) -> bool>(reg),
					register_event: std::mem::transmute::<*mut c_void, unsafe extern "system" fn(*mut c_void, u32, *mut c_void)>(ev),
					unregister: std::mem::transmute::<*mut c_void, unsafe extern "system" fn(*mut c_void)>(unreg),
				});
			}
		}
	}
	None
}

/// The handle ReShade wants as the add-on's identity: this exe's module.
pub fn exe_module() -> *mut c_void {
	#[cfg(windows)]
	unsafe {
		return win::GetModuleHandleW(std::ptr::null());
	}
	#[cfg(not(windows))]
	std::ptr::null_mut()
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::Mutex;

	static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());
	fn say(s: String) {
		LOG.lock().unwrap().push(s);
	}

	unsafe extern "system" fn get_queue(this: *mut c_void) -> *mut c_void {
		say("get_command_queue".into());
		this.wrapping_add(8)
	}
	unsafe extern "system" fn render(_: *mut c_void, list: *mut c_void, rtv: u64, srgb: u64) {
		say(format!("render_effects {list:?} {rtv} {srgb}"));
	}
	unsafe extern "system" fn bind(_: *mut c_void, sem: *const c_char, srv: u64, srgb: u64) {
		say(format!("update_texture_bindings {} {srv} {srgb}", unsafe { CStr::from_ptr(sem) }.to_str().unwrap()));
	}
	unsafe extern "system" fn get_def(_: *mut c_void, name: *const c_char, value: *mut c_char, size: *mut usize) -> bool {
		let name = unsafe { CStr::from_ptr(name) }.to_str().unwrap();
		say(format!("get_preprocessor_definition {name} cap {}", unsafe { *size }));
		if name != "KNOWN" {
			return false;
		}
		unsafe { std::ptr::copy_nonoverlapping(c"23301.0".as_ptr(), value, 8) };
		true
	}
	unsafe extern "system" fn set_def(_: *mut c_void, name: *const c_char, value: *const c_char) {
		let (n, v) = unsafe { (CStr::from_ptr(name), CStr::from_ptr(value)) };
		say(format!("set_preprocessor_definition {} {}", n.to_str().unwrap(), v.to_str().unwrap()));
	}
	unsafe extern "system" fn get_device(this: *mut c_void) -> *mut c_void {
		say("get_device".into());
		this.wrapping_add(16)
	}
	unsafe extern "system" fn create_view(_: *mut c_void, res: u64, usage: u32, desc: *const ViewDesc, out: *mut u64) -> bool {
		let d = unsafe { *desc };
		say(format!("create_resource_view {res} {usage:#x} ty {} fmt {} levels {} layers {}", d.ty, d.format, d.levels, d.layers));
		unsafe { *out = if res == 0 { 0 } else { res + 1000 } };
		res != 0
	}
	unsafe extern "system" fn destroy_view(_: *mut c_void, view: u64) {
		say(format!("destroy_resource_view {view}"));
	}
	unsafe extern "system" fn flush(_: *mut c_void) {
		say("flush_immediate_command_list".into());
	}
	unsafe extern "system" fn immediate(_: *mut c_void) -> *mut c_void {
		say("get_immediate_command_list".into());
		0x1000 as *mut c_void
	}

	/// An object whose first word points to a table of `n` slots, only the used ones filled.
	fn object(slots: &[(usize, usize)], n: usize) -> (Box<[usize]>, Box<[usize; 1]>) {
		let mut table = vec![0usize; n].into_boxed_slice();
		for &(i, f) in slots {
			table[i] = f;
		}
		let obj = Box::new([table.as_ptr() as usize]);
		(table, obj)
	}

	#[test]
	fn slots_are_the_header_indices() {
		// Own index in the class + 4 (api_object 3 + device_object 1): the values the stage-1 research read from v6.8.0.
		assert_eq!((RT_GET_COMMAND_QUEUE, RT_RENDER_EFFECTS, RT_UPDATE_TEXTURE_BINDINGS, RT_GET_DEFINITION, RT_SET_DEFINITION), (8, 9, 46, 57, 58));
		assert_eq!((Q_FLUSH_IMMEDIATE, Q_GET_IMMEDIATE), (6, 7));
		assert_eq!((RT_GET_DEVICE, D_CREATE_VIEW, D_DESTROY_VIEW), (3, 11, 12));
		assert_eq!(std::mem::size_of::<ViewDesc>(), 24);
		assert_eq!((USAGE_RENDER_TARGET, USAGE_SHADER_RESOURCE, FMT_R32_FLOAT, FMT_B8G8R8A8_UNORM_SRGB), (4, 0xC0, 41, 91));
		assert_eq!((API_NEWEST, API_OLDEST, EV_INIT_RUNTIME, EV_DESTROY_RUNTIME, EV_BEGIN_EFFECTS), (20, 10, 9, 10, 76));
	}

	#[test]
	fn each_wrapper_calls_its_slot() {
		let (_t, rt_obj) = object(
			&[
				(RT_GET_COMMAND_QUEUE, get_queue as *const () as usize),
				(RT_RENDER_EFFECTS, render as *const () as usize),
				(RT_UPDATE_TEXTURE_BINDINGS, bind as *const () as usize),
				(RT_GET_DEFINITION, get_def as *const () as usize),
				(RT_SET_DEFINITION, set_def as *const () as usize),
				(RT_GET_DEVICE, get_device as *const () as usize),
			],
			80,
		);
		let (_dt, d_obj) = object(&[(D_CREATE_VIEW, create_view as *const () as usize), (D_DESTROY_VIEW, destroy_view as *const () as usize)], 50);
		let (_qt, q_obj) = object(&[(Q_FLUSH_IMMEDIATE, flush as *const () as usize), (Q_GET_IMMEDIATE, immediate as *const () as usize)], 14);
		LOG.lock().unwrap().clear();
		let rt = Runtime(rt_obj.as_ptr() as *mut c_void);
		unsafe {
			assert_eq!(rt.command_queue().0 as usize, rt.0 as usize + 8);
			let q = Queue(q_obj.as_ptr() as *mut c_void);
			assert_eq!(q.immediate_list() as usize, 0x1000);
			rt.render_effects(0x1000 as *mut c_void, 7, 8);
			rt.update_texture_bindings(c"DEPTH", 5, 5);
			assert_eq!(rt.definition(c"KNOWN").as_deref(), Some("23301.0"));
			assert_eq!(rt.definition(c"OTHER"), None);
			rt.set_definition(c"RESHADE_DEPTH_INPUT_IS_REVERSED", "0");
			q.flush_immediate();
			assert_eq!(rt.device().0 as usize, rt.0 as usize + 16);
			let d = Device(d_obj.as_ptr() as *mut c_void);
			assert_eq!(d.create_view(7, USAGE_SHADER_RESOURCE, &ViewDesc::texture_2d(FMT_R32_FLOAT)), Some(1007));
			assert_eq!(d.create_view(0, USAGE_RENDER_TARGET, &ViewDesc::texture_2d(FMT_B8G8R8A8_UNORM)), None);
			d.destroy_view(1007);
		}
		assert_eq!(
			*LOG.lock().unwrap(),
			[
				"get_command_queue",
				"get_immediate_command_list",
				"render_effects 0x1000 7 8",
				"update_texture_bindings DEPTH 5 5",
				"get_preprocessor_definition KNOWN cap 128",
				"get_preprocessor_definition OTHER cap 128",
				"set_preprocessor_definition RESHADE_DEPTH_INPUT_IS_REVERSED 0",
				"flush_immediate_command_list",
				"get_device",
				"create_resource_view 7 0xc0 ty 4 fmt 41 levels 1 layers 1",
				"create_resource_view 0 0x4 ty 4 fmt 87 levels 1 layers 1",
				"destroy_resource_view 1007"
			]
		);
	}

	#[test]
	fn a_normal_process_has_no_reshade() {
		assert!(find_reshade().is_none());
	}
}
