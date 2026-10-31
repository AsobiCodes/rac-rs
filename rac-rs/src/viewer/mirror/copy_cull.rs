//! [PC] speed (lane water2, v0.4.2): the ties a water bed or mirror pass cannot show are left out of that pass. No retail counterpart.
//!
//! `Gpu::cull_ties` picks every tie inside the draw distance and the level pass lets the GPU clip the ones off screen. The bed and
//! the mirror drew that whole list again (Novalis: ~2.1 k draws, 240 k triangles per pass, about half of what the two cost). Here a
//! pass drops a tie whose box (`sunshadow/bounds.rs`: per LOD, holding both ends of its geomorph) lies wholly outside one of the five
//! clip planes of the pass's own view-projection (left, right, bottom, top, near; the mirror's near plane is the water plane, so
//! what lies wholly under the water goes too). Every triangle of such a tie is clipped away, so it makes no fragment, writes no
//! depth or stencil, and the picture cannot change. The far plane is not tested. The kept draws keep their order, pipelines and
//! state. The test is done in f64 with a slack of about a pixel (`REL`), far above the f32 rounding of the vertex shader's own
//! `view_proj * p`. RAC_COPY_CULL=0 draws the whole list as before (same-exe A/B).

use super::*;
use crate::viewer::sunshadow::bounds::Aabb;
use std::sync::Mutex;

/// Slack of the plane test, relative to the sum of the magnitudes of its terms (about 1 px of a 1080p view, half a pixel of the
/// mirror's half-size target; a sample lies at least a quarter pixel inside the viewport, and the f32 rounding of the vertex shader is
/// ~1e-7 of the same sum).
const REL: f64 = 1e-3;
/// Slack added in world units (a plane's normal is not unit length: the oblique near plane is scaled by 1/d), so 1 cm of distance.
const ABS: f64 = 0.01;

/// Whether the passes drop the ties they cannot show (RAC_COPY_CULL=0 turns it off).
pub(super) fn enabled() -> bool {
	static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*S.get_or_init(|| !std::env::var("RAC_COPY_CULL").is_ok_and(|v| v == "0"))
}

/// The five side planes of a column-major view-projection's clip volume (left, right, bottom, top, near), as clip-space rows:
/// the plane (a, b, c, d) with `a x + b y + c z + d >= 0` inside, and the magnitudes of its terms for the slack.
pub(super) struct Frustum {
	planes: [([f64; 4], [f64; 4]); 5],
}

impl Frustum {
	pub(super) fn new(vp: &[[f32; 4]; 4]) -> Frustum {
		let row = |r: usize| -> [f64; 4] { std::array::from_fn(|c| vp[c][r] as f64) };
		let (x, y, z, w) = (row(0), row(1), row(2), row(3));
		let sum = |a: &[f64; 4], b: &[f64; 4], s: f64| -> ([f64; 4], [f64; 4]) { (std::array::from_fn(|k| a[k] + s * b[k]), std::array::from_fn(|k| a[k].abs() + b[k].abs())) };
		Frustum { planes: [sum(&w, &x, 1.0), sum(&w, &x, -1.0), sum(&w, &y, 1.0), sum(&w, &y, -1.0), (z, z.map(f64::abs))] }
	}

	/// Whether the box lies wholly outside one plane (every corner has `a x + b y + c z + d < -slack`). A box with a NaN
	/// somewhere is never rejected.
	pub(super) fn rejects(&self, b: &Aabb) -> bool {
		let c: [f64; 3] = std::array::from_fn(|k| 0.5 * (b.lo[k] as f64 + b.hi[k] as f64));
		let h: [f64; 3] = std::array::from_fn(|k| 0.5 * (b.hi[k] as f64 - b.lo[k] as f64));
		self.planes.iter().any(|(p, m)| {
			// The largest value of the plane over the box: at its centre plus the half extents weighted by |a|, |b|, |c|.
			let top = p[3] + (0..3).map(|k| p[k] * c[k] + p[k].abs() * h[k]).sum::<f64>();
			let scale = m[3] + (0..3).map(|k| m[k] * (c[k].abs() + h[k])).sum::<f64>();
			top < -(REL * scale + ABS * (p[0].abs() + p[1].abs() + p[2].abs()))
		})
	}
}

