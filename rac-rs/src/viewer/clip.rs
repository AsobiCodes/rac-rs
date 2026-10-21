//! NOV-5.9c / NOV-5.9d: the CPU side of the retail moby guard-band clipper (VU1 prog11 `L_1b58`). Faces that cross the
//! guard band are pre-clipped here into the retail list-order polygon (`mobies::guard_clip`) and drawn as its strip
//! triangles; the GPU triangle they replace is masked or left out of the batch.

use super::{Camera, Vertex};
use crate::mesh::Mesh;
use std::collections::{BTreeMap, HashMap};
use crate::mobies::{clip_strip, guard_clip_weights, GuardBand};

/// One clippable face of a props / marker slot: the position of its first index in the batch's index buffer, the
/// stored indices, the strip phase and what its draw run needs (texture slot, cull mode).
/// [PC] perf (lane A7): `run` = u32::MAX: `slot` is that position; else `slot` counts from the start of the slot's run
/// `run` (`Scene::slot_runs`, the props batch), so a pool growth that moves the runs (`Gpu::append_props`) leaves the faces.
#[derive(Clone, Copy, Debug)]
pub(super) struct PropFace {
	pub slot: u32,
	pub run: u32,
	pub idx: [u32; 3],
	pub phase: u8,
	pub tex: usize,
	pub cull: u8,
}

/// A clipped face: the polygon's vertices (lerped) and its winding-fixed strip triangles, with the draw run key.
pub(super) struct ClipPoly {
	pub tex: usize,
	pub cull: u8,
	pub verts: Vec<Vertex>,
	pub tris: Vec<[usize; 3]>,
}

/// The view basis of this frame (`e` eye, `s` right, `u` up, `f` forward) and the tangents of the view volume.
#[derive(Clone, Copy, Debug)]
pub(super) struct ClipView {
	pub e: [f32; 3],
	pub s: [f32; 3],
	pub u: [f32; 3],
	pub f: [f32; 3],
	pub tx: f32,
	pub ty: f32,
	/// The guard band's and viewport's far plane (m): `RETAIL_FAR`, or the world far plane of Draw distance High
	/// (fardraw.rs, `with_far`).
	pub far: f32,
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
	a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

impl ClipView {
	/// The basis `view_proj` builds; the horizontal tangent follows the window aspect [PC] (REN-7a).
	pub(super) fn new(cam: &Camera, aspect: f32) -> ClipView {
		let f = super::forward(cam);
		// BRIDGE-56: the view's up (rolled under the Visibomb remote camera).
		let c = cross(f, cam.up);
		let l = dot(c, c).sqrt().max(1e-9);
		let s = [c[0] / l, c[1] / l, c[2] / l];
		let u = cross(s, f);
		ClipView { e: cam.pos, s, u, f, tx: cam.tan_half * aspect, ty: cam.tan_half, far: super::RETAIL_FAR }
	}

	/// [PC] The view with the world far plane `far` (m, `Gpu::world_far`): Draw distance High clips and culls at its own far plane
	/// instead of retail's (Low and Medium pass `RETAIL_FAR`: unchanged).
	pub(super) fn with_far(self, far: f32) -> ClipView {
		ClipView { far, ..self }
	}

	/// View-space (vx, vy, vz) of a world point.
	pub(super) fn view(&self, p: [f32; 3]) -> [f32; 3] {
		let d = sub(p, self.e);
		[dot(self.s, d), dot(self.u, d), dot(self.f, d)]
	}

	/// REN-MOBY-DRAWN: the moby draw's near and side rejects of a sphere `(c, r)` in 1/1024 m (`c` a world point, `r` the
	/// radius): the retail horizontal tangent is `ty / PROJ_Y_RATIO` and its vertical one 0.756 times that. Only the +0x31
	/// verdict uses it (the GPU clips what is drawn).
	pub(super) fn moby_side_near_out(&self, c: [f32; 3], r: f32) -> bool {
		let d: [f32; 3] = std::array::from_fn(|k| c[k] - self.e[k] * 1024.0);
		let (vx, vy, vz) = (dot(self.s, d), dot(self.u, d), dot(self.f, d));
		let tx = self.ty / super::PROJ_Y_RATIO;
		let ty = tx * super::PROJ_Y_RATIO;
		let (sx, sy) = ((1.0 + tx * tx).sqrt(), (1.0 + ty * ty).sqrt());
		let near = 32.0 - (r + vz);
		let cx = tx * vz - (vx.abs() - sx * r);
		let cy = ty * vz - (vy.abs() - sy * r);
		!near.is_sign_negative() || cx.is_sign_negative() || cy.is_sign_negative()
	}

