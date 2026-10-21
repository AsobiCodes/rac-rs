//! Mouse steering for the ship fights (a PC extra the player asked for: a keyboard stick is all-or-nothing, so aiming the ship
//! with WASD is nearly impossible).
//!
//! Retail [C]: while the hero is in state 0x32 (`0x1413d4`) and holds a ship moby (`0x140940`, class 0x45 Gemlik Base, 0x4da
//! Pokitaru, 0x563 Veldin orbit), the ship's update (Gemlik `FUN_002ba990`, flight part `FUN_002b9aa0`) steers from the LEFT
//! stick only, through the hero copies `0x141070` (x) and `0x141074` (y) of the pad floats `0x13ca48` / `0x13ca4c`: x turns the
//! ship (stick left = yaw up = turn left, bank follows), y pitches it (stick UP = nose down, the pull-back-to-climb scheme; the
//! game's Y-invert option `0x15eddc` is not read there). The right stick does nothing in a ship fight, and fire is Cross (0x40).
//!
//! What this file does: in a ship fight the mouse counts of the field (`FieldIn.mc.look`, as collected for the modern camera)
//! feed a leaky virtual stick that is added to the keyboard / gamepad stick bytes before `set_pad`:
//! `v = v * DECAY + counts * GAIN * sens`, deflection = `v` clamped to +-1 (x127 on the stick byte). Moving the mouse right /
//! left turns the ship right / left, up / down climbs / dives (the Y-invert option of the game flips it); stopping the mouse
//! lets the stick fall back to the centre in about half a second. With no mouse motion nothing is touched (the stick bytes stay
//! exactly what the keyboard / pad made), so a run without mouse input is bit-identical to a build without this file.
//! Outside a ship fight the state is cleared and the mouse does what it did before (modern camera).
//! `Snap.ship_fight` (same test) lets the left mouse button fire with Cross in a ship fight (retail's ship fire button;
//! elsewhere it stays Circle).
//!
//! Sensitivity: the existing `RAC_MODCAM_SENS` scales the gain (default 1.0).
//!
//! TURRETS (Batalia 8, Gaspar 9, Hoven 12; hero state 0x32 with NO ship moby held, game mode 0). The turret moby aims from the
//! same hero stick copies `0x141070` / `0x141074` (left stick, D-pad when its length is under 0.25), the game's Y option is
//! applied by the turret itself (default option 1: a stick UP looks DOWN in all three turrets [C measured, shots/t_*], option 0
//! flips it), fire is Circle (what the left mouse button already is). Two kinds of plant (decomp/notes/
//! ship_fights.md): Batalia and Hoven turn at a RATE set by the stick (spring toward `stick * max rate`), Gaspar's stick is an
//! ACCELERATION on a velocity that decays 0.97 a field. The pad bytes become floats with a dead zone of 47 bytes
//! (`(|b-128|-47)/76`), and under a vector length of 0.25 the game reads nothing, so the stick has a floor: any output sits in
//! 0.27..1.0.
//! The mouse therefore sends the opposite stick on Y (mouse up = stick down = look up with the default option) and inverts together
//! with the game's own option; mouse right turns right.
//! - Rate turrets: a leaky mouse-speed stick `d` (decay 0.7 a field: the turret stops soon after the mouse does), output
//!   `max(|d|, 0.27)` (the slowest speed the game reads, like a pad stick just past its dead zone), `d` scaled so `RAD_PER_COUNT`
//!   radians a count is full rate at `max rate` (per axis).
//! - Gaspar: the same `d` is a wanted turret speed `d * vmax`; each field the stick is the acceleration that brings the turret's
//!   real velocity (RAM `pvar+0x60/0x64`) to it, so it also brakes when the mouse stops; sigma-delta pulses make accelerations
//!   under the 0.27 floor.
//! A stick (keyboard / pad) held past 8 bytes from centre wins over the mouse on its axis. No mouse motion = bytes untouched.
use ee_oracle::Machine;
use crate::retail::disc::va;

