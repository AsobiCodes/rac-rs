//! [PC] lane dlit (2026-11-05; user: "i want real dynamic lighting system where shadows are brigtened by light like fire"): shadows
//! FROM the fire / weapon point lights (flit.wgsl). At Global illumination High the `lights(3)` strongest flit lights near the eye
//! get a cube shadow map: six 90-degree perspective faces (a little wider, `SCALE`, so the receivers' PCF at a face edge stays in
//! the face), `TILE` texels each, packed in the last layer of the sun's shadow map (`SunShadow::new`: layer CASCADES, a 4 x 4
//! atlas; no new binding: the level shaders read it through `sun_map`, flitshadow.wgsl). The casters are the sun pass's own lists
//! and pipelines (`SunShadow::lists` / `draw_runs`), each face culled to its frustum out to the light's reach (`face_touches`).
//! A light that gains or loses its map fades its shadow in / out over `FADE_S` (no popping); a tracked light keeps its map against
//! a new one until that one is 1.5x stronger (no flip-flop between two equal fires). Fire light v2 stage 5 (lane dlit3): the
//! receivers read the faces' prefiltered moment cubes (omni_mom.rs; flitshadow.wgsl `flit_shadow`, a 4-moment cone lookup), not
//! the atlas itself.

use super::*;
use super::super::flitlist::Group;

/// Texels per cube face side; the atlas holds `PER_SIDE` x `PER_SIDE` faces. Fire light v2 stage 5 (lane dlit3, FIRE_LIGHT_V2.md
/// 3.3): 256 for every map count (the moment cubes are 128 a face: 2 x 2 depth texels a moment texel; was 512 for 1-2 maps, 408
/// for 3-4, read by the PCSS taps).
pub(in crate::viewer) const TILE: u32 = 256;
const PER_SIDE: u32 = RES / TILE;
/// The largest face RAC_TUNE flsh_tile may ask for (5 x 5 tiles of 408 hold the 4 maps).
const TILE_MAX: u32 = 408;
/// Most shadowed fires (flit.rs `MAPN`, the rows `MAPS` ..).
pub(in crate::viewer) const MAX_LIGHTS: usize = 4;
pub(super) const FACES: usize = 6;
/// The faces' near plane (m). Fire light v2 stage 4 (lane dlit3; stage 3's reference showed the host hull hides a large share of its
/// own flames: the stage-2 exclusion near, 0.8 x the emitter's half extent up to 4 m, made the fire 15-120 % too bright): every map's
/// near plane is this (RAC_TUNE flsh_near), the host casts; flsh_excl=1 brings back stage 2's per-fire near (flitlist.rs
/// `Group::near`). Twin of flitshadow.wgsl `FLS_NEAR` (the shaders' floor).
pub(in crate::viewer) const NEAR: f32 = 0.25;

/// Face projection scale (< 1: a face sees a little more than 90 degrees). Twin of flitshadow.wgsl `FLS_SCALE`.
pub(in crate::viewer) const SCALE: f32 = 0.97;
/// Seconds a shadow takes to fade in or out.
const FADE_S: f32 = 0.35;
/// A light farther from the eye than this casts no shadow map (its light is small on screen).
const NEAR_EYE: f32 = 30.0;
/// A map whose fade is below this is not drawn (its shadow is invisible; tb4 fix S2: no 6 faces for a ~0 shadow).
const MIN_FADE: f32 = 0.02;
/// A track follows the nearest light within max(MATCH_MIN m, MATCH_SHARE x its reach) of where it was (tb4 fix S2: a fixed 2 m lost
/// a fire cluster whose centre jumps when a flame joins or leaves).
const MATCH_MIN: f32 = 2.0;
const MATCH_SHARE: f32 = 0.5;
/// Upper bound of the match radius: half of firefx_light.rs `CLUSTER` (6 m), so a track can never reach a different fire.
const MATCH_MAX: f32 = 3.0;
/// A shadow origin (flitsplit.rs; a whole fire when it is not split): (index in the origins, position, reach, score, near, its fire).
/// Only `near` ones start tracks or win the top slots; the others (lit but past NEAR_EYE / reach + 10 m) only keep an existing track
/// matched while it fades out drawn. A new track takes the highest fade of the tracks of its own fire (`step`: a split never flashes).
type Cand = (usize, [f32; 3], f32, f32, bool, u8);

/// Shadowed fires at Global illumination level `gi` (0 Off, 1 Low, 2 Medium, 3 High): lane dlit measured ~0.025 ms a map; lane dlit2
/// (every big fire near the camera casts): High 4, Medium 1, Low none (Steam Deck budget).
pub const fn lights(gi: u8) -> usize {
	match gi {
		0 | 1 => 0,
		2 => 1,
		_ => MAX_LIGHTS,
	}
}

/// The maps wanted at level `gi`: `lights` or RAC_TUNE flsh_lights, at most `MAX_LIGHTS`.
pub(in crate::viewer) fn wanted(gi: u8) -> usize {
	(crate::devtune::f32("flsh_lights", lights(gi) as f32).max(0.0) as usize).min(MAX_LIGHTS)
}

/// The atlas layout for at most `n` maps: (texels per face, faces per side). Fixed per level (from `lights`, not the maps drawn this
/// frame), so a map fading in never changes the others' resolution.
pub(super) const fn layout(_n: usize) -> (u32, u32) {
	(TILE, PER_SIDE)
}

/// [PC] tb5 review S1: the most maps the atlas of `layout(n)` holds (Medium 2, High 4) = the most live tracks (`step`) and drawn maps
/// (`drawn`); a third map at Medium put faces past the atlas (scissor out of the target: a wgpu validation error).
pub(super) const fn cap(n: usize) -> usize {
	// Stage 5: the 256 faces' atlas holds 10 maps; Medium keeps its 2 (one map plus one fading out), as with the old 512 faces.
	let (_, side) = layout(n);
	let c = (side * side) as usize / FACES;
	let lim = if n <= 2 { 2 } else { MAX_LIGHTS };
	if c < lim { c } else { lim }
}

