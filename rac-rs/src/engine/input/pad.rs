//! The gamepad as the game's input (engine split stage E5, 2026-10-27; from rac1 retail/prompts.rs): an XInput controller and a
//! PlayStation controller (`pspad`, read over HID) both arrive as an XInput report (`XRaw`) and become a PS2 DualShock 2 report
//! (`PadState`: button bits, sticks): A = Cross, B = Circle, X = Square, Y = Triangle, LB = L1, RB = R1, LT = L2, RT = R2, Back =
//! Select, Start = Start, d-pad, both sticks analog with radial deadzones and a DualShock 2's square range (`stick`), L3 / R3;
//! which physical button drives which PS2 button is `binds::pad_map`. The pads are merged with the keyboard (OR of buttons, the
//! larger deflection of each stick axis). Which device a game's prompts show, and a game's own buttons (R&C1's aim), stay with
//! the game (rac1 retail/prompts.rs `poll_pad`).
use super::xinput;
use crate::fmath::Fm;
use std::sync::OnceLock;

/// The pad as the game's input: PS2 button bits (those of a game's keyboard input) and sticks [rx, ry, lx, ly], 0x80 = centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PadState {
	pub buttons: u16,
	pub sticks: [u8; 4],
}

impl PadState {
	pub const IDLE: PadState = PadState { buttons: 0, sticks: [0x80; 4] };
}

/// An XInput report (`XINPUT_GAMEPAD`).
#[derive(Clone, Copy, Default, Debug)]
pub struct XRaw {
	pub buttons: u16,
	pub lt: u8,
	pub rt: u8,
	pub lx: i16,
	pub ly: i16,
	pub rx: i16,
	pub ry: i16,
}

/// XInput's recommended deadzones (XINPUT_GAMEPAD_LEFT/RIGHT_THUMB_DEADZONE) and trigger threshold. The stick deadzones only keep
/// the rest noise from rac-rs's own readers (`pad_activity`, the prompts): scaled by `AXIS_SCALE` they stay inside the game's own
/// dead zone, so the game never sees them.
const DZ_LEFT: f32 = 7849.0;
const DZ_RIGHT: f32 = 8689.0;
const TRIGGER: u8 = 30;

/// PCSX2's default "Analog Sensitivity" of a DualShock 2 (133 %, with no dead zone of its own): a modern stick reaches the full
/// byte before its edge, as a DualShock 2 did.
const AXIS_SCALE: f32 = 1.33;

/// One stick as PCSX2 gives it to the game by default: each axis times `AXIS_SCALE`, clamped to 1..=255; y up in XInput = low byte
/// on the PS2. Inside the radial deadzone `dz` the stick is centred. Retail reads each axis on its own (dead zone 47/127, full at
/// 124/127; decomp/notes/camera.md, the pad reader) and runs at a stick strength >= 0.82 (hero_movement.md), so its own dead zone
/// is the only one the player feels: Ratchet moves from about 28 % of the throw, runs from about 66 % and is at full speed from
/// about 74 %. v0.4.0 rescaled the throw past `dz` to the full range, so the game's dead zone came on top of it: Ratchet moved
/// only from about 53 % and ran from 90 % (player report #30, "having to tilt the analog stick quite far before he moves").
fn stick(x: i16, y: i16, dz: f32) -> (u8, u8) {
	let (x, y) = (x as f32, y as f32);
	if x.fhypot(y) <= dz {
		return (0x80, 0x80);
	}
	let b = |v: f32| (0x80 as f32 + v / 32767.0 * AXIS_SCALE * 127.0).round().clamp(1.0, 255.0) as u8;
	(b(x), b(-y))
}

/// The game's hook on the LEFT stick of an XInput report: `(x, y, retail_gate)` -> `((x, y), gate)` handed on to `stick` (R&C1: the
/// Stick dead zone row, rac1 retail/pcinput.rs `left_stick`). Not set = the vector and the gate untouched.
pub type LeftStick = fn(i16, i16, f32) -> ((i16, i16), f32);
static LEFT_STICK: OnceLock<LeftStick> = OnceLock::new();

/// A game's hook on the left stick (see `LeftStick`); the first call wins.
pub fn set_left_stick(f: LeftStick) {
	let _ = LEFT_STICK.set(f);
}

