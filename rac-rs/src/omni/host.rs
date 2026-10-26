//! The host side of the OmniLoader C ABI: the `OmniHost` the DLL calls back into, on this game's memory and run log.
//! Level 1: `log`. Level 2: `read_mem`, `write_mem`, `supports_field`, `base_address`. Level 3 (hooks) is in hooks.rs.
//! Input (key, pressed, mouse, window, pointer, take_input, capture_mouse) is in input_host.rs.
//! ABI 1.1 (native mods: ram_ptr, call_game, relocate, relocate_level, loaded_level, stack_push, stack_pop) is in native_host.rs.
//! ABI 1.3 (`moby_query`, `moby_valid`) is in mobys.rs; ABI 1.4 (`pose_*`) in pose_host.rs. `pvars` is in pvars.rs; the overlay window (fields in level data) is in window.rs.
//!
//! Memory: the callbacks work on the machine's CPU through `CPU`, a raw pointer that `scope` sets for the span of one call into
//! the DLL (`tick`, `pad` between two fields; `dispatch` inside a hook; all on the retail worker thread). Nothing else touches the
//! machine while the DLL runs.

use super::binding::Binding;
use super::cabi::{Config, Host, ABI_VERSION, HOST_SIZE_V1_6};
use ee_oracle::cpu::Cpu;
use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};

/// What every callback finds through `user`: the binding and the disc. Leaked, lives as long as the process.
pub struct HostState {
	pub binding: Binding,
	/// The USA disc runs (only the pad trace asks).
	pub ntsc: bool,
	/// The disc string the mods are told ("pal" or "ntsc", layout.rs): also picks the numbering of `call_game` functions.
	pub disc: &'static str,
}

/// Both discs we run (PAL v1.00 and USA) have the data layout of the binding's "ntsc" column (layout.rs).
const LAYOUT_NTSC: bool = true;

static CPU: AtomicPtr<Cpu> = AtomicPtr::new(std::ptr::null_mut());

/// Log lines from mods are capped so a mod that logs every tick cannot flood the run log.
const LOG_CAP: u32 = 4000;
static LOGGED: AtomicU32 = AtomicU32::new(0);

/// Runs `f` (a call into the DLL) with `c` as the machine the callbacks read and write. `f` must not use `c` itself.
pub fn scope<R>(c: &mut Cpu, f: impl FnOnce() -> R) -> R {
	let prev = CPU.swap(c as *mut Cpu, Ordering::SeqCst);
	let r = f();
	CPU.store(prev, Ordering::SeqCst);
	r
}

/// The machine of the current `scope`, for the callbacks that need registers (hooks.rs).
pub(super) fn cpu<'a>() -> Option<&'a mut Cpu> {
	let p = CPU.load(Ordering::SeqCst);
	// SAFETY: set by `scope` from a live `&mut Cpu` that is not used until `scope` returns.
	(!p.is_null()).then(|| unsafe { &mut *p })
}

