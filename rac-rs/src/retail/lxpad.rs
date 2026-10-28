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
//!
//! Only the Linux build reads a pad (`state`, `connected`, the `sys` module); the report maths (`Pad`, `axis`, `trigger`) has no
//! `gilrs` in it and is compiled for the unit tests on every platform. `retail.rs` names the reader of the platform `hidpad`.

use super::prompts::XRaw;
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "linux")]
use std::sync::{Mutex, Once};

/// The latest state of the pad; None while none is open (or before its first event).
#[cfg(target_os = "linux")]
static STATE: Mutex<Option<XRaw>> = Mutex::new(None);
/// Whether a gamepad is open.
#[cfg(target_os = "linux")]
static OPEN: AtomicBool = AtomicBool::new(false);

/// The first call looks for a gamepad at once, then starts the reading thread.
#[cfg(target_os = "linux")]
fn start() {
	static START: Once = Once::new();
	START.call_once(sys::start);
}

/// The gamepad's latest state; None without one.
#[cfg(target_os = "linux")]
pub fn state() -> Option<XRaw> {
	start();
	*STATE.lock().ok()?
}

/// Whether a gamepad is connected.
#[cfg(target_os = "linux")]
pub fn connected() -> bool {
	start();
	OPEN.load(Ordering::Relaxed)
}