const GAME_MODE: u32 = 0x15f5a4;
const HERO_STATE: u32 = 0x1413d4;
const HELD_MOBY: u32 = 0x140940;
/// The game's Y-invert option byte (modcam.rs `OPT_UD`): non-zero = normal (up = look up).
const OPT_UD: u32 = 0x15eddc;
/// Hero state "holding / piloting" and the three ship classes that fly with the left stick.
const STATE_SHIP: u32 = 0x32;
const SHIP_CLASSES: [u16; 3] = [0x45, 0x4da, 0x563];
/// Virtual stick: decay per field (20 ms) and deflection per mouse count. Steady mouse speed s counts / field gives
/// `s * GAIN / (1 - DECAY)` = 0.133 * s, so about 7.5 counts a field (375 counts a second) is full deflection. [I] feel, tune in game.
const DECAY: f32 = 0.85;
const GAIN: f32 = 0.02;

/// The turret levels (Batalia, Gaspar, Hoven) and the Gaspar turret moby class.
const LEVEL: u32 = 0x15ed84;
const NEAR_MOBY: u32 = 0x13f64c;
const GASPAR_TURRET: u16 = 0x4b1;
const FRAME_SCALE: u32 = 0x15ed70;
/// Turret tuning. [I] feel, tune in game.
/// Radians a turret turns per mouse count at full gain (the modern camera uses 0.003).
const RAD_PER_COUNT: f32 = 0.003;
/// Leak of the turret's mouse-speed stick per field (0.7 = the aim stops about 5 fields after the mouse).
const T_DECAY: f32 = 0.7;
/// Below this the leaky stick is at rest (about 1 count a field is needed to start).
const T_REST: f32 = 0.04;
/// Smallest stick the game reads (its 0.25 vector floor, plus a margin).
const FLOOR: f32 = 0.27;
/// Pad byte -> float: `(|b - 128| - 47) / 76`, clamped to 1 [C measured on Batalia, 2026-10-14].
const DEAD_BYTES: f32 = 47.0;
const SPAN_BYTES: f32 = 76.0;
/// A stick this far from centre (bytes) is a keyboard / pad press that wins over the mouse.
const PRESS_BYTES: i32 = 8;
/// Max turret rate per field at full stick, `[yaw, pitch]` (rad): Batalia 180 / 22.5 deg a second, Hoven 150 / 70, both x 0.02 s
/// [C gp constants]; Gaspar's terminal speed is `accel / (1 - 0.97)` (accel = frame scale 0.0004 * pi, * pi / 4).
const RATE_BATALIA: [f32; 2] = [0.062_83, 0.007_854];
const RATE_HOVEN: [f32; 2] = [0.052_36, 0.024_43];
const GASPAR_KEEP: f32 = 0.97;
const GASPAR_ACCEL: [f32; 2] = [std::f32::consts::PI, std::f32::consts::FRAC_PI_4];

/// Which turret and what its plant does this field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Turret {
	/// Rate controlled: max rate `[yaw, pitch]` per field.
	Rate([f32; 2]),
	/// Gaspar: the velocities `[yaw, pitch]` in stick sense (positive = what a stick right / down gives), and the acceleration a
	/// full stick adds per field.
	Gaspar { vel: [f32; 2], accel: [f32; 2] },
}

/// A turret section is on: gameplay (mode 0), hero state 0x32, no ship moby held, on Batalia / Gaspar / Hoven. Gaspar's turret is
/// the near moby 0x13f64c (class 0x4b1, state 5) whose pvar holds the velocities; without it the mouse leaves the sticks alone.
/// `invert` = the game's Y option is "inverted".
pub fn turret(m: &Machine, invert: bool) -> Option<Turret> {
	if m.rd32(va(GAME_MODE)) != 0 || m.rd32(HERO_STATE) != STATE_SHIP || in_ship_fight(m) {
		return None;
	}
	match m.rd32(LEVEL) {
		8 => Some(Turret::Rate(RATE_BATALIA)),
		12 => Some(Turret::Rate(RATE_HOVEN)),
		9 => {
			let t = m.rd32(NEAR_MOBY) & 0x01ff_ffff;
			if t == 0 || m.rd16(t + 0xa6) != GASPAR_TURRET || m.rd8(t + 0x20) != 5 {
				return None;
			}
			let pv = m.rd32(t + 0x78) & 0x01ff_ffff;
			let f = |a: u32| f32::from_bits(m.rd32(a));
			// Yaw: stick right adds -accel. Pitch: stick down adds +accel (Y option normal), the option flips it.
			let vel = [-f(pv + 0x60), if invert { -f(pv + 0x64) } else { f(pv + 0x64) }];
			let a = f(FRAME_SCALE);
			if !a.is_finite() || a <= 0.0 || vel.iter().any(|v| !v.is_finite()) {
				return None;
			}
			Some(Turret::Gaspar { vel, accel: [a * GASPAR_ACCEL[0], a * GASPAR_ACCEL[1]] })
		}
		_ => None,
	}
}

