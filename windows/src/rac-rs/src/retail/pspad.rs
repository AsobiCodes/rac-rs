//! PlayStation controllers (DualShock 4, DualSense, DualSense Edge) as gamepads, read directly over USB or Bluetooth.
//!
//! Windows sees them as HID devices, not as XInput controllers, so `prompts::poll_pad`'s XInput would miss them. A thread of
//! its own opens the first one (Sony, vendor 0x054C), reads its input reports and keeps the latest as an XInput report (`XRaw`),
//! so the same mapping, deadzones and device switching as with an Xbox controller apply, every button in its PS2 place (Cross =
//! Cross, Share / Create = Select, Options = Start). While none is open, the HID device list is looked at once a second and the
//! devices are opened only when the list changed. Nothing is sent to the controller (no light bar, no rumble) and nothing is
//! linked (hid.dll and cfgmgr32.dll are loaded by name, as xinput1_4.dll is).
//!
//! Report layouts (the controllers' HID input reports; the first byte is the report id):
//! - DualShock 4: report 0x01 (USB, and Bluetooth in its basic mode) holds the state from byte 1, Bluetooth reports
//!   0x11..=0x19 from byte 3. State: LX LY RX RY, buttons (hat in the low nibble, Square 0x10 Cross 0x20 Circle 0x40
//!   Triangle 0x80), buttons (L1 0x01 R1 0x02 L2 0x04 R2 0x08 Share 0x10 Options 0x20 L3 0x40 R3 0x80), PS / touchpad /
//!   counter, L2 and R2 analog.
//! - DualSense: USB report 0x01 and Bluetooth report 0x31 hold the state from byte 1 / byte 2: LX LY RX RY, L2 R2 analog,
//!   counter, then the same button bytes (Create in Share's place). Bluetooth report 0x01 (basic mode) has the DualShock 4
//!   layout.
//! - Sticks: 0 = up / left, 0x80 = centre, 0xFF = down / right (as on the PS2). The hat: 0 = up, clockwise in eighths, 8 = none.
//! - The DualShock 4 USB wireless adapter reports all-zero sticks while no controller is paired with it: read as an idle pad.
//!
//! Steam: a game started through Steam with its PlayStation support on gets Steam's virtual Xbox controller for the pad, and
//! Steam lists the devices it handles in `SDL_GAMECONTROLLER_IGNORE_DEVICES` ("0x054C/0x05C4,..."); those are left to Steam.
use super::prompts::XRaw;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once};

/// Sony's USB vendor id; the product ids of the DualShock 4 (both models and the wireless adapter) and the DualSense (and Edge).
const SONY: u16 = 0x054c;
const DS4: [u16; 3] = [0x05c4, 0x09cc, DS4_ADAPTER];
const DS5: [u16; 2] = [0x0ce6, 0x0df2];
const DS4_ADAPTER: u16 = 0x0ba0;

/// The hat as XInput d-pad bits (up 1, down 2, left 4, right 8).
const HAT: [u16; 9] = [1, 1 | 8, 8, 2 | 8, 2, 2 | 4, 4, 1 | 4, 0];
/// The first button byte's face buttons and the second byte's buttons as XInput bits (Square = X, Cross = A, Circle = B,
/// Triangle = Y; L1 = LB, R1 = RB, Share = Back, Options = Start, L3, R3). L2 / R2 come from the analog triggers.
const FACE: [(u8, u16); 4] = [(0x10, 0x4000), (0x20, 0x1000), (0x40, 0x2000), (0x80, 0x8000)];
const MISC: [(u8, u16); 6] = [(0x01, 0x100), (0x02, 0x200), (0x10, 0x20), (0x20, 0x10), (0x40, 0x40), (0x80, 0x80)];

/// A stick axis (0..=255, 0x80 centre) in XInput's range; `up` flips it (XInput's y is positive up, the pad's grows down).
fn axis(v: u8, up: bool) -> i16 {
	let d = v as i32 - 0x80;
	((if up { -d } else { d }) * 258).clamp(-32768, 32767) as i16
}

