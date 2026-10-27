//! Gamepads on Linux, read as evdev devices with `gilrs`.
//!
//! The Windows paths (`prompts::xinput` for an Xbox controller over XInput, `pspad` for a DualShock 4 / DualSense over HID) both
//! end in the same place: an `XRaw`, the XInput report, which `prompts::map_xinput` turns into the PS2 button bits. This file is
//! the Linux way to the same `XRaw`, so everything downstream (the mapping, the deadzones, the button prompts) is shared with the
//! Windows build and does not know which platform the pad came from.
//!
//! How it works: a thread of its own owns the `gilrs` handle and drains its events into the latest `XRaw` behind a mutex; `state()`
//! hands that out. The mapping is positional, like XInput's: `South` is the bottom button (A on an Xbox pad, Cross on a
//! PlayStation one), `East` the right one, `West` the left, `North` the top. `gilrs` resolves that for us: it reads the SDL
//! controller database, so a pad whose raw evdev codes disagree with the Xbox ones (a PlayStation controller reports its Square
//! and Triangle the other way round than an Xbox pad does) still lands in the right place.
//!
//! The analog triggers are the one thing to be careful about: in the SDL database a trigger is an *axis* (`lefttrigger:a2`), and
//! `gilrs` reports an axis mapped to a button as a `ButtonChanged` with the axis' value in it, not as an `AxisChanged`. So the
//! trigger comes from `ButtonChanged(LeftTrigger2 / RightTrigger2, value)` and not from an axis of its own. `Button::LeftTrigger`
//! is the L1 shoulder, not the L2 trigger.
//!
//! Permissions: the device node (`/dev/input/eventN`) has to be readable. On a normal desktop session systemd-logind gives the
//! seat's user access to it (the `uaccess` rule), so no group membership and no root are needed.

use super::prompts::XRaw;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once};

/// The latest state of the pad; None while none is open (or before its first event).
static STATE: Mutex<Option<XRaw>> = Mutex::new(None);
/// Whether a gamepad is open.
static OPEN: AtomicBool = AtomicBool::new(false);

/// The first call looks for a gamepad at once, then starts the reading thread.
fn start() {
	static START: Once = Once::new();
	START.call_once(sys::start);
}

/// The gamepad's latest state; None without one.
pub fn state() -> Option<XRaw> {
	start();
	*STATE.lock().ok()?
}

/// Whether a gamepad is connected.
pub fn connected() -> bool {
	start();
	OPEN.load(Ordering::Relaxed)
}

/// The gamepad reader is Linux only; on Windows the XInput and the PlayStation paths are the ones.
#[cfg(not(target_os = "linux"))]
mod sys {
	pub fn start() {
		let _ = (super::STATE, super::OPEN);
	}
}

/// The gamepad reader: evdev through `gilrs`.
#[cfg(target_os = "linux")]
mod sys {
	use super::{OPEN, STATE};
	use gilrs::ev::{Axis, AxisOrBtn, Button, EventType};
	use gilrs::{Gamepad, GamepadId, GilrsBuilder};
	use std::sync::atomic::Ordering;
	use std::time::Duration;

	/// The XInput button bits (`prompts::map_xinput`): d-pad up 1 down 2 left 4 right 8, Start 0x10, Back 0x20, L3 0x40, R3 0x80,
	/// LB 0x100, RB 0x200, A 0x1000, B 0x2000, X 0x4000, Y 0x8000.
	/// A is the bottom button, B the right, X the left, Y the top: the same places as the PS2's Cross, Circle, Square, Triangle.
	const BTN: [(Button, u16); 14] = [
		(Button::South, 0x1000),
		(Button::East, 0x2000),
		(Button::West, 0x4000),
		(Button::North, 0x8000),
		(Button::LeftTrigger, 0x100),
		(Button::RightTrigger, 0x200),
		(Button::Select, 0x20),
		(Button::Start, 0x10),
		(Button::LeftThumb, 0x40),
		(Button::RightThumb, 0x80),
		(Button::DPadUp, 1),
		(Button::DPadDown, 2),
		(Button::DPadLeft, 4),
		(Button::DPadRight, 8),
	];

