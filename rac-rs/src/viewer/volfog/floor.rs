//! The fog floor (user 2026-10-25: Eudora's chasms and Veldin's canyons "float" like Kerwan's buildings did): below the lowest
//! ground a level has, the air is solid fog in the level's retail fog colour, so cliff and building bottoms fade fully into it and a
//! view down into a bottomless chasm (or onto the flat fog-coloured plane retail lays at a canyon bottom) reads as fog. Classic
//! needs none: retail's full-strength GS fog does that job; with volumetric fog on, the GS fog keeps only `fogopt::gs_scale()` of
//! its share, which is what let the void and the cliff bottoms show through.
//!
//! The floor is a height fog of its own, solved in closed form per picture pixel in `fs_apply` (no march, no history: nothing to
//! pop or smear): extinction `sigma` below the floor top `z`, fading out with a smoothstep over `height` above it (zero above
//! `z + height`). It is fixed in the world (user 2026-10-25: "fog shouldnt lower based on your camera position"): set once per
//! level from the level's own collision (`set_level`): a map of the clear top per 8-unit cell, 1 unit under the lowest ground
//! within `REACH` cells (empty cells take their nearest ground's height), never under the level's lowest ground's clear top
//! (`from_level`). Walkable ground keeps clear air, and what hangs below it (cliff skirts, building bases, the void between
//! islands) fades into the fog right under the nearest ground.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::RwLock;

/// WGSL: the floor's optical depth along a segment (`floor_tau`) and where a downward ray meets its top (`floor_hit`), placed after
/// `scatter::BLOCK`. Uniform slots (unused by the fog otherwise): `fog_col.w` = floor top z, `amb_col.w` = height of its soft top,
/// `phase.w` = extinction below the top (0: no floor), `cam[1].w`, `cam[2].w` = the ground map's origin (x, y), `cam[3].w` = the
/// lowest the clear top goes (the level's lowest ground's clear top). Binding 10 = the clear-top map (`ground_texture`; a 1x1 dummy
/// = a flat floor at `fog_col.w`).
pub const FLOOR: &str = r#"
@group(0) @binding(10) var gmap: texture_2d<f32>;

// The antiderivative over z of the floor's density profile: 1 below `top`, 1 - smoothstep(0, hh, h) on the soft top, 0 above.
fn floor_g(z: f32, top: f32, hh: f32) -> f32 {
	let h = z - top;
	if (h <= 0.0) { return h; }
	let x = min(h / hh, 1.0);
	return hh * (x - x * x * x + 0.5 * x * x * x * x);
}

// Optical depth of the fog floor on the straight segment a..b, its solid top at `t.x` under a soft top `t.y` thick.
fn floor_tau(a: vec3<f32>, b: vec3<f32>, t: vec2<f32>) -> f32 {
	if (f.phase.w <= 0.0) { return 0.0; }
	let dz = a.z - b.z;
	var mean = 1.0 - smoothstep(0.0, t.y, 0.5 * (a.z + b.z) - t.x);
	if (abs(dz) > 0.01) { mean = (floor_g(a.z, t.x, t.y) - floor_g(b.z, t.x, t.y)) / dz; }
	return f.phase.w * distance(a, b) * mean;
}

// The floor over the ground point `xy` (the ground map, bilinear): x = clear top, y = solid top; outside the map or without one
// the level's lowest clear top with the solid top the level's soft height under it.
fn floor_at(xy: vec2<f32>) -> vec2<f32> {
	let gd = vec2<i32>(textureDimensions(gmap));
	let out = vec2<f32>(f.cam[3].w, f.cam[3].w - f.amb_col.w);
	if (gd.x < 2) { return out; }
	let g = (xy - vec2<f32>(f.cam[1].w, f.cam[2].w)) * (1.0 / 8.0) - vec2<f32>(0.5);
	let i0 = vec2<i32>(floor(g));
	let fr = g - floor(g);
	let hi = gd - vec2<i32>(1);
	let a = textureLoad(gmap, clamp(i0, vec2<i32>(0), hi), 0).rg;
	let b = textureLoad(gmap, clamp(i0 + vec2<i32>(1, 0), vec2<i32>(0), hi), 0).rg;
	let c = textureLoad(gmap, clamp(i0 + vec2<i32>(0, 1), vec2<i32>(0), hi), 0).rg;
	let d = textureLoad(gmap, clamp(i0 + vec2<i32>(1, 1), vec2<i32>(0), hi), 0).rg;
	let v = mix(mix(a, b, fr.x), mix(c, d, fr.x), fr.y);
	let o = max(max(-g.x, g.x - f32(hi.x)), max(-g.y, g.y - f32(hi.y)));
	return mix(v, out, clamp(o * 0.25, 0.0, 1.0));
}

fn floor_clear_at(xy: vec2<f32>) -> f32 { return floor_at(xy).x; }

// The floor for a surface point `p`: its solid top (x) and the height of the soft top over it (y), from the map over it. It
// belongs to the world (the map is made at the level load from the collision and the render geometry), never to the camera.
fn floor_top(p: vec3<f32>) -> vec2<f32> {
	if (textureDimensions(gmap).x < 2) { return vec2<f32>(f.fog_col.w, f.amb_col.w); }
	let c = floor_at(p.xy);
	return vec2<f32>(c.y, max(c.x - c.y, 0.5));
}

