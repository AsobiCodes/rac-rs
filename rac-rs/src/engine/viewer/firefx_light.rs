//! [PC] Fire as a light source (user 2026-11-04: "fire has to be a source of light", "i want dynamic illumination", "we need
//! low-frequency light flickering", "lets plug it into global illumination setting"). The level's fires (the tongues of its flame
//! quads, firefx_ground.rs) and the effect fire Fire draws (flamethrower puffs, burning things: `FireFx::big`) light the scene round
//! them with warm lights that breathe slowly. The Global illumination row of Graphics decides (`count`): Off none, Low 1, Medium 2,
//! High 4 lights, over the biggest fires first (tongues within `CLUSTER` m share one). With the Fire row Off the game's own flame
//! billboards light the scene the same way (`FireFx::gather`).
//!
//! The lights go with the effect lights on the weapon lights' path (colour .w = the surface gain) to the level pass (flit.rs: the near share
//! lights the surfaces' texture colour, so a shadowed or dark wall beside a fire glows too; post keeps the far share) and to the volumetric smoke. Flicker: three slow sines
//! per light (0.31, 0.67 and 1.13 Hz, phases from its place), so the light swells and sinks a few times a second at most; its centre
//! sways a little with them (`SWAY` of the reach, at most `SWAY_MAX` m), so the lit patch shifts like a real fire's. All on the fire's
//! game clock (a pause freezes it). Knobs (RAC_TUNE): fire_light (brightness), fire_flicker (depth), fire_lreach (reach cap, m),
//! fire_lgain (screen gain), fire_sway (sway share); fire_llog=1 prints the lights every 30th frame.

use super::firefx::Emit;
use crate::devtune;

/// Tongues within this distance (m) of a light's centre feed that light.
const CLUSTER: f32 = 6.0;
/// The centre sways by this share of the reach ...
pub(super) const SWAY: f32 = 0.05;
/// ... and never more than this (m) ...
const SWAY_MAX: f32 = 1.0;
/// ... or this (m) while the lights may cast shadows (sunshadow/omni.rs, Global illumination Medium / High): the shadow map's origin
/// is the light's centre, so a metre of sway made every shadow edge swim (lane dlit stage 2); a few cm keep it alive.
pub(super) const SWAY_SHADOWED: f32 = 0.04;
/// A light stands this share of its fires' height over their foot (low in the flames: the burning roof and walls round it are lit,
/// not only the air above).
const LIFT: f32 = 0.3;

/// The fire lights at Global illumination level `gi` (0 Off, 1 Low, 2 Medium, 3 High).
pub(super) const fn count(gi: u8) -> usize {
	match gi {
		0 => 0,
		1 => 1,
		2 => 2,
		_ => 4,
	}
}

/// The sway cap (m) at Global illumination level `gi`: `SWAY_SHADOWED` where the lights take shadow maps (sunshadow/omni.rs
/// `lights`), else `SWAY_MAX`.
/// `maps`: the light shadow maps may draw this frame (Dynamic shadows on, tb4 fix S3; sunshadow/omni.rs `light_shadows`).
pub(super) fn sway_max(gi: u8, maps: bool) -> f32 {
	if maps && super::sunshadow::omni::lights(gi) > 0 { SWAY_SHADOWED } else { SWAY_MAX }
}

/// The fire may light the scene this frame: Global illumination not Off, Lighting and Particles Enhanced, not the Classic half of
/// the split view.
pub(super) fn wanted() -> bool {
	let g = crate::gfx::graphics();
	g.gi_q > 0 && g.lighting > 0 && g.particles && crate::splitview::fx(true)
}

