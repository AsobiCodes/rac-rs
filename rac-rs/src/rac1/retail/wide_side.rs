//! [PC] Widescreen restore of retail's 4:3 sphere side test for the effects the worker builds from RAM (lane texedge, v0.4.1).
//!
//! Several retail draw callbacks cull a record by its bounding sphere against the frame's 4:3 frustum block (0x16d0c0 in Novalis,
//! `LEVEL_SHIFT` per level: rows +0, +0x10, +0x20, eye * k +0x30 (k = +0x3c = 1024), tangents +0x40, normalisers sqrt(1 + t^2) +0x60
//! [C: RAM of Hoven state hoven_e: +0x40 = (0.63, 0.47628), +0x60 = (1.18191, 1.10763)]). rac-rs draws a wider window, so a record
//! whose sphere lies outside retail's 4:3 side plane but inside the window's showed the clear colour in the screen margin (the Hoven
//! snow floe, user's report 2026-10-28). The ports keep retail's verdict and ask `keep` at their side reject only:
//! - `queue_q1::sphere_visible` (FUN_00222d20): Hoven queue-C door pools / ice floor (`hoven_pools`).
//! - `oldsim::sphere_vis` (FUN_0021faf0): Gaspar's second lava pass (`lava_patches`).
//! - `shadow_snap::sphere_fade` (FUN_001f2878 / FUN_00221d20): the ship ground shadow (`ship_shadow`).
//!
//! `keep` is the moby restore (`SideOnly::inside_lag`, props_sync.rs) on the same view-space sphere: the window's horizontal tangent
//! (vertical tangent `a.x * PROJ_Y_RATIO` times the aspect), both planes turned out by the lag of a smoothed frame (`lag_between` of
//! the last two fields' cameras, as `Retail::view_lag`). A window of 4:3 or narrower never restores: the ports run as before.
//!
//! The 4:3 sliver, on purpose: retail's horizontal tangent is 0.63, a window's is `0.47628 * aspect` = 0.63504 at 4:3 (the vertical
//! 0.47628 is the same), 0.8 % wider, so a 4:3 window shows about 4 to 6 px per side (960 to 1440 px wide) that retail's planes still
//! cull. `keep` does not restore them: at 4:3 the picture stays bit-identical to the ports before this file (`wide` is false up to
//! 4/3 + 1e-3), as retail draws it.

use super::*;
use std::sync::atomic::{AtomicU32, Ordering};

/// f32 bits of the window aspect (`set_aspect`, where `Retail.aspect` is set); 0 = not set yet (4:3).
static ASPECT: AtomicU32 = AtomicU32::new(0);

/// The worker's last field camera and the lag from the field before (`field`).
static LAG: Mutex<(Option<Cam>, ViewLag)> = Mutex::new((None, ViewLag::NONE));

/// Main thread: the window aspect (width / height) the next fields' restores use.
pub(super) fn set_aspect(aspect: f32) {
	ASPECT.store(aspect.to_bits(), Ordering::Relaxed);
}

fn aspect() -> f32 {
	match ASPECT.load(Ordering::Relaxed) {
		0 => 4.0 / 3.0,
		b => f32::from_bits(b),
	}
}

/// A window wider than retail's 4:3 picture (the only case that restores).
fn wide(aspect: f32) -> bool {
	aspect > 4.0 / 3.0 + 1e-3
}

/// [PC] Worker, once per field before the effect lists (snapshot.rs `snap`: `out.cam = field(read_cam(..))`): passes this field's render
/// camera through and keeps the lag, the move between it and the previous field's (the turn a smoothed frame between the two can
/// trail this field's verdict by), for the widescreen side restores of `queue_fx`.
pub(super) fn field(cam: Option<Cam>) -> Option<Cam> {
	let mut l = LAG.lock().unwrap_or_else(|e| e.into_inner());
	l.1 = match (l.0, cam) {
		(Some(a), Some(b)) => lag_between(a, b),
		_ => ViewLag::NONE,
	};
	l.0 = cam;
	cam
}

/// At a port's side reject (near and far passed): keep the record when the window shows it. `v` = view-space centre, `rad` = radius
/// (both times k), `tan_x` = the block's horizontal tangent (+0x40 x).
pub(super) fn keep(v: [f32; 3], rad: f32, tan_x: f32) -> bool {
	let a = aspect();
	wide(a) && keep_at(v, rad, tan_x, a, LAG.lock().map_or(ViewLag::NONE, |l| l.1))
}