// Where a ray `d` (forward part 1) from the camera first dips under the floor's clear top (x: view depth, huge when never) and the
// floor there (yz: solid top, soft height). A void pixel (no surface: the sky ray of a view down a chasm) takes its floor from it.
fn floor_ray(d: vec3<f32>) -> vec3<f32> {
	let o = f.cam[0].xyz;
	let none = vec3<f32>(1.0e30, f.fog_col.w, f.amb_col.w);
	if (f.phase.w <= 0.0 || d.z >= -1.0e-4) { return none; }
	var lo = 0.0;
	var hi = 4.0;
	var found = false;
	for (var i = 0; i < 34; i = i + 1) {
		let p = o + d * hi;
		if (p.z < floor_clear_at(p.xy)) { found = true; break; }
		lo = hi;
		hi = hi * 1.22;
	}
	if (!found) { return none; }
	for (var i = 0; i < 6; i = i + 1) {
		let m = 0.5 * (lo + hi);
		let p = o + d * m;
		if (p.z < floor_clear_at(p.xy)) { hi = m; } else { lo = m; }
	}
	return vec3<f32>(hi, floor_top(o + d * hi));
}

// 1 when a ray `d` from the camera that meets the floor's clear top at view depth `t` (huge: never) meets it only past the ground
// map's edge or never: a void pixel there looks past the end of the level's mesh (the open sea or plain retail draws up to its far
// fog), not down a chasm inside the level. 0 without a floor.
fn floor_past_edge(d: vec3<f32>, t: f32) -> f32 {
	if (f.phase.w <= 0.0) { return 0.0; }
	let gd = vec2<i32>(textureDimensions(gmap));
	if (gd.x < 2) { return select(0.0, 1.0, t > 1.0e29); }
	let p = f.cam[0].xyz + d * min(t, 1.0e6);
	let g = (p.xy - vec2<f32>(f.cam[1].w, f.cam[2].w)) * (1.0 / 8.0) - vec2<f32>(0.5);
	let hi = vec2<f32>(gd - vec2<i32>(1));
	return select(0.0, 1.0, any(g < vec2<f32>(0.0)) || any(g > hi));
}

// Where a downward ray meets the floor (x: view depth, huge when never) and `floor_past_edge` (y).
fn floor_hit(d: vec3<f32>) -> vec2<f32> {
	let t = floor_ray(d).x;
	return vec2<f32>(t, floor_past_edge(d, t));
}

// The floor's colour (`LevelFloor::colour`): the level's retail fog colour pulled toward grey or toward what retail shows under
// the level.
fn floor_col() -> vec3<f32> {
	return f.floor_c.rgb;
}

// How far the haze at `p` (view distance `t`) has turned into the floor: its light goes from the lit in-scatter (sun shafts) to
// `floor_col` over the `DEEP_H` units over the clear top there and from `DEEP_NEAR` to `DEEP_FAR` units away (user 2026-10-25,
// "u can literally see the bottom", "look how abruptly our buildings end vs how theyre blended in in the classic": sun-lit haze
// ending at the floor top while its neighbours got shadowed haze or the floor colour drew pale flat-bottomed shapes from a
// distance; one colour deep down and far out lets geometry fade into the fog like retail's distance fog). 0 without a floor.
const DEEP_H: f32 = 24.0;
const DEEP_NEAR: f32 = 80.0;
const DEEP_FAR: f32 = 220.0;
fn deep_k(p: vec3<f32>, t: f32) -> f32 {
	if (f.phase.w <= 0.0) { return 0.0; }
	let top = floor_clear_at(p.xy);
	return max(smoothstep(top + DEEP_H, top, p.z), smoothstep(DEEP_NEAR, DEEP_FAR, t));
}
"#;

/// The fog floor of a level (world units; `sigma` per unit below the top `z`, a soft top `height` units thick).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Floor {
	pub z: f32,
	pub height: f32,
	pub sigma: f32,
}

/// Extinction under the top: a retail canyon plane 1.5 units under it, seen at 45 degrees, is about 98% hidden.
const SIGMA: f32 = 1.0;

/// The floor geometry of the level now loaded (top z, soft height as f32 bits; NaN = none), set at level load (`set`).
static TOP: AtomicU32 = AtomicU32::new(0x7fc0_0000);
static SOFT: AtomicU32 = AtomicU32::new(0x7fc0_0000);
/// The level these belong to: the load job sets them on the prefetch thread while the old level still draws (travel), and that
/// level keeps no floor until its own load sets them again.
static LEVEL: AtomicU32 = AtomicU32::new(u32::MAX);

fn low(mut v: Vec<f32>, per_mille: usize) -> Option<f32> {
	v.retain(|z| z.is_finite());
	v.sort_by(f32::total_cmp);
	v.get(v.len() * per_mille / 1000).copied()
}

