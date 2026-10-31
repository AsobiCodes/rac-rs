//! [PC] perf (v0.4.2, 1% lows): the hero marker's guard-band clipper (NOV-5.9d path 2) without the whole CPU build.
//!
//! The old guard-band frame built all ~52k marker vertices on the CPU (blend of the two step poses, turn, light), uploaded 3.6 MB and
//! ran the clipper over every face: 3.5 ms on the dev PC, 10-13 ms on the laptop, up to twice per game step. Now the GPU build
//! (gpuhero.rs) draws the vertices as in every other frame and only the faces that may cross the guard band are built on the CPU
//! (with `MarkerCtx::vertex`, the same bytes as the full build) and given to `clip::clip_face`.
//!
//! Which faces: the model's vertices are cut into chunks of 32 (vertex >> 5). Each chunk has a bounding sphere over BOTH step poses
//! (model space, recomputed when a pose changed). Per frame the sphere is moved to the world and classified like the hero sphere of
//! `gpuskin::sphere_skips`: bit 7 = wholly inside the guard band, bits 0..5 = wholly beyond a viewport plane (the layout of
//! `GuardBand::flags`). A face is skipped when the AND of its three chunk classes is not zero: either all three chunks are inside the
//! band (every vertex flag 0, so `clip_face` has no vertex outside the band) or they share a plane that all their vertices are beyond
//! (the AND of the viewport flags is not zero). Both make `clip_face` return None, so the skipped faces are exactly faces it would
//! not clip, and every other face is a candidate that gets the real `clip_face` on exact vertices: the result is the full path's.
//! A chunk with a non-finite position, a face outside the chunk table or a blend factor outside 0..=1 takes the legacy full path.

use super::*;
use super::clip::{clip_face, ClipPoly, ClipView, PropFace};
use super::marker_ctx::{MarkerCtx, MarkerIn};
use crate::mobies::GuardBand;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

/// Vertices per chunk are `1 << CHUNK_SHIFT`.
const CHUNK_SHIFT: u32 = 5;
const CHUNK: usize = 1 << CHUNK_SHIFT;
/// From this many candidate faces on the clipper runs on the rayon pool (results in face order either way).
const PAR_MIN: usize = 2048;

/// The chunk spheres of the kept poses, the chunk triple of every face and the per-frame classes (kept in `HeroGpu`).
#[derive(Default)]
pub(in crate::viewer) struct ChunkCache {
	/// (pose version, the blend source counted) the spheres were made for.
	key: Option<(u64, bool)>,
	/// Model-space centre and radius per chunk.
	spheres: Vec<[f32; 4]>,
	faces: Vec<[u16; 3]>,
	faces_built: bool,
	faces_ok: bool,
	classes: Vec<u8>,
}

/// The positions of `r` in `now` and in `before`.
fn points<'a>(r: std::ops::Range<usize>, now: &'a Posed, before: Option<&'a Posed>) -> impl Iterator<Item = &'a [f32; 3]> {
	now.0[r.clone()].iter().chain(before.into_iter().flat_map(move |b| b.0[r.clone()].iter()))
}

/// Centre (box middle) and radius of the positions of chunk range `r` over both poses; a non-finite position gives an infinite
/// sphere (class 0: every face touching it is a candidate).
fn sphere_of(r: std::ops::Range<usize>, now: &Posed, before: Option<&Posed>) -> [f32; 4] {
	let (mut lo, mut hi, mut finite) = ([f32::MAX; 3], [f32::MIN; 3], true);
	for p in points(r.clone(), now, before) {
		for a in 0..3 {
			finite &= p[a].is_finite();
			lo[a] = lo[a].min(p[a]);
			hi[a] = hi[a].max(p[a]);
		}
	}
	if !finite {
		return [0.0, 0.0, 0.0, f32::INFINITY];
	}
	let c = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5];
	let d2 = points(r, now, before).map(|p| (p[0] - c[0]) * (p[0] - c[0]) + (p[1] - c[1]) * (p[1] - c[1]) + (p[2] - c[2]) * (p[2] - c[2])).fold(0.0f32, f32::max);
	[c[0], c[1], c[2], d2.sqrt()]
}

/// The per-frame constants of `sphere_class`.
struct ClassCtx {
	band: GuardBand,
	port: GuardBand,
	sx: f32,
	sy: f32,
	px: f32,
	py: f32,
}

