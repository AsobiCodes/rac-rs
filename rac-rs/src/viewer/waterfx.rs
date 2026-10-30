//! [PC] Water Enhanced (Pause -> Options -> Graphics -> Water, with Lighting Enhanced; retail/pcext.rs `graphics`; Split view: hangs on `DrawFx.water_on`, from `post_on` = `splitview::fx`; no retail
//! counterpart). The water itself stays retail: its meshes, wave motion, textures and blending are drawn by the level pass as
//! before, and Water Classic leaves the frame untouched.
//!
//! The level pass hands over the vertex and index buffers of every water draw it made (Novalis and Drek fleet water quads,
//! the Hoven queue-C pools, the Aridia / Kerwan / Batalia / Oltanis oceans and the Rilgar / Umbris / Gaspar RAM oceans). Then:
//! - `fs_pos`: those meshes again, into a full-size world-position target (nearest surface wins; a fragment hidden behind the
//!   level's depth is dropped, so whatever stands in front of the water keeps it);
//! - `fs_water`, inside the post chain after the occlusion and before the bloom (so glints bloom), on water pixels only: a
//!   normal from the surface's own shape (smoothed over a few pixels, so the mesh's triangles do not show) plus ten animated
//!   wave trains on world XY that fade out before they get finer than a few pixels; a refraction wobble of the picture under
//!   it; moving caustic light on the bed seen through blended water (and faintly on opaque water); a depth tint; screen-space
//!   reflections (a ray along the smooth surface marched against the full-size depth from a dithered start, refined on the
//!   crossing, read softened by its length, bent by the ripples, fading at screen edges and far hits to the fog colour as the
//!   far air), weighted by Fresnel with a minimum sheen so the surface shows from straight above, keeping the water's own
//!   hue; a sun glint with sparkles along the key light (`sunshadow::key_light`); foam where the water meets the ground
//!   (shallow water from the depth under it, or the edge where geometry cuts the surface); contact rings (`Rings`: ripples
//!   and a froth crest spreading from where the hero or a falling particle meets still water); a soft mask edge.
//!   A camera below the surface leaves the pixel retail, so underwater views stay as they were.
//! Gaspar's RAM ocean is lava (`LAVA_LEVEL`): it takes `LAVA_SLOT`, and `fs_water` gives it the lava look (`lava`) instead.
//! Aridia's mud (its ocean) and Orxon's toxic sludge (level terrain, `terrain_liquid`) take `MUD_SLOT` / `SLUDGE_SLOT`: the
//! mud and sludge looks.
//!
//! Modules (a pure move out of one file; each child starts with `use super::*;`):
//! - `shader`: the water shader (WGSL, one string literal)
//! - `fall_wgsl`: the waterfall sheets' WGSL (`vs_fall`, `fs_fall`), the tail of the shader moved out unchanged
//! - `far_lod`: the far-sea look and the near-identical water savings (WGSL + switches; both off by default)
//! - `shore_map`: the geometric sea's shore-map compute shader, its uniform and size constants, `shore_dist`
//! - `dry_tiles`: dry tiles for the sea grid (compute pass; lets `vs_grid` skip `edge_calm` where it would return 1.0)
//! - `bed_min`: 4 x 4 minimum of the bed depth (compute pass `cs_bedmin`): `shore_fade`'s 16 taps are skipped where all are deep
//! - `nwater`: speed cuts of `fs_water` (`mirror_layer`, `twinkle_at`; exact, the arithmetic is unchanged)
//! - `grid_waves`: the sea grid's wave heights, made once per vertex (compute pass `cs_wave`, read by `vs_grid`)
//! - `hero_crest`: the geometric sea's crest in front of the swimming hero (`sea_pic`, `geo_fill`)
//! - `looks`: look parameters: `GeoLook`, the uniform block `WaterU`, tuning constants, `march_steps`, `wave_look`
//! - `levels`: per-level tables (sea, pond, river, waterfall, cold, frozen, lava, mud, sludge) and the flow slots
//! - `fall_sheets`: waterfall sheets: shaping, `SheetMem`, foot finder, `WaterFx::sheet_falls`
//! - `draws`: `WaterDraw`, one water mesh for the water pass
//! - `contacts`: contact rings (`Rings`) and the movers: `WaterFx::contacts`, `track_props`, `shore_movers`
//! - `wakes`: wakes behind the hero and movers, churn where falling water lands, contact-effect constants
//! - `flows`: river and strip flow slots: `WaterFx::strip_flows`, `quad_flows`
//! - `setup`: `WaterFx::new` (pipelines, buffers, layouts) and `prepare` (targets)
//! - `passes`: the per-frame passes: `positions`, `sea_waves`, `shade` and the target accessors

use super::*;