/// One input report (the bytes as read, report id first) as an XInput report; None for a report that holds no pad state.
/// `bt`: the controller is connected over Bluetooth.
pub fn parse(product: u16, bt: bool, r: &[u8]) -> Option<XRaw> {
	// Where the state starts, and whether it has the DualSense's full layout (triggers before the buttons).
	let (at, full) = match (*r.first()?, DS5.contains(&product)) {
		(0x01, false) => (1, false),
		(0x11..=0x19, false) => (3, false),
		(0x01, true) => (1, !bt),
		(0x31, true) => (2, true),
		_ => return None,
	};
	let s = r.get(at..at + 9)?;
	if product == DS4_ADAPTER && s[..4] == [0; 4] {
		return Some(XRaw::default());
	}
	let (b0, b1, lt, rt) = if full { (s[7], s[8], s[4], s[5]) } else { (s[4], s[5], s[7], s[8]) };
	let mut buttons = HAT.get((b0 & 0x0f) as usize).copied().unwrap_or(0);
	buttons |= FACE.iter().filter(|(m, _)| b0 & m != 0).fold(0, |a, (_, x)| a | x);
	buttons |= MISC.iter().filter(|(m, _)| b1 & m != 0).fold(0, |a, (_, x)| a | x);
	Some(XRaw { buttons, lt, rt, lx: axis(s[0], false), ly: axis(s[1], true), rx: axis(s[2], false), ry: axis(s[3], true) })
}