/// The floor (top z, soft height) of a level from its terrain vertex heights and collision vertex heights. The lowest visible
/// terrain is the 1st percentile (a stray deep fragment does not drag it down; a canyon-bottom plane lies at or under it); the
/// ground is the lowest 0.1% of the collision at or above that height (a kill plane under all the visible terrain does not
/// count). None without terrain or collision.
pub fn from_level(terrain_z: impl Iterator<Item = f32>, collision_z: impl Iterator<Item = f32>) -> Option<(f32, f32)> {
	let lo = low(terrain_z.collect(), 10)?;
	let ground = low(collision_z.filter(|&z| z >= lo).collect(), 1)?;
	let clear = ground - 1.0;
	let top = (lo + 1.5).min(clear - 2.0);
	Some((top, clear - top))
}

/// The ground map of the level now loaded: per `CELL`-unit square the floor's clear top over it (`clear_map`), set at level load
/// (`set_level`). It belongs to the world: the floor never follows the camera (user 2026-10-25). `GEN` counts the maps (the fog
/// uploads its texture again when it changes).
struct Ground {
	x0: f32,
	y0: f32,
	w: usize,
	/// Clear top per cell (no floor fog over it).
	z: Vec<f32>,
	/// Solid top per cell (`solid_map`; empty until set: then the clear top less the level's soft height).
	s: Vec<f32>,
	/// Whether a ground triangle covers the cell (`clear_map`; the others are void, `lower_voids`).
	own: Vec<bool>,
	/// The lowest the clear top goes (the level's lowest clear top).
	min: f32,
}

static GROUND: RwLock<Option<Ground>> = RwLock::new(None);
static GEN: AtomicU32 = AtomicU32::new(0);

/// Ground map cell size (WGSL `floor_clear_at` has it as 1 / 8); a cell without ground holds `NONE`.
const CELL: f32 = 8.0;
const NONE: f32 = 1.0e9;
/// The reach of a cell's ground, in cells: the clear top over a cell is under the lowest ground within this many cells (a cliff
/// face is clear down to the ground at its foot when that is near, and fades into the fog under the rim when it is not).
const REACH: usize = 2;
/// How far under the lowest ground near it the clear top lies, in world units.
const MARGIN: f32 = 1.0;

/// Whether a collision triangle is ground: tilted under ~53 degrees (floors, ramps; also a ledge's underside: a lower floor is the
/// safe side), all corners finite.
fn flat(v: &[[f32; 3]; 3]) -> bool {
	let (a, b) = ([v[1][0] - v[0][0], v[1][1] - v[0][1], v[1][2] - v[0][2]], [v[2][0] - v[0][0], v[2][1] - v[0][1], v[2][2] - v[0][2]]);
	let n = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
	let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
	l > 1e-6 && n[2].abs() >= 0.6 * l && v.iter().flatten().all(|c| c.is_finite())
}

/// The lowest value within `r` cells (a square window) of every cell.
fn min_filter(z: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
	let mut rows = z.to_vec();
	for y in 0..h {
		for x in 0..w {
			rows[y * w + x] = z[y * w + x.saturating_sub(r)..=y * w + (x + r).min(w - 1)].iter().copied().fold(NONE, f32::min);
		}
	}
	let mut out = rows.clone();
	for y in 0..h {
		for x in 0..w {
			out[y * w + x] = (y.saturating_sub(r)..=(y + r).min(h - 1)).map(|ny| rows[ny * w + x]).fold(NONE, f32::min);
		}
	}
	out
}

/// Cells without ground take the lowest of their filled neighbours, pass by pass (the nearest ground's height spreads out over a
/// void), until none is left or `passes` ran out.
fn fill(z: &mut [f32], w: usize, h: usize, passes: usize) {
	for _ in 0..passes {
		let old = z.to_vec();
		let mut changed = false;
		for y in 0..h {
			for x in 0..w {
				if old[y * w + x] < NONE {
					continue;
				}
				let ring = (y.saturating_sub(1)..=(y + 1).min(h - 1)).flat_map(|ny| (x.saturating_sub(1)..=(x + 1).min(w - 1)).map(move |nx| (nx, ny)));
				let m = ring.map(|(nx, ny)| old[ny * w + nx]).fold(NONE, f32::min);
				if m < NONE {
					z[y * w + x] = m;
					changed = true;
				}
			}
		}
		if !changed {
			break;
		}
	}
}