	/// One gamepad's state while it is read: the button bits, the triggers and the two sticks.
	#[derive(Clone, Copy, Default)]
	struct Pad {
		buttons: u16,
		/// The analog trigger value (0..255), when the pad has one.
		lt: u8,
		rt: u8,
		/// The trigger as a plain button (a DualSense reports BTN_TL2 as well as the axis).
		lt_dig: bool,
		rt_dig: bool,
		/// The pad has an analog trigger (then its value is used, not the button).
		lt_analog: bool,
		rt_analog: bool,
		lx: i16,
		ly: i16,
		rx: i16,
		ry: i16,
	}

	impl Pad {
		/// The XInput report of this state.
		fn raw(&self) -> super::XRaw {
			super::XRaw {
				buttons: self.buttons,
				lt: if self.lt_analog || !self.lt_dig { self.lt } else { 255 },
				rt: if self.rt_analog || !self.rt_dig { self.rt } else { 255 },
				lx: self.lx,
				ly: self.ly,
				rx: self.rx,
				ry: self.ry,
			}
		}
	}

	/// A stick axis in XInput's range: `gilrs` gives -1.0..1.0, XInput wants -32768..32767.
	fn axis(v: f32) -> i16 {
		(v * 32767.0).clamp(-32768.0, 32767.0).round() as i16
	}

	/// A trigger in XInput's range: `gilrs` gives 0.0..1.0, XInput wants 0..255.
	fn trigger(v: f32) -> u8 {
		(v * 255.0).clamp(0.0, 255.0).round() as u8
	}

	/// The bit of a button, or None for a button that is not one of ours.
	fn bit(b: Button) -> Option<u16> {
		BTN.iter().find(|(x, _)| *x == b).map(|(_, v)| *v)
	}

	/// The state of a gamepad as it is now: a button that is already held when the game starts is held from the first frame.
	fn pad_from(gp: &Gamepad) -> Pad {
		let mut p = Pad::default();
		for (code, data) in gp.state().buttons() {
			if !data.is_pressed() {
				continue;
			}
			if let Some(AxisOrBtn::Btn(b)) = gp.axis_or_btn_name(code) {
				match b {
					Button::LeftTrigger2 => p.lt_dig = true,
					Button::RightTrigger2 => p.rt_dig = true,
					_ => {
						if let Some(v) = bit(b) {
							p.buttons |= v;
						}
					}
				}
			}
		}
		for (code, data) in gp.state().axes() {
			let v = data.value();
			match gp.axis_or_btn_name(code) {
				Some(AxisOrBtn::Axis(a)) => match a {
					Axis::LeftStickX => p.lx = axis(v),
					Axis::LeftStickY => p.ly = axis(v),
					Axis::RightStickX => p.rx = axis(v),
					Axis::RightStickY => p.ry = axis(v),
					_ => {}
				},
				Some(AxisOrBtn::Btn(Button::LeftTrigger2)) => {
					p.lt = trigger(v);
					p.lt_analog = true;
				}
				Some(AxisOrBtn::Btn(Button::RightTrigger2)) => {
					p.rt = trigger(v);
					p.rt_analog = true;
				}
				_ => {}
			}
		}
		p
	}