	/// REN-CULL-TFRAG step (3): whether a sphere (view-space centre `p`, radius `r`, both in 1/1024 m) lies wholly inside the retail
	/// frustum (far 728 m, near 32, side planes with the retail tangents and secants), so the box test is skipped.
	pub(super) fn sphere_wholly_inside(&self, p: [f32; 3], r: f32) -> bool {
		let tx = self.ty / super::PROJ_Y_RATIO;
		let ty = tx * super::PROJ_Y_RATIO;
		let (sx, sy) = ((1.0 + tx * tx).sqrt(), (1.0 + ty * ty).sqrt());
		// [PC] Draw distance High (`far` past retail's): its own far plane.
		let far = if self.far > super::RETAIL_FAR { self.far * 1024.0 } else { 745472.0 };
		p[2] + r - far < 0.0 && p[2] - r - 32.0 >= 0.0 && tx * p[2] - (p[0].abs() + r * sx) >= 0.0 && ty * p[2] - (p[1].abs() + r * sy) >= 0.0
	}

	/// The retail guard band (4 x the tangents; far = `far`, retail's 728.1632 m but with Draw distance High).
	pub(super) fn band(&self) -> GuardBand {
		GuardBand { far: self.far, ..GuardBand::retail(self.tx, self.ty) }
	}

	/// The real viewport planes (1 x the tangents, same near / far): a face wholly outside one is drawn by nothing.
	pub(super) fn viewport(&self) -> GuardBand {
		GuardBand { kx: self.tx, ky: self.ty, ..self.band() }
	}
}

/// The clippable faces of each mesh of an unordered `Gpu::batch` over `meshes`: replays its grouping (3 indices per face
/// per texture key, groups in ascending key order, faces in mesh order) to find each face's position in the batch's
/// index buffer. Meshes without a phase per face get an empty list. `slots` maps texture keys to bind-group slots.
pub(super) fn replay_faces(meshes: &[&Mesh], slots: &HashMap<u32, usize>) -> Vec<Vec<PropFace>> {
	let mut count: BTreeMap<u32, u32> = BTreeMap::new();
	for m in meshes {
		for &(key, _) in &m.faces {
			*count.entry(key).or_default() += 3;
		}
	}
	let mut next = BTreeMap::new();
	let mut at = 0;
	for (&key, &n) in &count {
		next.insert(key, at);
		at += n;
	}
	let mut base = 0u32;
	meshes
		.iter()
		.map(|m| {
			let phased = m.strip_phase.len() == m.faces.len();
			let mut out = Vec::new();
			for (i, &(key, f)) in m.faces.iter().enumerate() {
				let slot = next[&key];
				*next.get_mut(&key).unwrap() += 3;
				if phased && m.strip_phase[i] != crate::mesh::NO_PHASE {
					let tex = slots.get(&(key & crate::mobies::SLOT_MASK)).copied().unwrap_or(0);
					out.push(PropFace { slot, run: u32::MAX, idx: f.map(|x| x + base), phase: m.strip_phase[i], tex, cull: ((key >> 24) & 3) as u8 });
				}
			}
			base += m.positions.len() as u32;
			out
		})
		.collect()
}

/// Strip-order slots of a stored face by its phase (inverse of `strip_triangles`): phase 0 `(f2, f1, f0)`, phase 1 `(f1, f2, f0)`.
pub(super) fn strip_order(phase: u8) -> [usize; 3] {
	if phase == 0 {
		[2, 1, 0]
	} else {
		[1, 2, 0]
	}
}

/// Clips the world-space face `tri` (stored order) when the retail clipper would run on it: some vertex outside the
/// guard band and the three not all beyond one viewport plane. `None` = the GPU draws it unchanged.
pub(super) fn clip_face(view: &ClipView, tri: [Vertex; 3], phase: u8, tex: usize, cull: u8) -> Option<ClipPoly> {
	let (band, port) = (view.band(), view.viewport());
	let vs = tri.map(|v| view.view(v.pos));
	let or = vs.iter().fold(0u8, |a, &p| a | band.flags(p));
	let and = vs.iter().fold(0x3fu8, |a, &p| a & port.flags(p));
	if or == 0 || and != 0 {
		return None;
	}
	let order = strip_order(phase);
	let strip = order.map(|i| vs[i]);
	let poly = guard_clip_weights(strip, band);
	// Weights over the strip slots -> over the stored vertices.
	let mix = |w: [f32; 3], get: &dyn Fn(&Vertex) -> Vec<f32>| -> Vec<f32> {
		let mut out = vec![0.0; get(&tri[0]).len()];
		for (j, &i) in order.iter().enumerate() {
			for (o, x) in out.iter_mut().zip(get(&tri[i])) {
				*o += w[j] * x;
			}
		}
		out
	};
	let verts: Vec<Vertex> = poly
		.iter()
		.map(|&(tag, _, w)| {
			if tag < 3 {
				return tri[order[tag as usize]];
			}
			let pos = mix(w, &|v| v.pos.to_vec());
			let uv = mix(w, &|v| v.uv.to_vec());
			let normal = mix(w, &|v| v.normal.to_vec());
			let colour = mix(w, &|v| v.colour.to_vec());
			Vertex { pos: [pos[0], pos[1], pos[2]], uv: [uv[0], uv[1]], normal: [normal[0], normal[1], normal[2]], colour: [colour[0], colour[1], colour[2], tri[0].colour[3]], light: tri[0].light }
		})
		.collect();
	let c0 = cross(sub(vs[1], vs[0]), sub(vs[2], vs[0]));
	let tris = clip_strip(poly.len())
		.into_iter()
		.map(|t| {
			let c = cross(sub(poly[t[1]].1, poly[t[0]].1), sub(poly[t[2]].1, poly[t[0]].1));
			if dot(c, c0) < 0.0 {
				[t[0], t[2], t[1]]
			} else {
				t
			}
		})
		.collect();
	Some(ClipPoly { tex, cull, verts, tris })
}

#[cfg(test)]
mod tests {
	use super::*;

