//! A game's per-level looks (engine split stage E2 batch 5, 2026-10-26: the per-planet tables moved out of the effect code into
//! the game pack). The effects ask `get()` which draws of a level are water, lava, mud, sludge or ice, where its sky paints a
//! sun, which sky shells turn, how its clouds, stars and fog look; a game pack fills a `Looks` (R&C1: rac1/planet_looks.rs) and
//! hands it over in its `Pack`. Without a game (`NONE`) every level keeps its retail look: no liquids or ice, no painted sun,
//! no turning shells, the retail sky (no volumetric clouds), the fog from the retail fog alone. Level = the loaded level index.

/// The fog floor's defaults (engine/viewer/volfog/floor.rs): a cell's ground reach in cells, the least soft height.
pub use crate::viewer::volfog::floor::{REACH as FLOOR_REACH, SOFT_MIN as FLOOR_SOFT_MIN};

/// A liquid that is no water: the look of `fs_water` (its flow slot, engine/viewer/waterfx/levels.rs) a level ocean or a
/// terrain liquid takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Liquid {
	Lava,
	Mud,
	Sludge,
}

/// A frozen pool layer, held still: opaque, or clear like still water.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frozen {
	Opaque,
	Clear,
}

pub struct Looks {
	// Water (engine/viewer/waterfx/levels.rs, shore_ref.rs).
	/// The level ocean is this liquid, not water.
	pub ocean_liquid: fn(usize) -> Option<Liquid>,
	/// The level ocean is a fog sheet (the Enhanced water pass leaves it as retail draws it).
	pub fog_sheet_ocean: fn(usize) -> bool,
	/// A liquid drawn as level terrain: its terrain texture index (tfrag face key) and kind.
	pub terrain_liquid: fn(usize) -> Option<(u32, Liquid)>,
	/// The queue-C pool strips that are water, by fx texture.
	pub pool_water_tex: fn(usize) -> &'static [usize],
	/// Pool strips (level, fx texture, additive, FIX) with the cold look, and those held still as ice.
	pub cold_pool: fn(usize, usize, bool, u8) -> bool,
	pub frozen_pool: fn(usize, usize, bool, u8) -> Option<Frozen>,
	/// The fx textures of a sea, a pond and falling water drawn as effect quads, and of a river.
	pub sea_fx_tex: fn(usize) -> &'static [usize],
	pub pond_fx_tex: fn(usize) -> &'static [usize],
	pub fall_fx_tex: fn(usize) -> &'static [usize],
	pub river_fx_tex: fn(usize) -> Option<usize>,
	/// The level's world strips are water.
	pub strips_are_water: fn(usize) -> bool,
	/// The measured tide (lowest, highest plane) of the level's sea.
	pub known_tide: fn(usize) -> Option<(f32, f32)>,
	// The sky (post.rs, gpu/lighting.rs, gpu/sky_water.rs, star.rs, starfield.rs, volclouds/params.rs).
	/// The direction towards the sun the level's sky paints.
	pub sky_sun: fn(usize) -> Option<[f32; 3]>,
	/// The Enhanced sun (shadows, sky map, fog, clouds) follows the painted sun; `sky_key_lit`: the Enhanced shading too (light
	/// slot 0 of the frame uniforms: moby and hero shading, Metal highlights on the world and mobys, fog glow, smoke).
	pub sky_key: fn(usize) -> bool,
	pub sky_key_lit: fn(usize) -> bool,
	/// Of the `sky_key` levels, those whose water sky map (waterfx/skymap.rs: the water's shading from the ground towards the sun)
	/// follows the painted sun too; the others keep retail's key light there (an approved water look stays as it was).
	pub sky_key_water: fn(usize) -> bool,
	/// The z angle of a sky shell (level, shell, game frame), None for a shell that keeps its orientation.
	pub shell_angle: fn(usize, usize, u32) -> Option<f32>,
	/// The face key (without wrap bits) of the sky patch the animated star takes over.
	pub star: fn(usize) -> Option<u32>,
	/// The star field's twinkle dip depth.
	pub twinkle: fn(usize) -> f32,
	/// The cloud look (None: the retail sky stays), the retail sky shell draws the cloud sky replaces (a bit per draw), and
	/// whether it draws its own sun disc.
	pub clouds: fn(usize) -> Option<CloudLook>,
	pub sky_mask: fn(usize) -> u64,
	pub sun_disc: fn(usize) -> bool,
	/// A level with no sun to cast from (space and the ship insides under their roofs): Dynamic shadows draw no cascades there
	/// and the retail shadows stay (sunshadow.rs `casts`).
	pub sunless: fn(usize) -> bool,
	// The fog (volfog/density.rs, volfog/floor.rs).
	/// Open-air haze 0..1 on top of the retail fog weight, and the highest world z the fog pool's top may stand at.
	pub air: fn(usize) -> f32,
	pub pool_cap: fn(usize) -> f32,
	/// The fog floor's shape, and whether the level's bottom is a liquid (no floor).
	pub fog_floor: fn(usize) -> FogFloor,
	pub liquid_bottom: fn(usize) -> bool,
	// Ice and vegetation (icefx.rs, vegfx/).
	/// The ice textures: (tie keys, shrub keys).
	pub ice_keys: fn(usize) -> (&'static [u32], &'static [u32]),
	/// A dark swamp level (its plants grow on any ground texture), a level whose waters are natural (sea plants).
	pub swamp: fn(usize) -> bool,
	pub natural_water: fn(usize) -> bool,
}