/// Whether Steam handles this controller: `list` = `SDL_GAMECONTROLLER_IGNORE_DEVICES`, "0xVVVV/0xPPPP" entries split by commas.
fn steam_has(list: &str, vendor: u16, product: u16) -> bool {
	let hex = |v: &str| {
		let v = v.trim();
		u16::from_str_radix(v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")).unwrap_or(v), 16).ok()
	};
	list.split(',').any(|e| e.split_once('/').is_some_and(|(v, p)| hex(v) == Some(vendor) && hex(p) == Some(product)))
}

/// The latest state of the open controller; None while none is open (or before its first report).
static STATE: Mutex<Option<XRaw>> = Mutex::new(None);
/// Whether a controller is open.
static OPEN: AtomicBool = AtomicBool::new(false);

/// The first call looks for a controller at once, then starts the reading thread.
fn start() {
	static START: Once = Once::new();
	START.call_once(sys::start);
}

/// The PlayStation controller's latest state; None without one.
pub fn state() -> Option<XRaw> {
	start();
	*STATE.lock().ok()?
}

/// Whether a PlayStation controller is connected (and not left to Steam).
pub fn connected() -> bool {
	start();
	OPEN.load(Ordering::Relaxed)
}

#[cfg(windows)]
mod sys {
	use super::{parse, steam_has, DS4, DS5, OPEN, SONY, STATE};
	use std::ffi::c_void;
	use std::ptr::{null, null_mut};
	use std::sync::atomic::Ordering;
	use std::time::Duration;

	type Handle = *mut c_void;
	const INVALID: Handle = -1isize as Handle;
	const GENERIC_READ: u32 = 0x8000_0000;
	const SHARE_READ_WRITE: u32 = 3;
	const OPEN_EXISTING: u32 = 3;
	const HIDP_STATUS_SUCCESS: i32 = 0x0011_0000;
	/// Bluetooth HID devices have the HID-over-Bluetooth service in their interface path.
	const BT_SERVICE: &str = "00001124-0000-1000-8000-00805f9b34fb";

	#[repr(C)]
	#[derive(Default, Clone, Copy)]
	struct Guid {
		a: u32,
		b: u16,
		c: u16,
		d: [u8; 8],
	}
	/// HIDD_ATTRIBUTES.
	#[repr(C)]
	#[derive(Default)]
	struct Attributes {
		size: u32,
		vendor: u16,
		product: u16,
		version: u16,
	}

	#[link(name = "kernel32")]
	unsafe extern "system" {
		fn LoadLibraryA(name: *const u8) -> *mut c_void;
		fn GetProcAddress(m: *mut c_void, name: *const u8) -> *mut c_void;
		fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *mut c_void, disposition: u32, flags: u32, template: Handle) -> Handle;
		fn ReadFile(h: Handle, buf: *mut u8, len: u32, read: *mut u32, overlapped: *mut c_void) -> i32;
		fn CloseHandle(h: Handle) -> i32;
	}

	/// The functions of hid.dll and cfgmgr32.dll that are used, and the HID device interface class.
	struct Api {
		hid: Guid,
		attributes: unsafe extern "system" fn(Handle, *mut Attributes) -> u8,
		preparsed: unsafe extern "system" fn(Handle, *mut isize) -> u8,
		free: unsafe extern "system" fn(isize) -> u8,
		caps: unsafe extern "system" fn(isize, *mut [u16; 32]) -> i32,
		list_size: unsafe extern "system" fn(*mut u32, *const Guid, *const u16, u32) -> u32,
		list: unsafe extern "system" fn(*const Guid, *const u16, *mut u16, u32, u32) -> u32,
	}

	/// An export of a loaded dll as the function type `T` (a fn pointer of the export's signature).
	unsafe fn export<T: Copy>(m: *mut c_void, name: &[u8]) -> Option<T> {
		let f = unsafe { GetProcAddress(m, name.as_ptr()) };
		(!f.is_null()).then(|| unsafe { std::mem::transmute_copy::<*mut c_void, T>(&f) })
	}

	fn api() -> Option<Api> {
		unsafe {
			let hid = LoadLibraryA(b"hid.dll\0".as_ptr());
			let cfg = LoadLibraryA(b"cfgmgr32.dll\0".as_ptr());
			if hid.is_null() || cfg.is_null() {
				return None;
			}
			let class: unsafe extern "system" fn(*mut Guid) = export(hid, b"HidD_GetHidGuid\0")?;
			let mut g = Guid::default();
			class(&mut g);
			Some(Api {
				hid: g,
				attributes: export(hid, b"HidD_GetAttributes\0")?,
				preparsed: export(hid, b"HidD_GetPreparsedData\0")?,
				free: export(hid, b"HidD_FreePreparsedData\0")?,
				caps: export(hid, b"HidP_GetCaps\0")?,
				list_size: export(cfg, b"CM_Get_Device_Interface_List_SizeW\0")?,
				list: export(cfg, b"CM_Get_Device_Interface_ListW\0")?,
			})
		}
	}

	/// The open controller.
	struct Pad {
		h: Handle,
		product: u16,
		bt: bool,
		/// The length ReadFile needs: the longest input report with its id.
		len: usize,
	}
	// A file handle can be used from any thread.
	unsafe impl Send for Pad {}

	/// The interface paths of the present HID devices (UTF-16, each ended by a NUL, as cfgmgr32 lists them).
	fn paths(api: &Api) -> Vec<u16> {
		let mut n = 0u32;
		unsafe {
			if (api.list_size)(&mut n, &api.hid, null(), 0) != 0 || n == 0 {
				return Vec::new();
			}
			let mut v = vec![0u16; n as usize];
			if (api.list)(&api.hid, null(), v.as_mut_ptr(), n, 0) != 0 {
				return Vec::new();
			}
			v
		}
	}

	/// The first PlayStation controller of the list that opens for reading and is not Steam's.
	fn open(api: &Api, list: &[u16]) -> Option<Pad> {
		let steam = std::env::var("SDL_GAMECONTROLLER_IGNORE_DEVICES").unwrap_or_default();
		for p in list.split(|&c| c == 0).filter(|p| !p.is_empty()) {
			let z: Vec<u16> = p.iter().copied().chain([0]).collect();
			// Opened without access first: enough for the ids, and it opens any HID device (keyboards and mice too).
			let h = unsafe { CreateFileW(z.as_ptr(), 0, SHARE_READ_WRITE, null_mut(), OPEN_EXISTING, 0, null_mut()) };
			if h == INVALID {
				continue;
			}
			let mut a = Attributes { size: std::mem::size_of::<Attributes>() as u32, ..Default::default() };
			let ok = unsafe { (api.attributes)(h, &mut a) } != 0;
			unsafe { CloseHandle(h) };
			if !ok || a.vendor != SONY || !(DS4.contains(&a.product) || DS5.contains(&a.product)) || steam_has(&steam, a.vendor, a.product) {
				continue;
			}
			let h = unsafe { CreateFileW(z.as_ptr(), GENERIC_READ, SHARE_READ_WRITE, null_mut(), OPEN_EXISTING, 0, null_mut()) };
			if h == INVALID {
				continue;
			}
			let bt = String::from_utf16_lossy(p).to_ascii_lowercase().contains(BT_SERVICE);
			eprintln!("retail: pspad: {} {:04x} over {}", if DS5.contains(&a.product) { "DualSense" } else { "DualShock 4" }, a.product, if bt { "Bluetooth" } else { "USB" });
			return Some(Pad { h, product: a.product, bt, len: input_len(api, h) });
		}
		None
	}

	/// The device's longest input report with its id (HIDP_CAPS.InputReportByteLength); 1024 if it cannot be read.
	fn input_len(api: &Api, h: Handle) -> usize {
		let mut pd = 0isize;
		if unsafe { (api.preparsed)(h, &mut pd) } == 0 {
			return 1024;
		}
		let mut caps = [0u16; 32];
		let ok = unsafe { (api.caps)(pd, &mut caps) } == HIDP_STATUS_SUCCESS;
		unsafe { (api.free)(pd) };
		if ok && caps[2] > 0 { caps[2] as usize } else { 1024 }
	}

	/// Reads the controller's reports into STATE until it is gone (unplugged, switched off, out of range).
	fn read(p: &Pad) {
		let mut buf = vec![0u8; p.len.max(64)];
		loop {
			let mut n = 0u32;
			if unsafe { ReadFile(p.h, buf.as_mut_ptr(), buf.len() as u32, &mut n, null_mut()) } == 0 {
				return;
			}
			if let Some(x) = parse(p.product, p.bt, &buf[..(n as usize).min(buf.len())]) {
				if let Ok(mut s) = STATE.lock() {
					*s = Some(x);
				}
			}
		}
	}

	pub fn start() {
		let Some(api) = api() else { return };
		let mut seen = paths(&api);
		let first = open(&api, &seen);
		OPEN.store(first.is_some(), Ordering::Relaxed);
		let _ = std::thread::Builder::new().name("pspad".into()).spawn(move || {
			let mut pad = first;
			loop {
				if let Some(p) = pad.take() {
					read(&p);
					unsafe { CloseHandle(p.h) };
					if let Ok(mut s) = STATE.lock() {
						*s = None;
					}
					OPEN.store(false, Ordering::Relaxed);
					eprintln!("retail: pspad: controller gone");
					// Look through the whole list again (the same or another controller may still be there).
					seen.clear();
				}
				std::thread::sleep(Duration::from_secs(1));
				let now = paths(&api);
				if now != seen {
					pad = open(&api, &now);
					OPEN.store(pad.is_some(), Ordering::Relaxed);
					seen = now;
				}
			}
		});
	}
}

