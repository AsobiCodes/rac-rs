//! One XInput controller (Windows; xinput1_4.dll loaded by name, so no new dependency and nothing on other systems). From rac1
//! retail/prompts.rs (engine split stage E5).
use super::pad::XRaw;

#[cfg(windows)]
mod sys {
	use super::super::pad::XRaw;
	use std::ffi::c_void;
	use std::sync::atomic::{AtomicU32, Ordering};
	use std::sync::{Once, OnceLock};
	use std::time::Duration;

	#[repr(C)]
	#[derive(Default, Clone, Copy)]
	struct Gamepad {
		buttons: u16,
		lt: u8,
		rt: u8,
		lx: i16,
		ly: i16,
		rx: i16,
		ry: i16,
	}
	#[repr(C)]
	#[derive(Default, Clone, Copy)]
	struct State {
		packet: u32,
		pad: Gamepad,
	}
	type GetState = unsafe extern "system" fn(u32, *mut State) -> u32;
	#[link(name = "kernel32")]
	unsafe extern "system" {
		fn LoadLibraryA(name: *const u8) -> *mut c_void;
		fn GetProcAddress(m: *mut c_void, name: *const u8) -> *mut c_void;
	}
	fn get_state() -> Option<GetState> {
		static F: OnceLock<Option<GetState>> = OnceLock::new();
		*F.get_or_init(|| unsafe {
			for dll in [&b"xinput1_4.dll\0"[..], &b"xinput1_3.dll\0"[..], &b"xinput9_1_0.dll\0"[..]] {
				let m = LoadLibraryA(dll.as_ptr());
				if m.is_null() {
					continue;
				}
				let f = GetProcAddress(m, b"XInputGetState\0".as_ptr());
				if !f.is_null() {
					return Some(std::mem::transmute::<*mut c_void, GetState>(f));
				}
			}
			None
		})
	}

	/// The slot of the controller in use; NONE while there is none.
	static SLOT: AtomicU32 = AtomicU32::new(NONE);
	const NONE: u32 = u32::MAX;

	/// The first slot with a connected controller.
	fn find(f: GetState) -> Option<u32> {
		(0..4).find(|&i| unsafe { f(i, &mut State::default()) } == 0)
	}

	/// The connected controller. The first call looks at once; after that a missing one (switched on, plugged in or woken
	/// up later, or lost) is looked for once a second on a thread of its own, since polling an empty slot is slow.
	pub fn raw() -> Option<XRaw> {
		let f = get_state()?;
		static SEARCH: Once = Once::new();
		SEARCH.call_once(|| {
			if let Some(s) = find(f) {
				SLOT.store(s, Ordering::Relaxed);
			}
			let _ = std::thread::Builder::new().name("xinput search".into()).spawn(move || loop {
				std::thread::sleep(Duration::from_secs(1));
				if SLOT.load(Ordering::Relaxed) == NONE {
					if let Some(s) = find(f) {
						SLOT.store(s, Ordering::Relaxed);
					}
				}
			});
		});
		let s = SLOT.load(Ordering::Relaxed);
		if s == NONE {
			return None;
		}
		let mut st = State::default();
		if unsafe { f(s, &mut st) } != 0 {
			let _ = SLOT.compare_exchange(s, NONE, Ordering::Relaxed, Ordering::Relaxed);
			return None;
		}
		let p = st.pad;
		Some(XRaw { buttons: p.buttons, lt: p.lt, rt: p.rt, lx: p.lx, ly: p.ly, rx: p.rx, ry: p.ry })
	}
}

#[cfg(windows)]
pub fn connected() -> bool {
	sys::raw().is_some()
}
#[cfg(windows)]
pub fn raw() -> Option<XRaw> {
	sys::raw()
}
#[cfg(not(windows))]
pub fn connected() -> bool {
	false
}
#[cfg(not(windows))]
pub fn raw() -> Option<XRaw> {
	None
}
