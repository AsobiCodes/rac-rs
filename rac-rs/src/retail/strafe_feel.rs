//! Strafe feel (PC extra, part of strafe.rs's "Controls: Modern"): while RMB aims, the ground movement and the strafe flips
//! take the numbers of R&C2's strafe, measured with the same scripted input in both games (2026-10-23,
//! scratch_strafe_cmp\RESULTS.md in the dev folder). R&C1's own run, stand and crouch high jump still run; `pre_field` only changes
//! their speed words between two fields (core RAM, the same on every level and on both discs; speeds per field = u/s * dt).
//! - Start and reversal: the run gains speed 2.4 times as fast as retail's (18 u/s^2 like R&C2 instead of 7.5): full speed
//!   after about 16 fields instead of 39; the logic facing follows the stick at once, so the run goes straight along it, and
//!   keeps only its speed along the new heading (a reversal starts again from 0, like R&C2's).
//! - Stop: letting go of the stick takes retail's stand rule (state 0) instead of the 30 field skid (state 3), also right
//!   after a landing, and the slide the stand carries is over within 5 fields (about 0.15 u), like R&C2.
//! - Flips: 5 u/s along the flip from the press field (standing flips too), easing to 2 u/s while the stick is let go in the
//!   air; the landing runs on at the flip's speed. Chaining (Cross pressed just before the landing) is strafe/chain.rs.
//! Not aiming (or Strafe Off) nothing is written here: retail stays bit-exact.
use crate::fmath::Fm;

/// Core words (hero_movement.md, hero_crouch_ledge.md): state, run speed (gait, slews toward the target speed), the target
/// speed of the last field (0 with the stick let go), the carried slide of the stand state (vec4, SetState(0) copies the last
/// move 0x13f460 into it), and dt.
const HERO_STATE: u32 = 0x1413d4;
const RUN_SPEED: u32 = 0x13f4e4;
const RUN_TARGET: u32 = 0x13f4e0;
/// The logic facing and the stick's heading (camera relative) of the last field.
const HERO_YAW: u32 = 0x13f3e8;
const HERO_WISH: u32 = 0x13f4d0;
const CARRY: u32 = 0x13f4a0;
const DT: u32 = 0x15ed6c;
/// State 0xb's speed ramp along the flip heading (retail eases it toward 4.2 u/s at 8 u/s^2, toward 0 late in the fall) and
/// its landing count (0 in the air).
const FLIP_RAMP: u32 = 0x13f7a8;
const FLIP_LANDED: u32 = 0x13f768;
/// State 0xb's ground speed (SetState: the entry step, 0 under 1.5 u/s): its run speed every tick (0 for a back flip), so the
/// landing field's step and the run after the landing (Eudora OV:29726-29731).
const FLIP_ENTRY: u32 = 0x13f7a4;
/// The jump's descending flag (s16) and its predicted ticks to the ground (camera.md: written while descending), which pick
/// the ramp's target; the state tick (run state: ticks since it started); retail's tick scale (scale(n) = n * k + 0.5, PAL
/// k = 5/6, USA 1) and D2 (dt^2, the ramp step's unit).
const FALLING: u32 = 0x13f76e;
const LAND_TICKS: u32 = 0x13f79c;
const STATE_TICK: u32 = 0x13f4e8;
const TICK_K: u32 = 0x15ed68;
const D2: u32 = 0x15ed70;
const STAND: u32 = 0;
const RUN: u32 = 2;
const FLIP: u32 = 0xb;