#[cfg(not(windows))]
mod sys {
	pub fn start() {
		let _ = (super::parse, super::steam_has, super::SONY, super::DS4);
	}
}

#[cfg(test)]
mod tests {
	use super::super::prompts::{map_xinput, PadState};
	use super::*;

	/// A report: `id`, the 9 state bytes at `at`, `len` bytes in all.
	fn report(id: u8, at: usize, state: [u8; 9], len: usize) -> Vec<u8> {
		let mut r = vec![0u8; len];
		r[0] = id;
		r[at..at + 9].copy_from_slice(&state);
		r
	}
	/// The DualShock 4 state layout (also DualSense Bluetooth basic mode): sticks, buttons, buttons, PS byte, L2, R2.
	fn ds4(sticks: [u8; 4], b0: u8, b1: u8, l2: u8, r2: u8) -> [u8; 9] {
		[sticks[0], sticks[1], sticks[2], sticks[3], b0, b1, 0, l2, r2]
	}
	/// The DualSense full state layout: sticks, L2, R2, counter, buttons, buttons.
	fn ds5(sticks: [u8; 4], b0: u8, b1: u8, l2: u8, r2: u8) -> [u8; 9] {
		[sticks[0], sticks[1], sticks[2], sticks[3], l2, r2, 0x5a, b0, b1]
	}
	const MID: [u8; 4] = [0x80; 4];
	/// What the game gets: the PS2 pad state.
	fn ps2(product: u16, bt: bool, r: &[u8]) -> PadState {
		map_xinput(&parse(product, bt, r).expect("a pad state report"))
	}