	fn vert(p: [f32; 3]) -> Vertex {
		Vertex { pos: p, uv: [p[0], p[1]], normal: [0.0, 0.0, 1.0], colour: [1.0, 0.5, 0.25, 2.0], light: [3.0; 4] }
	}

	/// Camera at the origin looking along +x: right = -y, up = +z, forward = +x.
	fn view() -> ClipView {
		ClipView::new(&Camera { pos: [0.0; 3], yaw: 0.0, pitch: 0.0, tan_half: 0.47627997, up: [0.0, 0.0, 1.0] }, 1.0)
	}

	/// REN-MOBY-DRAWN: tx = 0.63, ty = 0.47628, sx = 1.1819053; spheres in 1/1024 m, camera at the origin looking along +x.
	#[test]
	fn moby_near_and_side_rejects() {
		let v = view();
		// 10 m ahead, 7 m to the side, radius 1.5 m: inside (the tie test (4) numbers).
		assert!(!v.moby_side_near_out([10240.0, 7168.0, 0.0], 1536.0));
		// Far to the side: outside the right plane.
		assert!(v.moby_side_near_out([10240.0, 9000.0, 0.0], 1536.0));
		// Above the top plane.
		assert!(v.moby_side_near_out([10240.0, 0.0, 9000.0], 1536.0));
		// Behind the near plane: 32.0 - (r + vz) = 122 >= 0.
		assert!(v.moby_side_near_out([-100.0, 0.0, 0.0], 10.0));
		// A sphere straddling the near plane passes (32 - (1536 + 0) < 0).
		assert!(!v.moby_side_near_out([0.0, 0.0, 0.0], 1536.0));
		// Near term exactly +0.0 rejects: 32 - (r + vz) with r = 32, vz = 0.
		assert!(v.moby_side_near_out([0.0, 0.0, 0.0], 32.0));
	}

	#[test]
	fn strip_phase_recovers_the_strip_triple() {
		assert_eq!(strip_order(0), [2, 1, 0]);
		assert_eq!(strip_order(1), [1, 2, 0]);
	}

	#[test]
	fn replay_matches_the_unordered_batch_grouping() {
		// Two meshes, keys 5 and 2: the batch lists key 2 first (ascending), then key 5, faces in mesh order.
		let mut a = Mesh::new("a".into());
		a.positions = vec![[0.0; 3]; 4];
		a.faces = vec![(5, [0, 1, 2]), (2, [1, 2, 3])];
		a.strip_phase = vec![0, 1];
		let mut b = Mesh::new("b".into());
		b.positions = vec![[0.0; 3]; 3];
		b.faces = vec![(5, [0, 1, 2])];
		let c = Mesh::new("c".into());
		let slots: HashMap<u32, usize> = [(5, 9), (2, 4)].into_iter().collect();
		let r = replay_faces(&[&a, &b, &c], &slots);
		assert_eq!(r[0].iter().map(|f| (f.slot, f.idx, f.phase, f.tex)).collect::<Vec<_>>(), vec![(3, [0, 1, 2], 0, 9), (0, [1, 2, 3], 1, 4)]);
		assert!(r[1].is_empty() && r[2].is_empty());
	}

