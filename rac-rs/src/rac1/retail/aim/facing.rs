//! [PC] Strafe aiming Steady (modcam/steady.rs, GitHub #73, lane nstr; child of `aim.rs`): there is no crosshair, so the weapons whose
//! retail code picks its own target (thrown, homing) aim as retail does with Ratchet facing the camera heading, not at the point under
//! the screen centre. Every function here runs only with Steady on, behind `active()`: Raised / Shoulder / Off never reach this file.
//! - Bomb Glove (held updater, class 192, Eudora 0x2b50b0 [C]): retail's own updater runs with the hero frame words it aims from turned
//!   by the drawn turn (`drawn_turn`: aim yaw - logic yaw while he strafe-walks; nothing standing): the rows 0x13f990 / 0x13f9a0 /
//!   0x13f9b0 (copy of hero moby +0xc0 / +0xd0 / +0xe0; row 0 = forward), the heading 0x13f9c0 and HERO_YAW (bomb_velocity's
//!   fallback test). So its default target (8.5 m ahead), its lock scan (cone pi/4, pi/2 under 4 m, 15 m, line of sight) and the throw
//!   point beside the glove are those of a Ratchet facing the heading. Its lock target word stays (retail's throw state turns him to
//!   it), and bomb_velocity runs retail's own code (`bomb_velocity` only logs).
//! - Mine Glove: retail's target point (60 steps ahead on the logic heading, no lock) turned to the drawn frame (`mine_target`).
//! - Devastator / RYNO: retail aims along the camera yaw and picks its own target (its own searches); only the muzzle is the drawn one
//!   (`muzzle`).
use super::*;

/// The (x, y) pairs the held updater aims from: rows 0..2 of the hero frame copy and the heading (cos yaw, sin yaw).
const XY: [u32; 4] = [0x13f990, 0x13f9a0, 0x13f9b0, HERO_HEADING];
const N: usize = 9;

/// The turned words: the four (x, y) pairs, then HERO_YAW.
fn addrs() -> [u32; N] {
	let mut a = [HERO_YAW; N];
	for (k, x) in XY.iter().enumerate() {
		a[2 * k] = *x;
		a[2 * k + 1] = x + 4;
	}
	a
}

fn wrap(a: f32) -> f32 {
	use std::f32::consts::PI;
	if a > PI {
		a - 2.0 * PI
	} else if a < -PI {
		a + 2.0 * PI
	} else {
		a
	}
}

/// The words turned by (sn, cs): each (x, y) pair rotated about z, the yaw word plus the angle (wrapped to -pi..pi).
fn turned(old: [u32; N], sn: f32, cs: f32) -> [u32; N] {
	let mut n = old;
	for k in 0..XY.len() {
		let (x, y) = (f32::from_bits(old[2 * k]), f32::from_bits(old[2 * k + 1]));
		n[2 * k] = (cs * x - sn * y).to_bits();
		n[2 * k + 1] = (sn * x + cs * y).to_bits();
	}
	n[N - 1] = wrap(f32::from_bits(old[N - 1]) + sn.fatan2(cs)).to_bits();
	n
}

/// The words after the call: the old value where retail left the turned one, retail's own write kept.
fn after(old: [u32; N], new: [u32; N], now: [u32; N]) -> [u32; N] {
	std::array::from_fn(|i| if now[i] == new[i] { old[i] } else { now[i] })
}

/// Whether the aim log (RAC_AIM_LOG) is on: the trace lines below are formatted only then.
fn logging() -> bool {
	ST.lock().is_ok_and(|mut s| log_on(&mut s))
}

/// `bomb_held_any` with Steady: retail's held updater with the hero frame words turned to the drawn Ratchet (none standing), the
/// words retail did not rewrite given back; the lock target word it set stays.
pub(super) fn bomb_held(c: &mut Cpu, entry: u32) -> bool {
	let a = addrs();
	let old = a.map(|x| c.mem.rd32(x));
	let turn = drawn_turn(c);
	let new = turn.map_or(old, |(sn, cs, _)| turned(old, sn, cs));
	for i in 0..N {
		if new[i] != old[i] {
			c.mem.wr32(a[i], new[i]);
		}
	}
	c.call(entry);
	let now = a.map(|x| c.mem.rd32(x));
	let keep = after(old, new, now);
	for i in 0..N {
		if keep[i] != now[i] {
			c.mem.wr32(a[i], keep[i]);
		}
	}
	if logging() {
		let lock = c.mem.rd32(LOCK_TARGET) & PTR;
		let h = c.mem.rd32(HERO_MOBY) & PTR;
		let hp = rv(c, h + 0x10);
		let lp = if lock != 0 { rv(c, lock + 0x10) } else { [0.0; 3] };
		let d = turn.map_or(0.0, |(sn, cs, _)| sn.fatan2(cs));
		let (yaw, ay) = (f32::from_bits(new[N - 1]), modcam::aim_yaw().unwrap_or(0.0));
		trace("bomb_held", format!("steady turn {d:.3} yaw {yaw:.3} aim {ay:.3} hero {:.2} {:.2} {:.2} lock {lock:#x} at {:.2} {:.2} {:.2}", hp[0], hp[1], hp[2], lp[0], lp[1], lp[2]));
	}
	true
}

