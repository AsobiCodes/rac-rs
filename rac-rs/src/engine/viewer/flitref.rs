//! [PC] DEV, fire light v2 stage 3 (lane dlit3, FIRE_LIGHT_V2.md 5.1): the brute-force shadow reference. RAC_TUNE knobs, all off by
//! default (then each call is one knob lookup and returns at once; the picture is unchanged):
//! - flit_solo=g: only the lights of fire g (index in flitlist.rs `groups`, biggest first), every other light off (light and map);
//! - flit_freeze=N: the fire light list of the N-th frame with fire lights is held from then on (flicker, sway and tracking stop, so
//!   the reference frames and the real-time picture see the same emitters);
//! - flsh_ref=1: the shadow map of each fire is drawn from one point of its emitter per display frame (a light picked by its
//!   intensity, a point uniform in its body sphere, Halton (2, 3, 5) + base 7 for the pick, by frame number), near plane
//!   sunshadow/omni.rs NEAR, full fade; with flsh_pmax=0.001 the moment lookup's narrowest cone (mip 0): hard shadows. The mean of 256 such
//!   frames (lane tool refavg.py) is the area light's true visibility x the same unshadowed light. flsh_ref=2: the same with the
//!   fire's own exclusion near (the filter's share of the error alone); flsh_ref=3: points on the frame's flame tongues instead of
//!   the body spheres (a tongue picked by its area, a point in its w-wide column of height h above its foot): the flames as drawn,
//!   none inside a hull (a body sphere half sunk into a burning wing puts emitter points inside it, dark everywhere).

use super::firefx::Emit;
use super::flitlist::{Group, Light};
use std::sync::Mutex;

/// The held list (flit_freeze) and the frames with fire lights seen so far.
static HELD: Mutex<(u32, Option<Vec<Light>>)> = Mutex::new((0, None));
/// Display frames drawn with flsh_ref (the Halton index).
static FRAME: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// flsh_ref=3: the frame's flame tongues (foot, height, width).
static TONGUES: Mutex<Vec<([f32; 3], f32, f32)>> = Mutex::new(Vec::new());

/// flsh_ref=3: keep the frame's flame tongues `es` (firefx.rs `surface_lights`); nothing without the knob.
pub(super) fn tongues<'a>(es: impl Iterator<Item = &'a Emit>) {
	if crate::devtune::f32("flsh_ref", 0.0) > 2.5 {
		let mut t = TONGUES.lock().unwrap_or_else(|e| e.into_inner());
		t.clear();
		t.extend(es.filter(|e| e.h > 0.0 && e.w > 0.0).map(|e| (e.base, e.h, e.w)));
	}
}

/// The flame point of sample `k` among tongues `ts` (foot, height, width): pick by area (base 7), height (base 2), a point of the
/// w-wide disc (bases 3, 5).
fn sample_tongue(ts: &[([f32; 3], f32, f32)], k: u32) -> Option<[f32; 3]> {
	let sum: f32 = ts.iter().map(|t| t.1 * t.2).sum();
	if ts.is_empty() || sum <= 0.0 {
		return None;
	}
	let mut u = halton(k, 7) * sum;
	let t = ts.iter().find(|t| {
		u -= t.1 * t.2;
		u < 0.0
	});
	let (b, h, w) = *t.unwrap_or(&ts[ts.len() - 1]);
	let r = 0.5 * w * halton(k, 3).sqrt();
	let ph = std::f32::consts::TAU * halton(k, 5);
	Some([b[0] + r * ph.cos(), b[1] + r * ph.sin(), b[2] + h * halton(k, 2)])
}

/// flit_freeze: the frame's fire lights `fires`, or the held list once the N-th frame with lights went by.
pub(super) fn freeze(fires: Vec<Light>) -> Vec<Light> {
	let n = crate::devtune::f32("flit_freeze", 0.0);
	if n < 0.5 {
		return fires;
	}
	let mut h = HELD.lock().unwrap_or_else(|e| e.into_inner());
	if let Some(l) = &h.1 {
		return l.clone();
	}
	if !fires.is_empty() {
		h.0 += 1;
		if h.0 as f32 >= n {
			h.1 = Some(fires.clone());
		}
	}
	fires
}

/// flit_solo: keep only fire `g`'s lights in `list` (as fire 0) and that fire in `groups`.
pub(super) fn solo(list: &mut Vec<Light>, groups: &mut Vec<Group>) {
	let g = crate::devtune::f32("flit_solo", -1.0);
	if g < 0.0 {
		return;
	}
	let g = g as usize;
	list.retain(|l| l.group == Some(g as u8));
	list.iter_mut().for_each(|l| l.group = Some(0));
	*groups = groups.get(g).map(|x| vec![*x]).unwrap_or_default();
}

