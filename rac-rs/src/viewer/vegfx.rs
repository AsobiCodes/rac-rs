//! [PC] PREVIEW (lane scratch_veg, 2026-10-16; not in dev): more vegetation, drawn only with Lighting Enhanced and the switch
//! RAC_VEG = 1 (moderate) or 2 (lush); RAC_VEG_KIND = both (default), blades or shrubs; RAC_VEG_LOG=1 prints the placement
//! census. Classic, and Enhanced without RAC_VEG, build and draw nothing new. Visual only: no collision, nothing goes back to
//! the game.
//!
//! Two kinds of plants:
//! - blades: clumps of grass blades made here in code, coloured with the ground texel and baked shade under each clump (a
//!   clump's root matches the ground it stands on), darker at the root, lighter at the tip;
//! - shrubs: more copies of the level's own plant shrubs (mesh, texture and baked colour of a placed one), turned and scaled at
//!   random, around the ones the level already has (the nearest plant shrub's class), tinted by the ground shade there.
//! Placement (`Gpu::plan_veg`, once per level): points on the near-LOD terrain faces facing up (slope under `COS_SLOPE`) whose
//! texel at the point is grass (green, opaque); then a collision ray from `RAY_UP` above must first meet walkable ground (not
//! water, lava, mud, ice, death floors: `BAD_SURF`) within `GROUND_TOL` of the visible surface (nothing over it, no floating
//! plant).
//! Draw: per model, one instanced draw per visible `CELL` cell; per instance a random rank thins the far cells (a cell draws the
//! instances whose rank is below keep(distance), the shader shrinks those about to go, so nothing pops); wind sway in the
//! vertex shader; fog, baked shade and the Enhanced sun shadow as the world's own texels.
//! Touch (`VegFx::actors`, every frame): the plants bend away from the hero and from the trail of his last steps (springing
//! back over `TRAIL_T`), and from the mobys retail draws this field; inside a drawn moby's model footprint they shrink away
//! (a crate, a vendor, an enemy standing in the grass; hidden mobys keep nothing clear).

use super::*;
use rayon::prelude::*;

pub(super) struct Cfg {
	/// Dev override RAC_VEG=0/1/2 (off, moderate, lush); unset = Options > Graphics > Vegetation (`want`).
	pub env: Option<u8>,
	pub blades: bool,
	pub shrubs: bool,
	/// Seaweed on the beds under water (`sea_spots`).
	sea: bool,
	/// Extra leaves on the tree crowns and bushes (`tree_spots`).
	trees: bool,
	pub log: bool,
}

/// F4 (handler.rs, dev) hides / shows the extra plants while they are on, for a quick in-game before / after.
static SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub(super) fn toggle() {
	SHOWN.fetch_xor(true, std::sync::atomic::Ordering::Relaxed);
}

/// Debug: RAC_VEG_DEBUG=1 colours each blade clump by its terrain texture index.
fn debug_tex() -> bool {
	std::env::var("RAC_VEG_DEBUG").is_ok_and(|v| v == "1")
}

/// Debug: RAC_VEG_DEBUG=2 keeps every candidate clump, coloured by why it was dropped (kept: white, not grass: magenta,
/// decal or terrain cover: cyan, no ground: red, ground off: blue, bad surface: black, dirt margin: orange).
fn debug_why() -> bool {
	std::env::var("RAC_VEG_DEBUG").is_ok_and(|v| v == "2")
}

/// Debug: RAC_VEG_DEBUG=3 colours the kept clumps by the nearest shrub face over them in XY (`decal_probe_col`; with
/// RAC_VEG_PROBE_COL set, by that face's texel as drawn).
fn debug_probe() -> bool {
	std::env::var("RAC_VEG_DEBUG").is_ok_and(|v| v == "3")
}

/// Debug: what shrub face lies over p (the one nearest in height): 0 none (white), 1 above by more than COVER_DZ (red),
/// 2 below (blue), 3 see-through or shade (yellow), 4 green (lime), 5 covers (cyan); and the colour of that shrub texel as
/// drawn (texel x vertex colour, 0..1; blue when see-through).
fn decal_probe_col(cover: &Cover, p: [f32; 3]) -> (usize, [f32; 3]) {
	let cell = ((p[0] / COVER_CELL).floor() as i32, (p[1] / COVER_CELL).floor() as i32);
	let mut best: Option<(f32, usize, [f32; 3])> = None;
	for &(si, fi) in cover.sh.get(&cell).map_or(&[][..], |v| &v[..]) {
		let m = &cover.shrubs[si as usize].mesh;
		let (key, f) = m.faces[fi as usize];
		let q = f.map(|i| m.positions[i as usize]);
		let Some(w) = bary_xy(q, p) else { continue };
		let dz = q[0][2] * w[0] + q[1][2] * w[1] + q[2][2] * w[2] - p[2];
		let mut seen = [1.0, 0.0, 1.0];
		let code = if dz > COVER_DZ {
			1
		} else if dz < -COVER_DZ {
			2
		} else {
			let (Some(img), _) = cover.shrub_tex(key) else { continue };
			let ix = f.map(|i| i as usize);
			let uv: [f32; 2] = std::array::from_fn(|j| (0..3).map(|v| m.uvs[ix[v]][j] * w[v]).sum());
			let t = texel3(img, uv[0], uv[1]);
			let shade: [f32; 3] = std::array::from_fn(|j| (0..3).map(|v| m.colours.get(ix[v]).map_or(128.0, |c| c[j]).min(255.0) / 128.0 * w[v]).sum());
			seen = if t[3] < DECAL_A { [0.0, 0.0, 1.0] } else { std::array::from_fn(|j| (t[j] * shade[j] / 255.0).min(1.0)) };
			let sn = [t[0] * shade[0], t[1] * shade[1], t[2] * shade[2], t[3]];
			if t[3] < DECAL_A || lum([sn[0], sn[1], sn[2]]) < DECAL_LUM { 3 } else if green(sn) { 4 } else { 5 }
		};
		if best.is_none_or(|b| dz.abs() < b.0) {
			best = Some((dz.abs(), code, seen));
		}
	}
	best.map_or((0, [1.0, 1.0, 1.0]), |b| (b.1, b.2))
}

/// [PC] The density asked for now: 0 off, 1 moderate, 2 lush (RAC_VEG, else the Graphics row; drawn only with Lighting Enhanced).
pub(super) fn want() -> u8 {
	cfg().env.unwrap_or_else(|| crate::retail::pcext::graphics().veg)
}

pub(super) fn cfg() -> &'static Cfg {
	static C: std::sync::OnceLock<Cfg> = std::sync::OnceLock::new();
	C.get_or_init(|| {
		let env = std::env::var("RAC_VEG").ok().and_then(|v| v.trim().parse::<u8>().ok()).map(|v| v.min(2));
		let (blades, shrubs, sea, trees) = match std::env::var("RAC_VEG_KIND").unwrap_or_default().trim() {
			"blades" => (true, false, false, false),
			"shrubs" => (false, true, false, false),
			"sea" => (false, false, true, false),
			"trees" => (false, false, false, true),
			_ => (true, true, true, true),
		};
		// Lane switch: RAC_VEG_TREES=0 leaves the tree leaves out (before / after with the rest on).
		let trees = trees && !std::env::var("RAC_VEG_TREES").is_ok_and(|v| v.trim() == "0");
		Cfg { env, blades, shrubs, sea, trees, log: std::env::var("RAC_VEG_LOG").is_ok_and(|v| v == "1") }
	})
}

/// One density setting: blade clumps per m2, blades per clump, blade height range (m), blade base width (m), clump radius (m),
/// blade full-density and end distances (m); shrubs: search radius around the level's own plants (m), small and big plants
/// per m2 there, full-density and end distances (m); the share of clumps that also carry flowers.
struct Dens {
	blade_m2: f32,
	blades: usize,
	blade_h: (f32, f32),
	blade_w: f32,
	clump_r: f32,
	blade_full: f32,
	blade_end: f32,
	near_r: f32,
	small_m2: f32,
	big_m2: f32,
	mesh_full: f32,
	mesh_end: f32,
	flower: f32,
}

const MODERATE: Dens = Dens { blade_m2: 2.5, blades: 10, blade_h: (0.20, 0.42), blade_w: 0.05, clump_r: 0.2, blade_full: 20.0, blade_end: 42.0, near_r: 10.0, small_m2: 0.05, big_m2: 0.012, mesh_full: 40.0, mesh_end: 70.0, flower: 0.03 };
const LUSH: Dens = Dens { blade_m2: 6.0, blades: 14, blade_h: (0.24, 0.58), blade_w: 0.055, clump_r: 0.25, blade_full: 28.0, blade_end: 60.0, near_r: 16.0, small_m2: 0.14, big_m2: 0.03, mesh_full: 55.0, mesh_end: 90.0, flower: 0.06 };

fn dens(on: u8) -> &'static Dens {
	if on >= 2 { &LUSH } else { &MODERATE }
}

/// Seaweed (`sea_spots`): clumps per m2 inside the patches (moderate, lush), the water depth it grows in (m), the patch noise
/// scale (m) and the share of the bed the patches cover, its height range (m, at most `SEA_REACH` of the depth and `SEA_GAP`
/// under the surface), its ground shade.
const SEA_M2: [f32; 2] = [0.6, 1.3];
const SEA_DEPTH: (f32, f32) = (0.9, 14.0);
const SEA_PATCH: f32 = 4.5;
const SEA_COVER: (f32, f32) = (0.42, 0.7);
const SEA_H: (f32, f32) = (0.9, 5.0);
const SEA_GAP: f32 = 0.5;
const SEA_REACH: f32 = 0.55;
const SEA_LUM: f32 = 0.85;
/// The planets whose waters are natural (sea, lakes, rock pools): Novalis and Pokitaru. Quartu's indoor tanks and Rilgar's
/// city canals also have deep water but no place for kelp.
const SEA_LEVELS: [usize; 2] = [1, 11];
/// The water surface ids (hero_surfaces.md: 0 water, 0xe water handled by the swim code).
const WATER_SURF: [u8; 2] = [0, 0xe];

/// Tree leaves (`tree_spots`): leaf clusters per m2 of crown surface (moderate, lush), leaves per cluster, leaf length range
/// (m), how far from the cluster centre the leaves start (m), full-density and end distances (m).
const LEAF_M2: [f32; 2] = [1.4, 2.8];
const LEAVES: [usize; 2] = [6, 8];
const LEAF_LEN: (f32, f32) = (0.22, 0.4);
const LEAF_SPREAD: f32 = 0.22;
const LEAF_DIST: [(f32, f32); 2] = [(55.0, 95.0), (75.0, 130.0)];
/// A tie gets leaves when at least this share of its surface is leafy as drawn and at least `TREE_M2` m2 of it: the crowns and
/// bushes, not the rock pillars with a grass top (those are grass blocks, `Cover::blocks`, and skipped anyway).
/// Leafy share for a crown that is mostly autumn (kind 2) texture.
const TREE_LEAFY_AUTUMN: f32 = 0.5;
const TREE_LEAFY: f32 = 0.6;
const TREE_M2: f32 = 6.0;
const CROWN_UP: f32 = 0.6;
/// A leafy face also needs a texture that is itself foliage: at least this share of its opaque texels grass-green
/// (`tex_stats`). Grey wall, building and rock textures that only the baked light tints green (Kerwan towers, Batalia walls,
/// 2026-10-17 sweep: grass share 0.00) get no leaves; leaf and moss textures are 0.36..1.00.
const TREE_TEX_G: f32 = 0.3;
/// Grass-block slopes (`tree_spots`): leaf clusters this much smaller on the leafy faces steeper than `SLOPE_NZ`.
const SLOPE_LEAF: f32 = 0.55;
/// Textures with at least this share of see-through texels are leaf cards (`foliage_tex`): no clusters.
const TREE_CUT: f32 = 0.15;
const SLOPE_NZ: f32 = 0.75;

/// A foliage texture: 1 = grass-green (`TREE_TEX_G`), 2 = autumn: a dark, saturated orange-brown leaf picture (Eudora's
/// red-brown crowns 0x822d / 0x722d, mean 107,70,13). Painted orange trims are brighter (216,145,43), trunks and crates bluer
/// (130,87,98), grey walls and rock tinted green only by the baked light (Kerwan 119,118,133, Pokitaru 82,76,71) fail both.
/// (Tried and dropped 2026-10-17: pale grey-green and bright yellow textures put leaves on Eudora's rock pillars and huts.)
fn foliage_tex(t: &Texture) -> u8 {
	let (g, cut, [r, gg, b]) = tex_stats(t);
	// A cut-out leaf card (palm fronds, ferns, vines: Kerwan 0x7e7e 0.51, 0x7275 0.39) is already leaf-shaped; clusters stuck
	// on it crumple it (user's Kerwan palm, 2026-10-17).
	if cut >= TREE_CUT {
		0
	} else if g >= TREE_TEX_G {
		1
	} else if r >= 40.0 && r <= 150.0 && gg >= r * 0.5 && gg <= r * 0.8 && b <= r * 0.2 {
		2
	} else {
		0
	}
}

/// Leafy as drawn (texel x baked colour) for a texture of kind `kind` (`foliage_tex`): green in hue however dark, or for
/// autumn leaves orange-brown in hue.
fn foliage_seen(c: [f32; 4], kind: u8) -> bool {
	c[3] >= 128.0
		&& match kind {
			1 => leafy(c),
			2 => c[0] >= 12.0 && c[1] >= c[0] * 0.45 && c[1] <= c[0] * 0.9 && c[2] <= c[0] * 0.35,
			_ => false,
		}
}

/// Model kinds: code-made blades and seaweed, copies of the level's plants, leaf clusters.
const KIND_BLADE: u8 = 0;
const KIND_MESH: u8 = 1;
const KIND_LEAF: u8 = 2;

/// Faces steeper than this (normal z) get no plants (about 37 degrees).
const COS_SLOPE: f32 = 0.8;
/// The collision check: a ray from `RAY_UP` above the point to `RAY_DOWN` below must first hit ground within `GROUND_TOL`.
const RAY_UP: f32 = 2.5;
const RAY_DOWN: f32 = 1.0;
const GROUND_TOL: f32 = 0.35;
/// Surface ids (collision attr & 0x1f) with no plants: water, lava, mud, water slide, ice, death mud, death ice
/// (decomp/notes/hero_surfaces.md).
const BAD_SURF: [u8; 7] = [0, 1, 3, 4, 7, 0xb, 0xd];
/// Touch: the hero bends plants within `HERO_R` (m) of his path: the current position and a point every `TRAIL_DT` s
/// (`TRAIL_N` in all), joined into segments, each fading over `TRAIL_T` s so the plants spring back behind him. A drawn moby bends them within `RING` (m) of its
/// model footprint (capped at `FOOT_MAX`) and shrinks them inside it; at most `OBJ_N` mobys, the largest on screen first.
const HERO_R: f32 = 1.0;
const TRAIL_N: usize = 8;
const TRAIL_T: f32 = 1.4;
const TRAIL_DT: f32 = TRAIL_T / (TRAIL_N - 1) as f32;
const RING: f32 = 0.5;
const FOOT_MAX: f32 = 4.0;
const OBJ_N: usize = 40;
/// A moby this close (m) to the hero is carried or worn (Clank, the held weapon): his own touch covers it.
const CARRIED: f32 = 1.0;
/// Draw cell size (m).
const CELL: f32 = 10.0;
/// Shrub classes larger than this (m, bounding radius) are not copied (trees, big rocks).
const MAX_PLANT_R: f32 = 4.5;
/// A bush class (not ground cover) lower than this times its radius is not copied: Eudora's curved leafy hedge strip (class
/// 80, 9 m across, 1.7 m tall, 0.39) edges its terraces, and copies of it scattered on the meadow drew square leafy frames
/// (user's screenshots, 2026-10-17). Every other bush class copied on any planet is at least 0.55.
/// Plant copies: a shrub template whose lowest point lies more than this (m) above its origin is a hanging plant.
const HANG_ZLO: f32 = 0.1;
const MIN_BUSH_H: f32 = 0.5;
/// A shrub class up to this radius (m) is ground cover (`small_m2`), a larger one a bush (`big_m2`).
const SMALL_R: f32 = 1.0;
/// Blade look: root darkness (x the ground colour) and tip lift (+), wind amplitude (m of tip travel per m of height), far
/// blade widening.
const ROOT_DARK: f32 = 0.78;
const TIP_LIFT: f32 = 0.3;
const WIND: f32 = 0.22;
const WIDEN: f32 = 1.2;
const SATURATE: f32 = 1.2;
const WIND_DIR: [f32; 2] = [0.8, 0.6];
/// Another terrain layer within this height (m) of a candidate point lies over it (a dirt path or decal drawn over the grass).
const COVER_DZ: f32 = 0.3;
/// Grass keeps this margin (m) from dirt, and clumps within `EDGE_SHORT` (m) of it grow `EDGE_H` of their height: tall blades
/// on the narrow strips between paths hid the paths behind them from a low camera (Novalis field 1360). Plant copies keep
/// the `EDGE_SHORT` margin.
const EDGE_R: f32 = 0.45;
const EDGE_SHORT: f32 = 1.0;
const EDGE_H: f32 = 0.45;
const COVER_CELL: f32 = 2.0;
/// A shrub class is copied only where the level itself puts it on grass: this share of its instances with a grass point
/// within `ON_GRASS_R` (m).
const ON_GRASS: f32 = 0.5;
const ON_GRASS_R: f32 = 1.5;
/// Flower heads (code-made, on `Dens::flower` of the clumps): petal length (m), heads per clump, patch size (m) of one colour.
const HEAD_R: f32 = 0.06;
const HEADS: usize = 4;
const PATCH: f32 = 7.0;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct VegVertex {
	/// Blades: centreline offset (x, y, in units of the blade height) and height fraction z; meshes: model-space position (m).
	pos: [f32; 3],
	/// Blades: root offset in the clump (m); meshes: 0.
	root: [f32; 2],
	/// Blades: (signed half width m, height fraction); meshes: texture uv.
	uv: [f32; 2],
	/// rgb: colour factor (blades: per-blade variation; meshes: baked colour, 1.0 = 0x80); a: meshes' sway weight (0 root, 1 top).
	col: [f32; 4],
	/// Blades: the width axis.
	side: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct VegInst {
	/// Position, yaw.
	a: [f32; 4],
	/// Horizontal scale, height (blades: m; meshes: the uniform scale), rank, sway phase.
	b: [f32; 4],
	/// Blades: the ground colour (texel x baked shade); meshes: the tint.
	c: [f32; 4],
	/// Ground slope (dz/dx, dz/dy), ground shade luminance, 0.
	d: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct VegU {
	/// time (s), wind direction, wind amplitude.
	t: [f32; 4],
	/// blade full / end distance, mesh full / end distance.
	f: [f32; 4],
	/// blade saturation, far widening, root darkness, tip lift.
	k: [f32; 4],
	/// Touch: trail points, mobys, `HERO_R`, `RING`.
	a: [f32; 4],
	/// Leaves: full / end distance, 0, 0.
	l: [f32; 4],
	/// `TRAIL_N` trail points (xyz, strength 0..1), then `OBJ_N` mobys (xyz, footprint m).
	act: [[f32; 4]; TRAIL_N + OBJ_N],
}

/// One model: blades (bind None = the plain white texture) or a shrub template; per texture: (texture slot key, first index, count).
struct ModelCpu {
	kind: u8,
	subs: Vec<(Option<u32>, u32, u32)>,
}

struct Model {
	kind: u8,
	subs: Vec<(Option<usize>, u32, u32)>,
}

struct Cell {
	model: u32,
	lo: [f32; 3],
	hi: [f32; 3],
	first: u32,
	ranks: Vec<f32>,
}

struct Plan {
	vbuf: wgpu::Buffer,
	ibuf: wgpu::Buffer,
	inst: wgpu::Buffer,
	models: Vec<Model>,
	cells: Vec<Cell>,
	white: wgpu::BindGroup,
	dens: &'static Dens,
	/// The density this plan was made for (1 moderate, 2 lush; `veg_follow` compares it with the row).
	on: u8,
	/// The scene the texture slots belong to (tfrag count, radius bits).
	scene_id: (usize, u32),
	/// Moby class -> model footprint (m).
	foot: HashMap<u32, f32>,
}

pub(super) struct VegFx {
	blade: wgpu::RenderPipeline,
	mesh: wgpu::RenderPipeline,
	leaf: wgpu::RenderPipeline,
	ubuf: wgpu::Buffer,
	ubind: wgpu::BindGroup,
	start: std::time::Instant,
	plan: Option<Plan>,
	/// The level data a replan reads (`Gpu::keep_veg_src`) and the replan running on its thread: (result, density).
	src: Option<std::sync::Arc<VegSrc>>,
	job: Option<(std::sync::mpsc::Receiver<Option<PlanCpu>>, u8)>,
	/// This frame's draws: (model, first instance, count).
	draws: Vec<(u32, u32, u32)>,
	/// The hero's trail (position, clock s), newest first.
	trail: Vec<([f32; 3], f32)>,
	/// The last field's hero and drawn mobys (class, position, scale), from `actors`.
	hero: Option<[f32; 3]>,
	objs: Vec<(u32, [f32; 3], f32)>,
}

const WGSL: &str = r#"
struct VegU { t: vec4<f32>, f: vec4<f32>, k: vec4<f32>, a: vec4<f32>, l: vec4<f32>, act: array<vec4<f32>, 48> };
@group(3) @binding(0) var<uniform> veg: VegU;
struct VegOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) c: vec3<f32>, @location(2) w: vec3<f32>, @location(3) @interpolate(linear) fogf: f32, @location(4) gl: f32 };