/// The clear-top map of a level's collision triangles: every ground triangle marks its lowest corner in the cells its bounds cover,
/// each cell takes the lowest ground within `reach` cells, empty cells take their nearest ground's, a 3x3 mean smooths the map
/// (it cannot rise over the ground: every neighbour's window holds the cell's ground too), and `MARGIN` goes under it, never under
/// `floor_min` (the level's lowest clear top).
fn clear_map(faces: impl Iterator<Item = [[f32; 3]; 3]> + Clone, reach: usize, floor_min: f32) -> Option<Ground> {
	let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
	for v in faces.clone().filter(flat) {
		for c in v {
			(lo[0], lo[1], hi[0], hi[1]) = (lo[0].min(c[0]), lo[1].min(c[1]), hi[0].max(c[0]), hi[1].max(c[1]));
		}
	}
	if lo[0] > hi[0] || (hi[0] - lo[0]) / CELL > 4000.0 || (hi[1] - lo[1]) / CELL > 4000.0 {
		return None;
	}
	let (w, h) = (((hi[0] - lo[0]) / CELL) as usize + 1, ((hi[1] - lo[1]) / CELL) as usize + 1);
	let mut z = vec![NONE; w * h];
	let cell = |c: f32, o: f32| ((c - o) / CELL) as usize;
	for v in faces.filter(flat) {
		let low = v[0][2].min(v[1][2]).min(v[2][2]);
		let (x0, x1) = (cell(v[0][0].min(v[1][0]).min(v[2][0]), lo[0]), cell(v[0][0].max(v[1][0]).max(v[2][0]), lo[0]).min(w - 1));
		let (y0, y1) = (cell(v[0][1].min(v[1][1]).min(v[2][1]), lo[1]), cell(v[0][1].max(v[1][1]).max(v[2][1]), lo[1]).min(h - 1));
		for y in y0..=y1 {
			for x in x0..=x1 {
				z[y * w + x] = z[y * w + x].min(low);
			}
		}
	}
	let own: Vec<bool> = z.iter().map(|&v| v < NONE).collect();
	let mut z = min_filter(&z, w, h, reach.max(1));
	fill(&mut z, w, h, if w * h > 2_000_000 { 8 } else { 96 });
	z.iter_mut().filter(|v| **v >= NONE).for_each(|v| *v = floor_min + MARGIN);
	let smooth = (0..w * h).map(|i| {
		let (x, y) = (i % w, i / w);
		let (mut sum, mut n) = (0.0, 0.0);
		for ny in y.saturating_sub(1)..=(y + 1).min(h - 1) {
			for nx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
				(sum, n) = (sum + z[ny * w + nx], n + 1.0);
			}
		}
		(sum / n - MARGIN).max(floor_min)
	});
	Some(Ground { x0: lo[0], y0: lo[1], w, z: smooth.collect(), s: Vec::new(), own, min: floor_min })
}

/// How far over the bottom of the render geometry in a cell the floor is solid, in world units (sigma 1: the bottom edge seen at 45
/// degrees is ~98% hidden).
const BOTTOM_MARGIN: f32 = 3.0;
/// The least soft top over a solid top raised to a render bottom (the solid fog stays this far under the clear top).
const GAP: f32 = 2.0;

/// The solid top per cell (user 2026-10-25, "some parts still expose the textures": cliff and tower bottoms ended in sharp
/// silhouettes over the void, since the visible geometry ends at other heights than the collision): the clear top less the soft
/// height `soft`, raised to `BOTTOM_MARGIN` over the lowest render-geometry vertex under the clear top in that cell or a neighbour
/// (the render geometry's bottom then lies in solid fog, whatever the collision says), never closer than `GAP` under the clear top
/// (walkable ground stays clear: the clear top is under the lowest ground near the cell). `flat`: every cell's clear top is that
/// height (Kerwan: a level-wide deck; the per-cell ground hid the towers under every platform). `void_fade` > 0: `lower_voids`.
fn solid_map(g: &mut Ground, render: impl Iterator<Item = [f32; 3]>, soft: f32, flat: Option<f32>, void_fade: f32) {
	if let Some(c) = flat {
		g.z.iter_mut().for_each(|z| *z = c);
	}
	let (w, n) = (g.w, g.z.len());
	let h = n / w;
	let mut low = vec![NONE; n];
	for p in render.filter(|p| p.iter().all(|c| c.is_finite())) {
		let (x, y) = (((p[0] - g.x0) / CELL).floor(), ((p[1] - g.y0) / CELL).floor());
		if x >= 0.0 && y >= 0.0 && (x as usize) < w && (y as usize) < h {
			let i = y as usize * w + x as usize;
			if p[2] < g.z[i] {
				low[i] = low[i].min(p[2]);
			}
		}
	}
	if void_fade > 0.0 && flat.is_none() {
		lower_voids(g, &low, void_fade);
	}
	// Raised solid top per cell, spread one cell (a bottom edge between two cells, the bilinear map between a cell and its neighbour).
	let raise: Vec<f32> = low.iter().map(|&l| if l < NONE { -(l + BOTTOM_MARGIN) } else { NONE }).collect();
	let raise = min_filter(&raise, w, h, 1);
	g.s = (0..n).map(|i| (-raise[i]).max(g.z[i] - soft).min(g.z[i] - GAP.min(soft))).collect();
}

/// The clear top moved down to `fade` units over `BOTTOM_MARGIN` above the lowest render geometry in or next to the cell, where that
/// is lower (user 2026-10-25, Eudora "gets too foggy in some places where it shouldnt": the gap beside an island took the island's
/// own height, so the fog stood as an opaque wall 1 unit under every rim and hid whole cliff faces that Classic and retail show; then
/// "still too high ... only from ground camera angle": a cliff face under a rim lies in the island's own cells, whose clear top was
/// still under the rim, and a level view crosses ~100 units of that soft top). The cliffs now show down to near their bottoms and
/// fade into the fog over their last `fade` units; walkable ground stays clear (it stands over its own cliff's bottom); void cells
/// with no render geometry near take their neighbours' lowest; never under `Ground::min`.
fn lower_voids(g: &mut Ground, low: &[f32], fade: f32) {
	let (w, n) = (g.w, g.z.len());
	let h = n / w;
	let near = min_filter(low, w, h, 1);
	let mut v: Vec<f32> = (0..n).map(|i| if near[i] < NONE { near[i] + BOTTOM_MARGIN + fade } else if g.own[i] { g.z[i] } else { NONE }).collect();
	fill(&mut v, w, h, if n > 2_000_000 { 8 } else { 96 });
	for i in (0..n).filter(|&i| v[i] < NONE) {
		g.z[i] = g.z[i].min(v[i]).max(g.min);
	}
}

