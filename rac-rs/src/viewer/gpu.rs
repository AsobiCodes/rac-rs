//! The wgpu renderer of the viewer: device and pipelines, scene upload, prop slots, per-frame draw.
//! The `Gpu` struct lives in `viewer.rs`; this module holds its `impl` blocks, one file per job:
//! - `setup`:        Gpu::new (instance, adapter, device, surface, every pipeline and buffer), resize, surface configuration and present mode.
//! - `renderer`:     Which graphics API the window opens (Vulkan / DirectX 12 from the options file), DXC for DX12, DX12 frame latency.
//! - `scene_ctx`:    SceneCtx (the device handles of the scene build) and its texture upload and batching, with the Gpu wrappers around it.
//! - `scene_build`:  Level scene build (terrain, ties, shrubs, sky) on the build context, and its parallel vertex helpers.
//! - `props_layout`: Prop slot layout: class faces, draw layout, slot allocation, and growing the props batch (append / regrow).
//! - `props_set`:    Per-frame prop slot rebuild (set_props): slot jobs and sources, GPU records, preview and panel poses, debris.
//! - `prop_clip`:    Retail polygon clipper for props: draw-distance alphas, per-prop verdicts and the clipped polygon queue.
//! - `marker`:       The posed hero marker model (set_marker) and its clipped polygons.
//! - `item_panel`:   Shop portrait pose and the item panel render targets.
//! - `metal`:        Metal (chrome) mobys: view and palette rows, packet to vertex conversion, upload and the per-frame metal props.
//! - `lighting`:     Frame uniforms and fog, dynamic light rows, relight of the scene vertices, and the moby / hero light words.
//! - `culling`:      Per-frame culling of ties, shrubs and tfrags, the far draw level, and the ice tag.
//! - `particle_fx`:  Particle sprites, ribbons and lines (set_particles) and their view tests.
//! - `sky_water`:    Sky spin and sprites, ocean surfaces (set_ocean, set_ocean_ram) and the sea shading mesh.
//! - `hud_art`:      HUD art textures and the retail overlay texture.
//! - `rscale`:       Render scale: the picture drawn at a percentage of the window size, the copy onto the window, the window size.
//! - `reshade`:      ReShade add-on: registers with a loaded ReShade, depth + picture before the HUD (files in `reshade/`).
//! - `warp_actors`:  Warp (travel) actors: load, pose and clear.
//! - `draw`:         The per-frame draw: Gpu::draw, strip drawing, debug switches, water mirror choice.
//! - `tests`:        Unit tests of the renderer helpers (kept together; they reach every sibling through `use super::*`).

use super::*;
use rayon::prelude::*;

mod setup;
pub(in crate::viewer) mod renderer;
mod scene_ctx;
pub(in crate::viewer) mod repl_upload;
pub(in crate::viewer) mod split_orig;
mod scene_build;
mod props_layout;
mod props_set;
mod prop_clip;
mod marker;
mod item_panel;
mod metal;
mod lighting;
mod culling;
mod particle_fx;
mod sky_water;
mod hud_art;
pub(in crate::viewer) mod reshade;
pub(in crate::viewer) mod rscale;
mod warp_actors;
mod draw;
mod tests;

use self::setup::*;
pub(super) use self::scene_ctx::*;
use self::scene_build::*;
pub(super) use self::props_layout::*;
pub(super) use self::props_set::*;
use self::prop_clip::*;
use self::metal::*;
use self::lighting::*;
use self::culling::*;
use self::particle_fx::*;
use self::sky_water::*;
use self::draw::*;