mod shader;
mod shore_map;
mod edge_calm;
mod dry_tiles;
mod bed_min;
mod grid_waves;
mod far_look;
mod far_lod;
mod nwater;
mod fall_wgsl;
mod steep_look;
mod shore_fade;
mod hero_crest;
mod rim;
mod steady;
mod shore_ref;
mod underside;
mod looks;
mod levels;
mod fall_sheets;
mod draws;
mod contacts;
mod wakes;
mod own_spray;
mod skymap;
mod flows;
mod setup;
mod passes;
mod tune;

use self::shader::SHADER;
use self::shore_map::*;
pub(super) use self::looks::*;
pub(super) use self::levels::*;
use self::fall_sheets::*;
pub(super) use self::draws::*;
use self::contacts::*;
use self::wakes::*;
pub(in crate::viewer) use self::skymap::{wgsl as sky_wgsl, SkyMap};
use self::flows::*;
use self::dry_tiles::{DryGroups, DryTiles};
use self::bed_min::{BedMin, BedMinGroups};
use self::grid_waves::{grid_dims, GridGroups, GridWaves};

struct Targets {
	/// (width, height, post generation, the sky map's real targets exist).
	key: (u32, u32, u64, bool),
	wpos: wgpu::TextureView,
	wdepth: wgpu::TextureView,
	copy: wgpu::Texture,
	g_pos: wgpu::BindGroup,
	g_shade: wgpu::BindGroup,
	/// The planar mirrors (mirror.rs), half size: MSAA colour and depth (shared by the passes, each clears them), the resolve
	/// (`MAX_MIRRORS` layers, read by `fs_water` as an array) and its single-layer views for the passes.
	mirror_ms: wgpu::TextureView,
	mirror_layers: Vec<wgpu::TextureView>,
	mirror_depth: wgpu::TextureView,
	/// The water bed (mirror.rs `draw_bed`), half size: MSAA colour, its resolve and MSAA depth (both read by `fs_water`).
	bed_ms: wgpu::TextureView,
	bed: wgpu::TextureView,
	bed_depth: wgpu::TextureView,
	/// [PC] Geometric sea waves: the flat sea mask (`fs_mask`), the grid's group (with the mask) and the shore map's group.
	seamask: wgpu::TextureView,
	g_grid: wgpu::BindGroup,
	g_shore: wgpu::BindGroup,
	/// [PC] The dry tiles of the sea grid (`dry_tiles.rs`).
	dry: DryGroups,
	/// [PC] The 4 x 4 minima of the bed depth (`bed_min.rs`), read by `fs_water` as `bedmin`.
	bed_min: BedMinGroups,
}


pub(super) struct WaterFx {
	layout: wgpu::BindGroupLayout,
	ubuf: wgpu::Buffer,
	sampler: wgpu::Sampler,
	p_pos: wgpu::RenderPipeline,
	p_shade: wgpu::RenderPipeline,
	p_ao_sea: wgpu::RenderPipeline,
	dummy: wgpu::TextureView,
	/// The same as an array view (the mirror slot).
	dummy_arr: wgpu::TextureView,
	dummy_r32: wgpu::TextureView,
	format: wgpu::TextureFormat,
	depth_format: wgpu::TextureFormat,
	start: std::time::Instant,
	/// The geometric sea planes' and the mirrors' heights, steadied (`steady.rs`).
	heave: steady::Heave,
	t: Option<Targets>,
	drift: std::collections::HashMap<(u32, u32, u32, u32), Drift>,
	flows: Vec<[f32; 2]>,
	/// The river quads' uv last frame by the bits of their first corner, and the smoothed uv step per tick (`quad_flows`).
	river_uv: std::collections::HashMap<[u32; 3], [f32; 2]>,
	river_step: [f32; 2],
	/// This frame's flows are river quads (`quad_flows`): they flow in two phases (flows w = -3).
	phased: bool,
	rings: Rings,
	wakes: Wakes,
	/// [PC] This frame's waterfall feet (`quad_flows`: the bottom of each steep river quad) and whether `quad_flows` ran.
	falls: Vec<[f32; 3]>,
	falls_fresh: bool,
	/// [PC] This frame's waterfall sheets (`sheet_falls`) and, made in `positions`, their draw with the mist puffs (`fs_fall`).
	p_fall: wgpu::RenderPipeline,
	/// Group 1 of `p_fall` on a level without a waterfall texture (`fall_fx_tex`).
	fall_dummy: wgpu::BindGroup,
	fall_verts: Vec<Vertex>,
	fall: Option<(wgpu::Buffer, wgpu::Buffer, u32)>,
	/// [PC] Pokitaru's sheets as seen over time (`SheetMem`): their shape does not depend on which triangles retail drew this frame.
	sheet_mem: SheetMem,
	/// The level `sheet_mem` was filled in.
	sheet_level: usize,
	/// The level the shore map's kept quarters (`shore_map.rs` `cs_shore_keep`) belong to: they are cleared when it changes.
	shore_game: usize,
	/// [PC] Geometric sea waves: the mask, grid and shore map pipelines, the shore map's layout, buffer and uniforms, the grid's
	/// index buffer (and its columns, rows), and this frame's grid draw (planes, indices) for `sea_waves`.
	p_mask: wgpu::RenderPipeline,
	p_grid: wgpu::RenderPipeline,
	dry: DryTiles,
	bed_min: BedMin,
	p_shore: wgpu::ComputePipeline,
	p_shore_keep: wgpu::ComputePipeline,
	p_shore_h: wgpu::ComputePipeline,
	p_shore_v: wgpu::ComputePipeline,
	shore_layout: wgpu::BindGroupLayout,
	shore: wgpu::Buffer,
	shore_u: wgpu::Buffer,
	grid_ib: Option<(wgpu::Buffer, u32, u32)>,
	geo_draw: Option<(u32, u32)>,
	/// [PC] Geometric sea: per prop slot its last place and turn and the frames since it last moved (`track_props`), and the
	/// slots that moved within SHORE_STILL frames (`shore_movers`).
	shore_still: Vec<([f32; 6], u32)>,
	shore_movers: Vec<usize>,
	/// [PC] Where the sun reaches the water (skymap.rs).
	sky: SkyMap,
}

