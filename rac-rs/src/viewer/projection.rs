//! Camera projection: the retail PAL constants, `view_proj`, `project`, the panel-target projection constants
//! and the tests of both.

use super::*;

/// PAL vertical/horizontal projection ratio (decomp/notes/camera.md "REN-7a port spec", f32 0x3f418937 / 0.63).
pub(crate) const PROJ_Y_RATIO: f32 = 0.756;
/// Vertical half-angle tangent of the retail gameplay projection: 0.63 * 0.756 (REN-7a).
pub(super) const TAN_HALF_FOV: f32 = crate::camera::PROJ_SCALAR * PROJ_Y_RATIO;
/// Effective retail world near/far planes in metres (REN-7a): B/(z_s range + 224)/1024 of the GS z mapping.
pub(super) const RETAIL_NEAR: f32 = 0.03125;
pub(super) const RETAIL_FAR: f32 = 728.1632;

/// NOV-30a: panel targets (item, popup, portrait) draw their vertices with the MAIN projection (retail
/// moby_draw_build reads the view x clip matrix the panel path never rebuilds), centred on the 512x128 target:
/// 256/0.63 px per unit tan horizontally, 224/(0.63*0.756) vertically. Over the 64 px target half height the
/// tan shrinks by 64/224, and the aspect makes the horizontal scalar 1/0.63 again (224 / (64 * 0.756)).
pub(super) const PANEL_TAN_SCALE: f32 = 64.0 / 224.0;
pub(super) const PANEL_ASPECT: f32 = 224.0 / (64.0 * PROJ_Y_RATIO);

/// Clip-space projection of a world point; None behind the camera.
pub(super) fn project(m: &[[f32; 4]; 4], p: [f32; 3]) -> Option<[f32; 2]> {
	let c: Vec<f32> = (0..4).map(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r]).collect();
	(c[3] > 0.1).then(|| [c[0] / c[3], c[1] / c[3]])
}

pub(super) fn forward(cam: &Camera) -> [f32; 3] {
	[cam.yaw.cos() * cam.pitch.cos(), cam.yaw.sin() * cam.pitch.cos(), cam.pitch.sin()]
}

/// Column-major perspective * look-at, Z up, depth 0..1. The world passes (RETAIL_NEAR, RETAIL_FAR); the vertical
/// tan is `cam.tan_half` and the horizontal follows the window aspect [PC] (REN-7a).
pub(super) fn view_proj(cam: &Camera, aspect: f32, near: f32, far: f32) -> [[f32; 4]; 4] {
	let f = forward(cam);
	let up = cam.up;
	let norm = |v: [f32; 3]| {
		let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
		[v[0] / l, v[1] / l, v[2] / l]
	};
	let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
	let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
	let s = norm(cross(f, up));
	let u = cross(s, f);
	let e = cam.pos;
	// View matrix rows: s, u, -f.
	let view = [
		[s[0], u[0], -f[0], 0.0],
		[s[1], u[1], -f[1], 0.0],
		[s[2], u[2], -f[2], 0.0],
		[-dot(s, e), -dot(u, e), dot(f, e), 1.0],
	];
	let t = 1.0 / cam.tan_half;
	let proj = [
		[t / aspect, 0.0, 0.0, 0.0],
		[0.0, t, 0.0, 0.0],
		[0.0, 0.0, far / (near - far), -1.0],
		[0.0, 0.0, near * far / (near - far), 0.0],
	];
	let mut out = [[0f32; 4]; 4];
	for c in 0..4 {
		for r in 0..4 {
			out[c][r] = (0..4).map(|k| proj[k][r] * view[c][k]).sum();
		}
	}
	out
}

#[cfg(test)]
mod proj_tests {
	use super::*;