/// The reader of the platform's PlayStation / evdev controllers (`state`, `connected`): pspad (HID) unless the game sets another one.
type HidReader = (fn() -> Option<XRaw>, fn() -> bool);
static HID: OnceLock<HidReader> = OnceLock::new();

/// A game's reader for the platform's PlayStation / evdev controllers in place of pspad (R&C1 on Linux: rac1 retail/lxpad.rs, evdev
/// through gilrs; pspad is HID, which Windows has); the first call wins.
pub fn set_hid_reader(state: fn() -> Option<XRaw>, connected: fn() -> bool) {
	let _ = HID.set((state, connected));
}

fn hid_state() -> Option<XRaw> {
	HID.get().map_or_else(super::pspad::state, |h| h.0())
}

fn hid_connected() -> bool {
	HID.get().map_or_else(super::pspad::connected, |h| h.1())
}

/// The controller's pressed physical buttons as `binds::PHYS` masks: the XInput buttons plus the triggers (past `TRIGGER`) as bits 16 / 17.
pub fn phys_mask(r: &XRaw) -> u32 {
	r.buttons as u32 | ((r.lt > TRIGGER) as u32) << 16 | ((r.rt > TRIGGER) as u32) << 17
}

/// The XInput report as a DualShock report: A = Cross, B = Circle, X = Square, Y = Triangle, LB = L1, RB = R1, LT = L2, RT = R2,
/// Back = Select, Start = Start, L3 / R3, d-pad, sticks analog.
pub fn map_xinput(r: &XRaw) -> PadState {
	// XInput: dpad up 1 down 2 left 4 right 8, Start 0x10, Back 0x20, L3 0x40, R3 0x80, LB 0x100, RB 0x200, A 0x1000, B 0x2000,
	// X 0x4000, Y 0x8000, the triggers as bits 16 / 17 here (past TRIGGER). PS2 bits as pad_from_keys: L2 1, R2 2, L1 4, R1 8,
	// Triangle 0x10, Circle 0x20, Cross 0x40, Square 0x80, Select 0x100, L3 0x200, R3 0x400, Start 0x800, up 0x1000, right 0x2000,
	// down 0x4000, left 0x8000. Which physical button drives which PS2 button is `binds::pad_map` (today's list unless the player
	// remapped it on Pause -> Options -> Controls).
	let b = super::binds::pad_buttons(phys_mask(r), &super::binds::pad_map());
	let (rx, ry) = stick(r.rx, r.ry, DZ_RIGHT);
	let ((x, y), dz) = LEFT_STICK.get().map_or(((r.lx, r.ly), DZ_LEFT), |f| f(r.lx, r.ly, DZ_LEFT)); // game's Stick dead zone row (R&C1 High = retail, unchanged)
	let (lx, ly) = stick(x, y, dz);
	PadState { buttons: b, sticks: [rx, ry, lx, ly] }
}

/// Keyboard and pad at once: the buttons of both, and per stick axis the larger deflection.
pub fn merge(kb: (u16, [u8; 4]), pad: &PadState) -> (u16, [u8; 4]) {
	let mut s = kb.1;
	for i in 0..4 {
		if (pad.sticks[i] as i32 - 0x80).abs() > (s[i] as i32 - 0x80).abs() {
			s[i] = pad.sticks[i];
		}
	}
	(kb.0 | pad.buttons, s)
}

/// Real pad input between two reports: a button went down, or a stick left the deadzone / moved clearly. Holding a button or a
/// stick (or an idle pad) is none.
pub fn pad_activity(prev: &PadState, now: &PadState) -> bool {
	now.buttons & !prev.buttons != 0 || (0..4).any(|i| now.sticks[i] != 0x80 && (prev.sticks[i] == 0x80 || (now.sticks[i] as i32 - prev.sticks[i] as i32).abs() > 24))
}

/// Agent test runs (RAC_TEST_WINDOW=hidden, RAC_RETAIL_SMOKE) never read a real controller: a pad plugged into the PC (or a
/// drifting stick) would steer the replay and change the gate's RAM hashes and pictures run to run (2026-10-23).
pub fn real_pads_off() -> bool {
	static OFF: OnceLock<bool> = OnceLock::new();
	*OFF.get_or_init(|| std::env::var_os("RAC_TEST_WINDOW").map_or(false, |v| v == "hidden") || std::env::var_os("RAC_RETAIL_SMOKE").is_some())
}