/// One gamepad's state while it is read: the button bits, the triggers and the two sticks.
#[derive(Clone, Copy, Default)]
struct Pad {
	buttons: u16,
	/// The d-pad when the pad reports it as a hat (`Axis::DPadX` / `DPadY`), in the same bits as `buttons`; ORed into the report.
	hat: u16,
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
	fn raw(&self) -> XRaw {
		XRaw {
			buttons: self.buttons | self.hat,
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

/// The d-pad bits (left 4, right 8) of a hat's X axis. gilrs' own filter `axis_dpad_to_button` (off here, see `sys::start`) calls
/// +1.0 right and -1.0 left (gilrs-0.11.2 `src/ev/filter.rs`, the `Axis::DPadX` arm: `val == 1.0` gives `DPadRight`).
fn hat_x(v: f32) -> u16 {
	if v > 0.5 {
		8
	} else if v < -0.5 {
		4
	} else {
		0
	}
}

/// The d-pad bits (up 1, down 2) of a hat's Y axis. +1.0 is up and -1.0 down (gilrs-0.11.2 `src/ev/filter.rs`, the `Axis::DPadY` arm:
/// `val == 1.0` gives `DPadUp`). gilrs flips the evdev value on Linux (`src/gamepad.rs` `axis_value`: `IS_Y_AXIS_REVERSED` is true in
/// gilrs-core-0.6.8 `src/platform/linux/mod.rs:16`), so the kernel's -1 (hat up) arrives as +1.
fn hat_y(v: f32) -> u16 {
	if v > 0.5 {
		1
	} else if v < -0.5 {
		2
	} else {
		0
	}
}

/// A trigger in XInput's range: `gilrs` gives 0.0..1.0, XInput wants 0..255.
fn trigger(v: f32) -> u8 {
	(v * 255.0).clamp(0.0, 255.0).round() as u8
}

/// The gamepad reader: evdev through `gilrs`.
#[cfg(target_os = "linux")]
mod sys {
	use super::{axis, hat_x, hat_y, trigger, Pad, OPEN, STATE};
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

	/// The bit of a button, or None for a button that is not one of ours.
	fn bit(b: Button) -> Option<u16> {
		BTN.iter().find(|(x, _)| *x == b).map(|(_, v)| *v)
	}

	/// The state of a gamepad as it is now: a button that is already held when the game starts is held from the first frame.
	fn pad_from(gp: &Gamepad) -> Pad {
		let mut p = Pad::default();
		for (code, data) in gp.state().buttons() {
			let Some(AxisOrBtn::Btn(b)) = gp.axis_or_btn_name(code) else {
				continue;
			};
			match b {
				// A trigger that is an axis in the SDL database is stored as a button whose value is the axis (a half-held one reads half).
				Button::LeftTrigger2 => {
					p.lt_dig |= data.is_pressed();
					if data.value() > 0.0 {
						p.lt = trigger(data.value());
						p.lt_analog = true;
					}
				}
				Button::RightTrigger2 => {
					p.rt_dig |= data.is_pressed();
					if data.value() > 0.0 {
						p.rt = trigger(data.value());
						p.rt_analog = true;
					}
				}
				_ => {
					if let (true, Some(v)) = (data.is_pressed(), bit(b)) {
						p.buttons |= v;
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
					Axis::DPadX => p.hat = (p.hat & !0b1100) | hat_x(v),
					Axis::DPadY => p.hat = (p.hat & !0b0011) | hat_y(v),
					_ => {}
				},
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
		// gilrs' default filters off: they add a stick dead zone and drop small changes (jitter), and the only dead zone must be the
		// game's own noise gate in `prompts::stick` (two dead zones on top of each other made the sticks feel dead before).
		let Ok(mut gilrs) = GilrsBuilder::new().with_default_filters(false).build() else {
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
				// Read the events. The timeout only wakes the loop now and then; an unplugged pad is noticed by its `Disconnected` event.
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
				// A hat d-pad (Xbox pads, DualShock 4, DualSense and Steam's virtual pad all have one) only arrives as an axis here:
				// the gilrs filter that would turn it into DPad buttons is off (see `start`). Kept apart from `buttons` so that a
				// pad with both kinds of d-pad reports the union.
				Axis::DPadX => p.hat = (p.hat & !0b1100) | hat_x(v),
				Axis::DPadY => p.hat = (p.hat & !0b0011) | hat_y(v),
				_ => {}
			},
			EventType::Disconnected => return true,
			_ => {}
		}
		false
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The sticks go from gilrs' -1.0..1.0 to XInput's -32768..32767, centre 0, and never past the ends.
	#[test]
	fn the_sticks_use_xinputs_range() {
		assert_eq!(axis(0.0), 0);
		assert_eq!(axis(1.0), 32767);
		assert_eq!(axis(-1.0), -32767);
		assert_eq!(axis(2.0), 32767, "past the end is clamped");
		assert_eq!(axis(-2.0), -32768);
		assert_eq!(axis(0.5), 16384);
	}

	/// The triggers go from 0.0..1.0 to 0..255 and stay inside it.
	#[test]
	fn the_triggers_use_xinputs_range() {
		assert_eq!((trigger(0.0), trigger(1.0), trigger(0.5)), (0, 255, 128));
		assert_eq!((trigger(-0.3), trigger(1.7)), (0, 255));
	}

	/// An analog trigger reports its value; a pad that only has the plain trigger button reports a full press; with both, the analog
	/// value wins (a DualSense reports both).
	#[test]
	fn a_trigger_is_analog_when_the_pad_has_one_else_the_button() {
		let mut p = Pad::default();
		assert_eq!((p.raw().lt, p.raw().rt), (0, 0));
		p.lt_dig = true;
		assert_eq!(p.raw().lt, 255, "button only: a full press");
		p.lt = 40;
		p.lt_analog = true;
		assert_eq!(p.raw().lt, 40, "analog and button: the analog value");
		p.lt_dig = false;
		assert_eq!(p.raw().lt, 40);
		p.rt_dig = true;
		assert_eq!((p.raw().rt, p.raw().lt), (255, 40), "the other trigger is independent");
	}

	/// A hat d-pad gives the XInput d-pad bits: X right 8 / left 4, Y up 1 / down 2 (+1.0 is up after gilrs' Linux flip), nothing in
	/// the middle, and a half-way value does not count.
	#[test]
	fn a_hat_gives_the_dpad_bits() {
		assert_eq!((hat_x(1.0), hat_x(-1.0), hat_x(0.0)), (8, 4, 0));
		assert_eq!((hat_y(1.0), hat_y(-1.0), hat_y(0.0)), (1, 2, 0));
		assert_eq!((hat_x(0.4), hat_x(-0.4), hat_y(0.4), hat_y(-0.4)), (0, 0, 0, 0));
	}

	/// The hat is ORed into the report next to the buttons (a hat press leaves the face buttons alone, and a pad that has both kinds of
	/// d-pad gives the union), and the two hat axes set and clear only their own bits.
	#[test]
	fn the_hat_is_ored_into_the_report() {
		let mut p = Pad { buttons: 0x1000 | 2, ..Pad::default() };
		p.hat = (p.hat & !0b1100) | hat_x(1.0);
		p.hat = (p.hat & !0b0011) | hat_y(1.0);
		assert_eq!(p.raw().buttons, 0x1000 | 2 | 8 | 1, "buttons + hat");
		p.hat = (p.hat & !0b1100) | hat_x(0.0);
		assert_eq!(p.hat, 1, "releasing X leaves Y");
		p.hat = (p.hat & !0b0011) | hat_y(-1.0);
		assert_eq!(p.hat, 2);
		assert_eq!(Pad::default().raw().buttons, 0);
	}

	/// The report carries the button bits and the four stick values as they are.
	#[test]
	fn the_report_carries_buttons_and_sticks() {
		let p = Pad { buttons: 0x1010, lx: -5, ly: 6, rx: 7, ry: -8, ..Pad::default() };
		let r = p.raw();
		assert_eq!((r.buttons, r.lx, r.ly, r.rx, r.ry), (0x1010, -5, 6, 7, -8));
	}
}