/// `keep` for a given window aspect and lag.
fn keep_at(v: [f32; 3], rad: f32, tan_x: f32, aspect: f32, lag: ViewLag) -> bool {
	wide(aspect) && SideOnly { vx: v[0], vy: v[1], vz: v[2], w: rad, tan_v: tan_x * crate::viewer::PROJ_Y_RATIO }.inside_lag(aspect, lag)
}

#[cfg(test)]
mod tests {
	use super::*;

	const TX: f32 = 0.63;
	const WIDE: f32 = 16.0 / 9.0;

	/// Retail's side test as the ports run it: a.c z - (|c| - b.c r) with a = (0.63, 0.63 * 0.756), b = sqrt(1 + a^2).
	fn retail_side(v: [f32; 3], r: f32) -> bool {
		let a = [TX, TX * crate::viewer::PROJ_Y_RATIO];
		(0..2).all(|i| !(a[i] * v[2] - (v[i].abs() - (1.0 + a[i] * a[i]).sqrt() * r)).is_sign_negative())
	}

	#[test]
	fn four_three_never_restores() {
		let mut seed = 0x1234_5678u32;
		let mut rnd = || {
			seed ^= seed << 13;
			seed ^= seed >> 17;
			seed ^= seed << 5;
			(seed % 20001) as f32 - 10000.0
		};
		for _ in 0..20000 {
			let v = [rnd() * 10.0, rnd() * 10.0, rnd() * 10.0];
			let r = rnd().abs();
			let lag = ViewLag { ang: rnd().abs() * 1e-4, grow: rnd().abs() };
			assert!(!keep_at(v, r, TX, 4.0 / 3.0, lag));
			assert!(!keep_at(v, r, TX, 1.0, lag));
		}
	}

	#[test]
	fn margin_sphere_kept_only_wide() {
		// 100 m ahead, 4 m radius, centre at 0.70 of the depth to the right: past retail's 0.63 plane (by more than the radius)
		// and inside the 16:9 window's 0.63 * 0.756 * 16/9 = 0.8467.
		let z = 100.0 * 1024.0;
		let v = [0.70 * z, 0.0, z];
		let r = 4.0 * 1024.0;
		assert!(!retail_side(v, r));
		// The global `keep` with no window aspect set (the tests never call `set_aspect`) is a 4:3 window: never restores.
		assert!(!keep(v, r, TX));
		assert!(keep_at(v, r, TX, WIDE, ViewLag::NONE));
		assert!(!keep_at(v, r, TX, 4.0 / 3.0, ViewLag::NONE));
		// Left side the same.
		assert!(keep_at([-v[0], 0.0, z], r, TX, WIDE, ViewLag::NONE));
		// Past the 16:9 plane too: stays culled, unless a smoothed frame's turn shows it.
		let far_side = [1.0 * z, 0.0, z];
		assert!(!keep_at(far_side, r, TX, WIDE, ViewLag::NONE));
		assert!(keep_at(far_side, r, TX, WIDE, ViewLag { ang: 0.2, grow: 0.0 }));
		// Above retail's (= the window's) vertical plane: the vertical test is unchanged.
		assert!(!keep_at([0.0, 0.6 * z, z], r, TX, WIDE, ViewLag::NONE));
	}

	#[test]
	fn behind_still_culled() {
		// Behind the eye (z < 0) at the side: no side plane passes.
		let z = -50.0 * 1024.0;
		assert!(!keep_at([0.7 * -z, 0.0, z], 1024.0, TX, WIDE, ViewLag::NONE));
	}

	/// The Hoven frustum block of state hoven_e with the view turned to the identity (view space = world * 1024).
	fn block() -> [[f32; 4]; 8] {
		[
			[1.0, 0.0, 0.0, 0.0],
			[0.0, 1.0, 0.0, 0.0],
			[0.0, 0.0, 1.0, 0.0],
			[0.0, 0.0, 0.0, 1024.0],
			[0.63, 0.47628, 1.1819054, 1.1076294],
			[4.0, 4.0, 0.25, 0.25],
			[1.1819054, 1.1076294, 2.1182187, 0.0],
			[745472.0, 32.0, 0.0, 0.0],
		]
	}