impl ClassCtx {
	fn new(view: &ClipView) -> ClassCtx {
		let (band, port) = (view.band(), view.viewport());
		let s = |k: f32| (1.0 + k * k).sqrt();
		ClassCtx { band, port, sx: s(band.kx), sy: s(band.ky), px: s(port.kx), py: s(port.ky) }
	}

	/// The class of the sphere (view-space centre `p`, radius `r`): the terms of `gpuskin::sphere_skips` (same margin for the rounding of
	/// the posing and of `ClipView::view`) split by plane. Bit 7 = wholly inside the guard band; bit 5 (-z), 4 (+z), 3 (-y), 2 (+y),
	/// 1 (-x), 0 (+x) = wholly beyond that viewport plane.
	fn class(&self, p: [f32; 3], r: f32) -> u8 {
		let (band, port) = (&self.band, &self.port);
		let r = r * 1.001 + 0.01;
		let inside = p[2] - r >= band.near && p[2] + r <= band.far && p[0].abs() + r * self.sx <= band.kx * p[2] && p[1].abs() + r * self.sy <= band.ky * p[2];
		let (ax, ay) = (port.kx * p[2].abs(), port.ky * p[2].abs());
		(inside as u8) << 7
			| (p[2] + r < port.near) as u8 * 0x20
			| (p[2] - r > port.far) as u8 * 0x10
			| (p[1] + ay + r * self.py < 0.0) as u8 * 0x08
			| (-p[1] + ay + r * self.py < 0.0) as u8 * 0x04
			| (p[0] + ax + r * self.px < 0.0) as u8 * 0x02
			| (-p[0] + ax + r * self.px < 0.0) as u8
	}
}

impl ChunkCache {
	/// The spheres for the kept poses (`key` = pose version and blend flag; `before` only when the blend source counts).
	fn refresh(&mut self, key: (u64, bool), now: &Posed, before: Option<&Posed>) {
		if self.key == Some(key) {
			return;
		}
		let _s = super::profile::span("cm_spheres");
		let n = now.0.len();
		self.spheres.resize(n.div_ceil(CHUNK), [0.0; 4]);
		// A blend source of another length cannot be bounded chunk by chunk: infinite spheres, every face a candidate.
		let before_ok = before.is_none_or(|b| b.0.len() == n);
		self.spheres.par_iter_mut().enumerate().for_each(|(ci, s)| {
			*s = if before_ok { sphere_of(ci * CHUNK..((ci + 1) * CHUNK).min(n), now, before) } else { [0.0, 0.0, 0.0, f32::INFINITY] };
		});
		self.key = Some(key);
	}

	/// The faces (indices into `faces`, ascending) the clipper may take this frame: all but those whose chunk classes share a bit.
	pub(in crate::viewer) fn candidates(&mut self, key: (u64, bool), now: &Posed, before: Option<&Posed>, view: &ClipView, inp: &MarkerIn, faces: &[PropFace]) -> Vec<u32> {
		self.refresh(key, now, before);
		let cc = ClassCtx::new(view);
		self.classes.clear();
		self.classes.extend(self.spheres.iter().map(|s| {
			let w = inp.turn([s[0], s[1], s[2]]);
			cc.class(view.view([w[0] + inp.feet[0], w[1] + inp.feet[1], w[2] + inp.feet[2]]), s[3])
		}));
		if self.classes.iter().fold(0xffu8, |a, &c| a & c) != 0 {
			return Vec::new();
		}
		if !self.faces_built || self.faces.len() != faces.len() {
			let n_chunks = self.spheres.len();
			self.faces = faces.iter().map(|f| f.idx.map(|i| (i >> CHUNK_SHIFT).min(u16::MAX as u32) as u16)).collect();
			self.faces_ok = n_chunks < u16::MAX as usize && faces.iter().all(|f| f.idx.iter().all(|&i| ((i >> CHUNK_SHIFT) as usize) < n_chunks));
			self.faces_built = true;
		}
		if !self.faces_ok {
			return (0..faces.len() as u32).collect();
		}
		let cls = &self.classes;
		self.faces.iter().enumerate().filter(|(_, f)| cls[f[0] as usize] & cls[f[1] as usize] & cls[f[2] as usize] == 0).map(|(i, _)| i as u32).collect()
	}
}