fn veg_yaw(v: vec2<f32>, a: f32) -> vec2<f32> {
	let s = sin(a);
	let c = cos(a);
	return vec2<f32>(c * v.x - s * v.y, s * v.x + c * v.y);
}

// Wind: a slow gust front rolling along the wind direction, a per-plant flutter, a little sideways sway (tip offset per m of height).
fn veg_wind(p: vec3<f32>, phase: f32) -> vec2<f32> {
	let t = veg.t.x;
	let dir = veg.t.yz;
	let along = dot(p.xy, dir);
	let gust = 0.5 + 0.5 * sin(t * 0.8 - along * 0.12);
	let flutter = sin(t * 2.6 + phase + along * 0.4);
	let cross = sin(t * 3.3 + phase * 1.3);
	return (dir * (0.25 + 0.75 * gust) * (0.7 + 0.3 * flutter) + vec2<f32>(-dir.y, dir.x) * cross * 0.2) * veg.t.w;
}

// Touch at a plant standing at c: (bend direction x strength 0..1, height kept 0..1). The hero's path (trail points joined
// into segments, strength along them) bends within a.z, away from it; a drawn moby bends within a.w of its footprint and
// shrinks the plants inside it.
fn veg_touch(c: vec3<f32>) -> vec3<f32> {
	var best = 0.0;
	var dir = vec2<f32>(0.0, 0.0);
	var keep = 1.0;
	let nt = i32(veg.a.x);
	for (var i = 0; i < nt; i++) {
		let q0 = veg.act[i];
		let q1 = veg.act[min(i + 1, nt - 1)];
		let e = q1.xy - q0.xy;
		let ee = dot(e, e);
		let u = select(0.0, clamp(dot(c.xy - q0.xy, e) / max(ee, 1e-6), 0.0, 1.0), ee > 1e-6);
		let v = c.xy - (q0.xy + e * u);
		let d = length(v);
		if (d > veg.a.z || abs(c.z - mix(q0.z, q1.z, u)) > 1.2) { continue; }
		let s = mix(q0.w, q1.w, u) * (1.0 - smoothstep(veg.a.z * 0.45, veg.a.z, d));
		if (s > best) { best = s; dir = select(veg.t.yz, v / d, d > 1e-3); }
	}
	let no = i32(veg.a.y);
	for (var j = 0; j < no; j++) {
		let q = veg.act[8 + j];
		let v = c.xy - q.xy;
		let d = length(v);
		if (d > q.w + veg.a.w || c.z - q.z > 1.0 || q.z - c.z > 2.5) { continue; }
		keep = min(keep, smoothstep(q.w * 0.75, q.w, d));
		let s = 1.0 - smoothstep(q.w, q.w + veg.a.w, d);
		if (s > best) { best = s; dir = select(veg.t.yz, v / d, d > 1e-3); }
	}
	return vec3<f32>(dir * best, keep);
}

// 1 = fully grown, 0 = gone: keep(d) falls from 1.1 at `full` to 0 at `end`; ranks just under keep shrink.
fn veg_fade(d: f32, full: f32, end: f32, rank: f32) -> f32 {
	let keep = 1.1 * clamp((end - d) / max(end - full, 0.001), 0.0, 1.0);
	return clamp((keep - rank) / 0.1, 0.0, 1.0);
}

// Flower colours (gamma space, as the GS look): white, yellow, purple, pink; flag 2 = the disc centre.
fn veg_petal(i: f32, flag: f32) -> vec3<f32> {
	if (flag > 1.5) {
		return select(vec3<f32>(1.0, 0.78, 0.2), vec3<f32>(0.85, 0.42, 0.1), i > 0.5 && i < 1.5);
	}
	var p = vec3<f32>(0.96, 0.94, 0.86);
	if (i > 0.5) { p = vec3<f32>(1.0, 0.84, 0.22); }
	if (i > 1.5) { p = vec3<f32>(0.66, 0.46, 0.95); }
	if (i > 2.5) { p = vec3<f32>(1.0, 0.55, 0.8); }
	return p;
}

fn veg_fog(i: VegOut) -> f32 {
	let d = distance(i.w, u.eye.xyz);
	var f = clamp(u.fog.z + (d - u.fog.x) * (u.fog.w - u.fog.z) / max(u.fog.y - u.fog.x, 0.001), min(u.fog.z, u.fog.w), 255.0) / 255.0;
	if (i.fogf >= 0.0) { f = i.fogf / 255.0; }
	return f;
}

@vertex fn vs_veg_blade(@location(0) p: vec3<f32>, @location(1) root: vec2<f32>, @location(2) uv: vec2<f32>, @location(3) c: vec4<f32>, @location(4) side: vec2<f32>, @location(5) ia: vec4<f32>, @location(6) ib: vec4<f32>, @location(7) ic: vec4<f32>, @location(8) id: vec4<f32>) -> VegOut {
	var o: VegOut;
	let d = distance(ia.xyz, u.eye.xyz);
	let f = veg_fade(d, veg.f.x, veg.f.y, ib.z);
	let widen = 1.0 + max(d - veg.f.x * 0.5, 0.0) / max(veg.f.x, 1.0) * veg.k.y;
	let h = uv.y;
	let touch = veg_touch(ia.xyz);
	// Blades shrink away right at the lens (a low camera in a meadow looked through a wall of giant blades: Eudora, by the ship).
	let height = ib.y * f * touch.z * smoothstep(0.8, 2.4, d);
	// Bent by touch: the blade follows an arc whose angle grows to `th` at the tip (its length kept); the wind calms on it.
	let th = min(length(touch.xy), 1.0) * 1.45;
	let bd = touch.xy / max(length(touch.xy), 1e-4);
	var l = root * ib.x * mix(0.4, 1.0, f) + p.xy * height + side * uv.x * widen * f;
	var wind = veg_wind(ia.xyz, ib.w) * height * h * h * (1.0 - 0.6 * th / 1.45);
	if (id.w > 0.5) {
		// Seaweed: no wind; the current waves it, a slow wave running up each ribbon.
		let t = veg.t.x;
		wind = vec2<f32>(sin(t * 0.9 + ib.w - h * 3.0), sin(t * 0.63 + ib.w * 1.7 - h * 2.3)) * 0.2 * height * h * (1.0 - 0.6 * th / 1.45);
	}
	l = veg_yaw(l, ia.w) + wind;
	var z = p.z * height - dot(wind, wind) * 0.5 / max(height, 0.05);
	if (th > 1e-3) {
		l = l + bd * height * (1.0 - cos(th * p.z)) / th;
		z = z - p.z * height + height * sin(th * p.z) / th;
	}
	let wp = vec3<f32>(ia.x + l.x, ia.y + l.y, ia.z + z + id.x * l.x + id.y * l.y - 0.03);
	o.pos = world_clip(u.view_proj, wp);
	o.w = wp;
	o.uv = uv;
	var col = ic.rgb * c.rgb * (mix(veg.k.z, 1.0, h) + veg.k.w * h * h);
	// A touch more saturation than the ground (the blades stand out of it, as the painted grass would).
	let lg = dot(col, vec3<f32>(0.2126, 0.7152, 0.0722));
	col = max(mix(vec3<f32>(lg), col, veg.k.x), vec3<f32>(0.0));
	if (c.a > 0.5) {
		col = veg_petal(ic.w, c.a) * clamp(id.z, 0.35, 1.25);
	}
	o.c = col;
	o.gl = id.z;
	o.fogf = fog_class(vec4<f32>(0.0, 0.0, 0.0, -1.0), o.pos.w);
	return o;
}

@fragment fn fs_veg_blade(i: VegOut) -> @location(0) vec4<f32> {
	let k = sun_baked(vec3<f32>(i.gl), i.w).x / max(i.gl, 1e-3);
	return vec4<f32>(mix(u.fog_colour.rgb, i.c * k, veg_fog(i)), 1.0);
}

@vertex fn vs_veg_mesh(@location(0) p: vec3<f32>, @location(1) root: vec2<f32>, @location(2) uv: vec2<f32>, @location(3) c: vec4<f32>, @location(4) side: vec2<f32>, @location(5) ia: vec4<f32>, @location(6) ib: vec4<f32>, @location(7) ic: vec4<f32>, @location(8) id: vec4<f32>) -> VegOut {
	var o: VegOut;
	let d = distance(ia.xyz, u.eye.xyz);
	let f = veg_fade(d, veg.f.z, veg.f.w, ib.z);
	let touch = veg_touch(ia.xyz);
	let s = ib.y * mix(0.15, 1.0, f) * select(0.0, 1.0, f > 0.0) * touch.z * smoothstep(1.0, 3.0, d);
	// Wind and touch lean the top (sway weight c.a) away; the touch gives up to 0.6 m per m of height.
	let wind = (veg_wind(ia.xyz, ib.w) * 0.5 + touch.xy * 0.6) * c.a * c.a * max(p.z, 0.0) * s;
	let l = veg_yaw(p.xy * s, ia.w) + wind;
	let wp = vec3<f32>(ia.x + l.x, ia.y + l.y, ia.z + p.z * s - dot(wind, wind) * 0.3 - 0.03);
	o.pos = world_clip(u.view_proj, wp);
	o.w = wp;
	o.uv = uv;
	o.c = c.rgb * ic.rgb;
	o.gl = 1.0;
	o.fogf = fog_class(vec4<f32>(0.0, 0.0, 0.0, -1.0), o.pos.w);
	return o;
}

// A leaf cluster on a crown: model z along the crown's outward normal (id.xyz), rolled by ia.w round it; the leaves flutter,
// their tips most (c.a). Colour: the crown as drawn there (ic) x the leaf's own shade (c); id.w the crown's baked luminance.
@vertex fn vs_veg_leaf(@location(0) p: vec3<f32>, @location(1) root: vec2<f32>, @location(2) uv: vec2<f32>, @location(3) c: vec4<f32>, @location(4) side: vec2<f32>, @location(5) ia: vec4<f32>, @location(6) ib: vec4<f32>, @location(7) ic: vec4<f32>, @location(8) id: vec4<f32>) -> VegOut {
	var o: VegOut;
	let d = distance(ia.xyz, u.eye.xyz);
	let f = veg_fade(d, veg.l.x, veg.l.y, ib.z);
	let s = ib.x * mix(0.35, 1.0, f) * select(0.0, 1.0, f > 0.0) * smoothstep(0.6, 2.0, d);
	let n = normalize(id.xyz);
	let up = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(n.z) < 0.9);
	let a0 = normalize(cross(up, n));
	let b0 = cross(n, a0);
	let t1 = a0 * cos(ia.w) + b0 * sin(ia.w);
	let t2 = b0 * cos(ia.w) - a0 * sin(ia.w);
	let fl = sin(veg.t.x * 4.1 + ib.w * 3.0 + p.x * 9.0 + p.y * 7.0);
	let wob = (vec3<f32>(veg_wind(ia.xyz, ib.w), 0.0) * 0.18 + n * fl * 0.035) * c.a;
	let wp = ia.xyz + (t1 * p.x + t2 * p.y + n * p.z + wob) * s;
	o.pos = world_clip(u.view_proj, wp);
	o.w = wp;
	o.uv = uv;
	o.c = ic.rgb * c.rgb;
	o.gl = id.w;
	o.fogf = fog_class(vec4<f32>(0.0, 0.0, 0.0, -1.0), o.pos.w);
	return o;
}

@fragment fn fs_veg_mesh(i: VegOut) -> @location(0) vec4<f32> {
	let t = textureSample(tex, samp, i.uv);
	let a = clamp((t.a - 0.3) / 0.35, 0.0, 1.0);
	if (a <= 0.01) { discard; }
	let shade = sun_baked(min(i.c, vec3<f32>(255.0 / 128.0)), i.w);
	return vec4<f32>(mix(u.fog_colour.rgb, t.rgb * shade, veg_fog(i)), a);
}
"#;