/// [PC] fire light v2 stage 4: the face layout with RAC_TUNE flsh_tile = `knob` texels a face (0 = `layout`'s; held to 128..408 so
/// the atlas still holds `cap` maps of `layout`).
pub(super) fn tiles(base: (u32, u32), knob: f32) -> (u32, u32) {
	if knob <= 0.0 {
		return base;
	}
	let t = (knob as u32).clamp(128, TILE_MAX);
	(t, RES / t)
}

/// The atlas texel origin of face tile `k` (`layout` (texels, per side)).
pub(super) const fn tile_xy(k: usize, (tile, side): (u32, u32)) -> (u32, u32) {
	((k as u32 % side) * tile, (k as u32 / side) * tile)
}

/// The maps drawn this frame, (track, candidate) in track order: followed and visible (fade >= MIN_FADE), at most `cap(n)`.
fn drawn(track: &[Track; MAX_LIGHTS], follow: &[Option<usize>; MAX_LIGHTS], n: usize) -> Vec<(usize, usize)> {
	track.iter().zip(follow).enumerate().filter_map(|(s, (t, f))| f.filter(|_| t.fade >= MIN_FADE).map(|j| (s, j))).take(cap(n)).collect()
}

/// [PC] fire light v2 stage 5 (lane dlit3): the moment lookup's knobs (flitshadow.wgsl `flit_shadow`). Row `MAPB` (RAC_TUNE flsh_nofs,
/// flsh_bias, flsh_slope, flsh_mb): the receiver moves `NOFS` mip-0 moment texels off its surface, each tap compares `BIAS` m + `SLOPE`
/// x its texel's depth spread on the receiver's plane nearer, the moments are pulled towards (0.5, ..) by `MBIAS` (Peters & Klein's
/// moment bias). Row `MAPQ` (flsh_lod, flsh_rsc, flsh_pmax, flsh_lbr): the taps' mip bias, the scale of the fire's radius (flitlist.rs
/// `Group::size`) as the shadows' emitter (search cone and penumbra), the widest penumbra cone (rad; a small one = hard shadows, the
/// reference runs), the light-bleeding reduction (0 = off). MBIAS 5e-4 (worker 9; was 1e-4): one half-float step of the stored moments
/// at [0.5, 1) is 4.9e-4, so 1e-4 let that quantisation flip the receivers on the sand next to their own depth (salt-and-pepper grain
/// at the crash site on Vulkan and DX12 alike, pp/t_crash120_sn.png: grain 0.065 -> 0.009 at 5e-4, rel vs truth 0.344 -> 0.361).
const NOFS: f32 = 3.0;
const BIAS: f32 = 0.02;
const SLOPE: f32 = 0.25;
pub(super) const MBIAS: f32 = 5e-4;
const LOD_BIAS: f32 = 0.0;
const RSCALE: f32 = 1.0;
const PHI_MAX: f32 = 0.35;
const LBR: f32 = 0.0;
/// Default RAC_TUNE flsh_mean (fire light v2 stage 6, flit.rs `fire_code`): 1 = every light of a fire takes the power-weighted mean
/// of its origins' maps (the stage-3 reference's estimator), 0 = each light its nearest origin's map (blended at the bisector).
const MEAN: f32 = 1.0;

/// Cube face `f` (0 +x, 1 -x, 2 +y, 3 -y, 4 +z, 5 -z) of a light at `p` reaching `far`, near plane `near` (at least `NEAR`): the
/// face looks along axis f / 2, its right / up are the next two axes (flitshadow.wgsl `flit_shadow` picks the same).
pub(super) fn face(p: [f32; 3], far: f32, near: f32, f: usize) -> Cascade {
	let ax = f / 2;
	let unit = |i: usize| {
		let mut v = [0.0f32; 3];
		v[i] = 1.0;
		v
	};
	let mut d = [0.0f32; 3];
	d[ax] = if f % 2 == 0 { 1.0 } else { -1.0 };
	let (right, up) = (unit((ax + 1) % 3), unit((ax + 2) % 3));
	let near = near.max(NEAR);
	let far = far.max(near * 2.0);
	let a = far / (far - near);
	let s = SCALE;
	let (rl, ul, dl) = (dot3(right, p), dot3(up, p), dot3(d, p));
	let col = |i: usize| [s * right[i], s * up[i], a * d[i], d[i]];
	let vp = [col(0), col(1), col(2), [-s * rl, -s * ul, -a * dl - a * near, -dl]];
	Cascade { vp, texel: 0.0, basis: [right, up, d], c: [0.0; 2], r: s, z: [near, far], eye: Some(p) }
}

/// Whether a sphere can cast into face `c` of the light at `p` (`Cascade::touches` of a face): in front of the near plane, within
/// the reach, inside the four side planes.
pub(super) fn face_touches(c: &Cascade, p: [f32; 3], centre: [f32; 3], radius: f32) -> bool {
	let v = [centre[0] - p[0], centre[1] - p[1], centre[2] - p[2]];
	let [right, up, d] = c.basis;
	let (x, y, z) = (dot3(v, right), dot3(v, up), dot3(v, d));
	if z < c.z[0] - radius || !in_reach(p, c.z[1], centre, radius) {
		return false;
	}
	let k = (c.r * c.r + 1.0).sqrt();
	let side = |t: f32| (c.r * t.abs() - z) / k <= radius;
	side(x) && side(y)
}