	#[test]
	fn dualshock4_buttons_reach_their_ps2_places() {
		// PS2 bits as pad_from_keys: L2 1, R2 2, L1 4, R1 8, Triangle 0x10, Circle 0x20, Cross 0x40, Square 0x80, Select 0x100,
		// L3 0x200, R3 0x400, Start 0x800, up 0x1000, right 0x2000, down 0x4000, left 0x8000.
		let b = |b0: u8, b1: u8, l2: u8, r2: u8| ps2(0x05c4, false, &report(0x01, 1, ds4(MID, b0, b1, l2, r2), 64)).buttons;
		assert_eq!(b(0x08, 0, 0, 0), 0, "hat 8 = no d-pad");
		assert_eq!(b(0x08 | 0x10, 0, 0, 0), 0x80, "Square");
		assert_eq!(b(0x08 | 0x20, 0, 0, 0), 0x40, "Cross");
		assert_eq!(b(0x08 | 0x40, 0, 0, 0), 0x20, "Circle");
		assert_eq!(b(0x08 | 0x80, 0, 0, 0), 0x10, "Triangle");
		assert_eq!(b(0x08, 0x01, 0, 0), 4, "L1");
		assert_eq!(b(0x08, 0x02, 0, 0), 8, "R1");
		assert_eq!(b(0x08, 0x10, 0, 0), 0x100, "Share = Select");
		assert_eq!(b(0x08, 0x20, 0, 0), 0x800, "Options = Start");
		assert_eq!(b(0x08, 0x40, 0, 0), 0x200, "L3");
		assert_eq!(b(0x08, 0x80, 0, 0), 0x400, "R3");
		assert_eq!(b(0x08, 0x04 | 0x08, 0, 0), 0, "L2 / R2 come from the analog triggers only");
		assert_eq!(b(0x08, 0, 200, 0), 1, "L2");
		assert_eq!(b(0x08, 0, 0, 200), 2, "R2");
		assert_eq!(b(0x08, 0, 20, 20), 0, "triggers below the threshold");
		let hat: Vec<u16> = (0..=8).map(|h| b(h, 0, 0, 0)).collect();
		assert_eq!(hat, [0x1000, 0x3000, 0x2000, 0x6000, 0x4000, 0xc000, 0x8000, 0x9000, 0], "hat clockwise from up");
	}

