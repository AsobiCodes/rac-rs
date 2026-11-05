//! [PC] Sun shadow pass (lane v042_shadow, RAC_SUN_FRUSTUM, default off): casters whose shadow cannot land in the main view.
//!
//! A shadow map texel is read only for a surface point the camera sees (the first cascade whose map holds the point answers,
//! `sun_occ`), and a caster only blackens points down the light from it. So a caster whose sweep along the light's travel
//! direction never enters the view pyramid changes no picture pixel of the main camera. The test is a separating plane: a side
//! plane of the pyramid with every corner of the caster's box outside it and the sweep not heading inward.
//! LOSSY for the other readers of the cascades, which render views other than the main camera's: the water mirror passes
//! (`mirror.rs`, the main camera reflected), the cloud sky capture (`volclouds/skymodel.rs`) and the water sky map
//! (`waterfx/skymap.rs`); with the switch on their shadows of casters outside the main pyramid can be missing. That is why it
//! stays behind a switch.

use super::bounds::Aabb;
use super::{dot3, forward, norm3, Camera};

/// Slack in world units (PCF taps, the 2-texel bias towards the light, rounding): the largest texel of a cascade is ~0.1.
const SLACK: f32 = 1.0;

/// The camera's view pyramid (four side planes through the eye, inward normals) and the light's travel direction.
#[derive(Clone, Copy, Debug)]
pub(super) struct View {
	eye: [f32; 3],
	planes: [[f32; 3]; 4],
	dir: [f32; 3],
}

impl View {
	/// None when the camera looks along its up vector (no side direction: no view cull that frame).
	pub(super) fn new(cam: &Camera, aspect: f32, dir: [f32; 3]) -> Option<View> {
		let f = forward(cam);
		let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
		let side = cross(f, cam.up);
		if !(dot3(side, side) > 1e-6) {
			return None;
		}
		let right = norm3(side);
		let up = cross(right, f);
		// 2% wider than the projection: the pyramid is a bound, not the picture.
		let (ty, tx) = (cam.tan_half * 1.02, cam.tan_half * 1.02 * aspect);
		let mix = |a: [f32; 3], k: f32, b: [f32; 3]| norm3([a[0] * k + b[0], a[1] * k + b[1], a[2] * k + b[2]]);
		let neg = |a: [f32; 3]| [-a[0], -a[1], -a[2]];
		let planes = [mix(f, tx, right), mix(f, tx, neg(right)), mix(f, ty, up), mix(f, ty, neg(up))];
		Some(View { eye: cam.pos, planes, dir: norm3(dir) })
	}

	/// Whether a caster at `c` whose extent along a plane normal `n` is `ext(n)` casts nothing into the view.
	fn rejects(&self, c: [f32; 3], ext: &dyn Fn([f32; 3]) -> f32) -> bool {
		self.planes.iter().any(|&n| {
			// The plane's value at the caster's most inward point is below -SLACK, and the sweep does not climb.
			let at = dot3(n, [c[0] - self.eye[0], c[1] - self.eye[1], c[2] - self.eye[2]]) + ext(n);
			at < -SLACK && dot3(n, self.dir) <= 0.0
		})
	}

	pub(super) fn rejects_sphere(&self, c: [f32; 3], r: f32) -> bool {
		self.rejects(c, &|_| r)
	}

	pub(super) fn rejects_box(&self, b: &Aabb) -> bool {
		let c = [0, 1, 2].map(|k| 0.5 * (b.lo[k] + b.hi[k]));
		let h = [0, 1, 2].map(|k| 0.5 * (b.hi[k] - b.lo[k]));
		self.rejects(c, &|n| n[0].abs() * h[0] + n[1].abs() * h[1] + n[2].abs() * h[2])
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn behind_and_beside_the_camera_is_out_and_ahead_is_in() {
		let cam = Camera { pos: [10.0, 20.0, 5.0], yaw: 0.7, pitch: 0.0, tan_half: 0.6, up: [0.0, 0.0, 1.0] };
		let f = forward(&cam);
		let v = View::new(&cam, 16.0 / 9.0, [0.0, 0.0, -1.0]).unwrap();
		let at = |k: f32| [cam.pos[0] + f[0] * k, cam.pos[1] + f[1] * k, cam.pos[2] + f[2] * k];
		assert!(!v.rejects_sphere(at(30.0), 1.0), "straight ahead is in view");
		assert!(v.rejects_sphere(at(-30.0), 1.0), "behind the camera cannot be seen");
		let right = norm3([f[1] * cam.up[2] - f[2] * cam.up[1], f[2] * cam.up[0] - f[0] * cam.up[2], f[0] * cam.up[1] - f[1] * cam.up[0]]);
		let side = at(30.0);
		assert!(v.rejects_sphere([side[0] + right[0] * 500.0, side[1] + right[1] * 500.0, side[2] + right[2] * 500.0], 1.0), "far to the side");
	}

	#[test]
	fn a_sweep_towards_the_view_keeps_the_caster() {
		let cam = Camera { pos: [10.0, 20.0, 5.0], yaw: 0.7, pitch: 0.0, tan_half: 0.6, up: [0.0, 0.0, 1.0] };
		let f = forward(&cam);
		let behind = [cam.pos[0] - f[0] * 30.0, cam.pos[1] - f[1] * 30.0, cam.pos[2] - f[2] * 30.0];
		// The light travels along the camera's forward: a caster behind it casts into the view.
		let v = View::new(&cam, 16.0 / 9.0, f).unwrap();
		assert!(!v.rejects_sphere(behind, 1.0));
	}

	#[test]
	fn looking_along_up_makes_no_view() {
		let cam = Camera { pos: [0.0; 3], yaw: 0.0, pitch: std::f32::consts::FRAC_PI_2, tan_half: 0.6, up: [0.0, 0.0, 1.0] };
		assert!(View::new(&cam, 16.0 / 9.0, [0.0, 0.0, -1.0]).is_none());
	}
}