pub(super) fn text(p: *const c_char) -> String {
	if p.is_null() {
		return String::new();
	}
	// SAFETY: the DLL passes NUL-terminated strings.
	unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

pub(super) fn state<'a>(user: *mut c_void) -> &'a HostState {
	// SAFETY: `user` is the leaked HostState given to `omni_create`.
	unsafe { &*(user as *const HostState) }
}

unsafe extern "C" fn log_cb(_user: *mut c_void, level: i32, mod_id: *const c_char, msg: *const c_char) {
	let (id, msg) = (text(mod_id), text(msg));
	// The loader's `loaded` / `failed:` lines say which mod owns a take_input / capture_mouse (input.rs `Holds`); never dropped.
	super::input::note_log(level, &id, &msg);
	let n = LOGGED.fetch_add(1, Ordering::Relaxed);
	if n >= LOG_CAP {
		if n == LOG_CAP {
			eprintln!("omni: {LOG_CAP} log lines written, the rest is dropped");
		}
		return;
	}
	let lvl = match level {
		1 => " warning",
		2 => " error",
		_ => "",
	};
	if id.is_empty() {
		eprintln!("omni:{lvl} {msg}");
	} else {
		eprintln!("omni:{lvl} [{id}] {msg}");
	}
}

/// The real address behind `addr`: the overlay window (window.rs) is mapped to the level loaded now, every other address is itself.
fn real_addr(user: *mut c_void, c: &Cpu, addr: u32, len: u32) -> Option<u32> {
	if !super::window::in_window(addr) {
		return Some(addr);
	}
	super::window::translate(state(user).ntsc, super::native_host::level_word(c), addr, len)
}

unsafe extern "C" fn read_cb(user: *mut c_void, addr: u32, out: *mut u8, len: u32) -> i32 {
	let Some(c) = cpu() else { return 0 };
	let Some(addr) = real_addr(user, c, addr, len) else { return 0 };
	let (a, n) = (addr as usize, len as usize);
	if a.checked_add(n).is_none_or(|e| e > c.mem.ram.len()) {
		return 0;
	}
	// SAFETY: bounds checked above; `out` has `len` bytes.
	unsafe { std::ptr::copy_nonoverlapping(c.mem.ram.as_ptr().add(a), out, n) };
	1
}

unsafe extern "C" fn write_cb(user: *mut c_void, addr: u32, data: *const u8, len: u32) -> i32 {
	let Some(c) = cpu() else { return 0 };
	let Some(addr) = real_addr(user, c, addr, len) else { return 0 };
	let (a, n) = (addr as usize, len as usize);
	if a.checked_add(n).is_none_or(|e| e > c.mem.ram.len()) {
		return 0;
	}
	// SAFETY: bounds checked above; `data` has `len` bytes.
	unsafe { std::ptr::copy_nonoverlapping(data, c.mem.ram.as_mut_ptr().add(a), n) };
	1
}

unsafe extern "C" fn supports_field_cb(user: *mut c_void, name: *const c_char) -> i32 {
	let s = state(user);
	i32::from(s.binding.has_field(&text(name), LAYOUT_NTSC))
}

unsafe extern "C" fn base_address_cb(user: *mut c_void, base: *const c_char, out: *mut u32) -> i32 {
	let s = state(user);
	match s.binding.base(&text(base), LAYOUT_NTSC) {
		Some(a) => {
			// SAFETY: `out` is one u32 the DLL gave.
			unsafe { *out = a };
			1
		}
		None => 0,
	}
}

/// The `OmniHost` for this disc. `size` is the struct's own size; the entries not set stay NULL.
pub fn make(state: &'static HostState) -> Host {
	let leak = |s: String| -> *const c_char { CString::new(s).unwrap_or_default().into_raw() as *const c_char };
	// SAFETY: all-zero is valid for this struct (integers, raw pointers, Option of fn pointers).
	let mut h: Host = unsafe { std::mem::zeroed() };
	h.size = std::mem::size_of::<Host>() as u32;
	debug_assert_eq!(std::mem::size_of::<Host>(), HOST_SIZE_V1_6);
	h.abi_version = ABI_VERSION;
	h.user = state as *const HostState as *mut c_void;
	h.game = c"rac1".as_ptr();
	h.disc = if state.disc == "ntsc" { c"ntsc".as_ptr() } else { c"pal".as_ptr() };
	// The platform the port runs on (native mods pick their library by it: `plugin` table of the manifest), not the game's.
	h.platform = if cfg!(all(windows, target_arch = "x86_64")) {
		c"windows-x86_64".as_ptr()
	} else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
		c"linux-x86_64".as_ptr()
	} else {
		c"unknown".as_ptr()
	};
	h.port = leak(format!("rac-rs {}", env!("CARGO_PKG_VERSION")));
	h.ram_size = 32 * 1024 * 1024;
	h.purist = 0;
	h.log = Some(log_cb);
	h.read_mem = Some(read_cb);
	h.write_mem = Some(write_cb);
	h.supports_field = Some(supports_field_cb);
	h.base_address = Some(base_address_cb);
	h.pvars = Some(super::pvars::pvars_cb);
	super::hooks::fill(&mut h);
	super::input_host::fill(&mut h);
	super::native_host::fill(&mut h);
	super::draw::fill(&mut h);
	super::mobys::fill(&mut h, &state.binding);
	super::pose_host::fill(&mut h, &state.binding);
	h
}

/// The config: status.json goes next to the mods. Leaks the path string.
pub fn config(status_path: &std::path::Path) -> Config {
	let p = CString::new(status_path.to_string_lossy().as_bytes()).unwrap_or_default().into_raw() as *const c_char;
	Config { size: std::mem::size_of::<Config>() as u32, budget_us: 0, memory_limit: 0, status_path: p }
}
