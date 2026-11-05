//! [PC] Volumetric smoke around fire (volsmoke.rs). The smoke is applied after the level pass that drew the flames, so on its own it
//! would cover a flame from behind and veil it where a smoke volume wraps it (a tan haze over the flame tops). The frame's flames and
//! fireballs (firefx_vol.rs `core`: the dense part as an ellipsoid) go to the shader after the smoke volumes, in the volumes' own
//! record (centre + horizontal radius, height ratio): a pixel's march ends where its ray enters a core (the flame hides the smoke
//! behind it, smoke in front still covers it), and the smoke thins out in a shell round each core (it rises from the tips). Fire Off:
//! no cores, the smoke is as before. Dev knobs (RAC_TUNE, ENV.md): smoke_cut (core size gain), smoke_hollow (shell over core).

use super::volumes::Vol;
use bytemuck::Zeroable;

/// Cores the shader tests per pixel: the ones biggest on screen.
pub const MAX_CUTS: usize = 16;

/// An ellipsoid: centre, horizontal radius, vertical half-extent.
pub type Core = ([f32; 3], f32, f32);

/// The cores in front of `eye` (looking along `fwd`) within `range`, biggest on screen first (ties: the first given), at most
/// `MAX_CUTS`, each grown by `gain`, as smoke volume records (c = centre and horizontal radius, mo.w = height over radius).
pub fn cuts(cores: impl Iterator<Item = Core>, eye: [f32; 3], fwd: [f32; 3], range: f32, gain: f32) -> Vec<Vol> {
	let mut seen: Vec<(f32, usize, Core)> = cores
		.enumerate()
		.filter_map(|(i, (c, r, h))| {
			let v = [c[0] - eye[0], c[1] - eye[1], c[2] - eye[2]];
			let b = r.max(h) * gain;
			let dist = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
			let ahead = v[0] * fwd[0] + v[1] * fwd[1] + v[2] * fwd[2];
			(r > 0.0 && h > 0.0 && ahead > -b && dist - b < range).then_some((b / dist.max(0.5), i, (c, r * gain, h * gain)))
		})
		.collect();
	seen.sort_by(|x, y| y.0.total_cmp(&x.0).then(x.1.cmp(&y.1)));
	seen.iter()
		.take(MAX_CUTS)
		.map(|&(_, _, (c, r, h))| Vol { c: [c[0], c[1], c[2], r], mo: [0.0, 0.0, 0.0, (h / r).clamp(0.1, 10.0)], ..Vol::zeroed() })
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn cores_in_view_biggest_first_capped() {
		let fwd = [1.0, 0.0, 0.0];
		let near_small = ([10.0, 0.0, 0.0], 0.5, 1.0);
		let far_big = ([40.0, 0.0, 0.0], 3.0, 6.0);
		let behind = ([-10.0, 0.0, 0.0], 2.0, 2.0);
		let too_far = ([500.0, 0.0, 0.0], 3.0, 3.0);
		let v = cuts([near_small, far_big, behind, too_far].into_iter(), [0.0; 3], fwd, 150.0, 1.0);
		assert_eq!(v.len(), 2, "behind the eye and beyond the range: no cut");
		assert_eq!(v[0].c, [40.0, 0.0, 0.0, 3.0], "6 m over 40 m beats 1 m over 10 m");
		assert_eq!((v[0].mo[3], v[1].mo[3]), (2.0, 2.0), "height over radius");
		let many = (0..40).map(|k| ([5.0 + k as f32, 0.0, 0.0], 1.0, 1.0));
		let v = cuts(many, [0.0; 3], fwd, 150.0, 1.5);
		assert_eq!(v.len(), MAX_CUTS);
		assert_eq!(v[0].c, [5.0, 0.0, 0.0, 1.5], "the nearest first, grown by the gain");
	}

	#[test]
	fn no_fire_no_cut() {
		assert!(cuts(std::iter::empty(), [0.0; 3], [1.0, 0.0, 0.0], 150.0, 1.0).is_empty());
	}
}