/// The selective clipper: `clip_face` on the candidate faces, vertices from `ctx`. Taken face indices and polygons in face order.
fn selective(view: &ClipView, ctx: &MarkerCtx, faces: &[PropFace], cands: &[u32]) -> (Vec<u32>, Vec<ClipPoly>) {
	let one = |&fi: &u32| {
		let f = &faces[fi as usize];
		clip_face(view, f.idx.map(|i| ctx.vertex(i as usize)), f.phase, f.tex, f.cull).map(|p| (fi, p))
	};
	let hits: Vec<(u32, ClipPoly)> = if cands.len() < PAR_MIN { cands.iter().filter_map(one).collect() } else { cands.par_iter().filter_map(one).collect() };
	hits.into_iter().unzip()
}

/// The faces the clipper takes in a frame the GPU built the vertices of, and their polygons, plus the candidate count.
/// Empty without a clip view or the GPU hero.
/// The vertices and the chunk spheres both come from the poses the GPU build was given (`HeroGpu::kept_pose`), so what is clipped
/// is what is drawn; `t` = the blend factor of the frame (0..=1).
pub(super) fn select(scene: &mut Scene, inp: &MarkerIn, t: f32) -> (Vec<u32>, Vec<ClipPoly>, usize) {
	let (Some(view), Some(hg)) = (scene.clip_view, scene.marker_gpu.as_mut()) else { return Default::default() };
	let s = super::profile::span("cm_sel");
	let cands = hg.marker_candidates(&view, inp, &scene.marker_faces);
	drop(s);
	let _c = super::profile::span("cm_clip");
	let ctx = MarkerCtx { inp: *inp, model: &scene.marker_model, pose: hg.kept_pose(t) };
	let (taken, polys) = selective(&view, &ctx, &scene.marker_faces, &cands);
	(taken, polys, cands.len())
}

/// The full clipper over the world-space vertices of the whole marker (the legacy path, kept for the CPU build): taken face indices
/// and polygons in face order.
pub(super) fn full(view: Option<ClipView>, faces: &[PropFace], verts: &[Vertex]) -> (Vec<u32>, Vec<ClipPoly>) {
	let (mut taken, mut polys) = (Vec::new(), Vec::new());
	let Some(view) = view else { return (taken, polys) };
	let (band, port) = (view.band(), view.viewport());
	let (mut inside, mut all_out) = (true, 0x3fu8);
	let _s = super::profile::span("cm_scan");
	for v in verts {
		let p = view.view(v.pos);
		inside &= band.flags(p) == 0;
		all_out &= port.flags(p);
	}
	drop(_s);
	if !inside && all_out == 0 {
		let _s = super::profile::span("cm_faces");
		for (fi, f) in faces.iter().enumerate() {
			if let Some(poly) = clip_face(&view, f.idx.map(|i| verts[i as usize]), f.phase, f.tex, f.cull) {
				taken.push(fi as u32);
				polys.push(poly);
			}
		}
	}
	(taken, polys)
}

/// Queues the polygons and updates the masked faces in `marker.ibuf` to `taken`.
pub(super) fn finish(queue: &GatedQueue, scene: &mut Scene, taken: Vec<u32>, polys: Vec<ClipPoly>) {
	scene.clip_queue.extend(polys);
	if taken != scene.marker_masked {
		let _s = super::profile::span("cm_mask");
		apply_mask(queue, &scene.marker.ibuf, &scene.marker_faces, &scene.marker_masked, &taken);
		scene.marker_masked = taken;
	}
}

/// The index writes that turn the buffer masked for `old` (ascending face indices) into the one masked for `new`: a merge of the two
/// lists; a face leaving the mask gets its indices back, a face entering it zeros. Sorted by slot, runs of adjacent slots are one write
/// (first slot, words).
fn diff_writes(faces: &[PropFace], old: &[u32], new: &[u32]) -> Vec<(u32, Vec<u32>)> {
	let mut changes: Vec<(u32, [u32; 3])> = Vec::new();
	let (mut i, mut j) = (0, 0);
	while i < old.len() || j < new.len() {
		match (old.get(i).copied(), new.get(j).copied()) {
			(Some(a), Some(b)) if a == b => {
				i += 1;
				j += 1;
			}
			(Some(a), None) => {
				changes.push((faces[a as usize].slot, faces[a as usize].idx));
				i += 1;
			}
			(Some(a), Some(b)) if a < b => {
				changes.push((faces[a as usize].slot, faces[a as usize].idx));
				i += 1;
			}
			(_, Some(b)) => {
				changes.push((faces[b as usize].slot, [0; 3]));
				j += 1;
			}
			(None, None) => break,
		}
	}
	changes.sort_unstable_by_key(|c| c.0);
	let mut out: Vec<(u32, Vec<u32>)> = Vec::new();
	for (slot, idx) in changes {
		match out.last_mut() {
			Some((first, words)) if *first + words.len() as u32 == slot => words.extend_from_slice(&idx),
			_ => out.push((slot, idx.to_vec())),
		}
	}
	out
}

