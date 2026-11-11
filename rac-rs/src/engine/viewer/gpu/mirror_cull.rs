//! [PC] Mirror-aware tfrag culling (lane pwat, 2026-11-07). The planar mirror passes (mirror.rs `draw_copy`: Water Medium / High,
//! the metal floor) draw the level's tfrags from the main view's culled list; with Culling On `Gpu::cull_tfrags` drops a fragment
//! beyond the draw-distance cap, masked by the occlusion grid or wholly outside the main viewport, so a reflection lost the ceiling
//! and far walls above the screen (the clear colour showed: a flat bright pool, and flicker as fragments entered and left the main
//! view). Such a fragment is kept here when its box mirrored about one of the mirror heights passes the viewport test (occlusion
//! read as Culling Off reads it); its draws go to `MirrorCull::draws`, drawn by the mirror passes only (the main pass, the bed and
//! the sun shadow lists stay as they were), posed with the camera-free near mesh (culling.rs `tfrag_verdict`).
//! Heights: the steadied water mirrors of the last two Enhanced frames (pass_post.rs; none with Water Low or Lighting Classic) and
//! the metal floor's of the last two (prepare.rs, after `plan_planar`). Split view: the Classic half tests none and changes none.

use super::*;
use crate::terrain::TfragCull;

/// The mirror heights `cull_tfrags` tests against and the draws of the fragments kept for the mirrors only.
#[derive(Default)]
pub(in crate::viewer) struct MirrorCull {
	/// The water mirrors' heights (m) of the last two water passes, [0] the latest (`set_water`).
	water: [Vec<f32>; 2],
	/// The metal floor mirror's height of the last two Enhanced frames, [0] the latest (`set_metal`).
	metal: [Option<f32>; 2],
	/// This frame's tested heights (`heights`; the buffer kept over frames).
	test: Vec<f32>,
	/// This frame's draws of the fragments only a mirror can show (indices into `scene.tfrags`; the buffer kept over frames).
	pub draws: Vec<Draw>,
}

impl MirrorCull {
	/// After a frame's water pass (`heights` = its steadied mirrors), or with none for a frame without the Enhanced passes: the
	/// latest set, the one before kept. Reuses both buffers.
	pub fn set_water(&mut self, heights: &[f32]) {
		self.water.swap(0, 1);
		self.water[0].clear();
		self.water[0].extend_from_slice(heights);
	}

	/// After an Enhanced frame's `plan_planar`: its metal floor mirror's height.
	pub fn set_metal(&mut self, h: Option<f32>) {
		self.metal = [h, self.metal[0]];
	}

	/// The heights `cull_tfrags` tests: the latest water set and metal height, plus the older ones that lie more than `MERGE` from
	/// all of them (a mirror kept over frames covers a pass that returned early; the older set covers a plane that just changed; a
	/// bobbing sea's steadied height moves a little each frame and is tested once, which halves the cost). None while the Classic half
	/// of the split view is drawn (`classic`: no mirror there). Nothing is taken: the sets stay until the next `set_water` / `set_metal`.
	pub fn heights(&mut self, classic: bool) -> &[f32] {
		let t = &mut self.test;
		t.clear();
		if !classic {
			t.extend(self.water[0].iter().chain(&self.metal[0]));
			for &h in self.water[1].iter().chain(&self.metal[1]) {
				if !t.iter().any(|&k| (k - h).abs() <= MERGE) {
					t.push(h);
				}
			}
			t.sort_by(f32::total_cmp);
			t.dedup();
		}
		t
	}
}

/// Heights (m) closer than this to a newer one are not tested again (`MirrorCull::heights`).
const MERGE: f32 = 0.1;

/// Whether a fragment the main view dropped (cap, occlusion or viewport; not the near test, `near`) is kept for the mirrors: a
/// mirror is up, the occlusion grid ever shows its cell (Culling Off's rule) and a mirror shows it (`keeps`) within the draw
/// distance: `beyond(view depth, radius)` (1024 units) = the main view's cap, read on the mirrored centre (the reflection keeps the
/// view's draw distance; it also rejects most capped fragments with one transform).
pub(super) fn rescue(v: &crate::viewer::clip::ClipView, c: &TfragCull, occ: &crate::occlusion::Occlusion, near: bool, heights: &[f32], beyond: impl Fn(f32, f32) -> bool) -> bool {
	if near || heights.is_empty() || !(occ.passes(c.occ) || occ.ever_visible(c.occ)) {
		return false;
	}
	let vp = v.viewport();
	heights.iter().any(|&h| {
		let q = v.view([c.centre[0], c.centre[1], 2.0 * h - c.centre[2]]);
		!beyond(q[2] * 1024.0, c.radius * 1024.0) && !sphere_out(&vp, q, c.radius) && keeps(v, c, &[h])
	})
}

