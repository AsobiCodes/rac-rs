//! The pack's per-level water tables (`looks::Looks`; R&C1: rac1/planet_looks/water.rs) as the water pass reads them: which
//! textures and terrain are sea, pond, river, waterfall, cold, frozen, lava, mud or sludge, and the flow slots the water
//! shader reads them by.

use crate::looks::{Frozen, Liquid};

/// The flow slot of a liquid that is no water (`fs_water`'s lava, mud and sludge looks).
fn slot(l: Liquid) -> u32 {
	match l {
		Liquid::Lava => LAVA_SLOT,
		Liquid::Mud => MUD_SLOT,
		Liquid::Sludge => SLUDGE_SLOT,
	}
}

/// The flow slot of the liquid (no water) the level ocean's draws take (`WaterDraw::liquid`; the pack's `Looks::ocean_liquid`).
pub(super) fn liquid_slot(level: usize) -> Option<u32> {
	(crate::looks::get().ocean_liquid)(level).map(slot)
}

/// [PC] The level ocean is a fog sheet, not a liquid (the pack's `Looks::fog_sheet_ocean`): the Enhanced water pass leaves it
/// as retail draws it (and the volumetric fog may build on it).
pub(in crate::viewer) fn fog_sheet_ocean(level: usize) -> bool {
	(crate::looks::get().fog_sheet_ocean)(level)
}

/// A liquid drawn as level terrain (the pack's `Looks::terrain_liquid`): its terrain texture index (tfrag face key) and slot.
pub(in crate::viewer) fn terrain_liquid(level: usize) -> Option<(u32, u32)> {
	(crate::looks::get().terrain_liquid)(level).map(|(tex, l)| (tex, slot(l)))
}

/// The queue-C pool strips (`Gpu.pool_draws`) that are water, by fx texture (the pack's `Looks::pool_water_tex`).
pub(in crate::viewer) fn pool_water_tex(level: usize) -> &'static [usize] {
	(crate::looks::get().pool_water_tex)(level)
}

/// Pool strips with the cold look (`COLD_SLOT`), by level, fx texture, additive blend and FIX (the pack's `Looks::cold_pool`).
pub(in crate::viewer) fn cold_pool(level: usize, tex: usize, add: bool, fix: u8) -> bool {
	(crate::looks::get().cold_pool)(level, tex, add, fix)
}

/// [PC] Pool strips held still as ice, by level, fx texture, additive blend and FIX (the pack's `Looks::frozen_pool`):
/// `FROZEN_SLOT` for the opaque layers, `FROZEN_CLEAR_SLOT` for the clear ones.
pub(in crate::viewer) fn frozen_pool(level: usize, tex: usize, add: bool, fix: u8) -> Option<u32> {
	(crate::looks::get().frozen_pool)(level, tex, add, fix).map(|f| match f {
		Frozen::Opaque => FROZEN_SLOT,
		Frozen::Clear => FROZEN_CLEAR_SLOT,
	})
}

/// The fx textures (level fx table index, `WorldQuad.tex`) of a sea drawn as effect quads (the pack's `Looks::sea_fx_tex`).
pub(in crate::viewer) fn sea_fx_tex(level: usize) -> &'static [usize] {
	(crate::looks::get().sea_fx_tex)(level)
}

/// [PC] The fx textures of a pond drawn as effect quads (the pack's `Looks::pond_fx_tex`): its own draw, its plane is not the sea's.
pub(in crate::viewer) fn pond_fx_tex(level: usize) -> &'static [usize] {
	(crate::looks::get().pond_fx_tex)(level)
}

/// [PC] The fx textures of falling water drawn as effect quads (the pack's `Looks::fall_fx_tex`); their feet churn the water
/// (`WaterFx::sheet_falls`).
pub(in crate::viewer) fn fall_fx_tex(level: usize) -> &'static [usize] {
	(crate::looks::get().fall_fx_tex)(level)
}

/// The fx texture of a river drawn as effect quads (the pack's `Looks::river_fx_tex`); its uv scrolls the same step every tick
/// on every quad (`WaterFx::quad_flows`).
pub(in crate::viewer) fn river_fx_tex(level: usize) -> Option<usize> {
	(crate::looks::get().river_fx_tex)(level)
}

/// Flow table slots (slot 0 = still water).
pub(super) const FLOWS: usize = 64;
/// The flow slot of still water that retail draws opaque (Hoven's FIX 128 pools): ripples and reflections, no clear shallows.
pub(in crate::viewer) const OPAQUE_SLOT: u32 = FLOWS as u32 - 1;
/// The flow slot of Hoven's abyss water (`cold_pool`): opaque like `OPAQUE_SLOT`, with the cold look `COLD_LOOK`.
pub(in crate::viewer) const COLD_SLOT: u32 = FLOWS as u32 - 2;
/// The flow slots of Hoven's frozen floor (`frozen_pool`), held still (flows w = -1): opaque like `OPAQUE_SLOT`, and clear
/// like slot 0.
pub(in crate::viewer) const FROZEN_SLOT: u32 = FLOWS as u32 - 3;
pub(in crate::viewer) const FROZEN_CLEAR_SLOT: u32 = FLOWS as u32 - 4;
/// The flow slot of Gaspar's lava (`WaterDraw::liquid`, flows w = -2): `fs_water`'s lava look.
pub(in crate::viewer) const LAVA_SLOT: u32 = FLOWS as u32 - 5;
/// The flow slots of Aridia's mud (`liquid_slot`, flows w = -4) and Orxon's toxic sludge (`terrain_liquid`, flows w = -5):
/// `fs_water`'s mud and sludge looks.
pub(in crate::viewer) const MUD_SLOT: u32 = FLOWS as u32 - 6;
pub(in crate::viewer) const SLUDGE_SLOT: u32 = FLOWS as u32 - 7;
/// The lowest reserved slot: flowing water takes the slots below it.
pub(super) const LAST_FLOW: u32 = SLUDGE_SLOT;
/// [PC] The cold look of Hoven's abyss (`fs_water`): 1 = long slow swells and a higher reflectance floor (what stands in the
/// water mirrors), 2 = also a cool light on the swell and a moonlit shimmer, 3 = also an icy rim where it meets the ground.
pub(super) const COLD_LOOK: f32 = 2.0;

/// Levels whose world strips (`Gpu::strip_draws`) are water (the pack's `Looks::strips_are_water`).
pub(in crate::viewer) fn strips_are_water(level: usize) -> bool {
	(crate::looks::get().strips_are_water)(level)
}

/// Whether a world strip's water (Novalis' river and pools) drifts its ripples with the strip's texture scroll. Off: the
/// ripples stand in world space, so neighbouring pieces match and nothing jumps; retail's own scroll stays in the picture.
pub(super) const STRIP_FLOW: bool = false;