	fn clip(m: &[[f32; 4]; 4], p: [f32; 3]) -> [f32; 4] {
		std::array::from_fn(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r])
	}

	// REN-7a tests 1, 2, 4, 5 (camera at the origin looking +X, so world x is depth, y is -right, z is up).
	#[test]
	fn retail_projection() {
		let cam = Camera { pos: [0.0; 3], yaw: 0.0, pitch: 0.0, tan_half: TAN_HALF_FOV, up: [0.0, 0.0, 1.0] };
		assert!((1.0 / (0.63f32 * 0.756f32) - 2.0996053).abs() < 1e-6);
		assert_eq!(TAN_HALF_FOV, 0.47627997);
		for (aspect, p00) in [(16.0 / 9.0, 1.181028f32), (512.0 / 448.0, 1.8371546)] {
			let m = view_proj(&cam, aspect, RETAIL_NEAR, RETAIL_FAR);
			assert!((m[1][0].abs() - p00).abs() < 1e-6, "{aspect}");
			assert!((m[0][2] - 1.0000429).abs() < 1e-6);
			assert!((m[3][2] + 0.03125134).abs() < 1e-7);
			let ndc_y = |vz: f32| {
				let c = clip(&m, [vz, 0.0, TAN_HALF_FOV * vz]);
				c[1] / c[3]
			};
			assert!((ndc_y(1.0) - 1.0).abs() < 1e-6 && (ndc_y(37.0) - 1.0).abs() < 1e-6);
		}
		let m = view_proj(&cam, 512.0 / 448.0, RETAIL_NEAR, RETAIL_FAR);
		let c = clip(&m, [1.0, -0.63, 0.0]);
		assert!((c[0] / c[3] - 1.1574).abs() < 1e-4);
		let depth = |vz: f32| {
			let c = clip(&m, [vz, 0.0, 0.0]);
			c[2] / c[3]
		};
		assert!((depth(1.0) - 0.96879154).abs() < 1e-6);
		assert!((depth(10.0) - 0.9969178).abs() < 1e-6);
		assert!(depth(RETAIL_NEAR).abs() < 1e-6);
		assert!((depth(RETAIL_FAR) - 1.0).abs() < 1e-6);
		// REN-7a test 3: the same depth from the retail GS z mapping.
		let zs = |raw: f32| -224.0f32 + 536860160.0 / raw;
		assert_eq!(zs(32.0), 16776656.0);
		assert!(((16776656.0 - zs(1024.0)) / (16776656.0 - 496.0) - 0.96879154f32).abs() < 1e-6);
	}

	// REN-7b test 2: the scene key scalar converts to the vertical tan.
	#[test]
	fn proj_scalar_sources() {
		assert_eq!(crate::camera::PROJ_SCALAR.to_bits(), 0x3f2147ae);
		assert!((0.597f32 * PROJ_Y_RATIO - 0.451332).abs() < 1e-6);
	}
}

#[cfg(test)]
mod item_panel_tests {
	use super::*;

	fn proj() -> [[f32; 4]; 4] {
		let cam = Camera { pos: [0.0; 3], yaw: 0.0, pitch: 0.0, tan_half: TAN_HALF_FOV * PANEL_TAN_SCALE, up: [0.0, 0.0, 1.0] };
		view_proj(&cam, PANEL_ASPECT, RETAIL_NEAR, RETAIL_FAR)
	}

	fn target_px(p: [f32; 4]) -> (f32, f32) {
		let m = proj();
		let c: Vec<f32> = (0..4).map(|r| (0..4).map(|k| m[k][r] * p[k]).sum()).collect();
		let (nx, ny) = (c[0] / c[3], c[1] / c[3]);
		let (w, h) = (ITEM_PANEL_SIZE.0 as f32, ITEM_PANEL_SIZE.1 as f32);
		((nx + 1.0) * 0.5 * w, (1.0 - ny) * 0.5 * h)
	}

	#[test]
	fn item_panel_projection_constants() {
		let m = proj();
		// NOV-30a: main projection on the target, out[column][row]: x row = -y column scaled by 1/0.63,
		// y row = z column scaled by 224/64 / 0.47628.
		assert!((TAN_HALF_FOV * PANEL_TAN_SCALE - 0.13608).abs() < 1e-6);
		assert!((PANEL_ASPECT - 4.6296296).abs() < 1e-5);
		assert!((m[1][0] + 1.5873016).abs() < 1e-5, "{}", m[1][0]);
		assert!((m[2][1] - 7.3486185).abs() < 1e-4, "{}", m[2][1]);
		assert!((m[0][3] - 1.0).abs() < 1e-6);
	}

	#[test]
	fn item_panel_projects_view_point() {
		// View point right 1, up 0.5, depth 4 (camera at origin looking +x: right = -y).
		let (px, py) = target_px([4.0, -1.0, 0.5, 1.0]);
		assert!((px - 357.5873).abs() < 1e-2, "{px}");
		assert!((py - 5.211052).abs() < 1e-2, "{py}");
		// Item 16 pivot (right -3.75, up -0.03, depth 7.2) lands in the left third of the source box.
		let (px, py) = target_px([7.2, 3.75, -0.03, 1.0]);
		assert!((px - 44.35977).abs() < 1e-2, "{px}");
		assert!((py - 65.95963).abs() < 1e-2, "{py}");
		// NOV-30d test 2: the bolt icon pivot (depth 3.8 + 10.2, right -4.25, up -1.5) sits right of the price, low in the box.
		let (px, py) = target_px([14.0, 4.25, -1.5, 1.0]);
		assert!((px - 132.64398).abs() < 1e-2, "{px}");
		assert!((py - 114.39053).abs() < 1e-2, "{py}");
	}

	#[test]
	fn item_poses_split_by_destination() {
		use game::vendor::{ItemDest, ItemPose};
		let pose = |class, dest| ItemPose { class, pos: [0.0; 3], rot: [0.0; 3], dest };
		let (world, panel) = split_item_poses(&[pose(1, ItemDest::World), pose(2, ItemDest::Panel), pose(3, ItemDest::World)]);
		assert_eq!(world.iter().map(|p| p.0).collect::<Vec<_>>(), [1, 3]);
		assert_eq!(panel.iter().map(|p| p.0).collect::<Vec<_>>(), [2]);
	}
}