/// The flicker (about 1 +- `depth`) at game time `t` (s) for phase `ph` (also the surface lights', flitlist.rs): three slow sines
/// (the fire swelling and sinking) plus, since lane rfire (user 2026-11-07 on test build 7: "the shadows should flicker i think
/// because fire flickers"), two small faster ones at the flames' own lick rate (2.3 and 3.7 Hz, `LICK` of the depth): smooth, no
/// per-frame noise, on the fire's game clock (frozen when paused).
/// `licks` false (Fire Off: the game's own flame billboards light the scene): the three slow sines only, test build 7's light.
pub(super) fn flicker(t: f64, ph: f64, depth: f32, licks: bool) -> f32 {
	use std::f64::consts::TAU;
	let s = 0.5 * (TAU * 0.31 * t + ph).sin() + 0.3 * (TAU * 0.67 * t + 1.7 * ph).sin() + 0.2 * (TAU * 1.13 * t + 2.9 * ph).sin();
	if !licks {
		return 1.0 + depth * s as f32;
	}
	let lick = 0.6 * (TAU * 2.3 * t + 4.1 * ph).sin() + 0.4 * (TAU * 3.7 * t + 5.3 * ph).sin();
	1.0 + depth * (s + LICK * lick) as f32
}

/// The faster licks' share of the flicker depth (`flicker`).
const LICK: f64 = 0.16;

/// Lane rfire: the flames' dance (each axis within about +-1, z half) at game time `t` for phase `ph`: the light centres and the
/// fire shadow origins move with it (`DANCE` of the emitter radius, at most `DANCE_MAX` m), so the lit patch and the shadows the
/// fire casts waver with the flames instead of standing still (firefx.rs `shadow_origins`, flitlist.rs `fire_lights`).
pub(super) fn dance_off(t: f64, ph: f64) -> [f32; 3] {
	use std::f64::consts::TAU;
	let x = 0.6 * (TAU * 0.9 * t + 1.3 * ph).sin() + 0.4 * (TAU * 1.9 * t + 3.1 * ph).sin();
	let y = 0.6 * (TAU * 1.3 * t + 2.3 * ph).cos() + 0.4 * (TAU * 2.2 * t + 0.7 * ph).sin();
	let z = 0.5 * (0.6 * (TAU * 1.6 * t + 4.7 * ph).sin() + 0.4 * (TAU * 2.4 * t + 1.9 * ph).sin());
	[x as f32, y as f32, z as f32]
}

/// The dance's share of a fire's emitter radius and its cap (m).
pub(super) const DANCE: f32 = 0.08;
pub(super) const DANCE_MAX: f32 = 0.2;

/// The dance offset of a light or shadow origin at `p` with emitter radius `rs` (m) at game time `t` (RAC_TUNE fire_dance scales
/// it; 0 = still, test build 7).
pub(super) fn dance(p: [f32; 3], rs: f32, t: f64) -> [f32; 3] {
	let amp = (DANCE * rs).min(DANCE_MAX) * devtune::f32("fire_dance", 1.0).clamp(0.0, 4.0);
	let ph = (0.11 * p[0] + 0.17 * p[1] + 0.07 * p[2]) as f64;
	dance_off(t, ph).map(|v| v * amp)
}

/// Up to `n` lights over the biggest `fires` at game time `t` (s): (position, reach), (rgb, screen gain); the centre sways at most
/// `sway_max` m (`SWAY_MAX`, or `SWAY_SHADOWED` while the lights cast shadows); `licks`: the Fire row draws (`flicker`).
pub(super) fn lights<'a>(fires: impl Iterator<Item = &'a Emit>, t: f64, n: usize, sway_max: f32, licks: bool) -> Vec<[[f32; 4]; 2]> {
	if n == 0 {
		return Vec::new();
	}
	let mut es: Vec<Emit> = fires.copied().filter(|e| e.h > 0.0 && e.w > 0.0).collect();
	es.sort_by(|a, b| (b.h * b.w).total_cmp(&(a.h * a.w)));
	// Per light: weighted position sum, weight sum.
	let mut acc: Vec<([f32; 3], f32)> = Vec::new();
	for e in &es {
		let p = [e.base[0], e.base[1], e.base[2] + LIFT * e.h];
		let wgt = e.h * e.w;
		let near = |a: &([f32; 3], f32)| (0..3).map(|k| (a.0[k] / a.1 - p[k]).powi(2)).sum::<f32>() < CLUSTER * CLUSTER;
		match acc.iter().position(near) {
			Some(i) => {
				for k in 0..3 {
					acc[i].0[k] += p[k] * wgt;
				}
				acc[i].1 += wgt;
			}
			None if acc.len() < n => acc.push(([p[0] * wgt, p[1] * wgt, p[2] * wgt], wgt)),
			None => {}
		}
	}
	let (gain, depth, cap) = (devtune::f32("fire_light", 1.4), devtune::f32("fire_flicker", 0.35), devtune::f32("fire_lreach", 32.0));
	let (screen, sway) = (devtune::f32("fire_lgain", 2.2), devtune::f32("fire_sway", 1.0));
	acc.iter()
		.map(|a| {
			let c = [a.0[0] / a.1, a.0[1] / a.1, a.0[2] / a.1];
			// The phases come from the place, smoothly (a light that moves a metre shifts its phase by a fifth of a radian, no jump).
			let ph = (0.11 * c[0] + 0.17 * c[1] + 0.07 * c[2]) as f64;
			let f = flicker(t, ph, depth, licks).max(0.2);
			let reach = (6.0 + 2.2 * a.1.sqrt()).min(cap.max(4.0));
			let amp = (SWAY * reach).min(sway_max) * sway;
			let off = sway_off(t, ph);
			// (The colour is what the smoke takes, as bright as before; the screen gain .w lights the surfaces brighter on top.)
			let b = (0.35 * a.1.sqrt()).min(1.3) * gain * f;
			[[c[0] + amp * off[0], c[1] + amp * off[1], c[2] + amp * off[2], reach], [b, b * 0.5, b * 0.12, screen.max(1e-3)]]
		})
		.collect()
}