/// Run acceleration and (stick let go) deceleration, times retail's own step: R&C2 gains 0.36 u/s per field (18 u/s^2, retail
/// 7.5) and loses about 0.36 u/s per field when the stick is let go (retail 8.5).
const ACCEL_K: f32 = 2.4;
const DECEL_K: f32 = 2.2;
/// The run speed of the second let-go field (u/s; under the stop rule's stand speed 2.7: about 1.2 u/s moved, as R&C2's second
/// stop field), and the extra shortening of the stand's slide after it (u/s^2, retail 12.6 + 5 = R&C2's 17.5).
const STOP_SNAP: f32 = 1.35;
const STOP_EXTRA: f32 = 5.0;
/// Below this stick length the stick counts as let go (retail's stop rule: 0.17), above `HELD` as pushed.
const RELEASE: f32 = 0.15;
const HELD: f32 = 0.5;
/// Flip speed along the flip with the stick pushed / let go (u/s), how fast it changes between them (u/s^2: R&C2 5 -> 2 in
/// 4 fields), and retail's ramp step (times D2 per tick) and ramp target (u/s) before the end of the fall.
const FLIP_FAST: f32 = 5.0;
const FLIP_SLOW: f32 = 2.0;
const FLIP_EASE: f32 = 37.0;
const RAMP_STEP: f32 = 8.0;
const RAMP_TARGET: f32 = 4.2;

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Feel {
	/// Hero state and run speed as the last `pre_field` left them (after its writes), and whether they are known.
	state: u32,
	speed: f32,
	known: bool,
	/// Fields since a strafe stop asked for the stand (0 = none).
	stop: u8,
	/// A strafe flip is on (from `flip_start` to the field after its landing) and its speed along the flip (u per field).
	flip: bool,
	want: f32,
}

impl Feel {
	pub(super) const fn new() -> Self {
		Feel { state: 0, speed: 0.0, known: false, stop: 0, flip: false, want: 0.0 }
	}

	/// A strafe flip is on (its fields and its touchdown field).
	pub(super) fn flipping(&self) -> bool {
		self.flip
	}
}

fn wrf(m: &mut ee_oracle::Machine, a: u32, v: f32) {
	m.cpu.mem.wr32(a, v.to_bits());
}

/// Length of a pad stick (bytes 0..255, centre 128) as -1..1 per axis.
pub(super) fn stick_len(x: u8, y: u8) -> f32 {
	let f = |b: u8| ((b as f32 - 128.0) / 127.0).clamp(-1.0, 1.0);
	(f(x) * f(x) + f(y) * f(y)).sqrt().min(1.0)
}

/// Retail's tick count for `n` 60 Hz ticks (FUN_001f14b8: (int)(n * k + 0.5) with the EE FPU rounding every step toward
/// zero, decomp/notes/README.md; PAL scale(25) = 21, scale(12) = 10, scale(9) = 7).
fn scale(k: f32, n: i32) -> i32 {
	// A positive f64 to f32, rounded toward zero (n * k and + 0.5 are exact in f64).
	let rz = |x: f64| {
		let f = x as f32;
		if f as f64 > x { f32::from_bits(f.to_bits() - 1) } else { f }
	};
	(rz(rz(n as f64 * k as f64) as f64 + 0.5)) as i32
}

/// `v` moved toward `to` by at most `step`.
fn ease(v: f32, to: f32, step: f32) -> f32 {
	if v < to { (v + step).min(to) } else { (v - step).max(to) }
}

/// The run speed after retail's step `d` of the last field (`g` now, `t` the target): a gain or a let-go loss scaled up.
fn run_speed(g: f32, d: f32, t: f32) -> f32 {
	if d > 0.0 && g < t {
		(g + d * (ACCEL_K - 1.0)).min(t)
	} else if d < 0.0 && t <= 0.0 {
		(g + d * (DECEL_K - 1.0)).max(0.0)
	} else {
		g
	}
}

/// The ramp value `v` that retail's step of a flip field turns into `want`: v moves toward `target` by at most `step`.
fn ahead(want: f32, target: f32, step: f32) -> f32 {
	if want > target {
		want + step
	} else if want < target {
		want - step
	} else {
		want
	}
}

/// The ramp value for this field to move at `want` (Eudora OV:29733-29741: retail eases the ramp toward 4.2 u/s, or toward 0
/// once the fall is under scale(12) ticks from the ground, by 8 * D2 per tick), from the words as the field starts.
fn ramp_for(mem: &mut ee_oracle::mem::Mem, want: f32) -> f32 {
	let dt = f32::from_bits(mem.rd32(DT));
	let late = mem.rd16(FALLING) != 0 && (mem.rd32(LAND_TICKS) as i32) < scale(f32::from_bits(mem.rd32(TICK_K)), 12);
	ahead(want, if late { 0.0 } else { dt * RAMP_TARGET }, f32::from_bits(mem.rd32(D2)) * RAMP_STEP)
}