/// The ground map as a texture (R32Float, one texel per cell) with its origin (None: no map), when its generation is not `have`.
pub fn ground_texture(device: &wgpu::Device, queue: &wgpu::Queue, have: u32) -> Option<(u32, Option<(wgpu::TextureView, [f32; 2])>)> {
	let now = GEN.load(Ordering::Relaxed);
	if now == have {
		return None;
	}
	let g = GROUND.read().unwrap_or_else(|e| e.into_inner());
	let tex = g.as_ref().map(|g| {
		let size = wgpu::Extent3d { width: g.w as u32, height: (g.z.len() / g.w) as u32, depth_or_array_layers: 1 };
		let texels: Vec<f32> = g.z.iter().zip(&g.s).flat_map(|(&z, &s)| [z, s]).collect();
		let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
		let t = device.create_texture(&wgpu::TextureDescriptor { label: Some("fog floor ground"), size, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rg32Float, usage, view_formats: &[] });
		let at = wgpu::TexelCopyTextureInfo { texture: &t, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All };
		queue.write_texture(at, bytemuck::cast_slice(&texels), wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(8 * g.w as u32), rows_per_image: None }, size);
		(t.create_view(&Default::default()), [g.x0, g.y0])
	});
	Some((now, tex))
}

/// Level load: the floor of the new level from its terrain vertices (`tfrag`), its other render geometry (`ties`) and collision
/// triangles (`from_level`), its clear-top map (`clear_map`) and solid-top map (`solid_map`).
pub fn set_level(level: usize, tfrag: impl Iterator<Item = [f32; 3]> + Clone, ties: impl Iterator<Item = [f32; 3]>, faces: impl Iterator<Item = [[f32; 3]; 3]> + Clone) {
	let at = from_level(tfrag.clone().map(|p| p[2]), faces.clone().flatten().map(|v| v[2]));
	let floor_min = at.map_or(f32::MIN, |(t, s)| t + s);
	let reach = crate::devtune::f32("fog_floor_reach", tune(level).reach as f32).max(1.0) as usize;
	let mut map = clear_map(faces, reach, floor_min);
	if let (Some(g), Some(lf)) = (map.as_mut(), floor(level, at)) {
		let flat = (tune(level).flat && crate::devtune::f32("fog_floor_map", 0.0) < 0.5).then_some(lf.clear);
		solid_map(g, tfrag.chain(ties), lf.soft, flat, crate::devtune::f32("fog_floor_void", tune(level).void_fade));
	}
	if let Some(g) = map.as_mut().filter(|g| g.s.len() != g.z.len()) {
		g.s = g.z.iter().map(|z| z - 1.0).collect();
	}
	if super::frame::fog_log() {
		let soft = floor(level, at).map_or(0.0, |l| l.soft);
		let mut z: Vec<f32> = map.iter().flat_map(|g| g.z.iter().copied()).collect();
		z.sort_by(f32::total_cmp);
		let at = |p: usize| z.get(z.len().saturating_sub(1) * p / 100).copied().unwrap_or(f32::NAN);
		let raised = map.iter().flat_map(|g| g.z.iter().zip(&g.s)).filter(|(z, s)| **z - **s < soft - 0.01).count();
		eprintln!("volfog: solid top raised to the render bottom in {raised} cells");
		eprintln!("volfog: ground map level {level} cells {} clear top percentiles 0/5/10/25/50/75/100: {:.1} {:.1} {:.1} {:.1} {:.1} {:.1} {:.1}", z.len(), at(0), at(5), at(10), at(25), at(50), at(75), at(100));
	}
	*GROUND.write().unwrap_or_else(|e| e.into_inner()) = map;
	GEN.fetch_add(1, Ordering::Relaxed);
	set(level, at);
}

/// Level load: the floor of the new level `level` from its geometry (None: no floor, and no ground map).
pub fn set(level: usize, floor: Option<(f32, f32)>) {
	if floor.is_none() {
		*GROUND.write().unwrap_or_else(|e| e.into_inner()) = None;
		GEN.fetch_add(1, Ordering::Relaxed);
	}
	let (t, s) = floor.unwrap_or((f32::NAN, f32::NAN));
	TOP.store(t.to_bits(), Ordering::Relaxed);
	SOFT.store(s.to_bits(), Ordering::Relaxed);
	LEVEL.store(level as u32, Ordering::Release);
}

/// The least height of the soft top, in world units (the floor fades in over it; RAC_TUNE fog_floor_soft).
const SOFT_MIN: f32 = 12.0;