impl VegFx {
	pub(super) fn new(device: &wgpu::Device, ulayout: &wgpu::BindGroupLayout, tex_layout: &wgpu::BindGroupLayout, format: wgpu::TextureFormat, depth_format: wgpu::TextureFormat) -> VegFx {
		let src = format!("{}{}", SHADER, WGSL);
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("veg"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("veg", src.into())) });
		let vlayout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("veg"),
			entries: &[wgpu::BindGroupLayoutEntry {
				binding: 0,
				visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
				ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
				count: None,
			}],
		});
		let empty = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("veg empty"), entries: &[] });
		let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("veg"), bind_group_layouts: &[Some(ulayout), Some(tex_layout), Some(&empty), Some(&vlayout)], immediate_size: 0 });
		let make = |vs: &str, fs: &str, a2c: bool| {
			device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
				label: Some(vs),
				layout: Some(&layout),
				vertex: wgpu::VertexState {
					module: &shader,
					entry_point: Some(vs),
					compilation_options: Default::default(),
					buffers: &[
						Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<VegVertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x2, 3 => Float32x4, 4 => Float32x2] }),
						Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<VegInst>() as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &wgpu::vertex_attr_array![5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4] }),
					],
				},
				fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some(fs), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
				primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
				depth_stencil: Some(wgpu::DepthStencilState { format: depth_format, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::LessEqual), stencil: Default::default(), bias: Default::default() }),
				multisample: wgpu::MultisampleState { count: MSAA, mask: !0, alpha_to_coverage_enabled: a2c },
				multiview_mask: None,
				cache: None,
			})
		};
		let blade = make("vs_veg_blade", "fs_veg_blade", false);
		let mesh = make("vs_veg_mesh", "fs_veg_mesh", true);
		let leaf = make("vs_veg_leaf", "fs_veg_blade", false);
		let ubuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("veg uniforms"), size: std::mem::size_of::<VegU>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		let ubind = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("veg"), layout: &vlayout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }] });
		VegFx { blade, mesh, leaf, ubuf, ubind, start: std::time::Instant::now(), plan: None, src: None, job: None, draws: Vec::new(), trail: Vec::new(), hero: None, objs: Vec::new() }
	}

	/// Every frame (handler.rs, with the particles): the hero's position (retail `RetailFx::hero`) and the mobys retail drew
	/// in the last field (class, position, scale); None / empty outside retail's levels.
	pub(super) fn actors<'a>(&mut self, hero: Option<[f32; 3]>, mobys: impl Iterator<Item = &'a crate::retail::Moby>) {
		self.hero = hero;
		self.objs.clear();
		if self.plan.is_none() {
			return;
		}
		let near_hero = |p: [f32; 3]| hero.is_some_and(|h| (h[0] - p[0]).hypot(h[1] - p[1]) < CARRIED && (h[2] - p[2]).abs() < 2.0);
		self.objs.extend(mobys.filter(|m| m.shown && m.class != 0 && !near_hero(m.pos)).map(|m| (m.class, m.pos, m.scale)));
	}

	/// The touch uniforms: the trail (updated on the clock `t`) and the drawn mobys nearest the eye, largest on screen first.
	fn touch(&mut self, t: f32, eye: [f32; 3], reach: f32) -> ([f32; 4], [[f32; 4]; TRAIL_N + OBJ_N]) {
		let mut act = [[0.0; 4]; TRAIL_N + OBJ_N];
		// A jump back in the clock (a new movie, a reload) starts a new trail.
		if self.trail.first().is_some_and(|q| q.1 > t) {
			self.trail.clear();
		}
		self.trail.retain(|q| t - q.1 < TRAIL_T);
		match self.hero {
			Some(h) => {
				let step = self.trail.get(1).is_none_or(|q| t - q.1 >= TRAIL_DT);
				// Entry 0 is always the current position; it stays behind as a trail point every `TRAIL_DT`.
				if step || self.trail.is_empty() {
					self.trail.insert(0, (h, t));
				} else {
					self.trail[0] = (h, t);
				}
				self.trail.truncate(TRAIL_N);
			}
			None => self.trail.clear(),
		}
		for (i, q) in self.trail.iter().enumerate() {
			let age = ((t - q.1) / TRAIL_T).clamp(0.0, 1.0);
			let k = 1.0 - age * age * (3.0 - 2.0 * age);
			act[i] = [q.0[0], q.0[1], q.0[2], k];
		}
		let foot = &self.plan.as_ref().unwrap().foot;
		let mut objs: Vec<(f32, [f32; 4])> = self
			.objs
			.iter()
			.filter_map(|&(c, p, sc)| {
				let r = (foot.get(&c)? * sc).min(FOOT_MAX);
				let d = ((p[0] - eye[0]).powi(2) + (p[1] - eye[1]).powi(2) + (p[2] - eye[2]).powi(2)).sqrt();
				(r > 0.05 && d - r < reach).then_some((r / d.max(1.0), [p[0], p[1], p[2], r]))
			})
			.collect();
		objs.sort_by(|a, b| b.0.total_cmp(&a.0));
		objs.truncate(OBJ_N);
		for (j, o) in objs.iter().enumerate() {
			act[TRAIL_N + j] = o.1;
		}
		([self.trail.len() as f32, objs.len() as f32, HERO_R, RING], act)
	}

	/// Before the level pass: this frame's cell draws and the uniforms (`on`: Lighting Enhanced, a level on screen; Vegetation
	/// not Off).
	pub(super) fn prepare(&mut self, queue: &wgpu::Queue, scene: &Scene, on: bool, eye: [f32; 3], vp: &[[f32; 4]; 4]) {
		self.draws.clear();
		let on = on && want() > 0;
		if !self.plan.as_ref().is_some_and(|p| on && SHOWN.load(std::sync::atomic::Ordering::Relaxed) && p.scene_id == (scene.tfrag_set.len(), scene.radius.to_bits())) {
			return;
		}
		let t = self.start.elapsed().as_secs_f32() % 3600.0;
		let dn = (WIND_DIR[0] * WIND_DIR[0] + WIND_DIR[1] * WIND_DIR[1]).sqrt();
		let d = self.plan.as_ref().unwrap().dens;
		let (a, act) = self.touch(t, eye, d.blade_end.max(d.mesh_end));
		let leaf_d = LEAF_DIST[(d.blade_m2 > MODERATE.blade_m2) as usize];
		let plan = self.plan.as_ref().unwrap();
		let ld = LEAF_DIST[(d.blade_m2 > MODERATE.blade_m2) as usize];
		let uni = VegU { t: [t, WIND_DIR[0] / dn, WIND_DIR[1] / dn, WIND], f: [d.blade_full, d.blade_end, d.mesh_full, d.mesh_end], k: [SATURATE, WIDEN, ROOT_DARK, TIP_LIFT], a, l: [ld.0, ld.1, 0.0, 0.0], act };
		queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(&uni));
		for c in &plan.cells {
			let (full, end) = match plan.models[c.model as usize].kind {
				KIND_MESH => (d.mesh_full, d.mesh_end),
				KIND_LEAF => leaf_d,
				_ => (d.blade_full, d.blade_end),
			};
			let q: [f32; 3] = std::array::from_fn(|k| eye[k].clamp(c.lo[k], c.hi[k]));
			let dist = (0..3).map(|k| (q[k] - eye[k]) * (q[k] - eye[k])).sum::<f32>().sqrt();
			let keep = 1.1 * ((end - dist) / (end - full)).clamp(0.0, 1.0);
			if keep <= 0.0 || !in_view(vp, c.lo, c.hi) {
				continue;
			}
			let n = c.ranks.partition_point(|&r| r < keep) as u32;
			if n > 0 {
				self.draws.push((c.model, c.first, n));
			}
		}
	}

	/// In the level pass, after the shrubs.
	pub(super) fn encode(&self, pass: &mut wgpu::RenderPass, scene: &Scene) {
		let Some(plan) = self.plan.as_ref().filter(|_| !self.draws.is_empty()) else { return };
		pass.set_bind_group(3, &self.ubind, &[]);
		pass.set_vertex_buffer(0, plan.vbuf.slice(..));
		pass.set_vertex_buffer(1, plan.inst.slice(..));
		pass.set_index_buffer(plan.ibuf.slice(..), wgpu::IndexFormat::Uint32);
		let mut cur: Option<u8> = None;
		for &(m, first, n) in &self.draws {
			let model = &plan.models[m as usize];
			if cur != Some(model.kind) {
				pass.set_pipeline(match model.kind {
					KIND_MESH => &self.mesh,
					KIND_LEAF => &self.leaf,
					_ => &self.blade,
				});
				cur = Some(model.kind);
			}
			for &(bind, start, count) in &model.subs {
				let group = match bind {
					Some(i) => match scene.bind_groups.get(i) {
						Some(g) => g,
						None => continue,
					},
					None => &plan.white,
				};
				pass.set_bind_group(1, group, &[]);
				pass.draw_indexed(start..start + count, 0, first..first + n);
			}
		}
	}
}

/// The box is not wholly outside one side of the view frustum (m = view-projection, column-major).
fn in_view(m: &[[f32; 4]; 4], lo: [f32; 3], hi: [f32; 3]) -> bool {
	let mut out = [true; 5];
	for k in 0..8 {
		let p = [if k & 1 != 0 { hi[0] } else { lo[0] }, if k & 2 != 0 { hi[1] } else { lo[1] }, if k & 4 != 0 { hi[2] } else { lo[2] }];
		let c: [f32; 4] = std::array::from_fn(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r]);
		out[0] &= c[0] < -c[3];
		out[1] &= c[0] > c[3];
		out[2] &= c[1] < -c[3];
		out[3] &= c[1] > c[3];
		out[4] &= c[3] < 0.0;
	}
	!out.iter().any(|&o| o)
}

struct Rng(u64);

impl Rng {
	fn new(a: u64, b: u64) -> Rng {
		let mut r = Rng(a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_mul(0xD1B5_4A32_D192_ED03) ^ 0x5851_F42D_4C95_7F2D);
		r.next();
		r
	}

	/// Uniform in [0, 1) (splitmix64).
	fn next(&mut self) -> f32 {
		self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
		let mut z = self.0;
		z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
		z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
		z ^= z >> 31;
		(z >> 40) as f32 / (1u64 << 24) as f32
	}
}

/// Grass texel (bytes): green over red and clearly over blue.
fn green(c: [f32; 4]) -> bool {
	let (r, g, b) = (c[0], c[1], c[2]);
	g >= 28.0 && g >= r * 0.92 && g >= b * 1.15 && g - b >= 12.0
}

/// Green in hue however dark (leaves, grass in shade): what grass may grow through.
fn leafy(c: [f32; 4]) -> bool {
	let (r, g, b) = (c[0], c[1], c[2]);
	g >= 12.0 && g >= r * 0.92 && g >= b * 1.15 && g - b >= 6.0
}

/// The 3x3 mean around the texel at (u, v): the ground colour under a clump, not one texel's speck.
fn texel3(t: &Texture, u: f32, v: f32) -> [f32; 4] {
	let (w, h) = (t.w as i64, t.h as i64);
	let x = ((u - u.floor()) * w as f32) as i64;
	let y = ((v - v.floor()) * h as f32) as i64;
	let mut s = [0.0f32; 4];
	for dy in -1..=1 {
		for dx in -1..=1 {
			let p = t.rgba[((y + dy).rem_euclid(h) * w + (x + dx).rem_euclid(w)) as usize];
			for k in 0..4 {
				s[k] += p[k] as f32 / 9.0;
			}
		}
	}
	s
}

/// Share of the opaque texels that are grass, the share of see-through texels, and the mean opaque colour.
fn tex_stats(t: &Texture) -> (f32, f32, [f32; 3]) {
	let (mut g, mut n, mut cut, mut sum) = (0usize, 0usize, 0usize, [0.0f32; 3]);
	for p in &t.rgba {
		if p[3] < 128 {
			cut += 1;
			continue;
		}
		n += 1;
		let c = [p[0] as f32, p[1] as f32, p[2] as f32, p[3] as f32];
		g += green(c) as usize;
		for k in 0..3 {
			sum[k] += c[k];
		}
	}
	let n1 = n.max(1) as f32;
	(g as f32 / n1, cut as f32 / t.rgba.len().max(1) as f32, sum.map(|s| s / n1))
}

/// A candidate point: position, ground slope (dz/dx, dz/dy), ground colour (texel 0..1 x shade), shade luminance, randoms.
#[derive(Clone, Copy)]
struct Spot {
	p: [f32; 3],
	slope: [f32; 2],
	col: [f32; 3],
	lum: f32,
	r: [f32; 6],
	/// Debug (RAC_VEG_DEBUG=1): the terrain texture index.
	tex: u32,
	/// Dirt within `EDGE_SHORT`: a short clump, no plant copy.
	short: bool,
}

fn lum(c: [f32; 3]) -> f32 {
	0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// Why a candidate was dropped (census): not grass, too steep, no ground hit, ground too far from the surface, bad surface, moby.
#[derive(Default, Clone, Copy)]
struct Census {
	tried: usize,
	not_green: usize,
	no_hit: usize,
	far: usize,
	bad: usize,
	covered: usize,
	edge: usize,
	kept: usize,
}

impl Census {
	fn add(mut self, o: Census) -> Census {
		self.tried += o.tried;
		self.not_green += o.not_green;
		self.no_hit += o.no_hit;
		self.far += o.far;
		self.bad += o.bad;
		self.covered += o.covered;
		self.edge += o.edge;
		self.kept += o.kept;
		self
	}
}

type CoverGrid = HashMap<(i32, i32), Vec<(u32, u32)>>;

/// What may lie over a grass face: the other near-LOD terrain faces and the shrub faces (the levels lay their dirt patches
/// and stepping stones on the grass as flat shrubs, drawn over the terrain: Novalis at the river, RAC_DRAWSKIP=shrub).
struct Cover<'a> {
	tf: CoverGrid,
	sh: CoverGrid,
	shrubs: &'a [shrubs::ShrubInstance],
	stex: &'a HashMap<u32, Texture>,
	/// The shrub textures as drawn: the authored chains by texture index (gpu.rs `batch_instances`), top level.
	schains: &'a [Option<Vec<Texture>>],
	/// The grass-topped ties (Eudora's raised grass blocks): their LOD 0 faces with a grass texture facing up, by `COVER_CELL`.
	tg: CoverGrid,
	ties: &'a [ties::LitTie],
	/// The tie textures by key as drawn (the authored chain's top level).
	ttex: HashMap<u32, &'a Texture>,
	/// The grass blocks: ties with at least `BLOCK_TOP` m2 of flat grass faces (a bush's rounded leaves have next to none).
	/// Retail gives their tops no floor of their own (raised decor beside the walkable ground), so their grass skips the floor test.
	blocks: std::collections::HashSet<u32>,
}

/// Flat grass top (m2) that makes a tie a grass block (`Cover::blocks`), and what counts as flat (normal z).
const BLOCK_TOP: f32 = 4.0;
const BLOCK_FLAT: f32 = 0.95;

/// The grass blocks among the ties (`Cover::blocks`).
fn tie_blocks(tie_set: &[ties::LitTie], ttex: &HashMap<u32, &Texture>) -> std::collections::HashSet<u32> {
	(0..tie_set.len() as u32)
		.filter(|&ii| {
			let m = &tie_set[ii as usize].lods[0];
			let flat: f32 = m
				.faces
				.iter()
				.filter(|(key, _)| ttex.contains_key(&(key & crate::mobies::KEY_MASK)))
				.map(|&(_, f)| {
					let [a, b, c] = f.map(|i| m.positions[i as usize]);
					let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
					let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
					let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
					let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
					if len > 1e-6 && n[2] / len >= BLOCK_FLAT { len * 0.5 } else { 0.0 }
				})
				.sum();
			flat >= BLOCK_TOP
		})
		.collect()
}

impl Cover<'_> {
	/// A shrub face's texture as drawn and its CLAMP wrap (bit 0 u, bit 1 v): the chain's `DECAL_MIP` level, where a decal's
	/// dithered alpha has averaged out as the GS samples it from a few metres (the top level's speckle left holes in the paths).
	fn shrub_tex(&self, key: u32) -> (Option<&Texture>, u32) {
		let tk = key & crate::mobies::KEY_MASK & !shrubs::MATERIAL_BITS;
		let img = self.schains.get((tk & 0xff) as usize).and_then(|c| c.as_ref()).and_then(|c| c.get(DECAL_MIP.min(c.len() - 1))).or_else(|| self.stex.get(&tk));
		(img, (key >> shrubs::WRAP_SHIFT) & 3)
	}
}

/// The grass tie textures: tie texture keys (`mobies::KEY_MASK`) with a fair share of grass texels, as drawn from the
/// authored chain's top level (the terrain rule).
fn tie_grass<'a>(tie_set: &[ties::LitTie], textures: &'a HashMap<u32, Texture>, tie_chains: &'a [Option<Vec<Texture>>]) -> HashMap<u32, &'a Texture> {
	let mut used: Vec<u32> = tie_set.iter().flat_map(|t| t.lods[0].faces.iter().map(|&(k, _)| k & crate::mobies::KEY_MASK)).collect();
	used.sort_unstable();
	used.dedup();
	used.into_iter()
		.filter_map(|tk| tie_chains.get((tk & 0xff) as usize).and_then(|c| c.as_ref()).and_then(|c| c.first()).or_else(|| textures.get(&tk)).map(|x| (tk, x)))
		.filter(|(_, img)| tex_stats(img).0 >= 0.25)
		.collect()
}

/// The upward LOD 0 faces of the ties with a grass texture (`ttex`) on a `COVER_CELL` grid (ids: tie, face).
fn tie_grid(tie_set: &[ties::LitTie], ttex: &HashMap<u32, &Texture>) -> CoverGrid {
	let mut g: CoverGrid = HashMap::new();
	for (ii, t) in tie_set.iter().enumerate() {
		let m = &t.lods[0];
		for (fi, &(key, f)) in m.faces.iter().enumerate() {
			if !ttex.contains_key(&(key & crate::mobies::KEY_MASK)) {
				continue;
			}
			let q = f.map(|i| m.positions[i as usize]);
			let lo = [q[0][0].min(q[1][0]).min(q[2][0]), q[0][1].min(q[1][1]).min(q[2][1])].map(|x| (x / COVER_CELL).floor() as i32);
			let hi = [q[0][0].max(q[1][0]).max(q[2][0]), q[0][1].max(q[1][1]).max(q[2][1])].map(|x| (x / COVER_CELL).floor() as i32);
			if hi[0] - lo[0] > 64 || hi[1] - lo[1] > 64 {
				continue;
			}
			for cx in lo[0]..=hi[0] {
				for cy in lo[1]..=hi[1] {
					g.entry((cx, cy)).or_default().push((ii as u32, fi as u32));
				}
			}
		}
	}
	g
}

impl Cover<'_> {
	/// A grass tie face's colour as drawn at barycentric `w`: texel x the baked vertex colour (byte scale, 0x80 = 1), and that
	/// colour (1 = 0x80).
	fn tie_seen(&self, ii: usize, fi: usize, w: [f32; 3]) -> ([f32; 4], [f32; 3]) {
		let m = &self.ties[ii].lods[0];
		let (key, f) = m.faces[fi];
		let Some(img) = self.ttex.get(&(key & crate::mobies::KEY_MASK)) else { return ([0.0; 4], [0.0; 3]) };
		let ix = f.map(|i| i as usize);
		let uv: [f32; 2] = std::array::from_fn(|j| (0..3).map(|v| m.uvs[ix[v]][j] * w[v]).sum());
		let shade: [f32; 3] = std::array::from_fn(|j| (0..3).map(|v| m.colours.get(ix[v]).map_or(128.0, |c| c[j]) / 128.0 * w[v]).sum::<f32>().max(0.0));
		let tx = texel3(img, uv[0], uv[1]);
		([tx[0] * shade[0], tx[1] * shade[1], tx[2] * shade[2], tx[3]].map(|x| x.min(255.0)), shade)
	}
}

/// Debug probe classes (`decal_probe_col`): a texel under this alpha (of 255) counts as see-through, a darker one (as drawn)
/// as a shade blob.
const DECAL_A: f32 = 64.0;
const DECAL_LUM: f32 = 45.0;
/// A plant class is copied only if each texture on at least this share of its faces is green as drawn.
const TEX_SHARE: f32 = 0.15;
/// The shrub chain level the decal test reads (see `Cover::shrub_tex`).
const DECAL_MIP: usize = 2;

/// Every shrub face (instance, face) on a `COVER_CELL` grid by its XY bounds.
fn shrub_grid(shrub_set: &[shrubs::ShrubInstance]) -> CoverGrid {
	let mut g: CoverGrid = HashMap::new();
	for (si, s) in shrub_set.iter().enumerate() {
		let m = &s.mesh;
		for (fi, &(_, f)) in m.faces.iter().enumerate() {
			let q = f.map(|i| m.positions[i as usize]);
			let lo = [q[0][0].min(q[1][0]).min(q[2][0]), q[0][1].min(q[1][1]).min(q[2][1])].map(|x| (x / COVER_CELL).floor() as i32);
			let hi = [q[0][0].max(q[1][0]).max(q[2][0]), q[0][1].max(q[1][1]).max(q[2][1])].map(|x| (x / COVER_CELL).floor() as i32);
			if hi[0] - lo[0] > 64 || hi[1] - lo[1] > 64 {
				continue;
			}
			for cx in lo[0]..=hi[0] {
				for cy in lo[1]..=hi[1] {
					g.entry((cx, cy)).or_default().push((si as u32, fi as u32));
				}
			}
		}
	}
	g
}

/// Every near-LOD terrain face (tfrag, face) on a `COVER_CELL` grid by its XY bounds.
fn cover_grid(scene: &TfView) -> CoverGrid {
	let parts: Vec<Vec<((i32, i32), (u32, u32))>> = scene
		.set
		.par_iter()
		.enumerate()
		.map(|(ti, t)| {
			let m = &t.near;
			let mut v = Vec::new();
			for (fi, &(_, f)) in m.faces.iter().enumerate() {
				let q = f.map(|i| m.positions[i as usize]);
				let lo = [q[0][0].min(q[1][0]).min(q[2][0]), q[0][1].min(q[1][1]).min(q[2][1])].map(|x| (x / COVER_CELL).floor() as i32);
				let hi = [q[0][0].max(q[1][0]).max(q[2][0]), q[0][1].max(q[1][1]).max(q[2][1])].map(|x| (x / COVER_CELL).floor() as i32);
				if hi[0] - lo[0] > 64 || hi[1] - lo[1] > 64 {
					continue;
				}
				for cx in lo[0]..=hi[0] {
					for cy in lo[1]..=hi[1] {
						v.push(((cx, cy), (ti as u32, fi as u32)));
					}
				}
			}
			v
		})
		.collect();
	let mut g: CoverGrid = HashMap::new();
	for v in parts {
		for (k, f) in v {
			g.entry(k).or_default().push(f);
		}
	}
	g
}