/// Every connected pad as one: the XInput controller and the PlayStation controller (pspad.rs), merged as `merge` does; also
/// the physical buttons pressed on either (`binds::PHYS` masks, for the Controls page's "press a button").
pub fn read() -> Option<(PadState, u32)> {
	if real_pads_off() {
		return None;
	}
	let (xr, pr) = (xinput::raw(), hid_state());
	if xr.is_none() && pr.is_none() {
		return None;
	}
	let phys = xr.as_ref().map_or(0, phys_mask) | pr.as_ref().map_or(0, phys_mask);
	let (x, p) = (xr.map(|r| map_xinput(&r)), pr.map(|r| map_xinput(&r)));
	let state = match (x, p) {
		(Some(x), Some(p)) => {
			let (buttons, sticks) = merge((x.buttons, x.sticks), &p);
			PadState { buttons, sticks }
		}
		(x, p) => x.or(p)?,
	};
	Some((state, phys))
}

/// A gamepad (XInput or PlayStation) is connected now.
pub fn connected() -> bool {
	xinput::connected() | hid_connected()
}

#[cfg(test)]
mod tests {
	use super::*;

	fn x(buttons: u16) -> XRaw {
		XRaw { buttons, ..Default::default() }
	}

	#[test]
	fn xinput_buttons_map_to_the_ps2_pad() {
		let one = |b: u16| map_xinput(&x(b));
		assert_eq!(one(0x1000).buttons, 0x40, "A = Cross");
		assert_eq!(one(0x2000).buttons, 0x20, "B = Circle");
		assert_eq!(one(0x4000).buttons, 0x80, "X = Square");
		assert_eq!(one(0x8000).buttons, 0x10, "Y = Triangle");
		assert_eq!(one(0x100).buttons, 4, "LB = L1");
		assert_eq!(one(0x200).buttons, 8, "RB = R1");
		assert_eq!(one(0x20).buttons, 0x100, "Back = Select");
		assert_eq!(one(0x10).buttons, 0x800, "Start = Start");
		assert_eq!(one(0x40).buttons, 0x200, "L3");
		assert_eq!(one(0x80).buttons, 0x400, "R3");
		assert_eq!(one(1).buttons, 0x1000, "up");
		assert_eq!(one(8).buttons, 0x2000, "right");
		assert_eq!(one(2).buttons, 0x4000, "down");
		assert_eq!(one(4).buttons, 0x8000, "left");
		assert_eq!(one(0x1000 | 0x2000).buttons, 0x60, "two at once");
		assert_eq!(map_xinput(&XRaw { lt: 200, ..Default::default() }).buttons, 1, "LT = L2");
		assert_eq!(map_xinput(&XRaw { rt: 200, ..Default::default() }).buttons, 2, "RT = R2");
		assert_eq!(map_xinput(&XRaw { lt: 30, rt: 30, ..Default::default() }), PadState::IDLE, "triggers below the threshold");
		assert_eq!(map_xinput(&x(0)), PadState::IDLE);
	}

	#[test]
	fn xinput_sticks_have_deadzones_and_the_ps2_orientation() {
		let st = |lx, ly, rx, ry| map_xinput(&XRaw { lx, ly, rx, ry, ..Default::default() }).sticks;
		// [rx, ry, lx, ly]; right and down are high bytes, up is low (the PS2 and `pad_from_keys`).
		assert_eq!(st(0, 0, 0, 0), [0x80; 4]);
		assert_eq!(st(7000, 3000, 6000, -4000), [0x80; 4], "inside the deadzones (left 7849, right 8689)");
		assert_eq!(st(32767, 0, 0, 0), [0x80, 0x80, 0xff, 0x80], "left stick full right");
		assert_eq!(st(-32768, 0, 0, 0)[2], 1, "full left");
		assert_eq!(st(0, 32767, 0, 0)[3], 1, "full up is the low byte");
		assert_eq!(st(0, -32768, 0, 0)[3], 0xff, "full down");
		assert_eq!(st(0, 0, 32767, 0)[0], 0xff, "right stick full right");
		assert_eq!(st(0, 0, 0, 32767)[1], 1, "right stick up");
		// Not rescaled past the deadzone: each axis is the throw times 1.33 (PCSX2's default), so half a throw is 0x80 + 84 and
		// three quarters are already full.
		assert_eq!(st(16384, 0, 0, 0)[2], 0x80 + 84);
		assert_eq!(st(-16384, 0, 0, 0)[2], 0x80 - 84);
		assert_eq!(st(24576, 0, 0, 0)[2], 0xff);
		// Diagonals keep their direction: a full push up-right is as far right as up.
		let d = st(23170, 23170, 0, 0);
		assert!(d[2] > 0x80 && d[3] < 0x80 && (d[2] as i32 - 0x80) == (0x80 - d[3] as i32), "{d:?}");
	}