/// One level's floor shape (2026-10-25, user: "Eudora needs higher fog", "Kerwan's buildings float"): `lift` raises the clear top,
/// `soft` is the least soft height, `cap` the most (a short soft top = a crisp fog deck), `reach` the ground map's reach in cells.
struct Tune {
	lift: f32,
	soft: f32,
	cap: f32,
	reach: usize,
	/// How far the floor's colour is pulled from the retail fog colour toward grey (0..1).
	desat: f32,
	/// How far it is then pulled toward `under` (0..1): what retail shows under the level (its sky below the horizon, gamma-space
	/// GS bytes), which its alpha-faded bottoms fade into.
	to_under: f32,
	under: [u8; 3],
	/// One clear top for the whole level (the level's lowest clear top), not the per-cell ground.
	flat: bool,
	/// Extinction scale (x `SIGMA`): a thin floor under a tall soft top fades what goes down into it over many units.
	sigma: f32,
	/// > 0: the clear top over the void lies this far over the render geometry's bottom there, not at the nearest ground (`lower_voids`).
	void_fade: f32,
}

fn tune(level: usize) -> Tune {
	match level {
		// Kerwan: a flat deck with its solid top at the retail fog sheet (fs_ocean_k, z -24; the lowest clear top is 4.6), grey like the
		// sheet's FOGCOL 96 (user 2026-10-25: retail's sheet hides every tower bottom, our deck at 2.6 covered whole buildings); a tower
		// ending over the sheet gets the solid top raised to its bottom (`solid_map`). Thin (sigma 0.12) under a 36-unit soft top
		// (clear top +12): the towers fade into the grey over ~30 units like retail's (user 2026-10-25, "look how abruptly our
		// buildings end"; a 6-unit soft top at sigma 1 cut them off).
		3 => Tune { lift: 7.4, soft: 36.0, cap: 36.0, reach: REACH, desat: 1.0, to_under: 0.0, under: [0; 3], flat: true, sigma: 0.12, void_fade: 0.0 },
		// Eudora: the fog hugs the nearest ground (reach 1 cell) and fades over 30 units, so each cliff tier fades under its own rim; over
	// the gaps between islands and under the rims it starts 12 units over the cliffs' bottoms (`lower_voids`), and it is thin (sigma
	// 0.06: user 2026-10-25 "too foggy in some places where it shouldnt", at sigma 1 an opaque wall stood a few units under every rim
	// and hid the cliff tiers Classic shows; at 0.15 the far cliffs seen across a gap from the paths still looked blue-washed).
		4 => Tune { lift: 0.0, soft: 30.0, cap: 1.0e9, reach: 1, desat: 0.5, to_under: 0.0, under: [0; 3], flat: false, sigma: 0.06, void_fade: 12.0 },
		// Quartu: the pit's green was a flat saturated plane the rock edges ended against.
		15 => Tune { lift: 0.0, soft: SOFT_MIN, cap: 1.0e9, reach: REACH, desat: 0.5, to_under: 0.0, under: [0; 3], flat: false, sigma: 1.0, void_fade: 0.0 },
		// Veldin 2: retail's canyon fades to the near-black clear colour (10,10,10), not to the dark green fog colour; the cliffs fade
		// into it over a thin floor under a 30-unit soft top raised 10 units (user 2026-10-25, the canyon cliffs ended against a flat
		// dark green with pale lit haze shapes).
		18 => Tune { lift: 10.0, soft: 30.0, cap: 1.0e9, reach: REACH, desat: 1.0, to_under: 0.0, under: [0; 3], flat: false, sigma: 0.15, void_fade: 0.0 },
		// Veldin: retail's alpha-faded cliff and rock bottoms fade into its red-brown sky below the horizon (77,40,25), not into the
		// darker fog colour (47,26,15) the floor painted (user 2026-10-25: Classic fades the bottoms into the red-brown, Enhanced into
		// near black).
		0 => Tune { lift: 0.0, soft: SOFT_MIN, cap: 1.0e9, reach: REACH, desat: 0.0, to_under: 1.0, under: [77, 40, 25], flat: false, sigma: 1.0, void_fade: 0.0 },
		_ => Tune { lift: 0.0, soft: SOFT_MIN, cap: 1.0e9, reach: REACH, desat: 0.0, to_under: 0.0, under: [0; 3], flat: false, sigma: 1.0, void_fade: 0.0 },
	}
}

/// A level's fog floor, fixed in the world: the lowest clear top (the ground map holds the clear top per cell, over this one).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LevelFloor {
	/// Clear top from the geometry (`from_level`: 1 unit under the lowest ground), soft height under it.
	pub clear: f32,
	pub soft: f32,
	pub sigma: f32,
	pub desat: f32,
	/// How far the colour goes on to `under` (0..1 display space): what retail shows under the level.
	pub to_under: f32,
	pub under: [f32; 3],
}

impl LevelFloor {
	/// The floor: its solid top `soft` under the clear top.
	pub fn at(&self) -> Floor {
		Floor { z: self.clear - self.soft, height: self.soft, sigma: self.sigma }
	}

	/// The floor's colour from the retail fog colour `fog`: pulled toward grey by `desat` (the haze it meets
	/// is lit by a sky and a sun that whiten it; a full-strength retail colour made a flat saturated plane the cliffs ended against),
	/// then toward `under` by `to_under` (what retail's alpha-faded bottoms fade into).
	pub fn colour(&self, fog: [f32; 3]) -> [f32; 3] {
		let grey = (fog[0] * 0.3 + fog[1] * 0.59 + fog[2] * 0.11) * 1.1;
		let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;
		std::array::from_fn(|k| mix(mix(fog[k], grey, self.desat), self.under[k], self.to_under))
	}
}