/// Barycentric weights of `p` in triangle `q` seen from above, if inside.
fn bary_xy(q: [[f32; 3]; 3], p: [f32; 3]) -> Option<[f32; 3]> {
	let d = (q[1][1] - q[2][1]) * (q[0][0] - q[2][0]) + (q[2][0] - q[1][0]) * (q[0][1] - q[2][1]);
	if d.abs() < 1e-9 {
		return None;
	}
	let a = ((q[1][1] - q[2][1]) * (p[0] - q[2][0]) + (q[2][0] - q[1][0]) * (p[1] - q[2][1])) / d;
	let b = ((q[2][1] - q[0][1]) * (p[0] - q[2][0]) + (q[0][0] - q[2][0]) * (p[1] - q[2][1])) / d;
	let c = 1.0 - a - b;
	(a >= -1e-4 && b >= -1e-4 && c >= -1e-4).then_some([a, b, c])
}

/// Another terrain layer at the point (within `COVER_DZ`) whose texel there is opaque and not leafy as drawn: a path or decal
/// over it. A leafy layer (Eudora's cut-out leaf borders: a grey texel the vertex colour turns dark green) lets the grass grow
/// through; read as a raw texel it covered, and the bare borders showed as dark squares in the grass.
/// `under`: the ground's own colour as drawn at p.
fn covered(scene: &TfView, textures: &HashMap<u32, &Texture>, cover: &Cover, own: (usize, usize), p: [f32; 3], under: [f32; 3]) -> bool {
	if decal_over(cover, p, under) {
		return true;
	}
	let cell = ((p[0] / COVER_CELL).floor() as i32, (p[1] / COVER_CELL).floor() as i32);
	let Some(list) = cover.tf.get(&cell) else { return false };
	for &(tj, fj) in list {
		if (tj as usize, fj as usize) == own {
			continue;
		}
		let m = &scene.set[tj as usize].near;
		let (key, f) = m.faces[fj as usize];
		let q = f.map(|i| m.positions[i as usize]);
		let Some(w) = bary_xy(q, p) else { continue };
		let z = q[0][2] * w[0] + q[1][2] * w[1] + q[2][2] * w[2];
		if (z - p[2]).abs() > COVER_DZ {
			continue;
		}
		let Some(img) = textures.get(&(key & 0xff)) else { continue };
		let (seen, _) = terrain_seen(scene, img, tj as usize, f, w);
		if seen[3] >= 128.0 && !leafy(seen) {
			return true;
		}
	}
	false
}

/// The terrain colour as drawn at a near-LOD face point (barycentric `w`): texel x the relit vertex colour of the vertices'
/// position records (`terrain::pose`; byte scale), the batch colour for a fragment without lit colours.
fn terrain_seen(scene: &TfView, img: &Texture, ti: usize, f: [u32; 3], w: [f32; 3]) -> ([f32; 4], [f32; 3]) {
	let t = &scene.set[ti];
	let m = &t.near;
	let ix = f.map(|i| i as usize);
	let uv: [f32; 2] = std::array::from_fn(|j| (0..3).map(|v| m.uvs[ix[v]][j] * w[v]).sum());
	let cols = &scene.cols[ti];
	let vr = scene.vranges.get(ti).map_or(0, |r| r[2].start);
	let vcol = |v: usize, j: usize| match t.vert_pos.get(ix[v]).and_then(|&q| cols.get(q as usize)) {
		Some(c) => (c[j] / 128.0).min(255.0 / 128.0),
		None => scene.verts.get(vr + ix[v]).map_or(1.0, |x| x.colour[j]),
	};
	let shade: [f32; 3] = std::array::from_fn(|j| (0..3).map(|v| vcol(v, j) * w[v]).sum::<f32>().max(0.0));
	let tx = texel3(img, uv[0], uv[1]);
	([tx[0] * shade[0], tx[1] * shade[1], tx[2] * shade[2], tx[3]].map(|x| x.min(255.0)), shade)
}

/// Whether the ground at p (XY; the near terrain face nearest p's height, within 1 m, that is not see-through there) is drawn
/// as grass: a grass texture opaque and green as drawn, or a leafy layer (`leafy`), no decal over it. No face: not grass (a
/// cliff edge, the water).
fn grass_at(scene: &TfView, textures: &HashMap<u32, &Texture>, grass: &HashMap<u32, f32>, cover: &Cover, p: [f32; 3]) -> bool {
	let cell = ((p[0] / COVER_CELL).floor() as i32, (p[1] / COVER_CELL).floor() as i32);
	let mut best: Option<(f32, f32, Option<[f32; 3]>)> = None;
	for &(tj, fj) in cover.tf.get(&cell).map_or(&[][..], |v| &v[..]) {
		let m = &scene.set[tj as usize].near;
		let (key, f) = m.faces[fj as usize];
		let q = f.map(|i| m.positions[i as usize]);
		let Some(w) = bary_xy(q, p) else { continue };
		let z = q[0][2] * w[0] + q[1][2] * w[1] + q[2][2] * w[2];
		let dz = (z - p[2]).abs();
		if dz > 1.0 || best.is_some_and(|b| b.0 <= dz) {
			continue;
		}
		let tex = key & 0xff;
		let Some(img) = textures.get(&tex) else { continue };
		let (seen, _) = terrain_seen(scene, img, tj as usize, f, w);
		if seen[3] < 128.0 {
			continue;
		}
		let ok = if grass.contains_key(&tex) { seen[3] >= 200.0 && green(seen) } else { leafy(seen) };
		best = Some((dz, z, ok.then_some([seen[0], seen[1], seen[2]])));
	}
	// The grass-topped ties are ground as well (Eudora's raised grass blocks).
	for &(ii, fi) in cover.tg.get(&cell).map_or(&[][..], |v| &v[..]) {
		let m = &cover.ties[ii as usize].lods[0];
		let q = m.faces[fi as usize].1.map(|i| m.positions[i as usize]);
		let Some(w) = bary_xy(q, p) else { continue };
		let z = q[0][2] * w[0] + q[1][2] * w[1] + q[2][2] * w[2];
		let dz = (z - p[2]).abs();
		if dz > 1.0 || best.is_some_and(|b| b.0 <= dz) {
			continue;
		}
		let (seen, _) = cover.tie_seen(ii as usize, fi as usize, w);
		if seen[3] < 128.0 {
			continue;
		}
		best = Some((dz, z, (seen[3] >= 200.0 && green(seen)).then_some([seen[0], seen[1], seen[2]])));
	}
	best.is_some_and(|(_, z, ok)| ok.is_some_and(|under| !decal_over(cover, [p[0], p[1], z], under)))
}

/// Whether the shrub decals drawn over p (within `COVER_DZ`) turn the ground there (`under`, as drawn) from green to not green:
/// each decal blended over it by its texel alpha (a dirt path, a stepping stone; a faint dry patch or a shade blob keeps the
/// grass).
fn decal_over(cover: &Cover, p: [f32; 3], under: [f32; 3]) -> bool {
	let mut col = under;
	let mut any = false;
	let cell = ((p[0] / COVER_CELL).floor() as i32, (p[1] / COVER_CELL).floor() as i32);
	for &(si, fi) in cover.sh.get(&cell).map_or(&[][..], |v| &v[..]) {
		let m = &cover.shrubs[si as usize].mesh;
		let (key, f) = m.faces[fi as usize];
		let q = f.map(|i| m.positions[i as usize]);
		let Some(w) = bary_xy(q, p) else { continue };
		let z = q[0][2] * w[0] + q[1][2] * w[1] + q[2][2] * w[2];
		if (z - p[2]).abs() > COVER_DZ {
			continue;
		}
		let (Some(img), wrap) = cover.shrub_tex(key) else { continue };
		let ix = f.map(|i| i as usize);
		let uv: [f32; 2] = std::array::from_fn(|j| {
			let x: f32 = (0..3).map(|v| m.uvs[ix[v]][j] * w[v]).sum();
			if wrap >> j & 1 != 0 { x.clamp(0.0, 0.9999) } else { x }
		});
		let t = texel3(img, uv[0], uv[1]);
		// As drawn: texel x baked vertex colour (byte scale, 0x80 = 1; `shrub_rgba`); the path decals get their brown from it.
		let shade: [f32; 3] = std::array::from_fn(|j| (0..3).map(|v| m.colours.get(ix[v]).map_or(128.0, |c| c[j]).min(255.0) / 128.0 * w[v]).sum());
		let a = (t[3] / 255.0).min(1.0);
		col = std::array::from_fn(|j| col[j] + ((t[j] * shade[j]).min(255.0) - col[j]) * a);
		any = true;
	}
	any && !leafy([col[0], col[1], col[2], 255.0])
}

/// Candidate spots at `per_m2` over the grass faces of every near-LOD terrain fragment (salt separates the kinds' streams).
fn spots(scene: &TfView, textures: &HashMap<u32, &Texture>, grass: &HashMap<u32, f32>, cover: &Cover, world: &World, per_m2: f32, salt: u64) -> (Vec<Spot>, Census) {
	let r: Vec<(Vec<Spot>, Census)> = scene
		.set
		.par_iter()
		.enumerate()
		.map(|(ti, t)| {
			let (mut out, mut cs) = (Vec::new(), Census::default());
			let Some(vr) = scene.vranges.get(ti).map(|r| r[2].clone()) else { return (out, cs) };
			let m = &t.near;
			for (fi, &(key, f)) in m.faces.iter().enumerate() {
				let tex = key & 0xff;
				if !grass.contains_key(&tex) {
					continue;
				}
				let Some(img) = textures.get(&tex) else { continue };
				let [a, b, c] = f.map(|i| m.positions[i as usize]);
				let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
				let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
				let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
				let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
				if len < 1e-6 || (n[2] / len).abs() < COS_SLOPE {
					continue;
				}
				let s = n[2].signum();
				let n = n.map(|x| x * s / len);
				let want = len * 0.5 * per_m2;
				let mut rng = Rng::new((ti as u64) << 24 | fi as u64, salt);
				let mut k = want.floor() as usize;
				if rng.next() < want.fract() {
					k += 1;
				}
				for _ in 0..k {
					let (r1, r2) = (rng.next(), rng.next());
					let rs: [f32; 6] = std::array::from_fn(|_| rng.next());
					let q = r1.sqrt();
					let w = [1.0 - q, q * (1.0 - r2), q * r2];
					let ix = f.map(|i| i as usize);
					let p: [f32; 3] = std::array::from_fn(|j| a[j] * w[0] + b[j] * w[1] + c[j] * w[2]);
					let uv: [f32; 2] = std::array::from_fn(|j| (0..3).map(|v| m.uvs[ix[v]][j] * w[v]).sum());
					// The drawn vertex colour: the relit record of the vertex's position (`terrain::pose`, `cols` = the tfrag's lit
					// colours, byte scale); the batch vertex colour stands in for a fragment without lit colours.
					let cols = &scene.cols[ti];
					let vcol = |v: usize, j: usize| match t.vert_pos.get(ix[v]).and_then(|&q| cols.get(q as usize)) {
						Some(c) => (c[j] / 128.0).min(255.0 / 128.0),
						None => scene.verts.get(vr.start + ix[v]).map_or(1.0, |x| x.colour[j]),
					};
					let shade: [f32; 3] = std::array::from_fn(|j| (0..3).map(|v| vcol(v, j) * w[v]).sum());
					cs.tried += 1;
					let tx = texel3(img, uv[0], uv[1]);
					// The ground as drawn: texel x baked vertex colour (the levels paint their paths and dry patches brown over the
					// grass texture with the vertex colours).
					let seen = [tx[0] * shade[0].max(0.0), tx[1] * shade[1].max(0.0), tx[2] * shade[2].max(0.0), tx[3]].map(|x| x.min(255.0));
					// Why the spot is dropped (0 kept, 1 not grass, 2 covered, 3 no ground, 4 ground off, 5 bad surface, 6 dirt margin).
					let why = if tx[3] < 200.0 || !green(seen) {
						1
					} else if covered(scene, textures, cover, (ti, fi), p, [seen[0], seen[1], seen[2]]) {
						2
					} else {
						match world.line([p[0], p[1], p[2] + RAY_UP], [p[0], p[1], p[2] - RAY_DOWN], crate::physics::qmask::TWO_SIDED) {
							None => 3,
							Some(h) if (h.p[2] - p[2]).abs() > GROUND_TOL => 4,
							Some(h) if BAD_SURF.contains(&(h.kind & 0x1f)) => 5,
							_ => 0,
						}
					};
					// The dirt margin: every point of a ring of `EDGE_R` around the clump must be grass too.
					let ring = |r: f32, n: usize| {
						(0..n).all(|k| {
							let a = (k as f32 + rs[4]) * std::f32::consts::TAU / n as f32;
							grass_at(scene, textures, grass, cover, [p[0] + r * a.cos(), p[1] + r * a.sin(), p[2]])
						})
					};
					let why = if why == 0 && !ring(EDGE_R, 6) { 6 } else { why };
					let short = why == 0 && !ring(EDGE_SHORT, 8);
					match why {
						1 => cs.not_green += 1,
						2 => cs.covered += 1,
						3 => cs.no_hit += 1,
						4 => cs.far += 1,
						5 => cs.bad += 1,
						6 => cs.edge += 1,
						_ => {}
					}
					if why != 0 && !debug_why() {
						continue;
					}
					if debug_probe() {
						const PROBE: [[f32; 3]; 6] = [[1.0, 1.0, 1.0], [1.0, 0.0, 0.0], [0.0, 0.2, 1.0], [1.0, 1.0, 0.0], [0.3, 1.0, 0.0], [0.0, 1.0, 1.0]];
						let (code, seen) = decal_probe_col(cover, p);
						let col = if std::env::var("RAC_VEG_PROBE_COL").is_ok() { seen } else { PROBE[code] };
						out.push(Spot { p, slope: [0.0, 0.0], col, lum: 1.0, r: rs, tex: u32::MAX - code as u32, short });
						continue;
					}
					if why != 0 {
						const WHY: [[f32; 3]; 7] = [[1.0, 1.0, 1.0], [1.0, 0.0, 1.0], [0.0, 1.0, 1.0], [1.0, 0.0, 0.0], [0.0, 0.2, 1.0], [0.0, 0.0, 0.0], [1.0, 0.5, 0.0]];
						out.push(Spot { p, slope: [0.0, 0.0], col: WHY[why], lum: 1.0, r: rs, tex: u32::MAX - why as u32, short });
						continue;
					}
					cs.kept += 1;
					let col = std::array::from_fn(|j| tx[j] / 255.0 * shade[j].max(0.0));
					out.push(Spot { p, slope: [-n[0] / n[2].max(0.2), -n[1] / n[2].max(0.2)], col, lum: lum(shade), r: rs, tex, short });
				}
			}
			(out, cs)
		})
		.collect();
	let mut all = Vec::new();
	let mut cs = Census::default();
	for (v, c) in r.into_iter().chain(tie_spots(scene, textures, grass, cover, world, per_m2, salt)) {
		all.extend(v);
		cs = cs.add(c);
	}
	(all, cs)
}

/// Candidate spots on the grass-topped ties (`Cover::tg`; Eudora's raised grass blocks, drawn as ties, not terrain): the same
/// tests as the terrain spots, the decals only for cover (the terrain under a block is not over it).
fn tie_spots(scene: &TfView, textures: &HashMap<u32, &Texture>, grass: &HashMap<u32, f32>, cover: &Cover, world: &World, per_m2: f32, salt: u64) -> Vec<(Vec<Spot>, Census)> {
	let mut faces: Vec<(u32, u32)> = cover.tg.values().flatten().copied().collect();
	faces.sort_unstable();
	faces.dedup();
	faces
		.par_chunks(256)
		.map(|chunk| {
			let (mut out, mut cs) = (Vec::new(), Census::default());
			for &(ii, fi) in chunk {
				let m = &cover.ties[ii as usize].lods[0];
				let f = m.faces[fi as usize].1;
				let [a, b, c] = f.map(|i| m.positions[i as usize]);
				let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
				let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
				let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
				let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
				// Up only: a block's sides and its underside get nothing.
				if len < 1e-6 || n[2] / len < COS_SLOPE {
					continue;
				}
				let n = n.map(|x| x / len);
				let want = len * 0.5 * per_m2;
				let mut rng = Rng::new(1 << 62 | (ii as u64) << 24 | fi as u64, salt);
				let mut k = want.floor() as usize;
				if rng.next() < want.fract() {
					k += 1;
				}
				for _ in 0..k {
					let (r1, r2) = (rng.next(), rng.next());
					let rs: [f32; 6] = std::array::from_fn(|_| rng.next());
					let q = r1.sqrt();
					let w = [1.0 - q, q * (1.0 - r2), q * r2];
					let p: [f32; 3] = std::array::from_fn(|j| a[j] * w[0] + b[j] * w[1] + c[j] * w[2]);
					cs.tried += 1;
					let (seen, shade) = cover.tie_seen(ii as usize, fi as usize, w);
					let why = if seen[3] < 200.0 || !green(seen) {
						1
					} else if decal_over(cover, p, [seen[0], seen[1], seen[2]]) {
						2
					} else {
						let block = cover.blocks.contains(&ii);
						match world.line([p[0], p[1], p[2] + RAY_UP], [p[0], p[1], p[2] - RAY_DOWN], crate::physics::qmask::TWO_SIDED) {
							// A grass block's top: no floor of its own; only a floor over it (the block buried) keeps it bare.
							None if block => 0,
							Some(h) if block && h.p[2] < p[2] - GROUND_TOL => 0,
							None => 3,
							Some(h) if (h.p[2] - p[2]).abs() > GROUND_TOL => 4,
							Some(h) if BAD_SURF.contains(&(h.kind & 0x1f)) => 5,
							_ => 0,
						}
					};
					let ring = |r: f32, n: usize| {
						(0..n).all(|k| {
							let a = (k as f32 + rs[4]) * std::f32::consts::TAU / n as f32;
							grass_at(scene, textures, grass, cover, [p[0] + r * a.cos(), p[1] + r * a.sin(), p[2]])
						})
					};
					let why = if why == 0 && !ring(EDGE_R, 6) { 6 } else { why };
					let short = why == 0 && !ring(EDGE_SHORT, 8);
					match why {
						1 => cs.not_green += 1,
						2 => cs.covered += 1,
						3 => cs.no_hit += 1,
						4 => cs.far += 1,
						5 => cs.bad += 1,
						6 => cs.edge += 1,
						_ => {}
					}
					if why != 0 && !debug_why() {
						continue;
					}
					if why != 0 {
						const WHY: [[f32; 3]; 7] = [[1.0, 1.0, 1.0], [1.0, 0.0, 1.0], [0.0, 1.0, 1.0], [1.0, 0.0, 0.0], [0.0, 0.2, 1.0], [0.0, 0.0, 0.0], [1.0, 0.5, 0.0]];
						out.push(Spot { p, slope: [0.0, 0.0], col: WHY[why], lum: 1.0, r: rs, tex: u32::MAX - why as u32, short });
						continue;
					}
					cs.kept += 1;
					let col = std::array::from_fn(|j| seen[j] / 255.0);
					out.push(Spot { p, slope: [-n[0] / n[2].max(0.2), -n[1] / n[2].max(0.2)], col, lum: lum(shade), r: rs, tex: 0x1_0000 | ii, short });
				}
			}
			(out, cs)
		})
		.collect()
}