/// Whether a sphere lies within `far` (+ its radius) of the light at `p`: the one test `face_touches` shares by all six faces (the same
/// float expression, so the pre-cull `Reach` keeps exactly what the faces could keep).
pub(super) fn in_reach(p: [f32; 3], far: f32, centre: [f32; 3], radius: f32) -> bool {
	let v = [centre[0] - p[0], centre[1] - p[1], centre[2] - p[2]];
	!(dot3(v, v) > (far + radius) * (far + radius))
}

/// [PC] tb4 fix S1: the casters within one light's reach (`in_reach`), indices in scene order, culled once per light; its six faces
/// then test only these (`SunShadow::lists` with `pre`), so the face lists are the same draws in the same order as a whole-scene walk.
#[derive(Default)]
pub(super) struct Reach {
	pub tf: Vec<u32>,
	pub tie: Vec<u32>,
	pub shrub: Vec<u32>,
	/// Props batch slots (`props::reach_slots`).
	pub slots: Vec<u32>,
}

impl Reach {
	fn fill(&mut self, scene: &Scene, p: [f32; 3], far: f32, nocull: bool) {
		let keep = |out: &mut Vec<u32>, it: &mut dyn Iterator<Item = ([f32; 3], f32)>| {
			out.clear();
			out.extend(it.enumerate().filter(|(_, (c, r))| in_reach(p, far, *c, *r)).map(|(i, _)| i as u32));
		};
		keep(&mut self.tf, &mut scene.tfrag_set.iter().map(|t| (t.cull.centre, t.cull.radius)));
		keep(&mut self.tie, &mut scene.tie_cull.iter().map(|t| (t.centre, t.radius)));
		keep(&mut self.shrub, &mut scene.shrub_cull.iter().map(|s| (s.centre, s.radius)));
		super::props::reach_slots(scene, (!nocull).then_some((p, far)), &mut self.slots);
	}
}

/// The indices `0..n`, or only those of `sub` (ascending): one walk for the whole scene or a light's `Reach`.
pub(super) enum Ix<'a> {
	All(std::ops::Range<usize>),
	Sub(std::slice::Iter<'a, u32>),
}

impl Iterator for Ix<'_> {
	type Item = usize;
	fn next(&mut self) -> Option<usize> {
		match self {
			Ix::All(r) => r.next(),
			Ix::Sub(s) => s.next().map(|&i| i as usize),
		}
	}
}

pub(super) fn ix(n: usize, sub: Option<&[u32]>) -> Ix<'_> {
	match sub {
		Some(s) => Ix::Sub(s.iter()),
		None => Ix::All(0..n),
	}
}

/// A light holding (or fading) a shadow map: where it is, its fade (0..1), whether it still wants the map.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Track {
	pos: [f32; 3],
	fade: f32,
	want: bool,
}

impl Track {
	fn live(&self) -> bool {
		self.want || self.fade > 0.0
	}
}

/// The fires of the frame's light list (flitlist.rs `groups`; weapon lights are in none, tb4 fix S2: they light surfaces without a
/// map), with the `near` flag (may take a map); score = power, weighed down with the distance from the eye.
fn candidates(groups: &[Group], eye: [f32; 3]) -> Vec<Cand> {
	groups
		.iter()
		.enumerate()
		.map(|(i, g)| {
			let v = [g.pos[0] - eye[0], g.pos[1] - eye[1], g.pos[2] - eye[2]];
			let de = dot3(v, v).sqrt();
			let near = de < NEAR_EYE && de < g.reach + 10.0 && !g.small;
			(i, g.pos, g.reach, g.power / (1.0 + (de / 8.0) * (de / 8.0)), near, g.fire)
		})
		.collect()
}

/// One frame of the tracks: match each live track to the nearest candidate within max(2 m, half its reach), at most MATCH_MAX (it moves with it; none:
/// the light is gone or no longer takes a map, the track fades out where it was, undrawn, and holds its slot until it is out; a light
/// back in range picks it up at its fade), rank (a tracked light's score x 1.5), the top `n` want a map, a new top light takes a free track; fades step by
/// `dt`. At most `cap(n)` tracks live (tb5 review S1): a new top light waits for a free one, and after a drop of the level the extra
/// fading tracks with the lowest fade end at once. Returns per track the candidate it follows.
fn step(track: &mut [Track; MAX_LIGHTS], cands: &[Cand], n: usize, dt: f32) -> [Option<usize>; MAX_LIGHTS] {
	let cap = cap(n);
	let mut follow: [Option<usize>; MAX_LIGHTS] = [None; MAX_LIGHTS];
	for s in 0..MAX_LIGHTS {
		if !track[s].live() {
			continue;
		}
		if n == 0 {
			track[s] = Track::default();
			continue;
		}
		let p = track[s].pos;
		let reach2 = |r: f32| MATCH_MIN.max(MATCH_SHARE * r).min(MATCH_MAX).powi(2);
		let near = cands.iter().enumerate().filter(|(j, _)| !follow.contains(&Some(*j))).map(|(j, c)| (j, dist2(c.1, p), reach2(c.2))).filter(|c| c.1 < c.2).min_by(|a, b| a.1.total_cmp(&b.1));
		if let Some((j, _, _)) = near {
			track[s].pos = cands[j].1;
			follow[s] = Some(j);
		}
	}
	let mut rank: Vec<(usize, f32)> = cands.iter().enumerate().filter(|(_, c)| c.4).map(|(j, c)| (j, if follow.contains(&Some(j)) { c.3 * 1.5 } else { c.3 })).collect();
	rank.sort_by(|a, b| b.1.total_cmp(&a.1));
	let top: Vec<usize> = rank.iter().take(n).map(|r| r.0).collect();
	for (t, f) in track.iter_mut().zip(follow.iter()) {
		t.want = f.is_some_and(|j| top.contains(&j));
	}
	// n <= cap, so the wanted tracks always fit: only fading ones go.
	while track.iter().filter(|t| t.live()).count() > cap {
		let Some(s) = (0..MAX_LIGHTS).filter(|&s| track[s].live() && !track[s].want).min_by(|&a, &b| track[a].fade.total_cmp(&track[b].fade)) else { break };
		track[s] = Track::default();
		follow[s] = None;
	}
	for &j in &top {
		if follow.contains(&Some(j)) {
			continue;
		}
		if track.iter().filter(|t| t.live()).count() >= cap {
			break;
		}
		if let Some(s) = track.iter().position(|t| !t.live()) {
			// Fire light v2 stage 6 (flitsplit.rs; review S2): a new origin of a fire that already holds maps takes their highest fade
			// (its lights read the fire's maps before: no lit flash while it fades in). By parentage, not by distance: at 10-30 fps
			// the split's halves are already metres apart on the frame the new track starts.
			let fade = (0..MAX_LIGHTS).filter(|&o| track[o].live() && follow[o].is_some_and(|f| cands[f].5 == cands[j].5)).map(|o| track[o].fade).fold(0.0f32, f32::max);
			track[s] = Track { pos: cands[j].1, fade, want: true };
			follow[s] = Some(j);
		}
	}
	let k = dt / FADE_S;
	for (t, f) in track.iter_mut().zip(follow.iter_mut()) {
		t.fade = if t.want { (t.fade + k).min(1.0) } else { (t.fade - k).max(0.0) };
		if !t.live() {
			*t = Track::default();
			*f = None;
		}
	}
	follow
}

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
	(a[0] - b[0]) * (a[0] - b[0]) + (a[1] - b[1]) * (a[1] - b[1]) + (a[2] - b[2]) * (a[2] - b[2])
}