/// Whether `level`'s bottom is a liquid (no floor there: nothing hangs over a void). Pokitaru (11): its retail sea writes no depth
/// and lies partly outside the water pass's position target, so its pixels read as sky to the fog: a floor 1 unit under the beach
/// stopped the deep downward march (scatter.rs `end_depth`) at the sea and laid solid fog over the seabed, and the open sea turned
/// bluer than dev s117's (the march stop made most of the change). Orxon (10): its toxic sea is terrain (`terrain_liquid`, z 27-62)
/// under the floor's clear top 29, which flattened it to the fog colour. Oltanis (14) has the floor again (agent 5, 2026-10-25:
/// fullgame1 14_0692 game and raised views vs dev s117: tower feet fade a little more, the ground and sea round them are unchanged).
fn liquid_bottom(level: usize) -> bool {
	matches!(level, 10 | 11)
}

/// The floor of `level` from its geometry `at` (top, soft height), or None: space levels, the levels without air haze
/// (`density::air`) and a liquid bottom (`liquid_bottom`) get none. RAC_TUNE fog_floor (extinction scale, 0 = off), fog_floor_lift
/// (world units) and fog_floor_soft (least soft height) tune it.
pub fn floor(level: usize, at: Option<(f32, f32)>) -> Option<LevelFloor> {
	let t = |name, d| crate::devtune::f32(name, d);
	let (z, soft) = at.filter(|(z, h)| z.is_finite() && h.is_finite())?;
	let w = tune(level);
	let sigma = SIGMA * t("fog_floor", w.sigma);
	let clear = z + soft.max(0.5) + t("fog_floor_lift", w.lift);
	let soft = soft.max(t("fog_floor_soft", w.soft)).min(t("fog_floor_cap", w.cap));
	(super::density::air(level) > 0.0 && !liquid_bottom(level) && sigma > 0.0).then_some(LevelFloor { clear, soft, sigma, desat: t("fog_floor_desat", w.desat), to_under: t("fog_floor_under", w.to_under), under: w.under.map(|c| c as f32 / 255.0) })
}