/// `bomb_velocity_any` with Steady: retail's own code runs (false); the log gets the from and target points retail solves the lob for.
pub(super) fn bomb_velocity(c: &mut Cpu) -> bool {
	if logging() {
		let (fp, tp) = (ptr(c.g(4)), ptr(c.g(5)));
		let (f, t) = (rv(c, fp), rv(c, tp));
		let (lock, hy) = (c.mem.rd32(LOCK_TARGET) & PTR, rf(c, HERO_YAW));
		let od = off_drawn(c, t);
		trace("bomb_velocity", format!("steady from {:.2} {:.2} {:.2} target {:.2} {:.2} {:.2} hyaw {hy:.3} lock {lock:#x} target_drawn {:.2} {:.2}", f[0], f[1], f[2], t[0], t[1], t[2], od.0, od.1));
	}
	false
}

/// `mine_velocity_any`'s target point: with Steady retail's own (the caller's target buffer) turned to the drawn frame, else `aim`.
pub(super) fn mine_target(c: &mut Cpu, aim: [f32; 3]) -> [f32; 3] {
	if !modcam::steady::on() {
		return aim;
	}
	let tp = ptr(c.g(5));
	let t = rv(c, tp);
	drawn_point(c, t)
}

/// The Devastator / RYNO creator hooks with Steady: the muzzle (the caller's buffer at a1) turned to the drawn Ratchet, then retail's
/// own creator runs (false) with its yaw, pitch and target.
pub(super) fn muzzle(c: &mut Cpu) -> bool {
	let mp = ptr(c.g(5));
	let mu = drawn_at(c, mp);
	if logging() {
		// Devastator: f12 yaw, f13 pitch; RYNO: f12 target height, f13 yaw, f14 -elevation.
		let f = [12, 13, 14].map(|i| f32::from_bits(c.f[i]));
		trace("steady_muzzle", format!("muzzle {:.2} {:.2} {:.2} f12 {:.3} f13 {:.3} f14 {:.3} target {:#x}", mu[0], mu[1], mu[2], f[0], f[1], f[2], c.g(6) as u32));
	}
	false
}

#[cfg(test)]
mod tests {
	use super::*;

	fn words(v: [f32; N]) -> [u32; N] {
		v.map(f32::to_bits)
	}

	#[test]
	fn the_frame_turns_with_the_drawn_ratchet() {
		// Logic yaw 0 (rows = identity, heading (1, 0)), drawn a quarter turn left (sin 1, cos 0).
		let old = words([1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
		let n = turned(old, 1.0, 0.0);
		let f = n.map(f32::from_bits);
		assert_eq!(&f[..8], &[0.0, 1.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0], "forward -> +y, left -> -x, up stays, heading -> +y");
		assert!((f[8] - std::f32::consts::FRAC_PI_2).abs() < 1e-6, "yaw + the turn: {}", f[8]);
		let w = turned(words([1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 3.0]), 1.0, 0.0).map(f32::from_bits);
		assert!((w[8] - (3.0 + std::f32::consts::FRAC_PI_2 - 2.0 * std::f32::consts::PI)).abs() < 1e-5, "wrapped to -pi..pi: {}", w[8]);
		assert_eq!(addrs(), [0x13f990, 0x13f994, 0x13f9a0, 0x13f9a4, 0x13f9b0, 0x13f9b4, 0x13f9c0, 0x13f9c4, HERO_YAW]);
	}

	#[test]
	fn retail_writes_are_kept_the_rest_goes_back() {
		let old = [1u32, 2, 3, 4, 5, 6, 7, 8, 9];
		let new = [1u32, 20, 3, 40, 5, 60, 7, 80, 90];
		let mut now = new;
		now[1] = 77; // retail rewrote a turned word
		now[2] = 33; // and an untouched one
		assert_eq!(after(old, new, now), [1, 77, 33, 4, 5, 6, 7, 8, 9]);
		assert_eq!(after(old, old, old), old, "standing: nothing turned, nothing changed");
	}
}