/// The sway direction (each axis within +-1) at game time `t` for phase `ph` (also the surface lights', flitlist.rs).
pub(super) fn sway_off(t: f64, ph: f64) -> [f32; 3] {
	use std::f64::consts::TAU;
	[(TAU * 0.23 * t + ph).sin() as f32, (TAU * 0.29 * t + 2.0 * ph).cos() as f32, 0.5 * (TAU * 0.41 * t + 3.0 * ph).sin() as f32]
}

/// RAC_TUNE fire_llog=1: the frame's fire lights on stderr every 30th frame (`lit` / `big` / `small`: the fires weighed, the small
/// flames only while the Fire row draws (lane rfire); `retail`: the effect lights after them).
pub(super) fn log(lit: usize, big: usize, small: usize, retail: usize, mine: &[[[f32; 4]; 2]]) {
	static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
	if devtune::f32("fire_llog", 0.0) > 0.5 && N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 30 == 0 {
		eprintln!("fire lights: fires {lit} + {big} + small {small}, effect lights {retail}, lights {mine:?}");
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn e(x: f32) -> Emit {
		Emit { base: [x, 0.0, 0.0], h: 4.0, w: 2.0, seed: 0.5 }
	}

	#[test]
	fn the_gi_level_picks_the_count() {
		assert_eq!((count(0), count(1), count(2), count(3)), (0, 1, 2, 4));
		let fires = [e(0.0), e(40.0), e(80.0), e(120.0), e(160.0)];
		for gi in 0..=3u8 {
			assert_eq!(lights(fires.iter(), 1.0, count(gi), SWAY_MAX, true).len(), count(gi));
		}
	}

	#[test]
	fn lights_cluster_near_fires_and_stay_warm_on_the_screen_path() {
		let fires = [e(0.0), e(2.0), e(40.0), e(80.0), e(120.0), e(160.0)];
		let l = lights(fires.iter(), 1.0, 4, SWAY_MAX, true);
		assert_eq!(l.len(), 4, "at most four, the first two share one");
		assert!(l[0][1][0] > l[0][1][1] && l[0][1][1] > l[0][1][2], "warm");
		assert!(l.iter().all(|l| l[1][3] > 0.0), "the screen gain: the surface's own colour is lit (post.rs fx_light)");
		assert!(lights([].iter(), 0.0, 4, SWAY_MAX, true).is_empty());
	}

	/// Low frequency: over 3 s at 60 fps the light never changes by more than a few percent from one frame to the next, but it does
	/// swell and sink (a real range over the 3 s).
	#[test]
	fn the_flicker_is_slow_but_alive() {
		let fires = [e(0.0)];
		let at = |t: f64| lights(fires.iter(), t, 1, SWAY_MAX, true)[0][1][0];
		let (mut lo, mut hi, mut step) = (f32::MAX, 0.0f32, 0.0f32);
		let mut last = at(10.0);
		for k in 1..=180 {
			let v = at(10.0 + k as f64 / 60.0);
			step = step.max((v - last).abs() / last);
			(lo, hi, last) = (lo.min(v), hi.max(v), v);
		}
		assert!(step < 0.06, "per frame at most {step}");
		assert!(hi / lo > 1.2, "it breathes: {lo} .. {hi}");
	}

	/// The centre sways a little and smoothly; the flicker has no jump when the shader clock wraps (it runs on the unwrapped age).
	#[test]
	fn the_centre_sways_and_nothing_jumps_at_the_clock_wrap() {
		let fires = [e(0.0)];
		let pos = |t: f64| lights(fires.iter(), t, 1, SWAY_MAX, true)[0][0];
		let (a, b) = (pos(5.0), pos(6.5));
		let d = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
		assert!(d > 0.01 && d <= 2.0 * SWAY_MAX, "{d}");
		let level = |t: f64| lights(fires.iter(), t, 1, SWAY_MAX, true)[0][1][0];
		let p = super::super::firefx_clock::PERIOD;
		assert!((level(p - 1e-3) - level(p + 1e-3)).abs() < 1e-2, "continuous over the wrap");
		assert!(level(4.6e7).is_finite() && level(4.6e7) > 0.0, "and still a light after days");
	}

	/// Lane rfire: the dance is smooth (at 60 fps a step of under a quarter of its amplitude, a few cm), bounded by its cap, and alive (it moves over 0.5 s);
	/// the same time gives the same place (deterministic, frozen when the fire clock stands).
	#[test]
	fn the_dance_is_smooth_bounded_and_deterministic() {
		let p = [3.0, -7.0, 1.0];
		let at = |t: f64| dance(p, 2.0, t);
		assert_eq!(at(12.5), at(12.5));
		let amp = (DANCE * 2.0).min(DANCE_MAX);
		let (mut step, mut far) = (0.0f32, 0.0f32);
		for k in 0..600 {
			let (a, b) = (at(10.0 + k as f64 / 60.0), at(10.0 + (k + 1) as f64 / 60.0));
			step = step.max((0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt());
			far = far.max(a.iter().map(|v| v.abs()).fold(0.0, f32::max));
		}
		assert!(step < 0.25 * amp, "per frame {step}");
		assert!(far <= amp * 1.001 && far > 0.5 * amp, "bounded and alive: {far} of {amp}");
		assert_eq!(dance(p, 40.0, 3.0).iter().fold(0.0f32, |m, v| m.max(v.abs())) <= DANCE_MAX * 1.001, true, "capped");
	}

	/// Fire Off (`licks` false): test build 7's flicker exactly (the three slow sines), so the Fire Off picture stays identical.
	#[test]
	fn fire_off_keeps_test_build_7s_flicker() {
		use std::f64::consts::TAU;
		for k in 0..200 {
			let (t, ph) = (0.37 * k as f64, 0.013 * k as f64);
			let s = 0.5 * (TAU * 0.31 * t + ph).sin() + 0.3 * (TAU * 0.67 * t + 1.7 * ph).sin() + 0.2 * (TAU * 1.13 * t + 2.9 * ph).sin();
			assert_eq!(flicker(t, ph, 0.35, false), 1.0 + 0.35 * s as f32);
		}
		assert_ne!(flicker(1.1, 0.2, 0.35, true), flicker(1.1, 0.2, 0.35, false), "the Fire row's light licks");
	}

	/// Where the lights cast shadows the centre sways only a few cm (the shadow edges would swim with a metre of sway).
	#[test]
	fn shadowed_lights_barely_sway() {
		assert_eq!((sway_max(1, true), sway_max(2, true), sway_max(3, true), sway_max(3, false)), (SWAY_MAX, SWAY_SHADOWED, SWAY_SHADOWED, SWAY_MAX));
		let fires = [e(0.0)];
		let pos = |t: f64| lights(fires.iter(), t, 1, SWAY_SHADOWED, true)[0][0];
		for k in 0..60 {
			let (a, b) = (pos(5.0), pos(5.0 + k as f64 * 0.1));
			assert!((0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt() <= 3.0 * SWAY_SHADOWED, "the offset is at most 1.5 x the amplitude");
		}
	}
}