pub(in crate::viewer) struct Omni {
	atlas: wgpu::TextureView,
	ubuf: Vec<wgpu::Buffer>,
	ubind: Vec<wgpu::BindGroup>,
	scratch: Scratch,
	indirect: Option<wgpu::Buffer>,
	cap: usize,
	track: [Track; MAX_LIGHTS],
	last: Option<std::time::Instant>,
	/// Per drawn light, its casters (`Reach`), kept between frames.
	reach: Vec<Reach>,
	/// The followed lights of the last frame (centre, reach): `SunShadow::light_spheres`.
	spheres: Vec<([f32; 3], f32)>,
	/// Fire light v2 stage 4: the drawn maps' moment cubes (omni_mom.rs).
	pub(in crate::viewer) mom: super::omni_mom::OmniMom,
}

impl Omni {
	pub(super) fn new(device: &wgpu::Device, ulayout: &wgpu::BindGroupLayout, atlas: wgpu::TextureView) -> Omni {
		let ubuf: Vec<wgpu::Buffer> = (0..MAX_LIGHTS * FACES).map(|_| device.create_buffer(&wgpu::BufferDescriptor { label: Some("flit shadow face"), size: grow::SU_BYTES, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })).collect();
		let ubind = ubuf.iter().map(|b| device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("flit shadow face"), layout: ulayout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: b.as_entire_binding() }] })).collect();
		let mom = super::omni_mom::OmniMom::new(device, &atlas);
		Omni { atlas, ubuf, ubind, scratch: Scratch::default(), indirect: None, cap: 0, track: [Track::default(); MAX_LIGHTS], last: None, reach: Vec::new(), spheres: Vec::new(), mom }
	}
}

impl SunShadow {
	/// [PC] lane dlit: this frame's point-light shadows. Picks the lights (`lights(gi)`, `step`), draws their cube faces into the
	/// atlas and answers which fires of `groups` they are and the receivers' rows (flit.rs `Shadows`). Nothing (no map drawn, all zero) when
	/// `on` is false (post off, warp, Dynamic shadows Off: gpu/draw/passes.rs), at Low / Off, or with the shadow map refused
	/// (pipe_guard.rs).
	#[allow(clippy::too_many_arguments)]
	pub(in crate::viewer) fn light_shadows(&mut self, device: &wgpu::Device, queue: &GatedQueue, enc: &mut wgpu::CommandEncoder, scene: &Scene, groups: &[Group], eye: [f32; 3], gi: u8, on: bool, skin: Option<(&wgpu::BindGroup, Option<&wgpu::Buffer>)>) -> super::super::flit::Shadows {
		let now = std::time::Instant::now();
		let dt = self.omni.last.map_or(0.0, |t| crate::viewer::test_clock::dt(crate::viewer::test_clock::Site::Flit, || now.duration_since(t).as_secs_f32())).min(0.1);
		self.omni.last = Some(now);
		let n = self.fire_maps(gi, on);
		let cands = candidates(groups, eye);
		let follow = step(&mut self.omni.track, &cands, n, dt);
		self.omni.spheres.clear();
		self.omni.spheres.extend(follow.iter().flatten().map(|&j| (cands[j].1, cands[j].2)));
		let mut out = super::super::flit::Shadows::default();
		let mut cs: Vec<Cascade> = Vec::with_capacity(MAX_LIGHTS * FACES);
		let draw = drawn(&self.omni.track, &follow, n);
		let t = crate::devtune::f32;
		// Fire light v2 stage 4 (stage 3's reference: the host hull hides a large share of its own flames): the faces' near plane is
		// NEAR (RAC_TUNE flsh_near, at least NEAR), the host casts; flsh_excl=1 = stage 2's exclusion near (the emitter half extent).
		// Default 1 since test build 7 (user 2026-11-07, test build 6: the burning ship at Veldin's start laid a big dark patch beside
		// its own flames, "why do we have shadows near a literal fucking inferno"): the host never shadows its own fire; flsh_excl=0
		// = stage 4's host-casting near.
		let (excl, near0) = (t("flsh_excl", 1.0) > 0.5, t("flsh_near", NEAR).clamp(NEAR, 4.0));
		for (s, &(k, j)) in draw.iter().enumerate() {
			let (i, mut pos, mut reach, _, _, _) = cands[j];
			out.idx[s] = Some(i);
			out.fade[s] = self.omni.track[k].fade;
			let excl_near = groups[i].near.max(NEAR);
			out.near[s] = if excl { excl_near } else { near0 };
			// DEV RAC_TUNE flsh_ref (flitref.rs): this frame's emitter point, full fade, near NEAR (flsh_ref=2: the fire's own
			// exclusion near kept, the filter's share of the error alone). None without the knob.
			if let Some(p) = groups[i].jit {
				reach += dist2(p, pos).sqrt();
				pos = p;
				out.fade[s] = 1.0;
				out.near[s] = if (t("flsh_ref", 0.0) - 2.0).abs() > 0.5 { NEAR } else { excl_near };
			}
			out.maps[s] = [pos[0], pos[1], pos[2], reach];
			out.size[s] = groups[i].size;
			(out.fire[s], out.share[s]) = (groups[i].fire, groups[i].share);
			cs.extend((0..FACES).map(|f| face(pos, reach, out.near[s], f)));
		}
		if draw.is_empty() {
			return super::super::flit::Shadows::default();
		}
		let (tile, side) = tiles(layout(n), t("flsh_tile", 0.0));
		out.mean = t("flsh_mean", MEAN) > 0.5;
		out.q = [t("flsh_lod", LOD_BIAS).clamp(-4.0, 4.0), t("flsh_rsc", RSCALE).clamp(0.0, 4.0), t("flsh_pmax", PHI_MAX).clamp(1e-3, 1.5), t("flsh_lbr", LBR).clamp(0.0, 0.9)];
		out.bias = [t("flsh_nofs", NOFS).max(0.0), t("flsh_bias", BIAS).max(0.0), t("flsh_slope", SLOPE).max(0.0), t("flsh_mb", MBIAS).clamp(0.0, 0.1)];
		self.encode_faces(device, queue, enc, scene, &cs, (tile, side), skin);
		let mut maps = [(0.0f32, 0.0f32); MAX_LIGHTS];
		cs.chunks(FACES).zip(maps.iter_mut()).for_each(|(c, m)| *m = (c[0].z[0], c[0].z[1]));
		self.omni.mom.encode(queue, enc, &maps[..cs.len() / FACES], (tile, side));
		out
	}