	#[test]
	fn sticks_keep_the_ps2_orientation() {
		let st = |s: [u8; 4]| ps2(0x09cc, false, &report(0x01, 1, ds4(s, 0x08, 0, 0, 0), 64)).sticks;
		// PadState sticks are [rx, ry, lx, ly]; the pad's own convention is the PS2's (0 = up / left).
		assert_eq!(st(MID), [0x80; 4]);
		assert_eq!(st([0x84, 0x7b, 0x80, 0x86]), [0x80; 4], "rest noise is inside the deadzone");
		assert_eq!(st([0x00, 0x80, 0x80, 0x80]), [0x80, 0x80, 1, 0x80], "left stick left");
		assert_eq!(st([0x80, 0x00, 0x80, 0x80]), [0x80, 0x80, 0x80, 1], "left stick up");
		assert_eq!(st([0x80, 0x80, 0xff, 0x80]), [255, 0x80, 0x80, 0x80], "right stick right");
		assert_eq!(st([0x80, 0x80, 0x80, 0xff]), [0x80, 255, 0x80, 0x80], "right stick down");
	}

	#[test]
	fn every_report_layout_reads_the_same_pad() {
		// Cross + L1 held, left stick fully left, R2 pulled: PS2 Cross 0x40 | L1 4 | R2 2.
		let want = PadState { buttons: 0x40 | 4 | 2, sticks: [0x80, 0x80, 1, 0x80] };
		let s = [0x00, 0x80, 0x80, 0x80];
		assert_eq!(ps2(0x05c4, false, &report(0x01, 1, ds4(s, 0x28, 0x01, 0, 255), 64)), want, "DualShock 4 USB");
		assert_eq!(ps2(0x05c4, true, &report(0x01, 1, ds4(s, 0x28, 0x01, 0, 255), 547)), want, "DualShock 4 Bluetooth basic");
		assert_eq!(ps2(0x09cc, true, &report(0x11, 3, ds4(s, 0x28, 0x01, 0, 255), 547)), want, "DualShock 4 Bluetooth full");
		assert_eq!(ps2(0x0ba0, false, &report(0x01, 1, ds4(s, 0x28, 0x01, 0, 255), 64)), want, "DualShock 4 wireless adapter");
		assert_eq!(ps2(0x0ce6, false, &report(0x01, 1, ds5(s, 0x28, 0x01, 0, 255), 64)), want, "DualSense USB");
		assert_eq!(ps2(0x0ce6, true, &report(0x31, 2, ds5(s, 0x28, 0x01, 0, 255), 78)), want, "DualSense Bluetooth full");
		assert_eq!(ps2(0x0ce6, true, &report(0x01, 1, ds4(s, 0x28, 0x01, 0, 255), 78)), want, "DualSense Bluetooth basic");
		assert_eq!(ps2(0x0df2, false, &report(0x01, 1, ds5(s, 0x28, 0x01, 0, 255), 64)), want, "DualSense Edge USB");
	}

	#[test]
	fn other_reports_hold_no_pad_state() {
		assert!(parse(0x05c4, false, &report(0x05, 1, ds4(MID, 0x08, 0, 0, 0), 64)).is_none(), "an output / feature id");
		assert!(parse(0x0ce6, true, &report(0x11, 3, ds4(MID, 0x08, 0, 0, 0), 78)).is_none(), "a DualShock 4 id on a DualSense");
		assert!(parse(0x05c4, false, &[0x01, 0x80, 0x80]).is_none(), "too short");
		assert!(parse(0x05c4, false, &[]).is_none());
		// The wireless adapter without a paired controller: all-zero sticks = an idle pad, not sticks pushed up-left.
		assert_eq!(ps2(0x0ba0, false, &report(0x01, 1, [0; 9], 64)), PadState::IDLE);
	}

	#[test]
	fn steam_ignore_list() {
		let l = "0x28DE/0x1142,0x054C/0x05C4, 0x054c/0x0ce6";
		assert!(steam_has(l, 0x054c, 0x05c4));
		assert!(steam_has(l, 0x054c, 0x0ce6));
		assert!(!steam_has(l, 0x054c, 0x09cc));
		assert!(!steam_has("", 0x054c, 0x05c4));
		assert!(!steam_has("garbage,/,0x054C", 0x054c, 0x05c4));
	}
}