/// A ship fight is on: gameplay (mode 0), hero state 0x32 holding a moby of a ship class.
pub fn in_ship_fight(m: &Machine) -> bool {
	if m.rd32(va(GAME_MODE)) != 0 || m.rd32(HERO_STATE) != STATE_SHIP {
		return false;
	}
	let h = m.rd32(HELD_MOBY) & 0x01ff_ffff;
	h != 0 && SHIP_CLASSES.contains(&m.rd16(h + 0xa6))
}

/// The virtual stick (x: right is positive, y: mouse down is positive), kept by the worker.
#[derive(Default)]
pub struct State {
	v: [f32; 2],
	/// Turret: the leaky mouse-speed stick, the sigma-delta remainder and whether the mouse is steering the axis.
	tv: [f32; 2],
	tacc: [f32; 2],
	tact: [bool; 2],
}

fn sens() -> f32 {
	static S: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
	*S.get_or_init(|| std::env::var("RAC_MODCAM_SENS").ok().and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite() && *v > 0.0).unwrap_or(1.0))
}

impl State {
	/// The mouse counts of this field -> the stick bytes `[rx, ry, lx, ly]` handed to `set_pad`. `ship` = `in_ship_fight`,
	/// `invert` = the game's Y option is "inverted". Returns `sticks` untouched while the virtual stick is at rest.
	pub fn step(&mut self, ship: bool, invert: bool, look: [i16; 2], sticks: [u8; 4]) -> [u8; 4] {
		if !ship {
			self.v = [0.0; 2];
			return sticks;
		}
		let k = GAIN * sens();
		for i in 0..2 {
			self.v[i] = (self.v[i] * DECAY + look[i] as f32 * k).clamp(-1.0, 1.0);
			// Rest: below half a stick byte it is gone, so the bytes are exactly the keyboard's again.
			if self.v[i].abs() < 0.004 {
				self.v[i] = 0.0;
			}
		}
		if self.v == [0.0; 2] {
			return sticks;
		}
		// The game reads `(|b - 128| - 47) / 76`: a non-zero virtual stick starts past the dead zone, so small mouse motion steers
		// gently instead of vanishing in it. A keyboard / pad press on that axis wins.
		let put = |base: u8, d: f32| {
			if (base as i32 - 128).abs() > PRESS_BYTES || d == 0.0 {
				return base;
			}
			let off = (DEAD_BYTES + d.abs().min(1.0) * SPAN_BYTES).round() as i32;
			(128 + d.signum() as i32 * off).clamp(0, 255) as u8
		};
		let ud = if invert { -1.0 } else { 1.0 };
		let mut s = sticks;
		// x: mouse right = stick right (lx byte up) = turn right. y: mouse down = stick up (ly byte down) = dive; mouse up climbs.
		s[2] = put(s[2], self.v[0]);
		s[3] = put(s[3], -self.v[1] * ud);
		s
	}

	#[cfg(test)]
	pub fn at_rest(&self) -> bool {
		self.v == [0.0; 2] && self.tv == [0.0; 2] && self.tact == [false; 2]
	}