	/// The fire shadow maps this frame may draw at GI level `gi` (`wanted`): none when `on` is false, the shadow map is refused
	/// (pipe_guard.rs) or the moment passes failed to build (review S5: the receivers would read a never-written cube, garbage
	/// shadows; omni_mom.rs `OmniMom::ok`). flitsplit.rs splits the fires out of the same count (gpu/draw/passes.rs).
	pub(in crate::viewer) fn fire_maps(&self, gi: u8, on: bool) -> usize {
		if on && self.omni.mom.ok() && !super::super::pipe_guard::off(super::super::pipe_guard::Effect::SunShadow) { wanted(gi) } else { 0 }
	}

	/// [PC] tb4 N7: the lights that held (or were fading in / out) a shadow map last frame, (centre, reach): the props phase poses the
	/// camera-culled mobys within them (rac1 handler sun_gate.rs `light_takes`) so they cast with their pose. Empty when no map draws.
	pub fn light_spheres(&self) -> &[([f32; 3], f32)] {
		&self.omni.spheres
	}

	/// The cube faces `cs` (tile k = face k of the atlas, `layout` (texels, per side)) from the scene's casters, in one pass over the
	/// atlas layer.
	#[allow(clippy::too_many_arguments)]
	fn encode_faces(&mut self, device: &wgpu::Device, queue: &GatedQueue, enc: &mut wgpu::CommandEncoder, scene: &Scene, cs: &[Cascade], (tile, side): (u32, u32), skin: Option<(&wgpu::BindGroup, Option<&wgpu::Buffer>)>) {
		let _s = super::super::profile::span("flit_shadow");
		let mut sc = std::mem::take(&mut self.omni.scratch);
		let mut pre = std::mem::take(&mut self.omni.reach);
		pre.resize_with(cs.len() / FACES, Reach::default);
		for (r, c) in pre.iter_mut().zip(cs.chunks(FACES)) {
			r.fill(scene, c[0].eye.unwrap_or_default(), c[0].z[1], super::super::gfxq::sun_nocull());
		}
		self.lists(scene, cs, None, None, Some(&pre), &mut sc);
		self.omni.reach = pre;
		for (k, c) in cs.iter().enumerate() {
			queue.write_buffer(&self.omni.ubuf[k], 0, bytemuck::cast_slice(&grow::uniform(c, k, 0.0)));
		}
		if sc.args.len() > self.omni.cap {
			self.omni.cap = sc.args.len().next_power_of_two().max(4096);
			self.omni.indirect = Some(device.create_buffer(&wgpu::BufferDescriptor { label: Some("flit shadow draws"), size: (self.omni.cap * 20) as u64, usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }));
		}
		if let Some(ind) = &self.omni.indirect {
			if !sc.args.is_empty() {
				queue.write_buffer(ind, 0, bytemuck::cast_slice(&sc.args));
			}
			let cuts = (self.cuts(scene), self.moby_cut(scene));
			let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("flit shadow"),
				color_attachments: &[],
				depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &self.omni.atlas, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }), stencil_ops: None }),
				timestamp_writes: crate::viewer::passtime::rw("flit shadow"),
				occlusion_query_set: None,
				multiview_mask: None,
			});
			crate::viewer::passtime::lap(&mut pass, "start");
			// One `Bound` for all faces: the texture array (group 0) is bound and flushed once for the pass (sunshadow.rs `Bound`).
			let mut bound = super::Bound::default();
			for k in 0..cs.len() {
				let (x, y) = tile_xy(k, (tile, side));
				pass.set_viewport(x as f32, y as f32, tile as f32, tile as f32, 0.0, 1.0);
				pass.set_scissor_rect(x, y, tile, tile);
				pass.set_bind_group(super::G_SU, &self.omni.ubind[k], &[]);
				self.draw_runs(&mut pass, &mut bound, scene, skin, ind, &sc.runs, k, cuts);
			}
		}
		self.omni.scratch = sc;
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn clip(c: &Cascade, p: [f32; 3]) -> [f32; 4] {
		let mut o = [0.0f32; 4];
		for (i, col) in c.vp.iter().enumerate() {
			let w = if i < 3 { p[i] } else { 1.0 };
			for r in 0..4 {
				o[r] += col[r] * w;
			}
		}
		o
	}

	#[test]
	fn a_face_projects_as_the_receiver_reads_it() {
		let l = [10.0, -4.0, 2.0];
		for f in 0..FACES {
			let c = face(l, 20.0, 1.6, f);
			let (ax, sg) = (f / 2, if f % 2 == 0 { 1.0 } else { -1.0 });
			let mut v = [0.3, -0.7, 0.5];
			v[ax] = 3.0 * sg;
			let q = clip(&c, [l[0] + v[0], l[1] + v[1], l[2] + v[2]]);
			// flitshadow.wgsl: w = |v[ax]|, ndc = SCALE x (v[ax + 1], v[ax + 2]) / w, depth A (1 - NEAR / w).
			assert!((q[3] - 3.0).abs() < 1e-5);
			assert!((q[0] / q[3] - SCALE * v[(ax + 1) % 3] / 3.0).abs() < 1e-5);
			assert!((q[1] / q[3] - SCALE * v[(ax + 2) % 3] / 3.0).abs() < 1e-5);
			let near = 1.6;
			let a = 20.0 / (20.0 - near);
			assert!((q[2] / q[3] - a * (1.0 - near / 3.0)).abs() < 1e-5);
			assert!(c.touches([l[0] + v[0], l[1] + v[1], l[2] + v[2]], 0.1));
			let mut back = v;
			back[ax] = -v[ax];
			assert!(!c.touches([l[0] + back[0], l[1] + back[1], l[2] + back[2]], 0.1));
			assert!(!c.touches([l[0] + v[0] * 9.0, l[1] + v[1] * 9.0, l[2] + v[2] * 9.0], 0.1));
		}
	}

	#[test]
	fn the_shader_twins_match() {
		let src = include_str!("../flitshadow.wgsl");
		assert!(src.contains(&format!("const FLS_NEAR: f32 = {NEAR};")));
		assert_eq!((TILE, PER_SIDE), (256, 8));
		assert!(MAX_LIGHTS * FACES <= (PER_SIDE * PER_SIDE) as usize && TILE * PER_SIDE <= RES);
		assert!(MAX_LIGHTS * FACES <= ((RES / TILE_MAX) * (RES / TILE_MAX)) as usize, "flsh_tile's largest face holds the maps");
		assert_eq!((layout(lights(2)), layout(lights(3))), ((TILE, PER_SIDE), (TILE, PER_SIDE)));
	}

	#[test]
	fn tracks_fade_in_hold_and_hand_over() {
		let mut t = [Track::default(); MAX_LIGHTS];
		let a = (0, [0.0, 0.0, 0.0], 20.0, 10.0, true, 0);
		let b = (1, [5.0, 0.0, 0.0], 20.0, 12.0, true, 1);
		// One map: the stronger light takes it and fades in.
		let f = step(&mut t, &[a, b], 1, 0.1);
		assert_eq!(f, [Some(1), None, None, None]);
		assert!(t[0].fade > 0.0 && t[0].fade < 1.0);
		// A light just a little stronger does not take it over (x 1.5 for the tracked one).
		let a2 = (0, a.1, 20.0, 15.0, true, 0);
		for _ in 0..10 {
			step(&mut t, &[a2, b], 1, 0.1);
		}
		assert_eq!(t[0].fade, 1.0);
		// A much stronger one does: the old shadow fades out first, the new one takes the free track.
		let a3 = (0, a.1, 20.0, 40.0, true, 0);
		let f = step(&mut t, &[a3, b], 1, 0.1);
		assert_eq!(f, [Some(1), Some(0), None, None]);
		assert!(!t[0].want && t[0].fade < 1.0 && t[1].want);
		// Off (n = 0): everything ends at once (no map is drawn any more).
		step(&mut t, &[a3, b], 0, 0.1);
		assert!(t.iter().all(|t| !t.live()));
	}

	/// tb4 fix S2: a light that moves a few metres a frame (a fire cluster's centre jumping) keeps its track (match within half its
	/// reach); a light that is gone fades its track out (no pop, slot held) instead of ending it at once.
	#[test]
	fn fast_lights_keep_their_track_and_lost_ones_fade_out() {
		let mut t = [Track::default(); MAX_LIGHTS];
		for k in 0..10 {
			let f = step(&mut t, &[(0, [2.5 * k as f32, 0.0, 0.0], 20.0, 10.0, true, 0)], 1, 0.1);
			assert_eq!(f[0], Some(0), "frame {k}: a 2.5 m jump with reach 20 stays matched");
		}
		assert_eq!(t[0].fade, 1.0);
		let f = step(&mut t, &[], 1, 0.1);
		assert_eq!(f, [None; MAX_LIGHTS]);
		assert!(t[0].live() && !t[0].want && t[0].fade < 1.0 && t[0].fade > 0.5, "fading out, not reset");
		// A new light cannot take the fading slot's place twice: one track free.
		let f = step(&mut t, &[(1, [500.0, 0.0, 0.0], 20.0, 10.0, true, 1)], 1, 0.1);
		assert_eq!(f, [None, Some(0), None, None]);
		for _ in 0..5 {
			step(&mut t, &[(1, [500.0, 0.0, 0.0], 20.0, 10.0, true, 1)], 1, 0.1);
		}
		assert!(!t[0].live(), "the lost track ends once faded");
	}

	/// tb4 second review S1: a track never takes a different fire 6 m away; after a one-frame dropout it re-matches its own light.
	#[test]
	fn a_track_does_not_jump_to_a_neighbour_fire() {
		let mut t = [Track::default(); MAX_LIGHTS];
		let a = (0, [0.0, 0.0, 0.0], 30.0, 10.0, true, 0);
		let b = (1, [6.0, 0.0, 0.0], 30.0, 9.0, true, 1);
		for _ in 0..10 {
			step(&mut t, &[a, b], 1, 0.1);
		}
		assert_eq!((t[0].fade, t[0].pos), (1.0, a.1));
		// a drops out for one frame: the track stays at a (fading), the neighbour does not inherit it.
		let f = step(&mut t, &[b], 1, 0.1);
		assert_eq!(t[0].pos, a.1, "stays at its last position");
		assert_ne!(f[0], Some(0));
		assert!(!t[0].want && t[0].fade < 1.0 && t[0].fade > 0.5);
		// a is back: its own track picks it up where it faded to.
		let f = step(&mut t, &[a, b], 1, 0.1);
		assert_eq!(f[0], Some(0));
		assert_eq!(t[0].pos, a.1);
	}

	/// tb4 second review S2: a light still lit but no longer near keeps its track matched and fades out drawn over FADE_S; it starts
	/// no new track and does not win the top slot.
	#[test]
	fn a_lit_but_far_light_fades_out_drawn() {
		let mut t = [Track::default(); MAX_LIGHTS];
		let a = (0, [0.0, 0.0, 0.0], 8.0, 10.0, true, 0);
		for _ in 0..5 {
			step(&mut t, &[a], 1, 0.1);
		}
		assert_eq!(t[0].fade, 1.0);
		let far = (0, a.1, 8.0, 10.0, false, 0);
		let mut drawn = 0;
		for _ in 0..10 {
			let f = step(&mut t, &[far], 1, 0.05);
			if t[0].fade >= MIN_FADE {
				assert_eq!(f[0], Some(0), "still drawn while visible");
				assert!(!t[0].want);
				drawn += 1;
			}
		}
		assert!(drawn >= 5, "fades over several frames, not one");
		assert!(!t[0].live());
		// A far light alone starts no track.
		let f = step(&mut t, &[far], 1, 0.1);
		assert_eq!(f, [None; MAX_LIGHTS]);
	}

	/// tb5 review S1: at every level (and level changes mid-run, devtune flsh_lights 0..4 too) with fires that come, go and jump
	/// past the match radius, at most `cap(n)` tracks live and maps drawn, and every drawn face tile lies inside the atlas.
	#[test]
	fn drawn_maps_always_fit_the_atlas() {
		let mut seed = 0x9e37_79b9_7f4a_7c15u64;
		let mut rnd = move || {
			seed ^= seed << 13;
			seed ^= seed >> 7;
			seed ^= seed << 17;
			(seed >> 40) as f32 / (1u64 << 24) as f32
		};
		for n in 0..=MAX_LIGHTS {
			let (_, side) = layout(n);
			assert!(n <= cap(n) && cap(n) * FACES <= (side * side) as usize, "n {n}");
		}
		assert_eq!((cap(lights(2)), cap(lights(3))), (2, MAX_LIGHTS), "Medium 2 maps of room, High all 4 (the approved look)");
		for run in 0..96usize {
			let mut t = [Track::default(); MAX_LIGHTS];
			let base: Vec<[f32; 3]> = (0..8).map(|_| [rnd() * 60.0, rnd() * 4.0, rnd() * 60.0]).collect();
			let mut most = 0;
			for frame in 0..240usize {
				let n = if run % 3 == 0 { (run / 3 + frame / 40) % (MAX_LIGHTS + 1) } else { lights(((run + frame / 60) % 4) as u8) };
				let jump = if run % 2 == 0 { 8.0 } else { 1.0 };
				let mut cands: Vec<Cand> = Vec::new();
				for (j, b) in base.iter().enumerate() {
					if rnd() < 0.85 {
						let p = [b[0] + jump * (rnd() - 0.5), b[1], b[2] + jump * (rnd() - 0.5)];
						let (reach, score, near) = (4.0 + rnd() * 30.0, rnd() * 100.0, rnd() < 0.9);
						cands.push((j, p, reach, score, near, j as u8));
					}
				}
				let f = step(&mut t, &cands, n, rnd() * 0.05);
				let live = t.iter().filter(|t| t.live()).count();
				assert!(live <= cap(n), "run {run} frame {frame}: {live} live tracks, n {n}");
				let d = drawn(&t, &f, n);
				assert!(d.len() <= cap(n));
				most = most.max(d.len());
				let (tile, side) = layout(n);
				let (t2, s2) = tiles(layout(n), 256.0);
				assert!(s2 * s2 >= (cap(n) * FACES) as u32 && t2 * s2 <= RES, "flsh_tile 256 holds the maps");
				for k in 0..d.len() * FACES {
					let (x, y) = tile_xy(k, (tile, side));
					assert!(x + tile <= RES && y + tile <= RES, "run {run} frame {frame}: face {k} at ({x}, {y}) past the atlas");
				}
			}
			assert!(most >= 1, "run {run}: some map drawn");
		}
	}

	/// tb5 review S1: a fire wider than flitlist.rs GROUP (five flames 3 m apart, each flickering with its own phase) keeps its
	/// track(s): the grouping and the centre follow the flames' steady area, so the centre never jumps past the match radius and no
	/// map restarts (fades out / in).
	/// Review S2: a fire splitting 1 -> 4 origins at 10, 30 and 60 fps: every map of the fire it draws keeps the full fade (the new
	/// origins take their fire's fade, however far apart the halves are on the frame their tracks start): no lit flash.
	#[test]
	fn a_split_inherits_its_fires_fade_at_any_frame_rate() {
		use super::super::super::{flitlist::Light, flitsplit::Split};
		let fl = |x: f32| Light { pos: [x, 0.0, 1.0], reach: 12.0, rgb: [1.0, 0.5, 0.1], rs: 1.0, group: Some(0), fire: true, area: 4.0, flick: 1.0, alt: None, small: false, carried: false };
		for dt in [0.1f32, 1.0 / 30.0, 1.0 / 60.0] {
			let (mut sp, mut t, mut time) = (Split::default(), [Track::default(); MAX_LIGHTS], 0.0f64);
			let (mut grown, mut most) = (false, 0);
			for f in 0..(2.0 / dt) as usize {
				// A small fire (one origin) for 0.6 s, then it flares up 8 m wide (four origins).
				let wide = f as f32 * dt >= 0.6;
				let mut list: Vec<Light> = (0..8).map(|i| fl(if wide { -4.0 + i as f32 * 8.0 / 7.0 } else { -0.3 + i as f32 * 0.6 / 7.0 })).collect();
				let g = super::super::super::flitlist::groups(&mut list);
				time += dt as f64;
				let origins = sp.step(&mut list, &g, [0.0, -5.0, 1.0], 4, time).to_vec();
				let follow = step(&mut t, &candidates(&origins, [0.0, -5.0, 1.0]), 4, dt);
				let d = drawn(&t, &follow, 4);
				most = most.max(d.len());
				grown |= wide;
				if grown {
					for &(k, _) in &d {
						assert!(t[k].fade > 0.999, "dt {dt} frame {f}: a map of the fire drawn at fade {}", t[k].fade);
					}
				}
			}
			assert_eq!(most, 4, "dt {dt}: the fire took four maps");
		}
		// A new track of ANOTHER fire still fades in from 0.
		let mut t = [Track { pos: [0.0; 3], fade: 1.0, want: true }, Track::default(), Track::default(), Track::default()];
		let f = step(&mut t, &[(0, [0.0; 3], 10.0, 5.0, true, 0), (1, [0.2, 0.0, 0.0], 10.0, 4.0, true, 1)], 4, 0.0);
		assert_eq!((f[1], t[1].fade), (Some(1), 0.0));
	}

	#[test]
	fn a_flickering_wide_fire_keeps_one_track() {
		use crate::viewer::flitlist::{groups, Light};
		for n in [1, MAX_LIGHTS] {
			let mut t = [Track::default(); MAX_LIGHTS];
			let mut first: Option<[Track; MAX_LIGHTS]> = None;
			for frame in 0..300 {
				let mut list: Vec<Light> = (0..5)
					.map(|k| {
						let area = 16.0 - k as f32;
						let flick = 1.0 + 0.35 * (frame as f32 * (0.7 + 0.37 * k as f32) + k as f32).sin();
						let i = 3.5 * area * flick;
						Light { pos: [3.0 * k as f32, 0.0, 0.0], reach: 20.0, rgb: [i, 0.5 * i, 0.12 * i], rs: 1.0, group: None, fire: true, area, flick, alt: None, small: false, carried: false }
					})
					.collect();
				let g = groups(&mut list);
				step(&mut t, &candidates(&g, [6.0, 2.0, 10.0]), n, 1.0 / 60.0);
				let live: Vec<usize> = (0..MAX_LIGHTS).filter(|&s| t[s].live()).collect();
				assert!(live.iter().all(|&s| t[s].want), "n {n} frame {frame}: no track lost its fire: {t:?}");
				if n == 1 {
					assert_eq!(live, vec![0], "frame {frame}: one track");
				}
				let f0 = *first.get_or_insert(t);
				for s in 0..MAX_LIGHTS {
					assert_eq!((t[s].live(), t[s].pos), (f0[s].live(), f0[s].pos), "n {n} frame {frame}: track {s} kept its place");
				}
			}
		}
	}

	#[test]
	fn the_reach_test_is_the_faces_own() {
		let l = [1.0, 2.0, 3.0];
		for f in 0..FACES {
			let c = face(l, 10.0, NEAR, f);
			for (centre, r) in [([1.0, 2.0, 14.0], 0.5), ([1.0, 2.0, 13.4], 0.5), ([12.0, 2.0, 3.0], 1.0), ([1.0, -8.5, 3.0], 0.6)] {
				if c.touches(centre, r) {
					assert!(in_reach(l, c.z[1], centre, r), "a face never keeps what the light's reach drops");
				}
			}
		}
		assert!(!in_reach(l, 10.0, [1.0, 2.0, 14.0], 0.5) && in_reach(l, 10.0, [1.0, 2.0, 13.4], 0.5));
		assert_eq!(ix(5, None).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
		assert_eq!(ix(5, Some(&[1, 3])).collect::<Vec<_>>(), vec![1, 3]);
	}

	/// Lane rfire review: a fire of small flames only (flitlist.rs `Group::small`) never takes a shadow map, however strong or near.
	#[test]
	fn small_fires_take_no_map() {
		let g = [Group { pos: [1.0, 0.0, 0.0], reach: 10.0, power: 9.0, small: true, ..Default::default() }, Group { pos: [2.0, 0.0, 0.0], reach: 10.0, power: 1.0, ..Default::default() }];
		let c = candidates(&g, [0.0; 3]);
		assert!(!c[0].4 && c[1].4, "{c:?}");
	}
}