impl Looks {
	/// A `sky_key_lit` level whose water does not follow the painted sun (`sky_key_water` false): the water's bed and mirror
	/// passes keep retail's key light in light slot 0 (gpu/lighting.rs `water_uniforms`), so the approved water stays as it was.
	pub fn water_keeps_key(&self, level: usize) -> bool {
		(self.sky_key_lit)(level) && !(self.sky_key_water)(level)
	}
}

/// No game: every level keeps its retail look.
pub const NONE: Looks = Looks {
	ocean_liquid: |_| None,
	fog_sheet_ocean: |_| false,
	terrain_liquid: |_| None,
	pool_water_tex: |_| &[],
	cold_pool: |_, _, _, _| false,
	frozen_pool: |_, _, _, _| None,
	sea_fx_tex: |_| &[],
	pond_fx_tex: |_| &[],
	fall_fx_tex: |_| &[],
	river_fx_tex: |_| None,
	strips_are_water: |_| false,
	known_tide: |_| None,
	sky_sun: |_| None,
	sky_key: |_| false,
	sky_key_lit: |_| false,
	sky_key_water: |_| false,
	shell_angle: |_, _, _| None,
	star: |_| None,
	twinkle: |_| 0.35,
	clouds: |_| None,
	sky_mask: |_| 0,
	sun_disc: |_| false,
	sunless: |_| false,
	air: |_| 1.0,
	pool_cap: |_| f32::INFINITY,
	fog_floor: |_| FogFloor::PLAIN,
	liquid_bottom: |_| false,
	ice_keys: |_| (&[], &[]),
	swamp: |_| false,
	natural_water: |_| false,
};