/// One clump of `n` blades (unit height; `w` base width m, `r` clump radius m).
fn clump(n: usize, w: f32, r: f32, heads: usize, verts: &mut Vec<VegVertex>, idx: &mut Vec<u32>) {
	let mut g = Rng::new(0x6772_6173_73, (n + 100 * heads) as u64);
	let tau = std::f32::consts::TAU;
	for _ in 0..n {
		let ang = g.next() * tau;
		let rad = r * g.next().sqrt();
		let root = [rad * ang.cos(), rad * ang.sin()];
		let face = g.next() * tau;
		let side = [face.cos(), face.sin()];
		let lean_a = ang + (g.next() - 0.5) * 1.6;
		let lean = 0.1 + 0.3 * g.next();
		let h = 0.55 + 0.45 * g.next();
		let v = 0.82 + 0.3 * g.next();
		let warm = g.next() * 0.14;
		let col = [v * (1.0 + warm), v, v * (1.0 - warm * 0.6), 0.0];
		let base = verts.len() as u32;
		for (k, &f) in [0.0f32, 0.35, 0.7, 1.0].iter().enumerate() {
			let hw = w * 0.5 * (1.0 - f).powf(0.7);
			let off = lean * h * f * f;
			let c = [lean_a.cos() * off, lean_a.sin() * off, h * f];
			if k == 3 {
				verts.push(VegVertex { pos: c, root, uv: [0.0, f], col, side });
			} else {
				for s in [-1.0f32, 1.0] {
					verts.push(VegVertex { pos: c, root, uv: [s * hw, f], col, side });
				}
			}
		}
		idx.extend([0, 1, 2, 1, 3, 2, 2, 3, 4, 3, 5, 4, 4, 5, 6].map(|i| base + i));
	}
	// Flower heads: a thin stem a little over the blades, a flat six-petal disc on top (colour flag in col.w: 1 petal rim,
	// 2 centre; the colour itself is the instance's patch colour, vs_veg_blade).
	for _ in 0..heads {
		let ang = g.next() * tau;
		let rad = r * 0.8 * g.next().sqrt();
		let root = [rad * ang.cos(), rad * ang.sin()];
		let face = g.next() * tau;
		let side = [face.cos(), face.sin()];
		let lean_a = ang + (g.next() - 0.5) * 1.0;
		let lean = 0.05 + 0.15 * g.next();
		let h = 1.0 + 0.2 * g.next();
		let col = [0.9, 1.0, 0.85, 0.0];
		let base = verts.len() as u32;
		let top = |f: f32| [lean_a.cos() * lean * h * f * f, lean_a.sin() * lean * h * f * f, h * f];
		for (k, &f) in [0.0f32, 0.5, 1.0].iter().enumerate() {
			if k == 2 {
				verts.push(VegVertex { pos: top(f), root, uv: [0.0, f], col, side });
			} else {
				for sgn in [-1.0f32, 1.0] {
					verts.push(VegVertex { pos: top(f), root, uv: [sgn * w * 0.25, f], col, side });
				}
			}
		}
		idx.extend([0, 1, 2, 1, 3, 2, 2, 3, 4].map(|i| base + i));
		let c = verts.len() as u32;
		let tilt = g.next() * tau;
		verts.push(VegVertex { pos: top(1.0), root, uv: [0.0, 1.0], col: [1.0, 1.0, 1.0, 2.0], side: [0.0; 2] });
		let size = HEAD_R * (0.7 + 0.5 * g.next());
		// Five petals: the rim alternates petal tips and notches (a star, not a coin), a little raised (a shallow cup).
		let cup = [0.0, 0.0, 0.08];
		for k in 0..10 {
			let a = tilt + k as f32 * tau / 10.0;
			let len = if k % 2 == 0 { size } else { size * 0.55 };
			let p = top(1.0);
			verts.push(VegVertex { pos: [p[0] + cup[0], p[1] + cup[1], p[2] + cup[2] * (k % 2 == 0) as u32 as f32], root, uv: [len, 1.0], col: [1.0, 1.0, 1.0, 1.0], side: [a.cos(), a.sin()] });
		}
		for k in 0..10u32 {
			idx.extend([c, c + 1 + k, c + 1 + (k + 1) % 10]);
		}
	}
}

/// One seaweed clump of `n` ribbons (unit height; `w` base width m, `r` clump radius m): long, a little wider at the middle,
/// eight segments so the current can wave it (vs_veg_blade, sea flag).
fn kelp(n: usize, w: f32, r: f32, verts: &mut Vec<VegVertex>, idx: &mut Vec<u32>) {
	let mut g = Rng::new(0x6b65_6c70, n as u64);
	let tau = std::f32::consts::TAU;
	const SEG: usize = 8;
	for _ in 0..n {
		let ang = g.next() * tau;
		let rad = r * g.next().sqrt();
		let root = [rad * ang.cos(), rad * ang.sin()];
		let face = g.next() * tau;
		let side = [face.cos(), face.sin()];
		let lean_a = ang + (g.next() - 0.5) * 1.2;
		let lean = 0.05 + 0.2 * g.next();
		let h = 0.6 + 0.4 * g.next();
		let v = 0.8 + 0.35 * g.next();
		let col = [v, v, v * (0.8 + 0.3 * g.next()), 0.0];
		let base = verts.len() as u32;
		for k in 0..=SEG {
			let f = k as f32 / SEG as f32;
			let hw = w * 0.5 * (0.55 + 0.45 * (std::f32::consts::PI * f).sin()) * (1.0 - f * f * f);
			let off = lean * h * f;
			let c = [lean_a.cos() * off, lean_a.sin() * off, h * f];
			if k == SEG {
				verts.push(VegVertex { pos: c, root, uv: [0.0, f], col, side });
			} else {
				for sg in [-1.0f32, 1.0] {
					verts.push(VegVertex { pos: c, root, uv: [sg * hw, f], col, side });
				}
			}
		}
		for k in 0..SEG as u32 - 1 {
			idx.extend([0, 1, 2, 1, 3, 2].map(|i| base + 2 * k + i));
		}
		let t = 2 * (SEG as u32 - 1);
		idx.extend([t, t + 1, t + 2].map(|i| base + i));
	}
}

/// Smooth value noise in 0..1 at p / `SEA_PATCH` (the seaweed patches).
fn patch_noise(x: f32, y: f32) -> f32 {
	let (fx, fy) = (x / SEA_PATCH, y / SEA_PATCH);
	let (ix, iy) = (fx.floor(), fy.floor());
	let h = |a: f32, b: f32| Rng::new(((a as i64 as u64) << 32) ^ (b as i64 as u64 & 0xffff_ffff), 77).next();
	let (tx, ty) = (fx - ix, fy - iy);
	let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
	let a = h(ix, iy) + (h(ix + 1.0, iy) - h(ix, iy)) * sx;
	let b = h(ix, iy + 1.0) + (h(ix + 1.0, iy + 1.0) - h(ix, iy + 1.0)) * sx;
	a + (b - a) * sy
}