	/// Turret sections: the mouse counts of the field -> the stick bytes `[rx, ry, lx, ly]`. `None` clears the state.
	/// Mouse right = stick right, mouse up = stick down (the turret applies the game's Y option itself, so the mouse inverts with it).
	pub fn step_turret(&mut self, t: Option<Turret>, look: [i16; 2], sticks: [u8; 4]) -> [u8; 4] {
		let Some(t) = t else {
			self.tv = [0.0; 2];
			self.tacc = [0.0; 2];
			self.tact = [false; 2];
			return sticks;
		};
		let k = RAD_PER_COUNT * sens() * (1.0 - T_DECAY);
		let full = match t {
			Turret::Rate(r) => r,
			Turret::Gaspar { accel, .. } => [accel[0] / (1.0 - GASPAR_KEEP), accel[1] / (1.0 - GASPAR_KEEP)],
		};
		let mut s = sticks;
		for i in 0..2 {
			if (sticks[2 + i] as i32 - 128).abs() > PRESS_BYTES {
				// Keyboard / pad press: it wins, the mouse starts over.
				self.tv[i] = 0.0;
				self.tacc[i] = 0.0;
				self.tact[i] = false;
				continue;
			}
			// x: mouse right = stick right. y: mouse up = stick DOWN (see the module doc: with the game's default Y option a stick
			// UP looks down in all three turrets, so the mouse needs the opposite stick; the turret's own Y option then inverts it).
			let sgn = if i == 0 { 1.0 } else { -1.0 };
			self.tv[i] = (self.tv[i] * T_DECAY + sgn * look[i] as f32 * k / full[i]).clamp(-1.0, 1.0);
			// The state keeps its tail (so a slow steady mouse builds up past the threshold); only the output has the threshold.
			if self.tv[i].abs() < 0.002 {
				self.tv[i] = 0.0;
			}
			let d = if self.tv[i].abs() < T_REST { 0.0 } else { self.tv[i] };
			let out = match t {
				Turret::Rate(_) => {
					if d == 0.0 {
						continue;
					}
					d.signum() * d.abs().max(FLOOR)
				}
				Turret::Gaspar { vel, accel } => {
					if d != 0.0 {
						self.tact[i] = true;
					}
					if !self.tact[i] {
						continue;
					}
					if d == 0.0 && vel[i].abs() < accel[i] * 0.1 {
						// Stopped: hand the stick back.
						self.tact[i] = false;
						self.tacc[i] = 0.0;
						continue;
					}
					let c = ((d * full[i] - vel[i]) / accel[i]).clamp(-1.0, 1.0);
					if c.abs() >= FLOOR {
						self.tacc[i] = 0.0;
						c
					} else {
						// Sigma-delta: accelerations under the floor are pulses of the floor, so the average is right.
						self.tacc[i] += c;
						if self.tacc[i].abs() >= FLOOR {
							let o = self.tacc[i].signum() * FLOOR;
							self.tacc[i] -= o;
							o
						} else {
							0.0
						}
					}
				}
			};
			s[2 + i] = if out == 0.0 {
				0x80
			} else {
				let off = (DEAD_BYTES + out.abs().min(1.0) * SPAN_BYTES).round() as i32;
				(128 + out.signum() as i32 * off).clamp(0, 255) as u8
			};
		}
		s
	}
}

/// The left mouse button as pad input: `(circle, extra buttons)`. Elsewhere it is Circle (fire the weapon); in a ship fight it is
/// Cross (0x40), the ship's fire button, and Circle stays off.
pub fn left_button(ship_fight: bool, lmb: bool) -> (bool, u16) {
	if ship_fight {
		(false, if lmb { 0x40 } else { 0 })
	} else {
		(lmb, 0)
	}
}