fn apply_mask(queue: &GatedQueue, ibuf: &wgpu::Buffer, faces: &[PropFace], old: &[u32], new: &[u32]) {
	for (slot, words) in diff_writes(faces, old, new) {
		queue.write_buffer(ibuf, slot as u64 * 4, bytemuck::cast_slice(&words));
	}
}

/// A check found the selective clipper's result different from the full path's: guard-band frames take the full CPU path from then
/// on (until the next device), as `gpuhero::self_check_result` does for the vertex build.
static FAILED: AtomicBool = AtomicBool::new(false);

pub(in crate::viewer) fn failed() -> bool {
	FAILED.load(Relaxed)
}

/// A new device starts the checks over (`gpuhero::device_created`).
pub(in crate::viewer) fn reset() {
	FAILED.store(false, Relaxed);
}

static CHECKS: AtomicU64 = AtomicU64::new(0);
static BAD: AtomicU64 = AtomicU64::new(0);
static CANDS: AtomicU64 = AtomicU64::new(0);
static TAKEN: AtomicU64 = AtomicU64::new(0);

fn polys_equal(a: &[ClipPoly], b: &[ClipPoly]) -> bool {
	a.len() == b.len()
		&& a.iter().zip(b).all(|(x, y)| x.tex == y.tex && x.cull == y.cull && x.tris == y.tris && bytemuck::cast_slice::<_, u8>(&x.verts) == bytemuck::cast_slice::<_, u8>(&y.verts))
}