/// Seaweed points: on the near terrain faces facing up (normal z >= 0.6, any texture) in noise patches, where the first thing
/// a collision ray from 16 m above meets is a water surface (`WATER_SURF`) between `SEA_DEPTH` above the point, and the bed
/// has collision right there. Spot.r[2] carries the water depth (m).
fn sea_spots(scene: &TfView, world: &World, per_m2: f32) -> (Vec<Spot>, [usize; 4]) {
	let r: Vec<(Vec<Spot>, [usize; 4])> = scene
		.set
		.par_iter()
		.enumerate()
		.map(|(ti, t)| {
			let (mut out, mut cs) = (Vec::new(), [0usize; 4]);
			let m = &t.near;
			for (fi, &(key, f)) in m.faces.iter().enumerate() {
				let [a, b, c] = f.map(|i| m.positions[i as usize]);
				let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
				let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
				let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
				let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
				if len < 1e-6 || (n[2] / len).abs() < 0.6 {
					continue;
				}
				let s = n[2].signum();
				let n = n.map(|x| x * s / len);
				let want = len * 0.5 * per_m2;
				let mut rng = Rng::new((ti as u64) << 24 | fi as u64, 5);
				let mut k = want.floor() as usize;
				if rng.next() < want.fract() {
					k += 1;
				}
				for _ in 0..k {
					let (r1, r2) = (rng.next(), rng.next());
					let rs: [f32; 6] = std::array::from_fn(|_| rng.next());
					let q = r1.sqrt();
					let w = [1.0 - q, q * (1.0 - r2), q * r2];
					let p: [f32; 3] = std::array::from_fn(|j| a[j] * w[0] + b[j] * w[1] + c[j] * w[2]);
					let nz = patch_noise(p[0], p[1]);
					let cover = ((nz - SEA_COVER.0) / (SEA_COVER.1 - SEA_COVER.0)).clamp(0.0, 1.0);
					if rs[5] >= cover {
						continue;
					}
					cs[0] += 1;
					let depth = match world.line([p[0], p[1], p[2] + 16.0], [p[0], p[1], p[2] + 0.15], crate::physics::qmask::TWO_SIDED) {
						Some(h) if WATER_SURF.contains(&(h.kind & 0x1f)) => h.p[2] - p[2],
						_ => continue,
					};
					if !(SEA_DEPTH.0..=SEA_DEPTH.1).contains(&depth) {
						continue;
					}
					cs[1] += 1;
					match world.line([p[0], p[1], p[2] + 0.5], [p[0], p[1], p[2] - 0.8], crate::physics::qmask::TWO_SIDED) {
						Some(h) if (h.p[2] - p[2]).abs() <= GROUND_TOL => {}
						_ => continue,
					}
					cs[2] += 1;
					let mut r = rs;
					r[2] = depth;
					out.push(Spot { p, slope: [-n[0] / n[2].max(0.2), -n[1] / n[2].max(0.2)], col: [0.0; 3], lum: SEA_LUM, r, tex: key & 0xff, short: false });
				}
			}
			(out, cs)
		})
		.collect();
	let mut all = Vec::new();
	let mut cs = [0usize; 4];
	for (v, c) in r {
		all.extend(v);
		for k in 0..4 {
			cs[k] += c[k];
		}
	}
	(all, cs)
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn norm3(a: [f32; 3]) -> [f32; 3] {
	let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt().max(1e-9);
	a.map(|x| x / l)
}

/// One leaf cluster of `n` leaves (model z = the crown's outward normal, m): each leaf starts a little inside the crown within
/// `LEAF_SPREAD` of the centre and points outward, tilted 25 to 80 degrees off the normal; a pointed oval folded along its vein.
/// uv: (signed width, length fraction); col: the leaf's shade, a = length fraction (flutter weight).
fn leaf_cluster(n: usize, verts: &mut Vec<VegVertex>, idx: &mut Vec<u32>) {
	let mut g = Rng::new(0x6c65_6166, n as u64);
	let tau = std::f32::consts::TAU;
	for _ in 0..n {
		let (ang, rad) = (g.next() * tau, LEAF_SPREAD * g.next().sqrt());
		let base = [rad * ang.cos(), rad * ang.sin(), -0.08 + 0.1 * g.next()];
		let (tilt, az) = ((25.0 + 55.0 * g.next()).to_radians(), g.next() * tau);
		let dir = [tilt.sin() * az.cos(), tilt.sin() * az.sin(), tilt.cos()];
		let h = if dir[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
		let s0 = norm3(cross3(h, dir));
		let s1 = cross3(dir, s0);
		let roll = g.next() * tau;
		let side: [f32; 3] = std::array::from_fn(|k| s0[k] * roll.cos() + s1[k] * roll.sin());
		let fold = cross3(dir, side);
		let len = LEAF_LEN.0 + (LEAF_LEN.1 - LEAF_LEN.0) * g.next();
		let wid = len * (0.38 + 0.14 * g.next());
		let v = 0.75 + 0.4 * g.next();
		let warm = 0.95 + 0.12 * g.next();
		let first = verts.len() as u32;
		// Outline: stem, two points down each side, tip; the sides drop a little behind the vein (the fold).
		for (fa, wa) in [(0.0f32, 0.0f32), (0.3, -0.5), (0.68, -0.4), (1.0, 0.0), (0.68, 0.4), (0.3, 0.5)] {
			let drop = -0.12 * wid * (wa.abs() * 2.0);
			let pos: [f32; 3] = std::array::from_fn(|k| base[k] + dir[k] * len * fa + side[k] * wid * wa + fold[k] * drop);
			let sh = v * (0.78 + 0.3 * fa);
			verts.push(VegVertex { pos, root: [0.0; 2], uv: [wa, fa], col: [sh * warm, sh, sh * 0.92, fa], side: [0.0; 2] });
		}
		idx.extend([0, 1, 5, 1, 4, 5, 1, 2, 4, 2, 3, 4].map(|i| first + i));
	}
}

/// A leaf cluster spot on a crown: point, outward normal, the crown's colour as drawn there (texel 0..1 x shade), its shade
/// luminance, randoms.
struct LeafSpot {
	p: [f32; 3],
	n: [f32; 3],
	col: [f32; 3],
	lum: f32,
	r: [f32; 4],
	/// Cluster scale (1, or `SLOPE_LEAF` on grass-block slopes).
	s: f32,
	why: u8,
}

/// Leaf cluster points on one tree crown or bush (`tree_spots`), or None when `m` is not one: at least `TREE_LEAFY` (mostly
/// autumn crowns: `TREE_LEAFY_AUTUMN`) of its
/// surface leafy as drawn (texel x baked colour green in hue, opaque) and at least `TREE_M2` m2 of it. On its leafy faces,
/// `per_m2` points per m2 where the texel is leafy and opaque, the normal turned away from the leafy centre. `slopes`
/// (grass blocks on the ground): only the faces steeper than `SLOPE_NZ` (the leafy borders draping over ledges), no share
/// test, smaller clusters; the flat tops keep their grass.
fn mesh_leaves<'t>(m: &Mesh, img_of: &dyn Fn(u32) -> Option<&'t Texture>, seed: u64, per_m2: f32, slopes: bool) -> Option<Vec<LeafSpot>> {
	let seen_at = |key: u32, ix: [usize; 3], w: [f32; 3]| -> Option<([f32; 4], [f32; 3])> {
		let img = img_of(key)?;
		let uv: [f32; 2] = std::array::from_fn(|j| (0..3).map(|v| m.uvs[ix[v]][j] * w[v]).sum());
		let shade: [f32; 3] = std::array::from_fn(|j| (0..3).map(|v| m.colours.get(ix[v]).map_or(128.0, |c| c[j]) / 128.0 * w[v]).sum::<f32>().max(0.0));
		let tx = texel3(img, uv[0], uv[1]);
		Some(([tx[0] * shade[0], tx[1] * shade[1], tx[2] * shade[2], tx[3]].map(|x| x.min(255.0)), shade))
	};
	let (mut total, mut green_a, mut autumn_a, mut cen) = (0.0f32, 0.0f32, 0.0f32, [0.0f32; 3]);
	let mut leafy_faces = Vec::new();
	let log_on = std::env::var("RAC_VEG_LEAFLOG").is_ok();
	let mut by_tex: HashMap<u32, f32> = HashMap::new();
	let mut tex_g: HashMap<u32, u8> = HashMap::new();
	for (fi, &(key, f)) in m.faces.iter().enumerate() {
		let q = f.map(|i| m.positions[i as usize]);
		let nn = cross3(std::array::from_fn(|k| q[1][k] - q[0][k]), std::array::from_fn(|k| q[2][k] - q[0][k]));
		let a = 0.5 * (nn[0] * nn[0] + nn[1] * nn[1] + nn[2] * nn[2]).sqrt();
		if a < 1e-6 {
			continue;
		}
		total += a;
		if slopes && (nn[2] / (2.0 * a)).abs() >= SLOPE_NZ {
			continue;
		}
		let kind = *tex_g.entry(key).or_insert_with(|| img_of(key).map_or(0, foliage_tex));
		if kind > 0 && seen_at(key, f.map(|i| i as usize), [1.0 / 3.0; 3]).is_some_and(|(c, _)| foliage_seen(c, kind)) {
			green_a += a;
			if kind == 2 {
				autumn_a += a;
			}
			for k in 0..3 {
				cen[k] += (q[0][k] + q[1][k] + q[2][k]) / 3.0 * a;
			}
			leafy_faces.push((fi, a));
			if log_on {
				*by_tex.entry(key & 0xffff).or_insert(0.0) += a;
			}
		}
	}
	if std::env::var("RAC_VEG_LEAFLOG").is_ok() && green_a >= TREE_M2 {
		let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
		for p in &m.positions {
			for k in 0..3 {
				lo[k] = lo[k].min(p[k]);
				hi[k] = hi[k].max(p[k]);
			}
		}
		let mut tx: Vec<(u32, f32)> = by_tex.into_iter().collect();
		tx.sort_by(|a, b| b.1.total_cmp(&a.1));
		let tl: Vec<String> = tx.iter().take(3).map(|&(k, a)| {
			let st = img_of(k).map(tex_stats).map_or("-".into(), |(g, c, mn)| format!("g{g:.2} c{c:.2} m{:.0},{:.0},{:.0}", mn[0], mn[1], mn[2]));
			format!("{k:#x}:{a:.0}:{st}")
		}).collect();
		eprintln!("veg: leafcand {seed} leafy {green_a:.0}/{total:.0} x {:.1} {:.1} y {:.1} {:.1} z {:.1} {:.1} tex {}", lo[0], hi[0], lo[1], hi[1], lo[2], hi[2], tl.join(" "));
	}
	// Autumn crowns (Eudora's mushroom trees 1093-1117: 53-57 %) sit on a thick bark trunk and root base.
	let share = if autumn_a * 2.0 >= green_a { TREE_LEAFY_AUTUMN } else { TREE_LEAFY };
	if green_a < TREE_M2 || (!slopes && green_a < share * total) {
		return None;
	}
	let (per_m2, scale) = if slopes { (per_m2 * 1.5, SLOPE_LEAF) } else { (per_m2, 1.0) };
	let cen = cen.map(|x| x / green_a);
	let mut out = Vec::new();
	for (fi, a) in leafy_faces {
		let (key, f) = m.faces[fi];
		let q = f.map(|i| m.positions[i as usize]);
		let mut n = norm3(cross3(std::array::from_fn(|k| q[1][k] - q[0][k]), std::array::from_fn(|k| q[2][k] - q[0][k])));
		let fc: [f32; 3] = std::array::from_fn(|k| (q[0][k] + q[1][k] + q[2][k]) / 3.0);
		if (0..3).map(|k| n[k] * (fc[k] - cen[k])).sum::<f32>() < 0.0 {
			n = n.map(|x| -x);
		}
		let want = a * per_m2;
		let mut rng = Rng::new(seed << 24 | fi as u64, 11);
		let mut k = want.floor() as usize;
		if rng.next() < want.fract() {
			k += 1;
		}
		for _ in 0..k {
			let (r1, r2) = (rng.next(), rng.next());
			let rs: [f32; 4] = std::array::from_fn(|_| rng.next());
			let sq = r1.sqrt();
			let w = [1.0 - sq, sq * (1.0 - r2), sq * r2];
			let Some((seen, shade)) = seen_at(key, f.map(|i| i as usize), w) else { continue };
			let kind = tex_g.get(&key).copied().unwrap_or(0);
			if !foliage_seen(seen, kind) {
				continue;
			}
			// Lane debug (RAC_VEG_LEAFRED=2): why = texture kind, 4 = olive as drawn.
			let why = kind;
			let p: [f32; 3] = std::array::from_fn(|j| q[0][j] * w[0] + q[1][j] * w[1] + q[2][j] * w[2]);
			out.push(LeafSpot { p, n, col: [seen[0], seen[1], seen[2]].map(|x| x / 255.0), lum: lum(shade).max(0.05), r: rs, s: scale, why });
		}
	}
	Some(out)
}

/// Leaf cluster points on the tree crowns and bushes (`mesh_leaves`): the ties and the shrubs, with the textures as drawn
/// (the authored chain's top level). A grass block (`Cover::blocks`) counts only when it floats at least `CROWN_UP` m over
/// the floor round its middle (3 of 4 probes; a floor must be found: a pillar's sunk foot starts the probe under the ground): Eudora's
/// round crowns are flat-topped leafy ties on a trunk, stacked about 1.2 m apart; its rock pillars stand on the ground. Also the number of ties and shrubs that got leaves.
fn tree_spots(cover: &Cover, world: &World, textures: &HashMap<u32, Texture>, tie_chains: &[Option<Vec<Texture>>], per_m2: f32) -> (Vec<LeafSpot>, usize, usize) {
	let floats = |m: &Mesh| {
		let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
		for p in &m.positions {
			for k in 0..3 {
				lo[k] = lo[k].min(p[k]);
				hi[k] = hi[k].max(p[k]);
			}
		}
		// Four probes round the middle (the middle itself is the trunk).
		let probe = |fx: f32, fy: f32| {
			let (x, y) = (lo[0] + (hi[0] - lo[0]) * fx, lo[1] + (hi[1] - lo[1]) * fy);
			world.line([x, y, lo[2] - 0.05], [x, y, lo[2] - 60.0], crate::physics::qmask::TWO_SIDED).is_some_and(|h| lo[2] - h.p[2] >= CROWN_UP)
		};
		[(0.2, 0.5), (0.8, 0.5), (0.5, 0.2), (0.5, 0.8)].iter().filter(|&&(fx, fy)| probe(fx, fy)).count() >= 3
	};
	let tie_img = |key: u32| {
		let tk = key & crate::mobies::KEY_MASK;
		tie_chains.get((tk & 0xff) as usize).and_then(|c| c.as_ref()).and_then(|c| c.first()).or_else(|| textures.get(&tk))
	};
	let shrub_img = |key: u32| {
		let tk = key & crate::mobies::KEY_MASK & !shrubs::MATERIAL_BITS;
		cover.schains.get((tk & 0xff) as usize).and_then(|c| c.as_ref()).and_then(|c| c.first()).or_else(|| cover.stex.get(&tk))
	};
	if std::env::var("RAC_VEG_LEAFLOG").is_ok_and(|v| v == "2") {
		for (ii, t) in cover.ties.iter().enumerate() {
			let m = &t.lods[0];
			let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
			for p in &m.positions {
				for k in 0..3 {
					lo[k] = lo[k].min(p[k]);
					hi[k] = hi[k].max(p[k]);
				}
			}
			let mut by: HashMap<u32, (f32, [f32; 3])> = HashMap::new();
			for &(key, f) in &m.faces {
				let q = f.map(|i| m.positions[i as usize]);
				let nn = cross3(std::array::from_fn(|k| q[1][k] - q[0][k]), std::array::from_fn(|k| q[2][k] - q[0][k]));
				let a = 0.5 * (nn[0] * nn[0] + nn[1] * nn[1] + nn[2] * nn[2]).sqrt();
				let e = by.entry(key & 0xffff).or_insert((0.0, [0.0; 3]));
				e.0 += a;
				for v in f {
					let c = m.colours.get(v as usize).copied().unwrap_or([128.0; 4]);
					for k in 0..3 {
						e.1[k] += c[k] * a / 3.0;
					}
				}
			}
			let mut tx: Vec<(u32, (f32, [f32; 3]))> = by.into_iter().collect();
			tx.sort_by(|a, b| b.1 .0.total_cmp(&a.1 .0));
			let tl: Vec<String> = tx.iter().take(3).map(|&(k, (a, c))| {
				let st = tie_img(k).map(tex_stats).map_or("-".into(), |(g, cu, mn)| format!("g{g:.2} c{cu:.2} m{:.0},{:.0},{:.0}", mn[0], mn[1], mn[2]));
				format!("{k:#x}:{a:.0}:{st}:v{:.0},{:.0},{:.0}", c[0] / a.max(1e-6), c[1] / a.max(1e-6), c[2] / a.max(1e-6))
			}).collect();
			eprintln!("veg: tie {ii} x {:.1} {:.1} y {:.1} {:.1} z {:.1} {:.1} tex {}", lo[0], hi[0], lo[1], hi[1], lo[2], hi[2], tl.join(" "));
		}
	}
	// A grass block on the ground (meadow block, terrace, ledge) gets small clusters on its leafy slopes only.
	let a: Vec<Vec<LeafSpot>> = cover.ties.par_iter().enumerate().filter_map(|(ii, t)| {
		let slopes = cover.blocks.contains(&(ii as u32)) && !floats(&t.lods[0]);
		mesh_leaves(&t.lods[0], &tie_img, ii as u64, per_m2, slopes)
	}).collect();
	let b: Vec<Vec<LeafSpot>> = cover.shrubs.par_iter().enumerate().filter_map(|(si, t)| mesh_leaves(&t.mesh, &shrub_img, (1 << 20) + si as u64, per_m2, false)).collect();
	let (nt, ns) = (a.len(), b.len());
	(a.into_iter().chain(b).flatten().collect(), nt, ns)
}

/// Small leaf clusters (`SLOPE_LEAF`) on the grass-textured terrain where no grass can stand: faces steeper than
/// `COS_SLOPE` (the grassy hillsides and banks round the play area) and grass faces with no floor under them in the
/// collision (the hills out of bounds), leafy as drawn and not covered by another layer. User's Eudora screenshots
/// 2026-10-17: the big bare green hills at the map edges.
fn terrain_leaves(scene: &TfView, textures: &HashMap<u32, &Texture>, grass: &HashMap<u32, f32>, cover: &Cover, world: &World, per_m2: f32) -> Vec<LeafSpot> {
	let per_m2 = per_m2 * 1.5;
	// Only a meadow-coloured grass texture (mean green over red and blue, not neon): Quartu's glowing green and teal panels
	// (3,175,1 / 20,142,121) and pale or olive rock (Eudora 152,161,145, Novalis 98,101,68) pass the blades' grass share but
	// would leaf whole walls.
	let meadow: std::collections::HashSet<u32> = grass
		.keys()
		.copied()
		.filter(|t| textures.get(t).is_some_and(|img| {
			let (g, _, [r, gg, b]) = tex_stats(img);
			g >= 0.5 && gg >= r * 1.15 && r >= gg * 0.3 && b <= gg * 0.7
		}))
		.collect();
	let r: Vec<Vec<LeafSpot>> = scene
		.set
		.par_iter()
		.enumerate()
		.map(|(ti, t)| {
			let mut out = Vec::new();
			let m = &t.near;
			for (fi, &(key, f)) in m.faces.iter().enumerate() {
				let tex = key & 0xff;
				if !meadow.contains(&tex) {
					continue;
				}
				let Some(img) = textures.get(&tex) else { continue };
				let q = f.map(|i| m.positions[i as usize]);
				let nn = cross3(std::array::from_fn(|k| q[1][k] - q[0][k]), std::array::from_fn(|k| q[2][k] - q[0][k]));
				let len = (nn[0] * nn[0] + nn[1] * nn[1] + nn[2] * nn[2]).sqrt();
				if len < 1e-6 {
					continue;
				}
				let steep = (nn[2] / len).abs() < COS_SLOPE;
				let s = if nn[2] < 0.0 { -1.0 } else { 1.0 };
				let n = nn.map(|x| x * s / len);
				let want = len * 0.5 * per_m2;
				let mut rng = Rng::new((ti as u64) << 24 | fi as u64, 23);
				let mut k = want.floor() as usize;
				if rng.next() < want.fract() {
					k += 1;
				}
				for _ in 0..k {
					let (r1, r2) = (rng.next(), rng.next());
					let rs: [f32; 4] = std::array::from_fn(|_| rng.next());
					let sq = r1.sqrt();
					let w = [1.0 - sq, sq * (1.0 - r2), sq * r2];
					let (seen, shade) = terrain_seen(scene, img, ti, f, w);
					if !foliage_seen(seen, 1) {
						continue;
					}
					let p: [f32; 3] = std::array::from_fn(|j| q[0][j] * w[0] + q[1][j] * w[1] + q[2][j] * w[2]);
					if covered(scene, textures, cover, (ti, fi), p, [seen[0], seen[1], seen[2]]) {
						continue;
					}
					// A flat grass face with a floor under it in the collision gets the grass blades instead.
					if !steep && world.line([p[0], p[1], p[2] + RAY_UP], [p[0], p[1], p[2] - RAY_DOWN], crate::physics::qmask::TWO_SIDED).is_some_and(|h| (h.p[2] - p[2]).abs() <= GROUND_TOL) {
						continue;
					}
					out.push(LeafSpot { p, n, col: [seen[0], seen[1], seen[2]].map(|x| x / 255.0), lum: lum(shade).max(0.05), r: rs, s: SLOPE_LEAF, why: 1 });
				}
			}
			out
		})
		.collect();
	r.into_iter().flatten().collect()
}

/// What the planner reads of the terrain: the near mesh of every terrain fragment and the lit colours the scene draws it with.
#[derive(Clone, Copy)]
struct TfRef<'a> {
	near: &'a Mesh,
	vert_pos: &'a [u32],
}

struct TfView<'a> {
	set: Vec<TfRef<'a>>,
	cols: &'a [Vec<[f32; 4]>],
	vranges: &'a [[std::ops::Range<usize>; 3]],
	verts: &'a [Vertex],
	/// The scene the texture slots belong to (tfrag count, radius bits).
	id: (usize, u32),
}

impl<'a> TfView<'a> {
	fn of(scene: &'a Scene) -> TfView<'a> {
		TfView { set: scene.tfrag_set.iter().map(|t| TfRef { near: &t.near, vert_pos: &t.vert_pos }).collect(), cols: &scene.tfrag_cols, vranges: &scene.tfrag_vranges, verts: &scene.tfrag_verts, id: (scene.tfrag_set.len(), scene.radius.to_bits()) }
	}
}

/// A copy of `TfView`'s data for the replan thread (made when the row changes, the colours as they are then).
struct TfOwned {
	near: Vec<Mesh>,
	vert_pos: Vec<Vec<u32>>,
	cols: Vec<Vec<[f32; 4]>>,
	vranges: Vec<[std::ops::Range<usize>; 3]>,
	verts: Vec<Vertex>,
	id: (usize, u32),
}

impl TfOwned {
	fn of(scene: &Scene) -> TfOwned {
		TfOwned { near: scene.tfrag_set.iter().map(|t| t.near.clone()).collect(), vert_pos: scene.tfrag_set.iter().map(|t| t.vert_pos.clone()).collect(), cols: scene.tfrag_cols.clone(), vranges: scene.tfrag_vranges.clone(), verts: scene.tfrag_verts.clone(), id: (scene.tfrag_set.len(), scene.radius.to_bits()) }
	}

	fn view(&self) -> TfView<'_> {
		TfView { set: self.near.iter().zip(&self.vert_pos).map(|(m, v)| TfRef { near: m, vert_pos: v }).collect(), cols: &self.cols, vranges: &self.vranges, verts: &self.verts, id: self.id }
	}
}

/// The level data the planner reads besides the terrain and the collision world.
struct VegIn<'a> {
	shrub_set: &'a [shrubs::ShrubInstance],
	textures: &'a HashMap<u32, Texture>,
	terrain_chains: &'a [Option<Vec<Texture>>],
	shrub_chains: &'a [Option<Vec<Texture>>],
	tie_set: &'a [ties::LitTie],
	tie_chains: &'a [Option<Vec<Texture>>],
	foot: &'a HashMap<u32, f32>,
	n_mobies: usize,
	level: usize,
}

/// `VegIn` kept after the level load (`Gpu::keep_veg_src`) so a Vegetation change replans without a reload.
pub(super) struct VegSrc {
	shrub_set: Vec<shrubs::ShrubInstance>,
	textures: HashMap<u32, Texture>,
	terrain_chains: Vec<Option<Vec<Texture>>>,
	shrub_chains: Vec<Option<Vec<Texture>>>,
	tie_set: Vec<ties::LitTie>,
	tie_chains: Vec<Option<Vec<Texture>>>,
	foot: HashMap<u32, f32>,
	n_mobies: usize,
	level: usize,
}

impl VegSrc {
	fn as_in(&self) -> VegIn<'_> {
		VegIn { shrub_set: &self.shrub_set, textures: &self.textures, terrain_chains: &self.terrain_chains, shrub_chains: &self.shrub_chains, tie_set: &self.tie_set, tie_chains: &self.tie_chains, foot: &self.foot, n_mobies: self.n_mobies, level: self.level }
	}
}

/// The CPU half of a plan: everything but the GPU buffers.
struct PlanCpu {
	verts: Vec<VegVertex>,
	idx: Vec<u32>,
	inst_data: Vec<VegInst>,
	models: Vec<ModelCpu>,
	cells: Vec<Cell>,
	d: &'static Dens,
	on: u8,
	scene_id: (usize, u32),
	foot: HashMap<u32, f32>,
	counts: (usize, usize, usize),
	t0: std::time::Instant,
}