/// What the shading pass needs of the frame.
pub(super) struct WaterFrame {
	pub vp: [[f32; 4]; 4],
	pub eye: [f32; 3],
	pub proj: [f32; 4],
	/// The key light's travel direction and brightness (`sunshadow::key_light`), if any.
	pub key: Option<([f32; 3], f32)>,
	/// The level's fog colour (gamma-space 0..1).
	pub fog: [f32; 3],
	/// Graphics level of Water (1 Low, 2 Medium, otherwise High): `march_steps`.
	pub level: u8,
	/// The game level (Pokitaru 11): `shore_ref.rs`.
	pub game: usize,
	/// Graphics -> Waves (retail/pcext.rs `Graphics::waves`): the geometric sea waves (`geo_look`); Off keeps the flat sea.
	pub waves: bool,
}


#[cfg(test)]
mod tests {
	/// A strip whose uv is (x / 4, y / 8): the world-from-uv map is diag(4, 8).
	#[test]
	fn uv_map_inverts_the_fit() {
		let pos = [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [0.0, 8.0, 0.0], [4.0, 8.0, 1.0]];
		let uv = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
		let m = super::uv_map(&pos, &uv).unwrap();
		assert!((m[0][0] - 4.0).abs() < 1e-4 && (m[1][1] - 8.0).abs() < 1e-4 && m[0][1].abs() < 1e-4 && m[1][0].abs() < 1e-4);
	}

	/// Rings: the hero in a pool leaves one at the surface height, a particle far from any water none, a paused game
	/// (same field) nothing new, and spent rings go.
	#[test]
	fn rings_follow_contacts_on_still_water() {
		let pool = [2.0, 0.0, 0.0, 10.0, 10.0];
		let mut r = super::Rings::default();
		r.feed(1, Some([5.0, 5.0, 1.5]), [[5.0, 5.0, 9.0], [7.0, 7.0, 2.2]].into_iter());
		let (rows, n) = r.update(10.0, [pool].into_iter());
		assert_eq!(n, 2);
		assert_eq!(&rows[0][..3], &[5.0, 5.0, 2.0]);
		assert!(rows[1][0] > 0.0 && rows[1][2] > 0.0);
		assert_eq!(&rows[2][..3], &[7.0, 7.0, 2.0]);
		r.feed(1, Some([6.0, 5.0, 1.5]), std::iter::empty());
		assert_eq!(r.update(10.5, [pool].into_iter()).1, 2);
		r.feed(2, None, std::iter::empty());
		assert_eq!(r.update(13.0, [pool].into_iter()).1, 0);
	}

	/// The shader compiles (naga) with both entry pairs.
	#[test]
	fn water_shader_parses() {
		let src = format!("{}{}{}{}{}{}{}{}{}{}", super::SHADER, super::fall_wgsl::FALL_WGSL, super::skymap::wgsl(12, 13), super::edge_calm::EDGE_WGSL, super::far_look::FAR_WGSL, super::steep_look::STEEP_WGSL, super::shore_fade::SHORE_FADE_WGSL, super::shore_map::SHORE_DIST_WGSL, super::hero_crest::HERO_CREST_WGSL, super::rim::RIM_WGSL) + super::underside::UNDER_WGSL + super::dry_tiles::DRY_WGSL + super::grid_waves::GRID_WAVES_WGSL + super::far_lod::FAR_LOD_WGSL + super::nwater::NWATER_WGSL + super::bed_min::BED_MIN_WGSL;
		let m = naga::front::wgsl::parse_str(&src).expect("water shader");
		let names: Vec<_> = m.entry_points.iter().map(|e| e.name.as_str()).collect();
		for n in ["vs_pos", "fs_pos", "vs_full", "fs_water", "vs_grid", "cs_dry", "cs_wave", "cs_bedmin"] {
			assert!(names.contains(&n), "{n}");
		}
		let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
		v.validate(&m).expect("water shader validates");
	}
}