	// NOV-5.9d test 2: appended gadget faces keep `strip_phase` parallel to `faces`; a source without phases gets 255 and
	// is left out of the replay while the body faces still clip.
	#[test]
	fn appended_gadget_faces_keep_phases_parallel() {
		let mut body = Mesh::new("hero".into());
		body.positions = vec![[0.0; 3]; 3];
		body.faces = vec![(1, [0, 1, 2])];
		body.strip_phase = vec![1];
		let mut wrench = Mesh::new("wrench".into());
		wrench.positions = vec![[0.0; 3]; 3];
		wrench.faces = vec![(1, [0, 1, 2])];
		let was = body.strip_phase.len() == body.faces.len();
		body.faces.extend(wrench.faces.iter().copied());
		body.extend_phases(&wrench, was);
		assert_eq!(body.strip_phase, vec![1, crate::mesh::NO_PHASE]);
		let slots: HashMap<u32, usize> = [(1, 0)].into_iter().collect();
		let r = replay_faces(&[&body], &slots);
		assert_eq!(r[0].len(), 1);
		assert_eq!((r[0][0].slot, r[0][0].phase), (0, 1));
		// A phased source appends its own phases; a misaligned body stays misaligned (never clipped).
		let mut phased = Mesh::new("g".into());
		phased.faces = vec![(1, [0, 1, 2])];
		phased.strip_phase = vec![0];
		body.faces.extend(phased.faces.iter().copied());
		body.extend_phases(&phased, true);
		assert_eq!(body.strip_phase, vec![1, crate::mesh::NO_PHASE, 0]);
		let mut bare = Mesh::new("b".into());
		bare.faces = vec![(1, [0, 1, 2])];
		bare.faces.extend(phased.faces.iter().copied());
		bare.extend_phases(&phased, false);
		assert!(bare.strip_phase.is_empty());
	}

	#[test]
	fn faces_inside_the_band_or_outside_the_viewport_are_left_alone() {
		let v = view();
		// Inside.
		let t = [vert([5.0, 0.0, 0.0]), vert([5.0, 0.5, 0.0]), vert([5.0, 0.0, 0.5])];
		assert!(clip_face(&v, t, 0, 0, 0).is_none());
		// All three above the top of the viewport and the band: same viewport side, nothing to draw.
		let t = [vert([1.0, 0.0, 9.0]), vert([1.0, 0.5, 9.0]), vert([1.0, 0.0, 9.5])];
		assert!(clip_face(&v, t, 0, 0, 0).is_none());
	}

	#[test]
	fn a_face_crossing_the_top_of_the_band_clips_to_a_quad_with_lerped_attributes() {
		let v = view();
		// View space A = (-0.5,-0.5,1), B = (0.5,-0.5,1), C = (0,2.5,1): world x = vz, y = -vx, z = vy.
		let w = |vx: f32, vy: f32, vz: f32| vert([vz, -vx, vy]);
		let t = [w(-0.5, -0.5, 1.0), w(0.5, -0.5, 1.0), w(0.0, 2.5, 1.0)];
		// Phase 2 strip order (f1, f2, f0) = B, C, A is not the spec's A, B, C; stored (c, b, a) with phase 0 recovers it.
		let stored = [t[2], t[1], t[0]];
		let p = clip_face(&v, stored, 0, 7, 1).expect("clipped");
		assert_eq!((p.tex, p.cull, p.verts.len(), p.tris.len()), (7, 1, 4, 2));
		// Retail list [B, I_BC, I_CA, A]: the two new vertices sit at view y = 4 * ty = 1.9051199.
		let q = v.view(p.verts[1].pos);
		assert!((q[1] - 1.9051199).abs() < 1e-5 && (q[0] - 0.09914669).abs() < 1e-5, "{q:?}");
		// uv lerps with the same weights (uv = world x, y of the vertex), colour[3] and light come from the first stored vertex.
		assert!((p.verts[1].uv[1] - (-q[0])).abs() < 1e-5);
		assert_eq!((p.verts[1].colour[3], p.verts[1].light), (2.0, [3.0; 4]));
		// Every emitted triangle keeps the stored face's orientation in view space.
		let vs = stored.map(|s| v.view(s.pos));
		let c0 = cross(sub(vs[1], vs[0]), sub(vs[2], vs[0]));
		for t in &p.tris {
			let ps = t.map(|i| v.view(p.verts[i].pos));
			assert!(dot(cross(sub(ps[1], ps[0]), sub(ps[2], ps[0])), c0) > 0.0);
		}
	}
}
