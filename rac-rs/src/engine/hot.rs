//! [PC] The game's busy threads (the window's main loop, the retail worker, the GS capture) at above-normal priority and out
//! of Windows power throttling (EcoQoS), so a hybrid CPU (Intel 12th gen and later) keeps them on its fast cores and a laptop
//! does not run them at its power-saving clock; the process keeps its timer resolution (the frame pacing's short sleeps).
//! No retail counterpart. Nothing on other platforms, or on Windows before 10 1709 (no throttling control there).

#[cfg(windows)]
mod win {
	use std::ffi::c_void;

	/// THREAD_POWER_THROTTLING_STATE / PROCESS_POWER_THROTTLING_STATE.
	#[repr(C)]
	pub struct Throttle {
		pub version: u32,
		pub control: u32,
		pub state: u32,
	}

	#[link(name = "kernel32")]
	unsafe extern "system" {
		pub fn GetCurrentThread() -> *mut c_void;
		pub fn GetCurrentProcess() -> *mut c_void;
		pub fn SetThreadPriority(thread: *mut c_void, priority: i32) -> i32;
		fn GetModuleHandleA(name: *const u8) -> *mut c_void;
		fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
	}

	/// SetThreadInformation / SetProcessInformation (looked up: Windows 8 and 10 1709 added them).
	pub type SetInfo = unsafe extern "system" fn(*mut c_void, i32, *const c_void, u32) -> i32;

	pub fn lookup(name: &std::ffi::CStr) -> Option<SetInfo> {
		// SAFETY: kernel32 is loaded in every process; a found export has this signature.
		unsafe {
			let f = GetProcAddress(GetModuleHandleA(c"kernel32.dll".as_ptr().cast()), name.as_ptr().cast());
			(!f.is_null()).then(|| std::mem::transmute::<*mut c_void, SetInfo>(f))
		}
	}
}

/// Marks the calling thread busy.
pub fn thread() {
	#[cfg(windows)]
	// SAFETY: plain Win32 calls on the current thread's pseudo handle.
	unsafe {
		let t = win::GetCurrentThread();
		// THREAD_PRIORITY_ABOVE_NORMAL
		win::SetThreadPriority(t, 1);
		if let Some(set) = win::lookup(c"SetThreadInformation") {
			// ThreadPowerThrottling: execution speed controlled, not throttled.
			let s = win::Throttle { version: 1, control: 1, state: 0 };
			set(t, 3, (&s as *const win::Throttle).cast(), 12);
		}
	}
}

/// Marks the calling thread as background work (the GPU capture's first kernel builds): below-normal priority, so it leaves
/// the busy threads alone. Not power throttled: EcoQoS clocks it down so far on a non-hybrid laptop CPU that the kernels were
/// still not built after minutes of play (the capture stayed on the CPU path).
pub fn idle() {
	#[cfg(windows)]
	// SAFETY: plain Win32 calls on the current thread's pseudo handle.
	unsafe {
		let t = win::GetCurrentThread();
		// THREAD_PRIORITY_BELOW_NORMAL
		win::SetThreadPriority(t, -1);
		if let Some(set) = win::lookup(c"SetThreadInformation") {
			// ThreadPowerThrottling: execution speed controlled, not throttled.
			let s = win::Throttle { version: 1, control: 1, state: 0 };
			set(t, 3, (&s as *const win::Throttle).cast(), 12);
		}
	}
}

/// Takes the process out of power throttling (execution speed, and timer resolution requests stay honoured).
pub fn process() {
	#[cfg(windows)]
	if let Some(set) = win::lookup(c"SetProcessInformation") {
		// ProcessPowerThrottling: EXECUTION_SPEED | IGNORE_TIMER_RESOLUTION controlled, neither on.
		let s = win::Throttle { version: 1, control: 1 | 4, state: 0 };
		// SAFETY: the current process's pseudo handle and a struct of the size given.
		unsafe { set(win::GetCurrentProcess(), 4, (&s as *const win::Throttle).cast(), 12) };
	}
}