	/// Reads the events of the gamepads and keeps the latest state of the first one.
	///
	/// `gilrs` finds the gamepads by itself (udev, or inotify on `/dev/input` when udev is not available) and notices a pad that
	/// is plugged in or out later, so a controller that is connected while the game runs is picked up without a restart. The first
	/// gamepad is the one that is played with; the others are ignored (the Windows paths work the same way: the first XInput slot
	/// with a controller, the first PlayStation device).
	pub fn start() {
		// The handle is made here; the thread that owns it is started below.
		let Ok(mut gilrs) = GilrsBuilder::new().build() else {
			return;
		};
		// Look for a gamepad right away, so that `connected` answers true from the first call (the game asks at the start).
		let found = gilrs.gamepads().next().map(|(id, gp)| (id, pad_from(&gp)));
		if let Some((_, p)) = &found {
			OPEN.store(true, Ordering::Relaxed);
			if let Ok(mut s) = STATE.lock() {
				*s = Some(p.raw());
			}
		}
		let _ = std::thread::Builder::new().name("linux gamepad".into()).spawn(move || {
			// The gamepad that is played with, and its state.
			let mut pad: Option<(GamepadId, Pad)> = found;
			loop {
				// No gamepad yet: look for one.
				if pad.is_none() {
					let found = gilrs.gamepads().next().map(|(id, gp)| (id, pad_from(&gp)));
					if let Some((id, p)) = found {
						pad = Some((id, p));
						OPEN.store(true, Ordering::Relaxed);
						if let Ok(mut s) = STATE.lock() {
							*s = pad.as_ref().map(|(_, p)| p.raw());
						}
					} else {
						// Nothing connected: wait (an event comes sooner when a pad is plugged in) and look again.
						let _ = gilrs.next_event_blocking(Some(Duration::from_secs(1)));
						continue;
					}
				}
				// Read the events. The short timeout is so that a pad that is unplugged is noticed without waiting for an event.
				let Some(event) = gilrs.next_event_blocking(Some(Duration::from_millis(200))) else {
					continue;
				};
				// Only the gamepad that is played with is read.
				let Some((id, mut p)) = pad.take() else {
					continue;
				};
				if event.id != id {
					pad = Some((id, p));
					continue;
				}
				if apply(&mut p, event.event) {
					// The pad is gone: forget it and look for another one.
					OPEN.store(false, Ordering::Relaxed);
					if let Ok(mut s) = STATE.lock() {
						*s = None;
					}
					continue;
				}
				let raw = p.raw();
				pad = Some((id, p));
				if let Ok(mut s) = STATE.lock() {
					*s = Some(raw);
				}
			}
		});
	}

	/// One event on the state; true when the gamepad is gone.
	fn apply(p: &mut Pad, event: EventType) -> bool {
		match event {
			EventType::ButtonPressed(b, _) => match b {
				Button::LeftTrigger2 => p.lt_dig = true,
				Button::RightTrigger2 => p.rt_dig = true,
				_ => {
					if let Some(v) = bit(b) {
						p.buttons |= v;
					}
				}
			},
			EventType::ButtonReleased(b, _) => match b {
				Button::LeftTrigger2 => p.lt_dig = false,
				Button::RightTrigger2 => p.rt_dig = false,
				_ => {
					if let Some(v) = bit(b) {
						p.buttons &= !v;
					}
				}
			},
			EventType::ButtonChanged(b, v, _) => {
				// A trigger: the analog value is in here (the SDL database has a trigger as an axis, and `gilrs` reports an axis
				// mapped to a button as a `ButtonChanged` with the axis' value). A digital trigger button (a DualSense reports
				// BTN_TL2 as well) comes as a plain press and is set above.
				match b {
					Button::LeftTrigger2 => {
						p.lt = trigger(v);
						p.lt_analog = true;
					}
					Button::RightTrigger2 => {
						p.rt = trigger(v);
						p.rt_analog = true;
					}
					_ => {}
				}
			}
			EventType::AxisChanged(a, v, _) => match a {
				Axis::LeftStickX => p.lx = axis(v),
				Axis::LeftStickY => p.ly = axis(v),
				Axis::RightStickX => p.rx = axis(v),
				Axis::RightStickY => p.ry = axis(v),
				_ => {}
			},
			EventType::Disconnected => return true,
			_ => {}
		}
		false
	}
}