/// The jump hand has just entered state 0xb for a strafe flip (strafe.rs `jump`): the ramp starts at the flip speed (written
/// again before every flip field).
pub(super) fn flip_start(mem: &mut ee_oracle::mem::Mem, f: &mut Feel) {
	let dt = f32::from_bits(mem.rd32(DT));
	f.want = FLIP_FAST * dt;
	f.flip = true;
	let v = ramp_for(mem, f.want);
	mem.wr32(FLIP_RAMP, v.to_bits());
}

/// Worker, before each field, with Strafe on (`aiming`: RMB aims and Ratchet is himself, not in a vehicle or cutscene; the
/// flip part also runs for a flip that outlasts the aim). `stick` = this field's left stick bytes. Returns a log line.
pub(super) fn pre_field(m: &mut ee_oracle::Machine, stick: (u8, u8), aiming: bool, f: &mut Feel) -> Option<String> {
	let dt = m.rdf(DT);
	let st = m.rd32(HERO_STATE);
	let len = stick_len(stick.0, stick.1);
	let mut line = None;
	if f.flip {
		line = flip_field(m, st, len > HELD, dt, f);
	}
	if !aiming {
		f.known = false;
		f.stop = 0;
		return line;
	}
	if st == RUN && f.known && f.state == RUN {
		let g = m.rdf(RUN_SPEED);
		let n = run_speed(g, g - f.speed, m.rdf(RUN_TARGET));
		if n != g {
			wrf(m, RUN_SPEED, n);
		}
	}
	if st == RUN && len < RELEASE {
		// Let go while running. Retail's stop rule asks for the stick let go a field after the pad (its stick length 0x1415ec
		// lags one field) and measures that field's own travel: a run speed under its stand speed there makes it stand up
		// (state 0) instead of skidding (state 3). The first let-go field runs on at speed, like R&C2's.
		if f.stop > 0 {
			if m.rdf(RUN_SPEED) > STOP_SNAP * dt {
				wrf(m, RUN_SPEED, STOP_SNAP * dt);
			}
			// The rule stands up only once the run is older than scale(25) ticks; a younger run (a landing ran on into it)
			// stays in the run state at speed 0, frozen in its pose, until then: counted as old enough.
			let old = scale(m.rdf(TICK_K), 25) + 1;
			if (m.rd32(STATE_TICK) as i32) < old {
				m.cpu.mem.wr32(STATE_TICK, old as u32);
			}
		}
		f.stop = f.stop.saturating_add(1);
	} else if st == RUN {
		// Pushed while running: the logic facing (the drawn one is the aim, modcam.rs) goes straight to the stick's heading of
		// the last field instead of turning at retail's rate (0.2 rad per field), so the run goes along the stick at any speed;
		// retail turns the path to the facing once the run is fast, which bent the faster start and reversal into an arc.
		// The speed kept is the run's along the new heading (R&C2: a reversal drops to about 0 the next field and builds up
		// again; a slight turn keeps nearly all of it). A stick turning no faster than retail's own turn loses nothing.
		let wish = m.rdf(HERO_WISH);
		let keep = (wish - m.rdf(HERO_YAW)).fcos().max(0.0);
		if keep < 1.0 {
			wrf(m, RUN_SPEED, m.rdf(RUN_SPEED) * keep);
		}
		wrf(m, HERO_YAW, wish);
		f.stop = 0;
	} else if st == STAND && f.stop > 0 {
		line = stop_slide(m, dt, f).or(line);
	} else {
		f.stop = 0;
	}
	f.state = st;
	f.speed = m.rdf(RUN_SPEED);
	f.known = true;
	line
}

/// A stand after a strafe stop: its carried slide (the run's last step, about `STOP_SNAP`) is shortened by `STOP_EXTRA` more
/// than retail does each field.
fn stop_slide(m: &mut ee_oracle::Machine, dt: f32, f: &mut Feel) -> Option<String> {
	let c = [m.rdf(CARRY), m.rdf(CARRY + 4), m.rdf(CARRY + 8)];
	let len = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
	if len <= 0.001 {
		f.stop = 0;
		return None;
	}
	let cap = (len - STOP_EXTRA * dt * dt).max(0.0);
	let k = f.stop;
	f.stop = f.stop.saturating_add(1);
	if len <= cap {
		return None;
	}
	for (i, v) in c.iter().enumerate() {
		wrf(m, CARRY + 4 * i as u32, v * cap / len);
	}
	Some(format!("strafe_stop slide {k} {:.3} -> {:.3} u/s", len / dt, cap / dt))
}