	/// `queue_q1::sphere_visible`'s verdict before this lane, verbatim from the maths after the block read (the side test, near and far
	/// in one `if`): the 4:3 runs of `sphere_test` must equal it.
	fn old_sphere_test(blk: &[[f32; 4]; 8], far: f32, s: [f32; 4]) -> i32 {
		let (r0, r1, r2, c, a, w, h, g) = (blk[0], blk[1], blk[2], blk[3], blk[4], blk[5], blk[6], blk[7]);
		let k = c[3];
		let p = s.map(|x| x * k);
		let d = [p[0] - c[0], p[1] - c[1], p[2] - c[2]];
		let v: [f32; 3] = std::array::from_fn(|i| r0[i] * d[0] + r1[i] * d[1] + r2[i] * d[2]);
		let rad = p[3];
		let zz = [rad + v[2], (0.0 - rad) + v[2]];
		let far_k = far * k;
		let behind = 0.0 - zz[0];
		let beyond = far_k - zz[1];
		let side: [f32; 2] = std::array::from_fn(|i| a[i] * v[2] - (v[i].abs() - h[i] * rad));
		if beyond.is_sign_negative() || !behind.is_sign_negative() || side[0].is_sign_negative() || side[1].is_sign_negative() {
			return -1;
		}
		let outer: [f32; 2] = std::array::from_fn(|i| a[i] * v[2] * w[i] - (v[i].abs() + h[i] * rad * h[2]));
		let near = [zz[0] - g[0], zz[1] - g[1]];
		if outer[1].is_sign_negative() || near[1].is_sign_negative() || outer[0].is_sign_negative() || !near[0].is_sign_negative() {
			0
		} else {
			1
		}
	}

	#[test]
	fn pools_test_four_three_identical() {
		let blk = block();
		let mut seed = 0x9e37_79b9u32;
		let mut rnd = |span: f32| {
			seed ^= seed << 13;
			seed ^= seed >> 17;
			seed ^= seed << 5;
			(seed % 100_001) as f32 / 100_000.0 * span
		};
		let (mut culled, mut restored) = (0, 0);
		for _ in 0..50_000 {
			let s = [rnd(400.0) - 200.0, rnd(300.0) - 150.0, rnd(400.0) - 100.0, rnd(20.0)];
			let lag = ViewLag { ang: rnd(0.3), grow: rnd(2048.0) };
			let old = old_sphere_test(&blk, 256.0, s);
			let retail = super::super::queue_q1::sphere_test(&blk, 256.0, s, |_, _, _| false);
			let four = super::super::queue_q1::sphere_test(&blk, 256.0, s, |v, r, t| keep_at(v, r, t, 4.0 / 3.0, lag));
			assert_eq!(old, retail, "{s:?}");
			assert_eq!(old, four, "{s:?}");
			let wide = super::super::queue_q1::sphere_test(&blk, 256.0, s, |v, r, t| keep_at(v, r, t, WIDE, lag));
			// The window only adds records retail's side planes culled; near / far verdicts stay.
			if retail >= 0 {
				assert_eq!(retail, wide, "{s:?}");
			} else {
				culled += 1;
				restored += usize::from(wide >= 0);
				let (z, r) = (s[2] * 1024.0, s[3] * 1024.0);
				if wide >= 0 {
					assert!(z + r > 0.0 && 256.0 * 1024.0 - (z - r) >= 0.0, "{s:?}");
				}
			}
		}
		assert!(culled > 1000 && restored > 100 && restored < culled, "{culled} {restored}");
	}

	#[test]
	fn pools_test_margin_behind_far() {
		let blk = block();
		let t = |s: [f32; 4], keep: &dyn Fn([f32; 3], f32, f32) -> bool| super::super::queue_q1::sphere_test(&blk, 256.0, s, keep);
		let wide = |v: [f32; 3], r: f32, tx: f32| keep_at(v, r, tx, WIDE, ViewLag::NONE);
		// 70 m right at 100 m ahead (4 m radius): culled by retail's 4:3 side plane, shown by the 16:9 window.
		assert_eq!(t([70.0, 0.0, 100.0, 4.0], &|_, _, _| false), -1);
		assert!(t([70.0, 0.0, 100.0, 4.0], &wide) >= 0);
		// Behind the eye and past far (256 m) stay culled even when every side reject is kept.
		assert_eq!(t([70.0, 0.0, -50.0, 4.0], &|_, _, _| true), -1);
		assert_eq!(t([0.0, 0.0, 300.0, 4.0], &|_, _, _| true), -1);
		assert_eq!(t([200.0, 0.0, 300.0, 4.0], &wide), -1);
	}
}