	/// Retail's reading of one stick byte (the pad reader, decomp/notes/camera.md: d = |byte - 0x7f|, 0 up to 0x2f, else
	/// (d - 0x30) / 0x4c up to 1, negative below 0x7f).
	fn retail_axis(b: u8) -> f32 {
		let d = (b as i32 - 0x7f).abs();
		let v = if d <= 0x2f { 0.0 } else { ((d - 0x30) as f32 / 0x4c as f32).min(1.0) };
		if b < 0x7f { -v } else { v }
	}

	/// What the game reads from a stick tilted by `t` (0..=1 of the throw) in the direction (ux, uy), both sticks the same.
	fn game_reads(ux: f32, uy: f32, t: f32) -> [f32; 4] {
		let (x, y) = ((ux * t * 32767.0) as i16, (uy * t * 32767.0) as i16);
		map_xinput(&XRaw { lx: x, ly: y, rx: x, ry: y, ..Default::default() }).sticks.map(retail_axis)
	}

	/// Player report #30 (v0.4.0): "having to tilt the analog stick quite far before he moves at all". On both sticks and in the
	/// four directions the game now reads the stick from about 28 % of the throw, the run threshold 0.82 from about 66 % and full
	/// from about 74 %, as with PCSX2's defaults; v0.4.0's rescaled throw put the game's dead zone on top of its own: about 53 %,
	/// 90 % and 98 %.
	#[test]
	fn the_game_reads_the_stick_from_its_own_dead_zone_on() {
		for (ux, uy) in [(1.0f32, 0.0f32), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
			for i in [0, 2] {
				// The tilt (in thousandths of the throw) where the game's reading of stick `i` reaches `level`.
				let at = |level: f32| (0..=1000).find(|&k| game_reads(ux, uy, k as f32 / 1000.0)[i..i + 2].iter().any(|v| v.abs() >= level)).unwrap_or(2000);
				let (moves, runs, full) = (at(1e-6), at(0.82), at(1.0));
				assert!((270..=300).contains(&moves), "({ux}, {uy}) stick {i}: the game reads it from {moves}/1000");
				assert!(runs <= 680 && full <= 760, "({ux}, {uy}) stick {i}: run from {runs}/1000, full from {full}/1000");
			}
		}
	}

	/// The bytes are within 2 of the ones PCSX2 gives the game by default (each half axis clamp(v * 1.33 * 255), then 127 + (raw + 1)
	/// / 2 or 127 - raw / 2; our centre is 0x80), and where the deadzone centres the stick, the game reads nothing from PCSX2's bytes
	/// either: the deadzone only keeps the rest noise from rac-rs's own readers.
	#[test]
	fn the_game_gets_pcsx2s_bytes_and_never_sees_the_deadzone() {
		let pcsx2 = |v: f32| {
			let raw = |v: f32| (v * 1.33 * 255.0).clamp(0.0, 255.0) as u32;
			if v > 0.0 { (127 + (raw(v) + 1) / 2) as u8 } else { (127 - raw(-v) / 2) as u8 }
		};
		for deg in (0..360).step_by(3) {
			let (ux, uy) = ((deg as f32).to_radians().fcos(), (deg as f32).to_radians().fsin());
			for k in 0..=200 {
				let t = k as f32 / 200.0;
				let (x, y) = ((ux * t * 32767.0) as i16, (uy * t * 32767.0) as i16);
				let s = map_xinput(&XRaw { lx: x, ly: y, rx: x, ry: y, ..Default::default() }).sticks;
				let p = [pcsx2(x as f32 / 32767.0), pcsx2(-(y as f32) / 32767.0)];
				for (j, &b) in s.iter().enumerate() {
					let q = p[j % 2];
					if b == 0x80 {
						assert_eq!(retail_axis(q), 0.0, "{deg} degrees, tilt {t}: the deadzone hides {q} from the game");
					} else {
						assert!((b as i32 - q as i32).abs() <= 2, "{deg} degrees, tilt {t}: {b} vs PCSX2's {q}");
					}
				}
			}
		}
	}