/// The floor of the level now loaded.
pub fn current(level: usize) -> Option<LevelFloor> {
	if LEVEL.load(Ordering::Acquire) != level as u32 {
		return None;
	}
	let ld = |a: &AtomicU32| f32::from_bits(a.load(Ordering::Relaxed));
	floor(level, Some((ld(&TOP), ld(&SOFT))))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn floor_lies_under_the_lowest_ground() {
		// Eudora-like: terrain down to 24 (a few strays at -50), walkable ground from 32, a kill plane far below.
		let terrain = (0..1000).map(|i| 24.0 + i as f32 * 0.05).chain([-50.0; 5]);
		let coll = (0..2000).map(|i| 32.0 + i as f32 * 0.02).chain([-80.0; 50]);
		let (top, soft) = from_level(terrain, coll).unwrap();
		assert!((top - 25.75).abs() < 0.1 && (top + soft - 31.0).abs() < 0.1, "{top} {soft}");
		let f = floor(4, Some((top, soft))).unwrap();
		assert!(f.sigma > 0.0 && (f.clear - 31.0).abs() < 0.1 && f.soft >= SOFT_MIN);
		let at = f.at();
		assert!((at.z + at.height - f.clear).abs() < 1e-3);
		// Walkable ground right above the lowest terrain (a canyon floor): the floor stays under it.
		let (top, soft) = from_level([6.0; 100].into_iter(), [10.2; 100].into_iter()).unwrap();
		assert!(top + soft < 10.2 && top >= 6.0, "{top} {soft}");
		// No floor in space (Gemlik Base) or without terrain / collision.
		assert!(floor(16, Some((top, soft))).is_none());
		// None where a liquid is the bottom (Orxon, Pokitaru).
		assert!([10, 11].iter().all(|&l| floor(l, Some((top, soft))).is_none()));
		assert!(floor(4, None).is_none());
		assert!(from_level(std::iter::empty(), [1.0].into_iter()).is_none());
		// The floor set at a load belongs to that level only (a prefetch during travel leaves the old level without one).
		set(4, Some((top, soft)));
		assert!(current(4).is_some() && current(0).is_none());
	}

	#[test]
	fn clear_map_lies_under_the_ground_near_each_cell() {
		let quad = |x: f32, y: f32, z: f32| [[[x, y, z], [x + 10.0, y, z], [x, y + 10.0, z]], [[x + 10.0, y, z], [x + 10.0, y + 10.0, z], [x, y + 10.0, z]]];
		// A deck at z 50, a street at z 27 300 units east, a wall down to 0 (not ground).
		let wall = [[[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 0.0, 30.0]]];
		let faces: Vec<_> = quad(0.0, 0.0, 50.0).into_iter().chain(quad(300.0, 0.0, 27.0)).chain(wall).collect();
		let g = clear_map(faces.iter().copied(), 3, 20.0).unwrap();
		let at = |x: f32, y: f32| g.z[((y - g.y0) / CELL) as usize * g.w + ((x - g.x0) / CELL) as usize];
		// Over the deck: just under it. Over the street: just under it. The void between takes its nearest ground's height, and
		// nothing goes under the level's lowest clear top.
		assert!(at(5.0, 5.0) <= 49.0 && at(5.0, 5.0) > 40.0, "{}", at(5.0, 5.0));
		assert!(at(305.0, 5.0) <= 26.0 && at(305.0, 5.0) >= 20.0, "{}", at(305.0, 5.0));
		assert!(g.z.iter().all(|&z| (20.0..=49.0).contains(&z)));
		assert!(clear_map(wall.into_iter(), 3, 20.0).is_none());
		// Reach: a deck 40 units from a street keeps its own height while the street is out of reach, and drops with a longer one.
		let faces: Vec<_> = quad(0.0, 0.0, 50.0).into_iter().chain(quad(50.0, 0.0, 27.0)).collect();
		let near = |reach| clear_map(faces.iter().copied(), reach, 20.0).unwrap();
		let (a, b) = (near(1), near(8));
		let cell0 = |g: &Ground| g.z[(5.0 / CELL) as usize];
		assert!(cell0(&a) > cell0(&b), "{} {}", cell0(&a), cell0(&b));
	}

	#[test]
	fn solid_map_hides_the_render_bottom_under_clear_ground() {
		let quad = |x: f32, y: f32, z: f32| [[[x, y, z], [x + 10.0, y, z], [x, y + 10.0, z]], [[x + 10.0, y, z], [x + 10.0, y + 10.0, z], [x, y + 10.0, z]]];
		// A deck at 50 and one at 50 80 units east; under the first a cliff skirt ends at 40, under the second one at 10.
		let faces: Vec<_> = quad(0.0, 0.0, 50.0).into_iter().chain(quad(80.0, 0.0, 50.0)).collect();
		let mut g = clear_map(faces.iter().copied(), 1, 0.0).unwrap();
		solid_map(&mut g, [[4.0, 4.0, 40.0], [84.0, 4.0, 10.0], [4.0, 4.0, 60.0]].into_iter(), 30.0, None, 0.0);
		let at = |v: &Vec<f32>, x: f32| v[((x - g.x0) / CELL) as usize];
		// The skirt at 40 lies BOTTOM_MARGIN under the solid top (the soft height alone left it 10 units over it), and the solid top
		// stays GAP under the clear top; the deep skirt keeps the plain soft height (its bottom is already in solid fog).
		assert!(at(&g.s, 4.0) >= 40.0 + BOTTOM_MARGIN - 1e-3 && at(&g.s, 4.0) <= at(&g.z, 4.0) - GAP + 1e-3, "{} {}", at(&g.s, 4.0), at(&g.z, 4.0));
		assert!((at(&g.s, 84.0) - (at(&g.z, 84.0) - 30.0)).abs() < 1e-3, "{}", at(&g.s, 84.0));
		// A flat clear top (Kerwan) replaces the per-cell one.
		solid_map(&mut g, std::iter::empty(), 6.0, Some(-18.0), 0.0);
		assert!(g.z.iter().all(|&z| z == -18.0) && g.s.iter().all(|&s| s == -24.0));
	}

	#[test]
	fn the_void_beside_an_island_clears_down_to_near_its_cliff_bottom() {
		let quad = |x: f32, y: f32, z: f32| [[[x, y, z], [x + 10.0, y, z], [x, y + 10.0, z]], [[x + 10.0, y, z], [x + 10.0, y + 10.0, z], [x, y + 10.0, z]]];
		// An island at 50 (x 0..20), a lower one at 30 far east (x 80..90); the island's cliff face goes down to 20 at x 22.
		let faces: Vec<_> = quad(0.0, 0.0, 50.0).into_iter().chain(quad(10.0, 0.0, 50.0)).chain(quad(80.0, 0.0, 30.0)).collect();
		let cliff = [[22.0, 4.0, 49.0], [22.0, 4.0, 20.0]];
		let mut plain = clear_map(faces.iter().copied(), 1, 0.0).unwrap();
		let mut g = clear_map(faces.iter().copied(), 1, 0.0).unwrap();
		solid_map(&mut plain, cliff.into_iter(), 30.0, None, 0.0);
		solid_map(&mut g, cliff.into_iter(), 30.0, None, 12.0);
		let at = |v: &Vec<f32>, x: f32| v[((x - g.x0) / CELL) as usize];
		// Away from the cliff and over the far island nothing moves; the void cell by the cliff and the island's own cell over it drop
		// from under the rim to 12 + 3 over its bottom.
		assert_eq!((at(&g.z, 4.0), at(&g.z, 84.0)), (at(&plain.z, 4.0), at(&plain.z, 84.0)));
		assert!(at(&g.z, 20.0) <= 20.0 + BOTTOM_MARGIN + 12.0 + 1e-3, "{}", at(&g.z, 20.0));
		assert!(at(&plain.z, 28.0) > 45.0 && at(&g.z, 28.0) <= 20.0 + BOTTOM_MARGIN + 12.0 + 1e-3, "{} {}", at(&plain.z, 28.0), at(&g.z, 28.0));
		assert!(g.z.iter().zip(&g.s).all(|(z, s)| s <= z && *z >= g.min));
		assert!(g.z.iter().zip(&plain.z).all(|(a, b)| a <= b));
	}
}