/// The ties a pass keeps as (tie, LOD), in tie order: `lods[i]` the LOD `cull_ties` drew tie `i` with (`u8::MAX` = not drawn),
/// `boxes[i][lod]` its box (none = no proven box: kept). None when a LOD is out of range (the bookkeeping is not what it should be).
pub(super) fn select(lods: &[u8], boxes: &[[Option<Aabb>; 3]], f: &Frustum) -> Option<Vec<(usize, usize)>> {
	let mut out = Vec::with_capacity(lods.len());
	for (i, &lod) in lods.iter().enumerate() {
		if lod == u8::MAX {
			continue;
		}
		let lod = lod as usize;
		if lod >= 3 {
			return None;
		}
		if boxes.get(i).and_then(|b| b[lod].as_ref()).is_some_and(|b| f.rejects(b)) {
			continue;
		}
		out.push((i, lod));
	}
	Some(out)
}

/// The tie draws a pass keeps, and (with the bindless path) its own indirect arguments for them.
pub(super) struct Kept {
	pub draws: Vec<Draw>,
	pub args: Option<(wgpu::Buffer, super::super::bindless::Runs)>,
}

/// A pass's own indirect argument buffer for the ties it keeps (each pass has one: all passes of a frame go in one submit).
pub(super) struct TieArgs(Mutex<Option<(wgpu::Buffer, usize)>>);

impl TieArgs {
	pub(super) fn new() -> TieArgs {
		TieArgs(Mutex::new(None))
	}

	/// Writes `draws` as indirect arguments (`bindless::push_args`: texture slot as the first instance, runs of one cull mode) and
	/// returns the buffer with the runs.
	fn upload(&self, device: &wgpu::Device, queue: &GatedQueue, draws: &[Draw]) -> (wgpu::Buffer, super::super::bindless::Runs) {
		let mut args: Vec<[u32; 5]> = Vec::with_capacity(draws.len());
		let runs = super::super::bindless::push_args(&mut args, draws.iter().copied(), true);
		let mut held = self.0.lock().unwrap_or_else(|e| e.into_inner());
		if held.as_ref().is_none_or(|(_, cap)| args.len() > *cap) {
			let cap = args.len().next_power_of_two().max(256);
			let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("copy tie draws"), size: (cap * 20) as u64, usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
			*held = Some((buf, cap));
		}
		let buf = held.as_ref().map(|(b, _)| b.clone()).expect("buffer just made");
		if !args.is_empty() {
			queue.write_buffer(&buf, 0, bytemuck::cast_slice(&args));
		}
		(buf, runs)
	}
}

impl Gpu {
	/// The tie draws of the pass with view-projection `vp` (`mirror`: for the stats line), in the order of `scene.ties.draws`, with
	/// their own indirect arguments when `multi` (the frame's list goes as multi-draw runs). None: draw the whole list.
	pub(super) fn kept_ties(&self, scene: &Scene, with: &Mirror, vp: &[[f32; 4]; 4], multi: bool, mirror: bool) -> Option<Kept> {
		if !enabled() || scene.tie_lod.len() != scene.tie_cull.len() || scene.tie_ranges.len() != scene.tie_cull.len() {
			return None;
		}
		let _s = crate::viewer::profile::span("copy_cull");
		let keep = select(&scene.tie_lod, &scene.sun_bb.ties, &Frustum::new(vp))?;
		let range = |i: usize, lod: usize| scene.tie_ranges[i][lod].start as usize..scene.tie_ranges[i][lod].end as usize;
		let mut draws = Vec::with_capacity(scene.ties.draws.len());
		for &(i, lod) in &keep {
			draws.extend_from_slice(&scene.tie_draws[range(i, lod)]);
		}
		if super::stats() {
			self.note(scene, &draws, mirror);
		}
		let args = multi.then(|| with.args.upload(&self.device, &self.queue, &draws));
		Some(Kept { draws, args })
	}