/// Check modes (RAC_HERO_GPU_CHECK=1, RAC_MARKER_CHECK=1, the hero self-check frames): the selective clipper's taken faces and polygons
/// against the full path's `taken` / `polys` of the same frame, byte for byte (stderr `marker_clip_check:` lines).
pub(super) fn check(scene: &mut Scene, inp: &MarkerIn, t: f32, taken: &[u32], polys: &[ClipPoly]) {
	let (st, sp, cands) = select(scene, inp, t);
	let n = CHECKS.fetch_add(1, Relaxed) + 1;
	CANDS.fetch_add(cands as u64, Relaxed);
	TAKEN.fetch_add(taken.len() as u64, Relaxed);
	if st != taken || !polys_equal(&sp, polys) {
		let m = BAD.fetch_add(1, Relaxed) + 1;
		if !FAILED.swap(true, Relaxed) {
			eprintln!("marker_clip_check: selective clipper FAILED -> full CPU path for the guard-band frames");
		}
		if m <= 20 {
			eprintln!("marker_clip_check: MISMATCH #{m}: taken {} vs {} faces, polygons {} vs {}", st.len(), taken.len(), sp.len(), polys.len());
		}
	}
	if n % 500 == 0 {
		eprintln!("marker_clip_check: {n} frames, {} mismatches, mean {} candidate faces of {}, mean {} taken", BAD.load(Relaxed), CANDS.load(Relaxed) / n, scene.marker_faces.len(), TAKEN.load(Relaxed) / n);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use super::super::marker_ctx::Pose;

	/// xorshift: the same numbers every run.
	struct Rng(u64);
	impl Rng {
		fn next(&mut self) -> f32 {
			self.0 ^= self.0 << 13;
			self.0 ^= self.0 >> 7;
			self.0 ^= self.0 << 17;
			(self.0 >> 40) as f32 / (1u64 << 24) as f32
		}
		fn range(&mut self, a: f32, b: f32) -> f32 {
			a + (b - a) * self.next()
		}
		fn dir(&mut self) -> [f32; 3] {
			loop {
				let v = [self.range(-1.0, 1.0), self.range(-1.0, 1.0), self.range(-1.0, 1.0)];
				let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
				if l > 0.1 && l < 1.0 {
					return v.map(|c| c / l);
				}
			}
		}
	}

	fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
		[a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
	}

	fn unit(v: [f32; 3]) -> [f32; 3] {
		let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
		v.map(|c| c / l)
	}

	fn view_of(rng: &mut Rng, eye: [f32; 3], f: [f32; 3]) -> ClipView {
		let up = if f[2].abs() > 0.99 { [1.0, 0.0, 0.0] } else { [0.0, 0.0, 1.0] };
		let s = unit(cross(f, up));
		let u = cross(s, f);
		let ty = rng.range(0.3, 0.9);
		ClipView { e: eye, s, u, f, tx: ty * rng.range(1.0, 1.8), ty, far: 728.1632 }
	}

	/// A body of `n` vertices: clusters of 32 (one chunk) around random centres inside 1.5 m, hidden (origin) vertices and jumped ones.
	fn pose(rng: &mut Rng, n: usize, prev: Option<&Vec<[f32; 3]>>) -> Vec<[f32; 3]> {
		let mut centres = Vec::new();
		for _ in 0..n.div_ceil(CHUNK) {
			centres.push([rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), rng.range(0.0, 1.5)]);
		}
		(0..n)
			.map(|i| {
				let c = prev.map_or(centres[i / CHUNK], |p| p[i]);
				let wob = if prev.is_some() { 0.05 } else { 0.3 };
				let mut p = [c[0] + rng.range(-wob, wob), c[1] + rng.range(-wob, wob), c[2] + rng.range(-wob, wob)];
				let r = rng.next();
				if r < 0.01 {
					p = [0.0; 3];
				} else if r < 0.02 {
					p[0] += 3.0;
				}
				p
			})
			.collect()
	}

	/// Faces: mostly within a chunk or its neighbour, some from far chunks.
	fn faces_of(rng: &mut Rng, n: usize, count: usize) -> Vec<PropFace> {
		(0..count)
			.map(|i| {
				let a = (rng.next() * n as f32) as u32 % n as u32;
				let near = |rng: &mut Rng| (a + (rng.next() * 40.0) as u32).min(n as u32 - 1);
				let far = rng.next() < 0.1;
				let idx = if far { [a, (rng.next() * n as f32) as u32 % n as u32, near(rng)] } else { [a, near(rng), near(rng)] };
				PropFace { slot: i as u32 * 3, run: u32::MAX, idx, phase: (i % 2) as u8, tex: i % 3, cull: 0 }
			})
			.collect()
	}

	fn model(n: usize) -> Vec<Vertex> {
		(0..n).map(|i| Vertex { pos: [i as f32 * 0.001, 0.0, 0.0], uv: [0.25, 0.5], normal: [0.0, 0.0, 1.0], colour: [0.5, 0.25, 0.75, 1.0], light: [1.0, 2.0, 3.0, 4.0] }).collect()
	}

	/// The selective result equals the full path's for every pose / camera / tilt / blend, and the cases are not vacuous.
	#[test]
	fn selective_clip_equals_full_clip() {
		let mut rng = Rng(0x9e3779b97f4a7c15);
		let n = 32 * 40 + 11;
		let (mut crossed, mut skipped_faces, mut total_faces, mut with_taken) = (0, 0, 0, 0);
		for case in 0..400u32 {
			// The face table of a cache is made once (the marker's faces never change); this test makes new faces per case.
			let mut cache = ChunkCache::default();
			let faces = faces_of(&mut rng, n, 3000);
			let now = (pose(&mut rng, n, None), (0..n).map(|_| rng.dir()).collect::<Vec<_>>());
			let before = (pose(&mut rng, n, Some(&now.0)), (0..n).map(|_| rng.dir()).collect::<Vec<_>>());
			let (feet, yaw) = ([rng.range(-5.0, 5.0), rng.range(-5.0, 5.0), rng.range(0.0, 3.0)], rng.range(0.0, 6.28));
			let tilt = case % 3 == 0;
			let rows = tilt.then(|| crate::particles::euler_m([rng.range(-0.5, 0.5), rng.range(-0.5, 0.5), yaw]));
			let inp = MarkerIn { feet, c: yaw.cos(), sn: yaw.sin(), rows, tint: Some(([0.1, 0.2, 0.3], 100)), hero_len: 900, clank: None, ghosts: Default::default(), bomb: None, shift: None, id: 2.0, light: [1.0; 4], own: Default::default() };
			// The camera: from touching the hero to far away, looking at a random point near it (or anywhere).
			let dist = [0.05, 0.3, 1.0, 2.0, 5.0, 30.0][case as usize % 6] * rng.range(0.5, 1.5);
			let eye = { let d = rng.dir(); [feet[0] + d[0] * dist, feet[1] + d[1] * dist, feet[2] + 1.0 + d[2] * dist] };
			let aim = if case % 7 == 0 { rng.dir() } else { unit([feet[0] - eye[0] + rng.range(-1.0, 1.0), feet[1] - eye[1] + rng.range(-1.0, 1.0), feet[2] + 1.0 - eye[2]]) };
			let view = view_of(&mut rng, eye, aim);
			let blend = case % 2 == 0;
			let t = if case % 4 == 0 { 1.0 } else { rng.next() };
			let pose = if blend { Pose::Blend { before: &before, now: &now, t } } else { Pose::Plain(&now) };
			let m = model(n);
			let ctx = MarkerCtx { inp, model: &m, pose };
			let verts: Vec<Vertex> = (0..n).map(|k| ctx.vertex(k)).collect();
			let (ft, fp) = full(Some(view), &faces, &verts);
			let key = (case as u64, blend);
			let cands = cache.candidates(key, &now, blend.then_some(&before), &view, &inp, &faces);
			let (st, sp) = selective(&view, &ctx, &faces, &cands);
			assert!(st == ft, "case {case}: taken faces differ ({} vs {})", st.len(), ft.len());
			assert!(polys_equal(&sp, &fp), "case {case}: polygons differ");
			assert!(cands.windows(2).all(|w| w[0] < w[1]));
			crossed += !ft.is_empty() as u32;
			with_taken += ft.len();
			skipped_faces += faces.len() - cands.len();
			total_faces += faces.len();
			// A face the full path clips is always a candidate.
			assert!(ft.iter().all(|f| cands.binary_search(f).is_ok()), "case {case}: a clipped face was dropped");
		}
		assert!(crossed > 60, "only {crossed} of 400 cases clipped a face: the test is too vacuous");
		assert!(with_taken > 2000);
		assert!(skipped_faces * 10 > total_faces, "the selection skipped only {skipped_faces} of {total_faces} faces");
	}

	/// A chunk's class is non-zero exactly when `sphere_skips` says skip, for the same sphere.
	#[test]
	fn chunk_class_matches_sphere_skips() {
		let mut rng = Rng(0x1234_5678_9abc_def1);
		let mut kinds = [0u32; 2];
		for _ in 0..5000 {
			let (eye, f) = ([rng.range(-3.0, 3.0), rng.range(-3.0, 3.0), rng.range(-3.0, 3.0)], rng.dir());
			let view = view_of(&mut rng, eye, f);
			let d = rng.range(0.0, 20.0);
			let c = { let v = rng.dir(); [eye[0] + v[0] * d, eye[1] + v[1] * d, eye[2] + v[2] * d] };
			let r = rng.range(0.0, 2.0);
			let class = ClassCtx::new(&view).class(view.view(c), r);
			let skips = super::super::gpuskin::sphere_skips(&view, c, r);
			assert_eq!(class != 0, skips);
			kinds[skips as usize] += 1;
		}
		assert!(kinds[0] > 200 && kinds[1] > 200, "{kinds:?}");
	}

	/// The mask merge writes the indices back for faces that left the mask and zeros for the ones that entered, coalescing neighbours.
	#[test]
	fn mask_diff_is_a_sorted_merge() {
		let faces: Vec<PropFace> = (0..10).map(|i| PropFace { slot: (9 - i) * 3, run: u32::MAX, idx: [i * 3 + 100, i * 3 + 101, i * 3 + 102], phase: 0, tex: 0, cull: 0 }).collect();
		let mut base = vec![0u32; 30];
		for f in &faces {
			base[f.slot as usize..f.slot as usize + 3].copy_from_slice(&f.idx);
		}
		let masked = |set: &[u32]| {
			let mut buf = base.clone();
			for f in set {
				let s = faces[*f as usize].slot as usize;
				buf[s..s + 3].copy_from_slice(&[0; 3]);
			}
			buf
		};
		let (old, new) = ([1u32, 2, 3, 7], [2u32, 3, 4, 5, 9]);
		let mut ibuf = masked(&old);
		let writes = diff_writes(&faces, &old, &new);
		for (slot, words) in &writes {
			ibuf[*slot as usize..*slot as usize + words.len()].copy_from_slice(words);
		}
		assert_eq!(ibuf, masked(&new));
		// Changed faces 1, 4, 5, 7, 9 sit in slots 24, 15, 12, 6, 0: 15 and 12 go up together.
		assert_eq!(writes.len(), 4);
		assert!(diff_writes(&faces, &new, &new).is_empty());
		assert_eq!(masked(&[]), base);
	}
}