/// A strafe flip, before each of its fields: the flip speed (fast with the stick pushed, easing to slow once it is let go) is
/// its ground speed, and in the air the ramp is set so the field moves at it. After the landing, with the stick pushed, the
/// run starts at it (a back flip lands with run speed 0).
fn flip_field(m: &mut ee_oracle::Machine, st: u32, held: bool, dt: f32, f: &mut Feel) -> Option<String> {
	if st != FLIP {
		f.flip = false;
		if held && st == RUN && m.rdf(RUN_SPEED) < f.want {
			wrf(m, RUN_SPEED, f.want);
			return Some(format!("strafe_flip landing run speed {:.3} u/s", f.want / dt));
		}
		return None;
	}
	f.want = ease(f.want, if held { FLIP_FAST } else { FLIP_SLOW } * dt, FLIP_EASE * dt * dt);
	wrf(m, FLIP_ENTRY, f.want);
	if m.rd32(FLIP_LANDED) == 0 {
		let v = ramp_for(&mut m.cpu.mem, f.want);
		wrf(m, FLIP_RAMP, v);
	}
	None
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn stick_len_centre_and_edges() {
		assert_eq!(stick_len(0x80, 0x80), 0.0);
		assert!((stick_len(0xff, 0x80) - 1.0).abs() < 1e-6);
		assert!((stick_len(0x00, 0x80) - 1.0).abs() < 1e-6);
		assert!((stick_len(0x00, 0x00) - 1.0).abs() < 1e-6);
		assert!(stick_len(0x88, 0x80) < RELEASE);
	}

	#[test]
	fn run_speed_scales_retail_steps() {
		let dt = 0.02;
		let (step, top) = (7.5 * dt * dt, 5.7 * dt);
		// A gain is scaled and stops at the target; a let-go loss is scaled and stops at 0; anything else is retail's.
		assert!((run_speed(0.05, step, top) - (0.05 + step * (ACCEL_K - 1.0))).abs() < 1e-7);
		assert_eq!(run_speed(top - 1e-4, step, top), top);
		assert!((run_speed(0.05, -step, 0.0) - (0.05 - step * (DECEL_K - 1.0))).abs() < 1e-7);
		assert_eq!(run_speed(1e-4, -step, 0.0), 0.0);
		assert_eq!(run_speed(0.05, -step, top), 0.05);
		assert_eq!(run_speed(top, 0.0, top), top);
	}

	#[test]
	fn scale_matches_retail_ticks() {
		let pal = f32::from_bits(0x3f555555);
		assert_eq!((scale(pal, 25), scale(pal, 12), scale(pal, 9), scale(pal, 45)), (21, 10, 7, 37));
		assert_eq!((scale(1.0, 25), scale(1.0, 12)), (25, 12));
	}

	#[test]
	fn ramp_ahead_lands_on_want() {
		// Retail's step (FUN_0024e428): toward the target by at most `step`.
		let slew = |v: f32, t: f32, s: f32| v + (t - v).clamp(-s, s);
		let (dt, s) = (0.02, 8.0 * 4e-4);
		for want in [FLIP_FAST * dt, FLIP_SLOW * dt, RAMP_TARGET * dt, 0.07] {
			for t in [RAMP_TARGET * dt, 0.0] {
				assert!((slew(ahead(want, t, s), t, s) - want).abs() < 1e-7, "{want} {t}");
			}
		}
	}

	#[test]
	fn ease_never_overshoots() {
		assert_eq!(ease(0.1, 0.04, 0.0148), 0.1 - 0.0148);
		assert_eq!(ease(0.045, 0.04, 0.0148), 0.04);
		assert_eq!(ease(0.04, 0.1, 0.0148), 0.04 + 0.0148);
		assert_eq!(ease(0.099, 0.1, 0.0148), 0.1);
	}
}
