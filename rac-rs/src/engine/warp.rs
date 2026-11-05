//! The planet-travel warp as a game pack hands it to the renderer (engine split stage E2 batch 4c, 2026-10-26: moved out of
//! rac1/scene.rs and game/transit.rs, which re-export them): the warp bundle's assets (`WarpAssets`, uploaded by
//! viewer/warpfx.rs `load_warp` and gpu/warp_actors.rs) and one frame of its draw pass (`WarpFrame`).

use crate::physics::V3;

/// Everything the warp bundle installs besides the streams (level_transition.md SCN-4.1b, loader `FUN_002a5198`
/// OV:103758-103809): the three ship classes with their own textures, the star-fan sky, the HUD (fx) textures, the
/// particle textures and defs, and the chrome texture.
pub struct WarpAssets {
	/// (class id, decoded class, core block kept for the skeleton) in table order; entries with no core are skipped.
	pub classes: Vec<(u32, crate::mobies::MobyClass, Vec<u8>)>,
	/// The bundle's moby texture table (the classes' remaps index it).
	pub moby_textures: Vec<Option<crate::level::Texture>>,
	pub sky: crate::sky::Sky,
	/// fx slot order: slot 0 the ribbon texture, 2 and 3 the emblem.
	pub fx_textures: Vec<crate::level::Texture>,
}

/// `fx_tex` key the big quad is uploaded under in the warp table (the ribbon texture is slot 0).
pub const WARP_BIG_TEX: usize = 0x100;

/// One frame of the arrival warp's draw pass (SCN-4.2), everything but the ship and the gloss.
#[derive(Clone, Debug, Default)]
pub struct WarpFrame {
	/// Star fan: (shell, scaled rotation rows, translation in sky units).
	pub fan: Vec<(usize, [[f32; 3]; 3], V3)>,
	/// Projection scalar of the fan (the camera key's, at least 0.63).
	pub fan_tan: f32,
	/// World quads in draw order: the big quad (type 4), then the ribbon strips (tex 0, additive).
	pub quads: Vec<crate::prims::WorldQuad>,
	/// Banner colour word `(a << 24) | 0x808080` (type 4, tick > 60).
	pub banner: Option<u32>,
	/// Black fade quad alpha, 0..=128 (0 = none).
	pub fade: u8,
}