/// The game's Y option of the field: true when inverted.
pub fn y_inverted(m: &Machine) -> bool {
	m.rd8(OPT_UD) == 0
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn no_mouse_leaves_the_sticks_alone() {
		let mut s = State::default();
		for _ in 0..100 {
			assert_eq!(s.step(true, false, [0, 0], [0x10, 0x20, 0x3f, 0xff]), [0x10, 0x20, 0x3f, 0xff]);
		}
		assert!(s.at_rest());
	}

	#[test]
	fn right_and_up_move_the_stick_right_and_back_then_decay() {
		let mut s = State::default();
		let out = s.step(true, false, [10, -10], [0x80; 4]);
		assert!(out[2] > 0x80, "mouse right = stick right");
		assert!(out[3] > 0x80, "mouse up = stick down = climb");
		assert_eq!((out[0], out[1]), (0x80, 0x80), "the right stick is never touched");
		let mut last = out;
		for _ in 0..60 {
			last = s.step(true, false, [0, 0], [0x80; 4]);
		}
		assert_eq!(last, [0x80; 4], "falls back to centre");
		assert!(s.at_rest());
		// Inverted option flips only the vertical.
		let mut s = State::default();
		let out = s.step(true, true, [10, -10], [0x80; 4]);
		assert!(out[2] > 0x80 && out[3] < 0x80);
	}

	fn byte_to_float(b: u8) -> f32 {
		(((b as i32 - 128).abs() as f32 - DEAD_BYTES) / SPAN_BYTES).clamp(0.0, 1.0) * (b as f32 - 128.0).signum()
	}

	#[test]
	fn turret_no_mouse_leaves_the_sticks_alone() {
		let mut s = State::default();
		for t in [Turret::Rate(RATE_BATALIA), Turret::Gaspar { vel: [0.0; 2], accel: [0.001, 0.0003] }] {
			for _ in 0..100 {
				assert_eq!(s.step_turret(Some(t), [0, 0], [0x10, 0x20, 0x3f, 0xff]), [0x10, 0x20, 0x3f, 0xff]);
				assert_eq!(s.step_turret(Some(t), [0, 0], [0x80; 4]), [0x80; 4]);
			}
		}
		assert!(s.at_rest());
	}

	#[test]
	fn rate_turret_follows_the_mouse_speed_above_the_game_floor() {
		let mut s = State::default();
		let t = Turret::Rate(RATE_BATALIA);
		let mut last = [0x80; 4];
		for _ in 0..40 {
			last = s.step_turret(Some(t), [6, -3], [0x80; 4]);
		}
		assert!(byte_to_float(last[2]) > 0.25, "mouse right = stick right, over the game's 0.25 floor");
		assert!(byte_to_float(last[3]) > 0.25, "mouse up = stick down (a turret looks down on stick up)");
		assert_eq!((last[0], last[1]), (0x80, 0x80));
		// Stopping the mouse stops the stick in a few fields.
		let mut n = 0;
		while s.step_turret(Some(t), [0, 0], [0x80; 4]) != [0x80; 4] {
			n += 1;
			assert!(n < 25);
		}
		for _ in 0..30 {
			s.step_turret(Some(t), [0, 0], [0x80; 4]);
		}
		assert!(s.at_rest());
		// A key held wins.
		let out = s.step_turret(Some(t), [20, 0], [0x80, 0x80, 0x00, 0x80]);
		assert_eq!(out[2], 0x00);
	}

	#[test]
	fn gaspar_brakes_to_a_stop() {
		// The game's integrator: v += accel * stick (stick sense), angle += v, v *= 0.97.
		let accel = [0.0004 * std::f32::consts::PI, 0.0004 * std::f32::consts::FRAC_PI_4];
		let mut s = State::default();
		let (mut v, mut ang) = ([0.0f32; 2], [0.0f32; 2]);
		for field in 0..400 {
			let look = if field < 30 { [6, 0] } else { [0, 0] };
			let out = s.step_turret(Some(Turret::Gaspar { vel: v, accel }), look, [0x80; 4]);
			for i in 0..2 {
				v[i] += accel[i] * byte_to_float(out[2 + i]);
				ang[i] += v[i];
				v[i] *= GASPAR_KEEP;
			}
			if field == 30 {
				assert!(ang[0] > 0.1, "the turret has turned: {}", ang[0]);
			}
		}
		assert!(v[0].abs() < 1e-4 && ang[0] > 0.1, "stopped: v {} ang {}", v[0], ang[0]);
		assert!(ang[1].abs() < 0.01, "pitch untouched: {}", ang[1]);
		assert!(s.at_rest());
	}

	#[test]
	fn left_button_fires_with_cross_in_a_ship_fight_only() {
		assert_eq!(left_button(false, true), (true, 0), "on foot: Circle");
		assert_eq!(left_button(true, true), (false, 0x40), "ship: Cross");
		assert_eq!(left_button(true, false), (false, 0));
		assert_eq!(left_button(false, false), (false, 0));
	}

	#[test]
	fn clamped_and_cleared_outside_a_ship_fight() {
		let mut s = State::default();
		let out = s.step(true, false, [3000, 3000], [0x80; 4]);
		assert_eq!((out[2], out[3]), (128 + 123, 128 - 123), "full stick = dead zone + span");
		let out = s.step(true, false, [3000, 3000], [0xf0, 0xf0, 0xf0, 0xf0]);
		assert_eq!((out[2], out[3]), (0xf0, 0xf0), "a keyboard press wins");
		let out = s.step(true, false, [1, 0], [0x80; 4]);
		assert!(out[2] as i32 - 128 > 47, "a small virtual stick starts past the game's dead zone");
		assert_eq!(s.step(false, false, [5, 5], [0x80; 4]), [0x80; 4]);
		assert!(s.at_rest());
	}
}