/// perf: the mirrored bounding sphere (view centre `q`, radius `r`, m) wholly outside one viewport plane: one transform instead of
/// eight. Retail's own cull trusts the sphere to hold the fragment's faces (a sphere wholly inside draws unclipped), so nothing
/// visible is lost where the box corners poke out of it.
fn sphere_out(vp: &crate::mobies::GuardBand, q: [f32; 3], r: f32) -> bool {
	q[2] + r < vp.near || q[2] - r > vp.far || (q[2] - r > 0.0 && (q[0].abs() - vp.kx * q[2] > r * (1.0 + vp.kx * vp.kx).sqrt() || q[1].abs() - vp.ky * q[2] > r * (1.0 + vp.ky * vp.ky).sqrt()))
}

/// Whether a fragment shows in the mirror about z = `h` for one of `heights`: some box corner above the plane (the mirror clips
/// below it) and the mirrored box not wholly outside one viewport plane (the main test's AND of the corner flags, on the mirrored
/// corners: mirroring the point is mirroring the camera).
pub(super) fn keeps(v: &crate::viewer::clip::ClipView, c: &TfragCull, heights: &[f32]) -> bool {
	let vp = v.viewport();
	heights.iter().any(|&h| c.corners.iter().any(|k| k[2] > h) && c.corners.iter().fold(0x3f, |a, k| a & vp.flags(v.view([k[0], k[1], 2.0 * h - k[2]]))) == 0)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::viewer::clip::ClipView;
	use crate::viewer::Camera;

	/// Camera 2 m above the origin looking along +x and 30 degrees down (tan 0.577); 90-degree-ish view.
	fn view() -> ClipView {
		let (c, s) = (30f32.to_radians().cos(), 30f32.to_radians().sin());
		let mut v = ClipView::new(&Camera { pos: [0.0, 0.0, 2.0], yaw: 0.0, pitch: 0.0, tan_half: 0.5, up: [0.0, 0.0, 1.0] }, 1.0);
		v.f = [c, 0.0, -s];
		v.u = [s, 0.0, c];
		v
	}

	fn cube(at: [f32; 3], half: f32) -> TfragCull {
		let corners = std::array::from_fn(|k| [at[0] + if k & 1 == 0 { -half } else { half }, at[1] + if k & 2 == 0 { -half } else { half }, at[2] + if k & 4 == 0 { -half } else { half }]);
		TfragCull { centre: at, radius: half * 1.8, corners, force_far: false, occ: (0x80, 0x7f), id: 0 }
	}

	fn main_out(v: &ClipView, c: &TfragCull) -> bool {
		let vp = v.viewport();
		c.corners.iter().fold(0x3f, |a, &k| a & vp.flags(v.view(k))) != 0
	}

	#[test]
	fn ceiling_above_the_screen_shows_in_the_pool() {
		let v = view();
		// A ceiling piece 10 m ahead, 12 m up: above the main view; mirrored about the pool at z 0 it lies 12 m down, in view.
		let c = cube([10.0, 0.0, 12.0], 0.5);
		assert!(main_out(&v, &c));
		assert!(keeps(&v, &c, &[0.0]));
		// No mirror (Water Low / Classic): dropped as before.
		assert!(!keeps(&v, &c, &[]));
	}

	#[test]
	fn below_the_water_or_off_to_the_side_stays_out() {
		let v = view();
		// Wholly under the mirror plane: the mirror clips it.
		let under = cube([10.0, 0.0, -30.0], 0.5);
		assert!(!keeps(&v, &under, &[0.0]));
		// Far off to the side: outside the viewport in the mirror as well.
		let side = cube([10.0, 60.0, 12.0], 0.5);
		assert!(main_out(&v, &side));
		assert!(!keeps(&v, &side, &[0.0]));
		// Behind the camera: mirrored still behind it.
		let behind = cube([-10.0, 0.0, 12.0], 0.5);
		assert!(!keeps(&v, &behind, &[0.0]));
	}

	/// Camera 2 m up looking level along +x.
	fn level_view() -> ClipView {
		let mut v = ClipView::new(&Camera { pos: [0.0, 0.0, 2.0], yaw: 0.0, pitch: 0.0, tan_half: 0.5, up: [0.0, 0.0, 1.0] }, 1.0);
		v.f = [1.0, 0.0, 0.0];
		v.u = [0.0, 0.0, 1.0];
		v
	}

	fn verdict(v: &ClipView, c: &TfragCull, cull: bool, heights: &[f32]) -> (Option<(u8, bool)>, [u32; 4]) {
		let mut st = [0; 4];
		let occ = crate::occlusion::Occlusion::new(None, None);
		(super::super::culling::tfrag_verdict(v, c, &occ, (cull, None, [0; 3]), heights, &mut st), st)
	}

	/// Review fix 3: a fragment kept for the mirrors only is posed with the camera-free near mesh 0x14 (memo-skipped, what the sun
	/// shadow casts of an undrawn fragment), whatever LOD code its distance would give.
	#[test]
	fn rescued_fragments_use_the_camera_free_near_mesh() {
		let c = cube([10.0, 0.0, 12.0], 0.5);
		assert_eq!(verdict(&view(), &c, true, &[0.0]), (Some((0x14, true)), [0; 4]));
		assert!(super::super::tfrag_memo::camera_free(0x14));
		// No mirror: dropped by the viewport test, as before.
		assert_eq!(verdict(&view(), &c, true, &[]), (None, [0, 0, 0, 1]));
	}

	/// Review fix 4: a fragment beyond the main view's draw-distance cap is kept for a mirror that shows it within the cap (the
	/// mirrored centre's depth): camera pitched 30 degrees up, so the mirrored box lies nearer than the box. Mirror also beyond the
	/// cap, or no mirror: the cap drops it as before. Culling Off (no cap) draws it in the main view.
	#[test]
	fn the_cap_does_not_hide_what_the_mirror_shows() {
		let (co, si) = (30f32.to_radians().cos(), 30f32.to_radians().sin());
		let mut v = ClipView::new(&Camera { pos: [0.0, 0.0, 2.0], yaw: 0.0, pitch: 0.0, tan_half: 1.0, up: [0.0, 0.0, 1.0] }, 1.0);
		v.f = [co, 0.0, si];
		v.u = [-si, 0.0, co];
		let c = cube([560.0, 0.0, 60.0], 2.0);
		assert_eq!(verdict(&v, &c, true, &[0.0]), (Some((0x14, true)), [0; 4]));
		assert_eq!(verdict(&v, &c, true, &[]), (None, [1, 0, 0, 0]));
		assert_eq!(verdict(&v, &c, false, &[0.0]).0.map(|k| k.1), Some(false));
		// Level camera: the mirrored box is as deep as the box, beyond the cap too.
		assert_eq!(verdict(&level_view(), &cube([600.0, 0.0, 40.0], 2.0), true, &[0.0]), (None, [1, 0, 0, 0]));
		// Behind the camera (the near test) stays dropped, mirror or not.
		assert_eq!(verdict(&level_view(), &cube([-10.0, 0.0, 12.0], 0.5), true, &[0.0]).0, None);
	}

	/// Review fix 1: the split view's Classic half tests no heights and takes nothing, so the Enhanced half after it still has them.
	#[test]
	fn split_classic_half_takes_nothing() {
		let mut m = MirrorCull::default();
		m.set_water(&[1.5]);
		m.set_metal(Some(-3.0));
		assert!(m.heights(true).is_empty());
		assert_eq!(m.heights(false), &[-3.0, 1.5]);
		assert_eq!(m.heights(false), &[-3.0, 1.5], "reading keeps the sets (no take)");
	}

	/// Review fix 2: the sets stay until replaced (a frame that ends before its water pass keeps them); the set before the latest is
	/// tested too; a frame with no mirror (`set_water(&[])`) empties them over two frames; heights are listed once.
	#[test]
	fn heights_are_kept_and_roll_over_two_frames() {
		let mut m = MirrorCull::default();
		m.set_water(&[2.0, 5.0]);
		m.set_water(&[5.05, 7.0]);
		assert_eq!(m.heights(false), &[2.0, 5.05, 7.0], "the older 5.0 lies within MERGE of 5.05");
		m.set_water(&[]);
		assert_eq!(m.heights(false), &[5.05, 7.0]);
		m.set_water(&[]);
		assert!(m.heights(false).is_empty());
		m.set_metal(Some(1.0));
		m.set_metal(None);
		assert_eq!(m.heights(false), &[1.0]);
		m.set_metal(None);
		assert!(m.heights(false).is_empty());
	}

	/// The sphere pre-test rejects only spheres wholly outside one plane.
	#[test]
	fn sphere_out_is_one_plane_wholly() {
		let vp = crate::mobies::GuardBand { near: 0.03125, far: 728.0, kx: 1.0, ky: 0.75 };
		assert!(!sphere_out(&vp, [0.0, 0.0, 10.0], 1.0));
		assert!(!sphere_out(&vp, [10.5, 0.0, 10.0], 1.0), "straddles the right plane");
		assert!(sphere_out(&vp, [12.0, 0.0, 10.0], 1.0));
		assert!(sphere_out(&vp, [0.0, -9.0, 10.0], 1.0));
		assert!(sphere_out(&vp, [0.0, 0.0, -2.0], 1.0), "behind the camera");
		assert!(!sphere_out(&vp, [3.0, 0.0, 0.5], 1.0), "around the eye");
	}

	#[test]
	fn any_of_several_heights_keeps() {
		let v = view();
		let c = cube([10.0, 0.0, 12.0], 0.5);
		assert!(keeps(&v, &c, &[40.0, 0.0]));
	}
}