/// The plant plan for density `on` (1 moderate, 2 lush): placement, models and cells, no GPU calls (runs on the load thread
/// or on the replan thread). None: nothing placed.
fn plan_cpu(scene: &TfView, world: &World, src: &VegIn, on: u8) -> Option<PlanCpu> {
		let c = cfg();
		let (shrub_set, textures, terrain_chains, shrub_chains, tie_set, tie_chains, level) = (src.shrub_set, src.textures, src.terrain_chains, src.shrub_chains, src.tie_set, src.tie_chains, src.level);
		let t0 = std::time::Instant::now();
		let d = dens(on);
		// Grass textures: the terrain textures the near faces use with a fair share of grass texels.
		let mut used: Vec<u32> = scene.set.iter().flat_map(|t| t.near.faces.iter().map(|&(k, _)| k & 0xff)).collect();
		used.sort_unstable();
		used.dedup();
		// The terrain pass samples each texture's authored mip chain (gpu.rs `batch_instances`, TerrainRule): its top level is the
		// picture on the ground; the level's texture table only stands in where a chain is missing.
		let terr: HashMap<u32, &Texture> = used.iter().filter_map(|&t| terrain_chains.get(t as usize).and_then(|c| c.as_ref()).and_then(|c| c.first()).or_else(|| textures.get(&t)).map(|x| (t, x))).collect();
		let mut grass = HashMap::new();
		for &t in &used {
			let Some(&img) = terr.get(&t) else { continue };
			let (g, cut, mean) = tex_stats(img);
			if c.log {
				eprintln!("veg: terrain tex {t:3} {}x{} grass {g:.2} cut {cut:.2} mean {:.0} {:.0} {:.0}", img.w, img.h, mean[0], mean[1], mean[2]);
				if g >= 0.25 {
					// Debug: the authored chain's levels (the terrain pass samples GS LOD levels of it).
					if let Some(ch) = terrain_chains.get(t as usize).and_then(|c| c.as_ref()) {
						let lv: Vec<String> = ch.iter().map(|l| { let (g, c, m) = tex_stats(l); format!("{}x{} g{g:.2} c{c:.2} m{:.0},{:.0},{:.0}", l.w, l.h, m[0], m[1], m[2]) }).collect();
						eprintln!("veg:   chain {}", lv.join(" | "));
					} else {
						eprintln!("veg:   no chain");
					}
				}
			}
			if g >= 0.25 {
				grass.insert(t, g);
			}
		}
		// The model footprint (m, horizontal radius) of every moby class with a model: the drawn mobys' touch (`VegFx::actors`).
		// Only drawn ones count: a load-time rule over the placements left bare circles round objects retail hides at the time
		// (Eudora, by the ship).
		let foot = src.foot.clone();
		if c.log {
			eprintln!("veg: {} moby classes with a footprint, {} placed mobys", foot.len(), src.n_mobies);
		}
		let (mut verts, mut idx, mut models, mut insts): (Vec<VegVertex>, Vec<u32>, Vec<ModelCpu>, Vec<(u32, VegInst)>) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
		let ttex = tie_grass(tie_set, textures, tie_chains);
		let cover = Cover { tf: cover_grid(scene), sh: shrub_grid(shrub_set), shrubs: shrub_set, stex: textures, schains: shrub_chains, tg: tie_grid(tie_set, &ttex), ties: tie_set, blocks: tie_blocks(tie_set, &ttex), ttex };
		if c.log {
			eprintln!("veg: {} grass tie textures, {} tie grass cells, {} grass blocks", cover.ttex.len(), cover.tg.len(), cover.blocks.len());
			let (mut n, mut miss, mut chained) = (0, 0, 0);
			for s in shrub_set {
				for &(k, _) in &s.mesh.faces {
					n += 1;
					let tk = k & crate::mobies::KEY_MASK & !shrubs::MATERIAL_BITS;
					chained += shrub_chains.get((tk & 0xff) as usize).is_some_and(|c| c.is_some()) as usize;
					miss += cover.shrub_tex(k).0.is_none() as usize;
				}
			}
			eprintln!("veg: shrub faces {n}, chained {chained}, no texture {miss}");
		}
		// The grass points: the blade clumps, and the test of where the level itself puts each plant class.
		let (bsp, cs) = spots(scene, &terr, &grass, &cover, world, d.blade_m2, 1);
		if let Ok(dir) = std::env::var("RAC_VEG_MAP") {
			// Debug (lane only): the near faces (centroid, texture, colour as drawn, slope) and the blade spots (why code in debug
			// view 2) for a top-down map.
			use std::io::Write;
			let mut f = std::io::BufWriter::new(std::fs::File::create(format!("{dir}/faces.csv")).unwrap());
			for (ti, t) in scene.set.iter().enumerate() {
				let m = &t.near;
				for &(key, fc) in &m.faces {
					let q = fc.map(|i| m.positions[i as usize]);
					let e1 = [q[1][0] - q[0][0], q[1][1] - q[0][1], q[1][2] - q[0][2]];
					let e2 = [q[2][0] - q[0][0], q[2][1] - q[0][1], q[2][2] - q[0][2]];
					let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
					let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-9);
					let tex = key & 0xff;
					let Some(img) = terr.get(&tex) else { continue };
					let (seen, _) = terrain_seen(scene, img, ti, fc, [1.0 / 3.0; 3]);
					let _ = writeln!(f, "{ti},{tex},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.3},{:.0},{:.0},{:.0},{:.0},{}", q[0][0], q[0][1], q[0][2], q[1][0], q[1][1], q[1][2], q[2][0], q[2][1], q[2][2], (n[2] / len).abs(), seen[0], seen[1], seen[2], seen[3], grass.contains_key(&tex) as u8);
				}
			}
			let mut f = std::io::BufWriter::new(std::fs::File::create(format!("{dir}/ties.csv")).unwrap());
			for (ii, t) in tie_set.iter().enumerate() {
				let m = &t.lods[0];
				for &(key, fc) in &m.faces {
					let q = fc.map(|i| m.positions[i as usize]);
					let e1 = [q[1][0] - q[0][0], q[1][1] - q[0][1], q[1][2] - q[0][2]];
					let e2 = [q[2][0] - q[0][0], q[2][1] - q[0][1], q[2][2] - q[0][2]];
					let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
					let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-9);
					let tk = key & crate::mobies::KEY_MASK;
					let img = tie_chains.get((tk & 0xff) as usize).and_then(|c| c.as_ref()).and_then(|c| c.first()).or_else(|| textures.get(&tk));
					let ix = fc.map(|i| i as usize);
					let uv: [f32; 2] = std::array::from_fn(|j| (0..3).map(|v| m.uvs[ix[v]][j] / 3.0).sum());
					let tx = img.map_or([0.0; 4], |im| texel3(im, uv[0], uv[1]));
					let sh: [f32; 3] = std::array::from_fn(|j| (0..3).map(|v| m.colours.get(ix[v]).map_or(1.0, |c| c[j]) / 3.0).sum());
					let _ = writeln!(f, "{ii},{tk:x},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.3},{:.0},{:.0},{:.0},{:.0},{:.2},{:.2},{:.2}", q[0][0], q[0][1], q[0][2], q[1][0], q[1][1], q[1][2], q[2][0], q[2][1], q[2][2], n[2] / len, tx[0], tx[1], tx[2], tx[3], sh[0], sh[1], sh[2]);
				}
			}
			let mut f = std::io::BufWriter::new(std::fs::File::create(format!("{dir}/tieg.csv")).unwrap());
			for (&(cx, cy), list) in &cover.tg {
				let _ = (cx, cy);
				for &(ii, fi) in list {
					let m = &tie_set[ii as usize].lods[0];
					let q = m.faces[fi as usize].1.map(|i| m.positions[i as usize]);
					let c: [f32; 3] = std::array::from_fn(|j| (q[0][j] + q[1][j] + q[2][j]) / 3.0);
					let (seen, _) = cover.tie_seen(ii as usize, fi as usize, [1.0 / 3.0; 3]);
					let hit = world.line([c[0], c[1], c[2] + RAY_UP], [c[0], c[1], c[2] - RAY_DOWN], crate::physics::qmask::TWO_SIDED).map_or(String::from("-"), |h| format!("{:.2}", h.p[2] - c[2]));
					let _ = writeln!(f, "{ii},{fi},{:.2},{:.2},{:.2},{hit},{:.0},{:.0},{:.0},{:.0}", c[0], c[1], c[2], seen[0], seen[1], seen[2], seen[3]);
				}
			}
			let mut f = std::io::BufWriter::new(std::fs::File::create(format!("{dir}/coll.csv")).unwrap());
			for xi in 0..60 {
				for yi in 0..70 {
					let (x, y) = (250.0 + xi as f32, 200.0 + yi as f32);
					let mut z = 100.0f32;
					let mut row = format!("{x},{y}");
					for _ in 0..8 {
						let Some(h) = world.line([x, y, z], [x, y, 20.0], crate::physics::qmask::TWO_SIDED) else { break };
						row += &format!(",{:.2}:{}", h.p[2], h.kind & 0x1f);
						z = h.p[2] - 0.05;
					}
					let _ = writeln!(f, "{row}");
				}
			}
			let mut f = std::io::BufWriter::new(std::fs::File::create(format!("{dir}/spots.csv")).unwrap());
			for sp in &bsp {
				let why = if sp.tex > u32::MAX - 8 { u32::MAX - sp.tex } else { 0 };
				let hit = if why == 3 || why == 4 {
					world.line([sp.p[0], sp.p[1], sp.p[2] + 40.0], [sp.p[0], sp.p[1], sp.p[2] - 40.0], crate::physics::qmask::TWO_SIDED).map_or(String::from("-,-"), |h| format!("{:.2},{}", h.p[2], h.kind & 0x1f))
				} else {
					String::from("-,-")
				};
				let _ = writeln!(f, "{:.2},{:.2},{:.2},{why},{hit}", sp.p[0], sp.p[1], sp.p[2]);
			}
		}
		if c.log {
			eprintln!("veg: blades tried {} not grass {} covered {} no ground {} ground off {} bad surface {} dirt margin {} kept {}", cs.tried, cs.not_green, cs.covered, cs.no_hit, cs.far, cs.bad, cs.edge, cs.kept);
		}
		let mut blade_n = 0;
		if c.blades {
			// Model 0 plain clumps, model 1 clumps with flower heads (`Dens::flower` of them, one colour per `PATCH` square).
			for heads in [0, HEADS] {
				let first = idx.len() as u32;
				clump(d.blades, d.blade_w, d.clump_r, heads, &mut verts, &mut idx);
				models.push(ModelCpu { kind: KIND_BLADE, subs: vec![(None, first, idx.len() as u32 - first)] });
			}
			for s in &bsp {
				let h = (d.blade_h.0 + (d.blade_h.1 - d.blade_h.0) * s.r[2]) * if s.short { EDGE_H } else { 1.0 };
				let flower = s.r[5] < d.flower;
				let patch = Rng::new(((s.p[0] / PATCH).floor() as i64 as u64) << 32 ^ (s.p[1] / PATCH).floor() as i64 as u64, 3).next();
				let col = if debug_tex() { let mut q = Rng::new(s.tex as u64, 9); [q.next(), q.next(), q.next()] } else if debug_why() || debug_probe() { if s.tex > u32::MAX - 8 { s.col } else { [1.0; 3] } } else { s.col };
				insts.push((flower as u32, VegInst { a: [s.p[0], s.p[1], s.p[2], s.r[0] * std::f32::consts::TAU], b: [0.85 + 0.3 * s.r[1], h, s.r[3], s.r[4] * std::f32::consts::TAU], c: [col[0], col[1], col[2], (patch * 4.0).floor().min(3.0)], d: [s.slope[0], s.slope[1], s.lum.max(0.05), 0.0] }));
			}
			blade_n = bsp.len();
		}
		let mut sea_n = 0;
		if c.sea && SEA_LEVELS.contains(&level) {
			let (ssp, scs) = sea_spots(scene, world, SEA_M2[(on >= 2) as usize]);
			if !ssp.is_empty() {
				let mi = models.len() as u32;
				let first = idx.len() as u32;
				kelp(if on >= 2 { 6 } else { 4 }, 0.14, 0.25, &mut verts, &mut idx);
				models.push(ModelCpu { kind: KIND_BLADE, subs: vec![(None, first, idx.len() as u32 - first)] });
				// Olive, deep green and brown kelp; each patch keeps one colour family.
				const SEA_COL: [[f32; 3]; 3] = [[0.40, 0.52, 0.15], [0.20, 0.42, 0.16], [0.48, 0.36, 0.13]];
				for sp in &ssp {
					let fam = (patch_noise(sp.p[0] * 0.37 + 91.0, sp.p[1] * 0.37 - 17.0) * 2.999) as usize;
					let col = SEA_COL[fam].map(|x| x * (0.8 + 0.4 * sp.r[3]));
					let h = (SEA_H.0 + (SEA_H.1 - SEA_H.0) * sp.r[1] * sp.r[1]).min(sp.r[2] * SEA_REACH).min(sp.r[2] - SEA_GAP);
					if h < 0.4 {
						continue;
					}
					insts.push((mi, VegInst { a: [sp.p[0], sp.p[1], sp.p[2], sp.r[0] * std::f32::consts::TAU], b: [0.8 + 0.4 * sp.r[4], h, sp.r[3], sp.r[4] * std::f32::consts::TAU], c: [col[0], col[1], col[2], 0.0], d: [sp.slope[0], sp.slope[1], sp.lum, 1.0] }));
					sea_n += 1;
				}
			}
			if let Ok(dir) = std::env::var("RAC_VEG_MAP") {
				use std::io::Write;
				let mut f = std::io::BufWriter::new(std::fs::File::create(format!("{dir}/sea.csv")).unwrap());
				for sp in &ssp {
					let _ = writeln!(f, "{:.2},{:.2},{:.2},{:.2},{}", sp.p[0], sp.p[1], sp.p[2], sp.r[2], sp.tex);
				}
			}
			if c.log {
				eprintln!("veg: seaweed tried {} under water {} on the bed {} clumps {sea_n}", scs[0], scs[1], scs[2]);
			}
		}
		let mut ggrid: HashMap<(i32, i32), Vec<([f32; 3], f32)>> = HashMap::new();
		for s in &bsp {
			ggrid.entry(((s.p[0] / 2.0).floor() as i32, (s.p[1] / 2.0).floor() as i32)).or_default().push((s.p, s.lum));
		}
		// The ground shade of the nearest grass point within `ON_GRASS_R` of a plant's origin (none: not on grass).
		let on_grass = |o: [f32; 3]| -> Option<f32> {
			let cell = ((o[0] / 2.0).floor() as i32, (o[1] / 2.0).floor() as i32);
			let mut best: Option<(f32, f32)> = None;
			for dx in -1..=1 {
				for dy in -1..=1 {
					for &(q, l) in ggrid.get(&(cell.0 + dx, cell.1 + dy)).map_or(&[][..], |v| &v[..]) {
						let dd = (q[0] - o[0]).powi(2) + (q[1] - o[1]).powi(2);
						if dd < ON_GRASS_R * ON_GRASS_R && (q[2] - o[2]).abs() < 1.0 && best.is_none_or(|b| dd < b.0) {
							best = Some((dd, l));
						}
					}
				}
			}
			best.map(|b| b.1)
		};
		let mut shrub_n = 0;
		if c.shrubs && !shrub_set.is_empty() {
			// Plant classes: per class its first instance as the template; green or see-through foliage, upright, not too big.
			let mut classes: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
			for (i, s) in shrub_set.iter().enumerate() {
				classes.entry(s.class_id).or_default().push(i);
			}
			// class -> (template instance, small, template ground shade)
			let mut plants: HashMap<u32, (usize, bool, f32)> = HashMap::new();
			for (&cid, list) in &classes {
				// The template: the instance of median baked brightness (not one lit for an odd dark or bright corner).
				let vlum = |i: usize| {
					let c = &shrub_set[i].mesh.colours;
					c.iter().map(|c| lum([c[0] / 128.0, c[1] / 128.0, c[2] / 128.0])).sum::<f32>() / c.len().max(1) as f32
				};
				let mut by_lum: Vec<(f32, usize)> = list.iter().map(|&i| (vlum(i), i)).collect();
				by_lum.sort_by(|a, b| a.0.total_cmp(&b.0));
				let (tlum_v, tpl) = by_lum[by_lum.len() / 2];
				let s = &shrub_set[tpl];
				let Some(lit) = s.mesh.lit.as_ref() else { continue };
				let o = lit.origin;
				let (mut zlo, mut zhi, mut r) = (f32::MAX, f32::MIN, 0.0f32);
				for p in &s.mesh.positions {
					zlo = zlo.min(p[2] - o[2]);
					zhi = zhi.max(p[2] - o[2]);
					r = r.max(((p[0] - o[0]).powi(2) + (p[1] - o[1]).powi(2)).sqrt());
				}
				let mut keys: Vec<u32> = s.mesh.faces.iter().map(|&(k, _)| k & crate::mobies::KEY_MASK & !shrubs::MATERIAL_BITS).collect();
				keys.sort_unstable();
				keys.dedup();
				let (mut g, mut cut, mut mean) = (0.0f32, 0.0f32, [0.0f32; 3]);
				for k in &keys {
					if let Some(t) = textures.get(k) {
						let (a, b, m) = tex_stats(t);
						g += a / keys.len() as f32;
						cut += b / keys.len() as f32;
						for j in 0..3 {
							mean[j] += m[j] / keys.len() as f32;
						}
					}
				}
				let height = zhi - zlo;
				let size = r.max(height);
				let small = size <= SMALL_R;
				// The class as drawn: its mean texel times the template's mean baked vertex colour (a tuft class painted dark blue
				// for shade drew navy copies on open grass, Novalis field 1360).
				let vc = s.mesh.colours.iter().fold([0.0f32; 3], |a, c| std::array::from_fn(|j| a[j] + c[j] / 128.0 / s.mesh.colours.len().max(1) as f32));
				let drawn = [mean[0] * vc[0], mean[1] * vc[1], mean[2] * vc[2], 255.0];
				// And every texture on a fair share (`TEX_SHARE`) of its faces green as drawn: the tuft class whose second texture
				// is dark blue passed on its mean and drew dark blue tufts (Novalis fields 760 and 1210).
				let all_green = keys.iter().all(|k| {
					let share = s.mesh.faces.iter().filter(|f| f.0 & crate::mobies::KEY_MASK & !shrubs::MATERIAL_BITS == *k).count() as f32 / s.mesh.faces.len().max(1) as f32;
					share < TEX_SHARE || textures.get(k).is_none_or(|t| {
						let (_, _, m) = tex_stats(t);
						green([m[0] * vc[0], m[1] * vc[1], m[2] * vc[2], 255.0])
					})
				});
				let leafy = green(drawn) && all_green && if small { g >= 0.25 || (cut >= 0.2 && g >= 0.1) } else { g >= 0.4 || (cut >= 0.2 && g >= 0.3) };
				let shades: Vec<f32> = list.iter().filter_map(|&i| on_grass(shrub_set[i].mesh.lit.as_ref().map_or([0.0; 3], |l| l.origin))).collect();
				let grassy = shades.len() as f32 / list.len() as f32;
				let ground = if shades.is_empty() { 1.0 } else { shades.iter().sum::<f32>() / shades.len() as f32 };
				// Lane debug: RAC_VEG_NOCLASS="a,b" never copies those shrub classes.
				let banned = std::env::var("RAC_VEG_NOCLASS").is_ok_and(|v| v.split(',').any(|x| x.trim().parse::<u32>().ok() == Some(cid)));
				// A template whose lowest point sits above its own origin (Novalis cliff ferns 135/139, zlo 0.13/0.49) hangs from a
				// ledge in retail: copied onto flat ground it floats, so it is never copied.
				let plant = !banned && zlo <= HANG_ZLO && !s.mesh.faces.is_empty() && leafy && grassy >= ON_GRASS && height >= 0.15 && height >= 0.15 * 2.0 * r && size <= MAX_PLANT_R && (small || height >= MIN_BUSH_H * r);
				if c.log {
					eprintln!("veg: shrub class {cid:4} x{:4} r {r:.2} h {height:.2} grass {g:.2} cut {cut:.2} on grass {grassy:.2} mean {:.0} {:.0} {:.0} drawn {:.0} {:.0} {:.0} baked {tlum_v:.2} ground {ground:.2} zlo {zlo:.2} -> {}", list.len(), mean[0], mean[1], mean[2], drawn[0], drawn[1], drawn[2], if plant { if small { "ground cover" } else { "bush" } } else { "-" });
				}
				if plant {
					plants.insert(cid, (tpl, small, ground.max(0.05)));
				}
			}
			// The level's plant instances on a grid, to find the nearest one around a spot.
			let mut pgrid: HashMap<(i32, i32), Vec<(usize, u32)>> = HashMap::new();
			let pc = d.near_r.max(4.0);
			for (i, s) in shrub_set.iter().enumerate() {
				if plants.contains_key(&s.class_id) {
					let o = s.mesh.lit.as_ref().map_or([0.0; 3], |l| l.origin);
					pgrid.entry(((o[0] / pc).floor() as i32, (o[1] / pc).floor() as i32)).or_default().push((i, s.class_id));
				}
			}
			if !plants.is_empty() {
				let (sp, cs) = spots(scene, &terr, &grass, &cover, world, d.small_m2, 2);
				if c.log {
					eprintln!("veg: shrub spots tried {} not grass {} covered {} no ground {} ground off {} bad surface {} dirt margin {} kept {}", cs.tried, cs.not_green, cs.covered, cs.no_hit, cs.far, cs.bad, cs.edge, cs.kept);
				}
				// Template models, one per plant class used.
				let mut model_of: HashMap<u32, (u32, f32, f32)> = HashMap::new();
				for s in sp.iter().filter(|s| !s.short) {
					let cell = ((s.p[0] / pc).floor() as i32, (s.p[1] / pc).floor() as i32);
					let mut best: Option<(f32, u32)> = None;
					for dx in -1..=1 {
						for dy in -1..=1 {
							for &(i, cid) in pgrid.get(&(cell.0 + dx, cell.1 + dy)).map_or(&[][..], |v| &v[..]) {
								let o = shrub_set[i].mesh.lit.as_ref().map_or([0.0; 3], |l| l.origin);
								let dd = (o[0] - s.p[0]).powi(2) + (o[1] - s.p[1]).powi(2);
								if dd < d.near_r * d.near_r && (o[2] - s.p[2]).abs() < 6.0 && best.is_none_or(|b| dd < b.0) {
									best = Some((dd, cid));
								}
							}
						}
					}
					let Some((_, cid)) = best else { continue };
					let (ti, small, ground) = plants[&cid];
					if !small && s.r[5] >= d.big_m2 / d.small_m2 {
						continue;
					}
					let (mi, href, tlum) = *model_of.entry(cid).or_insert_with(|| {
						let t = &shrub_set[ti];
						let o = t.mesh.lit.as_ref().map_or([0.0; 3], |l| l.origin);
						let zlo = t.mesh.positions.iter().map(|p| p[2] - o[2]).fold(f32::MAX, f32::min);
						let zhi = t.mesh.positions.iter().map(|p| p[2] - o[2]).fold(f32::MIN, f32::max);
						let base = verts.len() as u32;
						for (k, p) in t.mesh.positions.iter().enumerate() {
							let col = t.mesh.colours.get(k).map_or([1.0; 3], |c| [c[0] / 128.0, c[1] / 128.0, c[2] / 128.0]);
							let wgt = ((p[2] - o[2] - zlo) / (zhi - zlo).max(0.05)).clamp(0.0, 1.0);
							verts.push(VegVertex { pos: [p[0] - o[0], p[1] - o[1], p[2] - o[2]], root: [0.0; 2], uv: t.mesh.uvs[k], col: [col[0], col[1], col[2], wgt], side: [0.0; 2] });
						}
						let mut by_key: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
						for &(k, f) in &t.mesh.faces {
							let wrap = (k >> shrubs::WRAP_SHIFT) & 3;
							by_key.entry(k & crate::mobies::KEY_MASK & !shrubs::MATERIAL_BITS | wrap << shrubs::WRAP_SHIFT).or_default().extend(f.map(|i| i + base));
						}
						let mut subs = Vec::new();
						for (k, list) in by_key {
							subs.push((Some(k), idx.len() as u32, list.len() as u32));
							idx.extend(list);
						}
						models.push(ModelCpu { kind: KIND_MESH, subs });
						// The template's baked colours are those of its own spot: a copy is shaded by its ground against the class's own.
						let tl = ground;
						(models.len() as u32 - 1, (zhi - zlo).max(0.1), tl)
					});
					let _ = href;
					let tint = (s.lum / tlum).clamp(0.6, 1.4);
					let sc = 0.8 + 0.35 * s.r[1];
					insts.push((mi, VegInst { a: [s.p[0], s.p[1], s.p[2], s.r[0] * std::f32::consts::TAU], b: [sc, sc, s.r[3], s.r[4] * std::f32::consts::TAU], c: [tint, tint, tint, 0.0], d: [0.0, 0.0, 1.0, 0.0] }));
					shrub_n += 1;
				}
			}
		}
		let mut leaf_n = 0;
		if c.trees {
			let lush = (on >= 2) as usize;
			let (mut lsp, tie_n, shrub_n2) = tree_spots(&cover, world, textures, tie_chains, LEAF_M2[lush]);
			let hill = terrain_leaves(scene, &terr, &grass, &cover, world, LEAF_M2[lush]);
			if c.log {
				eprintln!("veg: terrain slope leaves: {}", hill.len());
			}
			lsp.extend(hill);
			// Lane debug: RAC_VEG_LEAFRED=1 draws the leaves red.
			let red = std::env::var("RAC_VEG_LEAFRED").ok();
			if !lsp.is_empty() {
				let mi = models.len() as u32;
				let first = idx.len() as u32;
				leaf_cluster(LEAVES[lush], &mut verts, &mut idx);
				models.push(ModelCpu { kind: KIND_LEAF, subs: vec![(None, first, idx.len() as u32 - first)] });
				for sp in &lsp {
					// The crown's colour as drawn (texel x shade), as the blades' ground colour; the fragment stage relights it by its
					// luminance (sun_baked).
					let col = match (red.as_deref(), sp.why) {
					(None, _) => sp.col,
					(Some("2"), 2) => [0.0, 0.3, 1.0],
					(Some("2"), 3) => [1.0, 0.0, 1.0],
					(Some("2"), 4) => [0.0, 1.0, 1.0],
					_ => [1.0, 0.0, 0.0],
				};
					insts.push((mi, VegInst { a: [sp.p[0], sp.p[1], sp.p[2], sp.r[0] * std::f32::consts::TAU], b: [(0.8 + 0.4 * sp.r[1]) * sp.s, 0.0, sp.r[2], sp.r[3] * std::f32::consts::TAU], c: [col[0], col[1], col[2], 0.0], d: [sp.n[0], sp.n[1], sp.n[2], sp.lum] }));
					leaf_n += 1;
				}
			}
			if c.log {
				eprintln!("veg: tree leaves: {tie_n} ties, {shrub_n2} shrubs, {leaf_n} clusters");
			}
		}
		if insts.is_empty() {
			if c.log {
				eprintln!("veg: nothing placed ({:.0} ms)", t0.elapsed().as_secs_f64() * 1e3);
			}
			return None;
		}
		// Instances by (model, cell, rank); one cell entry per run.
		let key = |m: u32, v: &VegInst| (m, (v.a[0] / CELL).floor() as i32, (v.a[1] / CELL).floor() as i32);
		insts.sort_by(|x, y| key(x.0, &x.1).cmp(&key(y.0, &y.1)).then(x.1.b[2].total_cmp(&y.1.b[2])));
		let mut cells: Vec<Cell> = Vec::new();
		let mut last = None;
		for (i, (m, v)) in insts.iter().enumerate() {
			let k = key(*m, v);
			let reach = match models[*m as usize].kind {
				KIND_MESH => 3.0,
				KIND_LEAF => LEAF_SPREAD + LEAF_LEN.1 * 1.3,
				_ => d.blade_h.1 * 1.25 + d.clump_r,
			};
			if last != Some(k) {
				cells.push(Cell { model: *m, lo: [k.1 as f32 * CELL, k.2 as f32 * CELL, f32::MAX], hi: [(k.1 + 1) as f32 * CELL, (k.2 + 1) as f32 * CELL, f32::MIN], first: i as u32, ranks: Vec::new() });
				last = Some(k);
			}
			let c = cells.last_mut().unwrap();
			c.ranks.push(v.b[2]);
			c.lo = [c.lo[0].min(v.a[0] - reach), c.lo[1].min(v.a[1] - reach), c.lo[2].min(v.a[2] - reach)];
			c.hi = [c.hi[0].max(v.a[0] + reach), c.hi[1].max(v.a[1] + reach), c.hi[2].max(v.a[2] + reach)];
		}
		let inst_data: Vec<VegInst> = insts.iter().map(|x| x.1).collect();
		Some(PlanCpu { verts, idx, inst_data, models, cells, d, on, scene_id: scene.id, foot, counts: (blade_n, shrub_n, leaf_n), t0 })
	}