/// How a planet's sky takes clouds: the mix of the four genera (engine/viewer/volclouds/genera.rs) and their altitudes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CloudLook {
	/// Coverage (0..1) of each genus: cumulus (single heaps of many sizes, from small puffs to towers), stratocumulus (a broad deck
	/// of soft rolls low in the shell), altocumulus (rippled wave bands higher up), cirrus (thin streaks) and cirrocumulus (small
	/// cells in patches) in the veil at the top.
	pub cu: f32,
	pub sc: f32,
	pub ac: f32,
	pub ci: f32,
	pub cc: f32,
	/// Vertical development of the cumulus: 0 flat fair-weather heaps, 0.5 congestus, 1 cumulonimbus towers to the shell's top.
	pub tower: f32,
	/// Cloud shell base and top (the tallest towers), the altocumulus band and the cirrus veil above the level's reference height
	/// (world units).
	pub base: f32,
	pub top: f32,
	pub mid: f32,
	pub high: f32,
	/// Extinction scale (thicker, darker clouds above 1) and the distance of the aerial haze (world units).
	pub density: f32,
	pub haze: f32,
	/// Wind speed (world units per second) and heading (radians).
	pub wind: f32,
	pub heading: f32,
	/// Sun gain and how far the sun's colour leans to the painted sun's hue (0 the key light's colour, 1 the painted hue).
	pub sun: f32,
	pub hue: f32,
	/// 0 day; 1 dusk: dark bodies lit by the sky away from the low sun, the thin fringes catching it.
	pub dusk: f32,
	/// Gain of the dusk ambient (the sky away from the sun lights the bodies).
	pub dusk_amb: f32,
	/// Gain of the sky ambient on the cloud bodies (brighter bodies against a bright hazy sky above 0.9).
	pub amb: f32,
	/// Scale of the cloud fields and shapes (1 = weather tile 14000, shape tile 3000 world units).
	pub size: f32,
	/// Extra cumulus coverage of the far band low in the sky (on top of `cu`).
	pub farcov: f32,
	/// Grade of the sky model (0 the painted sky's colours; 1 paler, softer, hazier toward the horizon).
	pub pale: f32,
	/// Exposure of the cloud light, and extra soft multi-scattered light in the bodies (softer, lower-contrast shading).
	pub expo: f32,
	pub soft: f32,
	/// The main light's colour in place of the level key light's (night: a cool moonlight; storm: a flat grey), or None.
	pub tint: Option<[f32; 3]>,
	/// Cumulonimbus form (1 on the storm levels: leaning towers of their own height and width with cauliflower crowns, all on the
	/// shell's base; 0 the fair-weather towers of the heaps), the fringe light's gain (times `dusk`), the silver lining (1 the full
	/// forward glow toward the sun, 0 none: an overcast), the undersides' light (below 1 darker) and how soft the cloud surfaces are
	/// (0 crisp painted masses; above 0 diffuse, no hard edges).
	pub form: f32,
	pub rim: f32,
	pub lining: f32,
	pub under: f32,
	pub feather: f32,
}

/// One level's floor shape (2026-10-25, user: "Eudora needs higher fog", "Kerwan's buildings float"): `lift` raises the clear top,
/// `soft` is the least soft height, `cap` the most (a short soft top = a crisp fog deck), `reach` the ground map's reach in cells.
pub struct FogFloor {
	pub lift: f32,
	pub soft: f32,
	pub cap: f32,
	pub reach: usize,
	/// How far the floor's colour is pulled from the retail fog colour toward grey (0..1).
	pub desat: f32,
	/// How far it is then pulled toward `under` (0..1): what retail shows under the level (its sky below the horizon, gamma-space
	/// GS bytes), which its alpha-faded bottoms fade into.
	pub to_under: f32,
	pub under: [u8; 3],
	/// One clear top for the whole level (the level's lowest clear top), not the per-cell ground.
	pub flat: bool,
	/// Extinction scale (x `SIGMA`): a thin floor under a tall soft top fades what goes down into it over many units.
	pub sigma: f32,
	/// > 0: the clear top over the void lies this far over the render geometry's bottom there, not at the nearest ground (`lower_voids`).
	pub void_fade: f32,
}

impl FogFloor {
	/// A plain floor: the fog colour, the least soft height, the ground map's reach.
	pub const PLAIN: FogFloor = FogFloor { lift: 0.0, soft: FLOOR_SOFT_MIN, cap: 1.0e9, reach: FLOOR_REACH, desat: 0.0, to_under: 0.0, under: [0; 3], flat: false, sigma: 1.0, void_fade: 0.0 };
}

#[cfg(any(test, feature = "test-api"))]
thread_local! {
	static TEST: std::cell::Cell<Option<&'static Looks>> = const { std::cell::Cell::new(None) };
}

/// Tests: runs `f` with `looks` as this thread's looks.
#[cfg(any(test, feature = "test-api"))]
pub(crate) fn with<R>(looks: &'static Looks, f: impl FnOnce() -> R) -> R {
	TEST.with(|t| t.set(Some(looks)));
	let r = f();
	TEST.with(|t| t.set(None));
	r
}

/// The game's looks (`Pack::looks`), or `NONE`.
#[inline]
pub fn get() -> &'static Looks {
	#[cfg(any(test, feature = "test-api"))]
	if let Some(l) = TEST.with(|t| t.get()) {
		return l;
	}
	crate::pack::get().looks
}