	/// RAC_COPY_STATS: kept / all draws and triangles, and whether the unculled list is `scene.ties.draws` (the bookkeeping holds).
	fn note(&self, scene: &Scene, kept: &[Draw], mirror: bool) {
		let tris = |d: &[Draw]| d.iter().map(|d| d.count as u64 / 3).sum::<u64>();
		let all: Vec<Draw> = scene.tie_lod.iter().enumerate().filter(|(_, l)| **l != u8::MAX).flat_map(|(i, &l)| scene.tie_draws[scene.tie_ranges[i][l as usize].start as usize..scene.tie_ranges[i][l as usize].end as usize].iter().copied()).collect();
		let key = |d: &Draw| (d.inst, d.cull, d.texture, d.start, d.count);
		let same = all.len() == scene.ties.draws.len() && all.iter().zip(&scene.ties.draws).all(|(a, b)| key(a) == key(b));
		eprintln!("copycull {} ties kept draws {}/{} tris {}/{} list_matches {}", if mirror { "mirror" } else { "bed" }, kept.len(), all.len(), tris(kept), tris(&all), same);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn cam() -> [[f32; 4]; 4] {
		let cam = Camera { pos: [0.0, -20.0, 10.0], yaw: std::f32::consts::FRAC_PI_2, pitch: -0.3, tan_half: 0.6, up: [0.0, 0.0, 1.0] };
		super::super::super::view_proj(&cam, 1.6, 0.5, 2000.0)
	}

	fn bx(lo: [f32; 3], hi: [f32; 3]) -> Aabb {
		Aabb { lo, hi }
	}

	/// The vertex shader's clip position of `p` (viewer.rs `world_clip`, in f32): `m * vec4(p, 1)`, with clip z rebuilt as `a w + b` from the
	/// matrix's own rows when its z row is nearly a multiple of its w row plus a constant (every bed matrix; a mirror matrix only when the
	/// view is within ~1e-5 rad of vertical). The flag says whether that happened.
	fn world_clip(m: &[[f32; 4]; 4], p: [f32; 3]) -> ([f32; 4], bool) {
		let mut c: [f32; 4] = std::array::from_fn(|r| (0..4).map(|k| m[k][r] * if k < 3 { p[k] } else { 1.0 }).sum());
		let wr = [m[0][3], m[1][3], m[2][3], m[3][3]];
		let zr = [m[0][2], m[1][2], m[2][2], m[3][2]];
		let dot3 = |a: &[f32; 4], b: &[f32; 4]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
		let ww = dot3(&wr, &wr);
		if ww > 1.0e-6 {
			let a = dot3(&zr, &wr) / ww;
			let d: [f32; 4] = std::array::from_fn(|k| zr[k] - a * wr[k]);
			if dot3(&d, &d) <= 1.0e-10 * ww {
				c[2] = a * c[3] + (zr[3] - a * wr[3]);
				return (c, true);
			}
		}
		(c, false)
	}

	/// Which of the five planes (left, right, bottom, top, near; wgpu's clip volume -w <= x <= w, -w <= y <= w, 0 <= z) the corner `p` is
	/// outside of, by the shader's clip position.
	fn outside(m: &[[f32; 4]; 4], p: [f32; 3]) -> [bool; 5] {
		let (c, _) = world_clip(m, p);
		[c[0] < -c[3], c[0] > c[3], c[1] < -c[3], c[1] > c[3], c[2] < 0.0]
	}

	/// The camera of `cam()` looking `pitch` radians off the horizon.
	fn pitched(pitch: f32) -> [[f32; 4]; 4] {
		let cam = Camera { pos: [0.0, -20.0, 10.0], yaw: std::f32::consts::FRAC_PI_2, pitch, tan_half: 0.6, up: [0.0, 0.0, 1.0] };
		super::super::super::view_proj(&cam, 1.6, 0.5, 2000.0)
	}

	/// The matrices the passes draw with: the bed (two near/far pairs, one of them the retail world's), mirrors of it about three
	/// heights, and mirrors of a view within 3e-6 rad of straight down and of exactly straight down (where the shader's z substitution
	/// can apply to a mirror matrix).
	fn mats() -> Vec<[[f32; 4]; 4]> {
		let retail = Camera { pos: [0.0, -20.0, 10.0], yaw: std::f32::consts::FRAC_PI_2, pitch: -0.3, tan_half: TAN_HALF_FOV, up: [0.0, 0.0, 1.0] };
		let down = -std::f32::consts::FRAC_PI_2;
		vec![
			cam(),
			super::super::super::view_proj(&retail, 16.0 / 9.0, RETAIL_NEAR, RETAIL_FAR),
			super::super::mirrored(&cam(), 2.0),
			super::super::mirrored(&cam(), -30.0),
			super::super::mirrored(&pitched(down + 3.0e-6), 2.0),
			super::super::mirrored(&pitched(down), 2.0),
		]
	}

	#[test]
	fn a_box_in_view_or_straddling_a_plane_is_kept() {
		let f = Frustum::new(&cam());
		assert!(!f.rejects(&bx([-1.0, 20.0, 5.0], [1.0, 22.0, 7.0])), "in front of the camera");
		// A huge box round the eye straddles every plane.
		assert!(!f.rejects(&bx([-500.0; 3], [500.0; 3])));
		// Wide enough to cross the left plane only partly.
		assert!(!f.rejects(&bx([-200.0, 20.0, 5.0], [0.0, 22.0, 7.0])));
	}

	#[test]
	fn a_box_wholly_behind_or_beside_the_view_is_dropped() {
		let f = Frustum::new(&cam());
		assert!(f.rejects(&bx([-1.0, -60.0, 5.0], [1.0, -50.0, 7.0])), "behind the camera (near plane)");
		assert!(f.rejects(&bx([300.0, 20.0, 5.0], [310.0, 22.0, 7.0])), "far to the right");
		assert!(f.rejects(&bx([-310.0, 20.0, 5.0], [-300.0, 22.0, 7.0])), "far to the left");
		assert!(f.rejects(&bx([-1.0, 20.0, 400.0], [1.0, 22.0, 410.0])), "far above");
		assert!(f.rejects(&bx([-1.0, 20.0, -410.0], [1.0, 22.0, -400.0])), "far below");
	}

	#[test]
	fn the_mirror_drops_what_lies_wholly_under_the_water() {
		let h = 2.0;
		let m = super::super::mirrored(&cam(), h);
		let f = Frustum::new(&m);
		// Under the plane (z < h) the oblique near plane clips it; the mirrored view looks at the reflection of the world above.
		assert!(f.rejects(&bx([-1.0, 20.0, -9.0], [1.0, 22.0, 1.5])), "wholly below the water");
		assert!(!f.rejects(&bx([-1.0, 20.0, 1.0], [1.0, 22.0, 6.0])), "straddling the water plane");
		assert!(!f.rejects(&bx([-1.0, 20.0, 3.0], [1.0, 22.0, 7.0])), "above the water, in the mirrored view");
	}

	/// The model of the shader's z really takes the substitution for a bed matrix and for a near-vertical mirror, and leaves a tilted
	/// mirror's own z alone, so the property test below sees both clip-z forms.
	#[test]
	fn the_shader_model_substitutes_clip_z_where_the_shader_does() {
		let m = mats();
		let flags: Vec<bool> = m.iter().map(|m| world_clip(m, [1.0, 2.0, 3.0]).1).collect();
		assert_eq!(flags[..4], [true, true, false, false], "bed matrices substitute, tilted mirrors do not: {flags:?}");
		assert!(flags[4] || flags[5], "a near-vertical mirror matrix must take the substitution: {flags:?}");
	}

	/// The two properties one box must have: if `rejects` drops it, ONE plane has all 8 corners outside it (the shader's clip position,
	/// f32 incl. its z substitution), so no corner is in view and no triangle can reach the pass; if any corner is inside the clip
	/// volume it is kept. Returns whether it was dropped and the planes that have all corners outside.
	fn check_box(m: &[[f32; 4]; 4], f: &Frustum, lo: [f32; 3], hi: [f32; 3]) -> (bool, [bool; 5]) {
		let corners: Vec<[f32; 3]> = (0..8).map(|k| [if k & 1 == 0 { lo[0] } else { hi[0] }, if k & 2 == 0 { lo[1] } else { hi[1] }, if k & 4 == 0 { lo[2] } else { hi[2] }]).collect();
		let all_out: [bool; 5] = std::array::from_fn(|k| corners.iter().all(|&p| outside(m, p)[k]));
		let rejected = f.rejects(&bx(lo, hi));
		if rejected {
			assert!(all_out.iter().any(|&o| o), "the rejected box {lo:?} {hi:?} has no plane with all 8 corners outside");
		}
		if corners.iter().any(|&p| !outside(m, p).iter().any(|&o| o)) {
			assert!(!rejected, "the box {lo:?} {hi:?} has a corner in view and was rejected");
		}
		(rejected, all_out)
	}

	#[test]
	fn a_rejected_box_has_all_corners_outside_one_plane() {
		let mut rng = 987654321u32;
		let mut next = || {
			rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
			(rng >> 8) as f32 / (1 << 24) as f32
		};
		let (mut dropped, mut kept, mut by_plane) = (0, 0, [0usize; 5]);
		for m in &mats() {
			let f = Frustum::new(m);
			for _ in 0..6000 {
				let lo = [next() * 300.0 - 150.0, next() * 300.0 - 100.0, next() * 100.0 - 50.0];
				let hi = [lo[0] + next() * 20.0, lo[1] + next() * 20.0, lo[2] + next() * 20.0];
				let (rejected, all_out) = check_box(m, &f, lo, hi);
				if rejected {
					dropped += 1;
					for k in 0..5 {
						by_plane[k] += all_out[k] as usize;
					}
				} else {
					kept += 1;
				}
			}
		}
		assert!(dropped > 1500 && kept > 600, "{dropped} dropped, {kept} kept: the test should see both");
		assert!(by_plane.iter().all(|&n| n > 20), "every plane should reject boxes: {by_plane:?}");
	}

	/// The mirror's near plane is the water plane: boxes ending at, a hair under (the 1 cm slack and beyond) or reaching above it, in
	/// views from tilted to straight down, are dropped only when the shader's own clip position clips all of them.
	#[test]
	fn a_box_at_the_water_plane_is_dropped_only_if_the_shader_clips_all_of_it() {
		let h = 2.0f32;
		let (mut dropped, mut kept, mut near) = (0, 0, 0);
		for pitch in [-0.3, -1.2, -1.5, -std::f32::consts::FRAC_PI_2 + 3.0e-6, -std::f32::consts::FRAC_PI_2] {
			let m = super::super::mirrored(&pitched(pitch), h);
			let f = Frustum::new(&m);
			for (i, gap) in [-0.5f32, 0.0, 0.004, 0.02, 0.1, 1.0].into_iter().enumerate() {
				for k in 0..40 {
					let (x, y) = (-30.0 + 1.7 * k as f32, -60.0 + 3.1 * k as f32 + i as f32);
					let (rejected, all_out) = check_box(&m, &f, [x, y, h - gap - 4.0], [x + 3.0, y + 3.0, h - gap]);
					dropped += rejected as usize;
					kept += !rejected as usize;
					near += (rejected && all_out[4]) as usize;
				}
			}
		}
		assert!(dropped > 50 && kept > 50 && near > 20, "{dropped} dropped ({near} by the near plane), {kept} kept: the test should see both");
	}

	#[test]
	fn a_nan_box_or_matrix_is_never_rejected() {
		let f = Frustum::new(&cam());
		assert!(!f.rejects(&bx([f32::NAN; 3], [1.0; 3])));
		let mut m = cam();
		m[0][3] = f32::NAN; // w: the four side planes are NaN, the near plane (z row) is not
		assert!(!Frustum::new(&m).rejects(&bx([300.0, 20.0, 5.0], [310.0, 22.0, 7.0])), "in front of the near plane, the rest NaN");
	}

	#[test]
	fn select_keeps_tie_order_skips_undrawn_and_keeps_a_tie_without_a_box() {
		let f = Frustum::new(&cam());
		let inview = bx([-1.0, 20.0, 5.0], [1.0, 22.0, 7.0]);
		let behind = bx([-1.0, -60.0, 5.0], [1.0, -50.0, 7.0]);
		// tie 0 undrawn, 1 in view at LOD 1, 2 behind at LOD 0 (dropped), 3 no box at LOD 2 (kept), 4 box only at another LOD (kept), 5 in view at LOD 0.
		let boxes = vec![[Some(inview); 3], [None, Some(inview), None], [Some(behind), None, None], [None, None, None], [Some(behind), None, None], [Some(inview), None, None]];
		let lods = [u8::MAX, 1, 0, 2, 1, 0];
		assert_eq!(select(&lods, &boxes, &f), Some(vec![(1, 1), (3, 2), (4, 1), (5, 0)]));
		// Fewer boxes than ties: the missing ones are kept.
		assert_eq!(select(&[0, 0], &boxes[..1], &f), Some(vec![(0, 0), (1, 0)]));
		assert_eq!(select(&[3], &boxes, &f), None, "an impossible LOD gives up");
	}
}