/// Radical inverse of `i` in base `b` (Halton).
fn halton(mut i: u32, b: u32) -> f32 {
	let (mut f, mut r) = (1.0f32, 0.0f32);
	while i > 0 {
		f /= b as f32;
		r += f * (i % b) as f32;
		i /= b;
	}
	r
}

/// The emitter point of sample `k` among `lights` (centre, intensity, body radius): pick by intensity (base 7), uniform in the
/// picked body sphere (radius cbrt, base 2; direction, bases 3 and 5).
fn sample(lights: &[([f32; 3], f32, f32)], k: u32) -> Option<[f32; 3]> {
	let sum: f32 = lights.iter().map(|l| l.1).sum();
	if lights.is_empty() || sum <= 0.0 {
		return None;
	}
	let mut u = halton(k, 7) * sum;
	let l = lights.iter().find(|l| {
		u -= l.1;
		u < 0.0
	});
	let (c, _, rs) = *l.unwrap_or(&lights[lights.len() - 1]);
	let r = rs * halton(k, 2).cbrt();
	let z = 1.0 - 2.0 * halton(k, 3);
	let ph = std::f32::consts::TAU * halton(k, 5);
	let s = (1.0 - z * z).max(0.0).sqrt();
	Some([c[0] + r * s * ph.cos(), c[1] + r * s * ph.sin(), c[2] + r * z])
}

/// flsh_ref: each fire of `groups` gets this frame's emitter point (`Group::jit`, read by sunshadow/omni.rs `light_shadows`).
pub(super) fn jitter(list: &[Light], groups: &mut [Group]) {
	let mode = crate::devtune::f32("flsh_ref", 0.0);
	if mode < 0.5 {
		return;
	}
	let k = FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
	let ts = TONGUES.lock().unwrap_or_else(|e| e.into_inner()).clone();
	for (g, gr) in groups.iter_mut().enumerate() {
		if mode > 2.5 {
			// The tongues of this fire: those nearer its centre than any other fire's (within 6 m, flitlist.rs GROUP).
			let near = |p: [f32; 3], c: [f32; 3]| (p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2) + (p[2] - c[2]).powi(2);
			let mine: Vec<_> = ts.iter().copied().filter(|t| near(t.0, gr.pos) < 36.0).collect();
			gr.jit = sample_tongue(&mine, k);
			continue;
		}
		let ls: Vec<([f32; 3], f32, f32)> = list.iter().filter(|l| l.group == Some(g as u8)).map(|l| (l.pos, l.rgb[0].max(l.rgb[1]).max(l.rgb[2]), l.rs)).collect();
		gr.jit = sample(&ls, k);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn reference_samples_fill_the_emitter_by_intensity() {
		let ls = [([0.0, 0.0, 0.0], 3.0, 1.0), ([10.0, 0.0, 0.0], 1.0, 2.0)];
		let (mut a, mut mean) = (0, [0.0f32; 3]);
		for k in 1..=256 {
			let p = sample(&ls, k).unwrap();
			let (c, r) = if p[0] < 5.0 { a += 1; (ls[0].0, 1.0) } else { (ls[1].0, 2.0) };
			let d = ((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2) + (p[2] - c[2]).powi(2)).sqrt();
			assert!(d <= r + 1e-4);
			if p[0] < 5.0 {
				(0..3).for_each(|i| mean[i] += p[i]);
			}
		}
		assert!((a as f32 / 256.0 - 0.75).abs() < 0.03, "{a}");
		assert!(mean.iter().all(|m| (m / a as f32).abs() < 0.1), "{mean:?}");
		assert_eq!(sample(&[], 1), None);
		let ts = [([0.0, 0.0, 0.0], 2.0, 1.0)];
		for k in 1..=64 {
			let p = sample_tongue(&ts, k).unwrap();
			assert!(p[0].hypot(p[1]) <= 0.5 + 1e-4 && (0.0..=2.0).contains(&p[2]), "{p:?}");
		}
	}

	#[test]
	fn the_knobs_are_off_by_default() {
		let l = vec![Light { group: Some(1), fire: true, ..Default::default() }];
		let mut list = l.clone();
		let mut groups = vec![Group::default(); 2];
		solo(&mut list, &mut groups);
		jitter(&list, &mut groups);
		assert_eq!((list, groups.len(), groups[1].jit), (l.clone(), 2, None));
		assert_eq!(freeze(l.clone()), l);
	}
}