/// A replan is due when the Vegetation row (`want`: 0 off, 1 moderate, 2 lush) asks for a density other than the plan's, with
/// Lighting Enhanced and the row not Off (Off keeps the plan: the plants are only hidden), without the RAC_VEG override, and
/// the plan belongs to the scene on screen.
fn replan_wanted(env: bool, want: u8, plan_on: u8, enhanced: bool, same_scene: bool) -> bool {
	!env && want > 0 && want != plan_on && enhanced && same_scene
}

impl Gpu {
	/// Once per level after the scene is built (app.rs): the vegetation plan, or none (RAC_VEG=0, front end). The density is the
	/// one asked for at load; with Vegetation Off (or Lighting Classic) a plan is still made, so switching the row on in the pause
	/// menu shows plants at once. A later Moderate <-> Lush change replans on a thread (`veg_follow`, `keep_veg_src`).
	pub(super) fn plan_veg(&mut self, world: &World, mobies: &[gameplay::MobyInstance], shrub_set: &[shrubs::ShrubInstance], textures: &HashMap<u32, Texture>, terrain_chains: &[Option<Vec<Texture>>], shrub_chains: &[Option<Vec<Texture>>], prop_meshes: &HashMap<u32, Mesh>, tie_set: &[ties::LitTie], tie_chains: &[Option<Vec<Texture>>], level: usize) {
		self.veg.plan = None;
		self.veg.job = None;
		self.veg.src = None;
		let c = cfg();
		// Without RAC_VEG only the retail game's menu turns it on (none with RAC_PC_EXT=0, the smoke path).
		if c.env == Some(0) || (c.env.is_none() && !(crate::retail::active() && crate::retail::pcext::enabled())) {
			return;
		}
		let on = c.env.unwrap_or_else(crate::retail::pcext::veg_for_load).max(1);
		let Some(scene) = self.scene.as_ref() else { return };
		if scene.tfrag_set.is_empty() {
			return;
		}
		// The model footprint (m, horizontal radius) of every moby class with a model: the drawn mobys' touch (`VegFx::actors`).
		// Only drawn ones count: a load-time rule over the placements left bare circles round objects retail hides at the time
		// (Eudora, by the ship).
		let foot: HashMap<u32, f32> = prop_meshes.iter().filter(|(_, x)| !x.faces.is_empty()).map(|(&k, x)| (k, x.positions.iter().map(|v| v[0].hypot(v[1])).fold(0.0, f32::max))).collect();
		let view = TfView::of(scene);
		let src = VegIn { shrub_set, textures, terrain_chains, shrub_chains, tie_set, tie_chains, foot: &foot, n_mobies: mobies.len(), level };
		let cpu = plan_cpu(&view, world, &src, on);
		self.finish_veg(cpu);
		self.veg.trail.clear();
	}

	/// After the level load (app.rs), with the level data about to be freed: keeps what a replan needs (only when a plan exists).
	pub(super) fn keep_veg_src(&mut self, shrub_set: Vec<shrubs::ShrubInstance>, textures: HashMap<u32, Texture>, terrain_chains: (Vec<Option<Vec<Texture>>>, Vec<Option<Vec<Texture>>>, Vec<Option<Vec<Texture>>>), tie_set: Vec<ties::LitTie>, n_mobies: usize, level: usize) -> Option<Box<dyn std::any::Any + Send>> {
		let Some(plan) = self.veg.plan.as_ref() else { return Some(Box::new((shrub_set, textures, terrain_chains, tie_set))) };
		let (mut tc, mut sc, mut ic) = terrain_chains;
		// The planner reads the top level of the terrain and tie chains and level DECAL_MIP of the shrub chains and nothing beyond: the smaller mips are
		// not kept (memory, a replan needs none of them).
		let trim = |c: &mut Vec<Option<Vec<Texture>>>, n: usize| c.iter_mut().flatten().for_each(|ch| ch.truncate(n));
		trim(&mut tc, 1);
		trim(&mut ic, 1);
		trim(&mut sc, DECAL_MIP + 1);
		if cfg().log {
			let mb = |t: &Texture| (t.rgba.len() * 4) as f64 / 1e6;
			let chains = |c: &[Option<Vec<Texture>>]| c.iter().flatten().flatten().map(mb).sum::<f64>();
			eprintln!("veg: kept for replans: textures {:.1} MB, terrain chains {:.1} MB, shrub chains {:.1} MB, tie chains {:.1} MB, {} shrubs, {} ties", textures.values().map(mb).sum::<f64>(), chains(&tc), chains(&sc), chains(&ic), shrub_set.len(), tie_set.len());
		}
		self.veg.src = Some(std::sync::Arc::new(VegSrc { shrub_set, textures, terrain_chains: tc, shrub_chains: sc, tie_set, tie_chains: ic, foot: plan.foot.clone(), n_mobies, level }));
		None
	}

	/// Every frame (handler.rs): finishes a replan that is ready, or starts one when the Vegetation row asks for a density other
	/// than the plan's (Moderate <-> Lush; with Lighting Enhanced and the row not Off). The plan is made on a thread from a copy
	/// of the terrain, so the game keeps its 50 steps; the old plan draws until the new one is uploaded.
	pub(super) fn veg_follow(&mut self, world: Option<&World>) {
		if let Some((rx, on)) = &self.veg.job {
			let on = *on;
			match rx.try_recv() {
				Ok(cpu) => {
					self.veg.job = None;
					let ms = cpu.as_ref().map(|p| p.t0.elapsed().as_secs_f64() * 1e3);
					self.finish_veg(cpu);
					if let Some(p) = self.veg.plan.as_mut() {
						p.on = on;
					}
					eprintln!("veg: replanned {} ({} ms on the thread)", ["off", "moderate", "lush"][on as usize], ms.map_or(0, |m| m as u64));
				}
				Err(std::sync::mpsc::TryRecvError::Empty) => {}
				Err(std::sync::mpsc::TryRecvError::Disconnected) => {
					self.veg.job = None;
					if let Some(p) = self.veg.plan.as_mut() {
						p.on = on;
					}
				}
			}
			return;
		}
		let (Some(plan), Some(src), Some(world), Some(scene)) = (self.veg.plan.as_ref(), self.veg.src.as_ref(), world, self.scene.as_ref()) else { return };
		let want = want();
		if !replan_wanted(cfg().env.is_some(), want, plan.on, crate::retail::pcext::graphics().enhanced(), plan.scene_id == (scene.tfrag_set.len(), scene.radius.to_bits())) {
			return;
		}
		let t = std::time::Instant::now();
		let (owned, world, src) = (TfOwned::of(scene), world.clone(), src.clone());
		let copy_ms = t.elapsed().as_secs_f64() * 1e3;
		let (tx, rx) = std::sync::mpsc::channel();
		let started = std::thread::Builder::new().name("veg plan".into()).stack_size(16 << 20).spawn(move || {
			// Background work: below-normal priority, so a 4-core PC keeps its game and render threads first.
			crate::retail::hot::idle();
			let view = owned.view();
			let _ = tx.send(plan_cpu(&view, &world, &src.as_in(), want));
		});
		eprintln!("veg: replan to {} (copy {:.0} ms on the game thread)", ["off", "moderate", "lush"][want as usize], copy_ms);
		match started {
			Ok(_) => self.veg.job = Some((rx, want)),
			Err(_) => {
				if let Some(p) = self.veg.plan.as_mut() {
					p.on = want;
				}
			}
		}
	}

	/// The GPU half of a plan: the buffers, and the models' texture slots of the current scene.
	fn finish_veg(&mut self, cpu: Option<PlanCpu>) {
		let Some(PlanCpu { verts, idx, inst_data, models, cells, d, on, scene_id, foot, counts: (blade_n, shrub_n, leaf_n), t0 }) = cpu else { return };
		let c = cfg();
		let Some(scene) = self.scene.as_ref() else { return };
		if scene_id != (scene.tfrag_set.len(), scene.radius.to_bits()) {
			return;
		}
		let vbuf = buffer(&self.device, "veg vertices", bytemuck::cast_slice(&verts), wgpu::BufferUsages::VERTEX);
		self.queue.write_buffer(&vbuf, 0, bytemuck::cast_slice(&verts));
		let ibuf = buffer(&self.device, "veg indices", bytemuck::cast_slice(&idx), wgpu::BufferUsages::INDEX);
		self.queue.write_buffer(&ibuf, 0, bytemuck::cast_slice(&idx));
		let inst = buffer(&self.device, "veg instances", bytemuck::cast_slice(&inst_data), wgpu::BufferUsages::VERTEX);
		self.queue.write_buffer(&inst, 0, bytemuck::cast_slice(&inst_data));
		let white = self.upload_texture(Some(&Texture { w: 1, h: 1, rgba: vec![[255; 4]] }));
		let scene = self.scene.as_ref().unwrap();
		let models: Vec<Model> = models.into_iter().map(|m| Model { kind: m.kind, subs: m.subs.into_iter().filter_map(|(k, s, n)| match k { None => Some((None, s, n)), Some(k) => scene.tex_slots.get(&k).map(|&i| (Some(i), s, n)) }).collect() }).collect();
		if c.log {
			eprintln!("veg: {blade_n} blade clumps, {shrub_n} shrubs, {leaf_n} leaf clusters, {} models, {} cells, {} instances ({:.1} MB), {:.0} ms", models.len(), cells.len(), inst_data.len(), (inst_data.len() * std::mem::size_of::<VegInst>() + verts.len() * std::mem::size_of::<VegVertex>()) as f64 / 1e6, t0.elapsed().as_secs_f64() * 1e3);
		}
		self.veg.plan = Some(Plan { vbuf, ibuf, inst, models, cells, white, dens: d, on, scene_id, foot });
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn lush_is_denser_than_moderate() {
		let (m, l) = (dens(1), dens(2));
		assert!(l.blade_m2 > m.blade_m2 && l.blade_end > m.blade_end && l.blades > m.blades);
		assert!(std::ptr::eq(dens(0), m), "the plan of an Off row is the moderate one");
	}

	#[test]
	fn row_change_replans() {
		// Moderate plan, row Lush, Enhanced: replan. Same density, Off, Classic, env override, another scene: none.
		assert!(replan_wanted(false, 2, 1, true, true));
		assert!(replan_wanted(false, 1, 2, true, true), "Lush -> Moderate too");
		assert!(!replan_wanted(false, 1, 1, true, true));
		assert!(!replan_wanted(false, 0, 1, true, true), "Off only hides");
		assert!(!replan_wanted(false, 2, 1, false, true), "Lighting Classic draws no plants");
		assert!(!replan_wanted(true, 2, 1, true, true), "RAC_VEG fixes the density");
		assert!(!replan_wanted(false, 2, 1, true, false));
		// The pause cycle Moderate -> Lush -> Off -> Moderate on a Lush-at-load plan: a replan only for the changes that need one.
		let mut plan = 1;
		let mut n = 0;
		for want in [2u8, 0, 1, 2] {
			if replan_wanted(false, want, plan, true, true) {
				plan = want;
				n += 1;
			}
		}
		assert_eq!((plan, n), (2, 3));
	}
}