	/// Retail's own per-axis reading (decomp/notes/hero_crouch_ledge.md: d = |byte - 0x7f|, zero up to 0x2f, else (d - 0x30) / 0x4c
	/// up to 1) of a full gamepad push at any angle gives a stick strength of at least 0.82, the run threshold.
	#[test]
	fn a_full_push_runs_at_every_angle() {
		let axis = |b: u8| {
			let d = (b as i32 - 0x7f).abs();
			if d <= 0x2f { 0.0 } else { ((d - 0x30) as f32 / 0x4c as f32).min(1.0) }
		};
		for deg in 0..360 {
			let a = (deg as f32).to_radians();
			let s = map_xinput(&XRaw { lx: (a.fcos() * 32767.0) as i16, ly: (a.fsin() * 32767.0) as i16, ..Default::default() }).sticks;
			let m = axis(s[2]).fhypot(axis(s[3])).min(1.0);
			assert!(m >= 0.82, "{deg} degrees: strength {m}");
		}
	}

	#[test]
	fn keyboard_and_pad_work_together() {
		let kb = (0x40u16, [0x80, 0x80, 0x00, 0x80]); // Space held, A held (left stick left)
		let pad = PadState { buttons: 0x20, sticks: [0x80, 0x80, 0xff, 0x80] }; // Circle, stick right (smaller deflection than the key? no: bigger)
		let (b, s) = merge(kb, &pad);
		assert_eq!(b, 0x60, "both buttons");
		assert_eq!(s[2], 0x00, "the keyboard's full deflection wins over the pad's 0x7f push");
		let (b2, s2) = merge((0, [0x80; 4]), &pad);
		assert_eq!((b2, s2[2]), (0x20, 0xff), "the pad alone");
		assert_eq!(merge(kb, &PadState::IDLE), kb, "an idle pad changes nothing");
	}

	#[test]
	fn only_real_pad_input_switches_the_device() {
		let idle = PadState::IDLE;
		assert!(!pad_activity(&idle, &idle), "idle");
		let held = PadState { buttons: 0x40, sticks: [0x80; 4] };
		assert!(pad_activity(&idle, &held), "button down");
		assert!(!pad_activity(&held, &held), "button held");
		assert!(!pad_activity(&held, &idle), "button released");
		let pushed = PadState { buttons: 0, sticks: [0x80, 0x80, 0xc0, 0x80] };
		assert!(pad_activity(&idle, &pushed), "stick leaves the deadzone");
		assert!(!pad_activity(&pushed, &pushed), "stick held steady");
		assert!(!pad_activity(&pushed, &PadState { buttons: 0, sticks: [0x80, 0x80, 0xc8, 0x80] }), "small wobble while held");
		assert!(pad_activity(&pushed, &PadState { buttons: 0, sticks: [0x80, 0x80, 0xff, 0x80] }), "a big move");
		assert!(!pad_activity(&pushed, &idle), "stick back to centre");
	}

	/// The controller map reaches `map_xinput`; a remapped pad drives the new PS2 buttons.
	#[test]
	fn remapped_controller_buttons() {
		use super::super::binds;
		binds::init(None, Some("cross:b,circle:a,l2:lb"));
		assert_eq!(map_xinput(&x(0x1000)).buttons, 0x20, "A is Circle now");
		assert_eq!(map_xinput(&x(0x2000)).buttons, 0x40, "B is Cross now");
		assert_eq!(map_xinput(&x(0x100)).buttons, 1 | 4, "LB is L2 now and still L1 (a button may drive several functions)");
		assert_eq!(map_xinput(&x(0x1000 | 0x2000)).buttons, 0x60);
		// The file of a player who moved L1 off LB as well (the Controls page lists every function that differs from the default).
		binds::init(None, Some("cross:b,circle:a,l2:lb,l1:lt"));
		assert_eq!(map_xinput(&x(0x100)).buttons, 1, "LB is L2 only");
		binds::init(None, None);
		assert_eq!(map_xinput(&x(0x1000)).buttons, 0x40);
		assert_eq!(phys_mask(&XRaw { buttons: 0x1000, lt: 200, rt: 10, ..Default::default() }), 0x1000 | 1 << 16);
	}
}
