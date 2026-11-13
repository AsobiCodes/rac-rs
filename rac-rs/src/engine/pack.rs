//! The game pack's part of the window and renderer (engine split stage E2 batch 4a, 2026-10-26): the answers the engine asks the
//! game that runs (its Graphics page, its Debug mode, the vegetation row of a level load, the sharp UI rule, the GS capture's
//! device hooks). Set once per process by the pack (`set`; R&C1: `rac1/pack_bind.rs` `RAC1`, from main.rs before the window
//! opens); without one (tools, unit tests) the engine sees `NONE`: no game, the Classic picture.

use crate::gfx::Graphics;
use std::sync::{Mutex, MutexGuard, OnceLock};

/// Metal kinds of `Pack::metal_textures` (2 bits on the draw; viewer/metalfx.rs `KIND_SHIFT`).
pub const METAL_POLISHED: u8 = 0;
pub const METAL_MIRROR: u8 = 1;
pub const METAL_BRUSHED: u8 = 2;
pub const METAL_PAINTED: u8 = 3;

pub struct Pack {
	/// The keyboard + mouse action table (`input/kbm`); `NONE` and packs without their own use the generic PS2 table.
	pub kbm: &'static crate::input::kbm::Table,
	/// The Graphics page's verdict before the split view (render thread, every frame; `gfx::graphics`).
	pub graphics: fn() -> Graphics,
	/// The game's Debug mode is on: the split view's keys work and texture packs keep the original textures (`splitview`).
	pub debug_mode: fn() -> bool,
	/// A game session runs: the vegetation follows the game's menu (viewer/vegfx.rs), the prop batch keeps room for grown pools.
	pub running: fn() -> bool,
	/// The game's PC extensions are on (off = the retail smoke path: no vegetation).
	pub pc_ext: fn() -> bool,
	/// The Vegetation row for a level load (viewer/vegfx.rs `plan_veg`).
	pub veg_for_load: fn() -> u8,
	/// The sharp UI rule for a window this many pixels high (the mods' draw list, viewer/gpu/draw/pass_omni.rs).
	pub ui_sharp: fn(u32) -> bool,
	/// The GS capture's device hooks (viewer/gpu/setup.rs): the features it needs, the window's device, the lock held around
	/// `Surface::configure`.
	pub capture_features: fn(&wgpu::Adapter) -> wgpu::Features,
	pub capture_window: fn(&wgpu::Device, &wgpu::Queue, &wgpu::AdapterInfo, &wgpu::Adapter),
	pub capture_submit_lock: fn() -> MutexGuard<'static, ()>,
	/// The shop's purchase prompt class (its own batch between HUD groups, BRIDGE-36) and portrait class (BRIDGE-37); u32::MAX = none.
	pub prompt_class: u32,
	pub portrait_class: u32,
	/// Prop classes `first..end` without a mesh of their own draw class `first`'s (R&C1: the 5, 20 and 50 bolt pieces the bolt's).
	pub mesh_fallback: (u32, u32),
	/// The game's moby pool size: the debris instance buffers hold this many pieces of the largest class (viewer/gpu/props_set.rs).
	pub moby_limit: usize,
	/// fx table keys that also get a clamped / a no-mip glass variant (the split view's originals, viewer/gpu/split_orig.rs).
	pub fx_clamped: fn(usize) -> bool,
	pub fx_glass: fn(usize) -> bool,
	/// [PC] Art the game hangs in its world on this level (loaded level index; viewer/gpu/draw/pass_art.rs), None = nothing.
	pub world_art: fn(usize) -> Option<&'static WorldArt>,
	/// The game's per-level looks (engine/looks.rs: water, sky, clouds, fog, ice, vegetation by level).
	pub looks: &'static crate::looks::Looks,
	/// Original texture identities selected by the game pack for the optional metal look (sorted by identity), each with its
	/// metal kind (`METAL_POLISHED` .. `METAL_PAINTED`: roughness and reflection per kind, viewer/metalfx.rs).
	pub metal_textures: &'static [(u64, u8)],
	/// Materials and original reflection classes that take a gentler added sheen.
	pub metal_subdued_textures: &'static [u64],
	pub metal_subdued_classes: &'static [u32],
	/// The smoke puffs of the effect pool the volumetric smoke (viewer/volsmoke.rs) turns into volumes (engine/smoke_rows.rs); none = no smoke volumes.
	pub smoke: &'static [crate::smoke_rows::SmokeRow],
	/// The alpha-blended world quads of loaded level `n` that are smoke (a level fire's grey columns and volume sheets): the volumetric
	/// smoke turns them into volumes and the quad pass leaves them out (engine/smoke_rows.rs `QuadRow`). None = they stay quads.
	pub smoke_quads: fn(usize) -> &'static [crate::smoke_rows::QuadRow],
	/// [PC] Weapon lights (engine/shot_rows.rs, viewer/shotlight.rs): what the game's shots light from (bolts, rockets, flame streams,
	/// arcs); none = no weapon lights.
	pub shot_lights: &'static [crate::shot_rows::ShotRow],
	/// Which draws of a level are fire (engine/fire_option.rs `FireRow`, viewer/firefx.rs): fx table keys of the level's fire quads, particle
	/// texture keys of flames and explosions. None = the retail look, whatever the Fire option says.
	pub fire_rows: fn(usize) -> &'static [crate::fire_option::FireRow],
	/// Moby classes of a level that are fire (a fireball's sphere, a pilot flame): their meshes draw as flames while Fire is on
	/// (viewer/gpu/moby_fire.rs). None = the retail look.
	pub fire_mobys: fn(usize) -> &'static [u32],
	/// The game's world far plane: (effective, in m; raw, in 1/1024 m, the tfrag sphere test's) (viewer/projection.rs
	/// `retail_far`). R&C1: `RETAIL_FAR`, 745472.0.
	pub far: (f32, f32),
	/// The player dir when RAC_PLAYER_DIR is unset, relative to rac-rs (`pcopts::player_dir`; an install's Play / launcher sets
	/// RAC_PLAYER_DIR). R&C1: "player"; a second game its own, so cards and options never mix.
	pub player: &'static str,
	/// Where the game keeps its mods (texture packs, script mods) and which `mod.json` they are made for (`texpack::mods`).
	pub mods: ModsRule,
	/// [PC] Button glyphs option: the game's font-sheet patcher (gscap/font.rs `FontPatch`), None = sheets stay as sent.
	pub font_patch: Option<crate::gscap::FontPatch>,
	/// [PC] The game's own `renderer=dx12` writer for the RivaTuner crash box (rtss_crash.rs `Dx12Writer`), None = the
	/// plain pc_options.txt edit.
	pub dx12_writer: Option<crate::rtss_crash::Dx12Writer>,
	/// [PC] FPS limit row: 0 = the display's refresh rate, `u32::MAX` = Off, else the limit in Hz (viewer/pace.rs).
	pub fps_setting: fn() -> u32,
	/// [PC] The pacer saw the monitor's refresh rate (R&C1 shows it in the FPS limit row).
	pub note_refresh: fn(u32),
	/// [PC] Render scale row in percent, 100 = the window size (viewer/gpu/rscale.rs; RAC_RENDER_SCALE wins).
	pub render_scale_pct: fn() -> u32,
	/// [PC] The game's options that size the render targets, for lasting wgpu errors (viewer/gpu_errors.rs `frame_check`). None =
	/// the game has none: a lasting error crashes at its first step (R&C2).
	pub reset_sizes: Option<SizeReset>,
	/// The disc is the USA one (the retail overlay guard, viewer/hud_hold.rs `Held::next`).
	pub is_usa: fn() -> bool,
	/// [PC] Fur on the hero model (viewer/furfx.rs): which of its textures are fur and how each part looks. None = no fur.
	pub fur: fn() -> Option<&'static crate::furspec::FurSpec>,
	/// [PC] The game's achievements: its list and detector (engine/achievements.rs), None = the game has none (R&C2).
	pub achievements: Option<&'static crate::achievements::AchPack>,
	/// [PC] The modern options screen (engine/ui): its rows and the hooks to read and write them. None = the game has no modern menu.
	pub ui: Option<&'static crate::ui::UiPack>,
	/// [PC] A sound of the modern UI (move, select, back); default none.
	pub ui_sound: fn(crate::ui::UiSound),
	/// [PC] RAC_DIAG (diag.rs): the game's facts for the diagnostic record (level, paused / menu, hero, camera); default none.
	pub diag_facts: fn() -> crate::diag::Facts,
	/// [PC] The scene MSAA mode when the options file has no `scene_msaa` (viewer/scene_msaa.rs `NAMES` index; no menu row): 0 = 4x,
	/// 1 = Off while DLSS / FSR runs; default 4x.
	pub scene_msaa_default: u8,
}

/// [PC] A game's size options (`Pack::reset_sizes`; R&C1: rac1/retail/pcext/ui_bridge.rs `reset_sizes`).
#[derive(Clone, Copy)]
pub struct SizeReset {
	/// Their names for the log and the crash text (R&C1: "Upscaler, Render scale and Aspect ratio").
	pub names: &'static str,
	/// Main thread, on any screen (boot, title, level): puts them back to their defaults, in force from the next frame, and saves
	/// them. false = not possible yet (the game has not read its options file): asked again next frame.
	pub reset: fn() -> bool,
}

/// The mods folder and the mod format id of a game (`Pack::mods`).
#[derive(Clone, Copy)]
pub struct ModsRule {
	/// The `game` a mod.json must name to load here (R&C1 "rac1", R&C2 "rac2"); a mod.json without `game` loads in any game.
	pub game: &'static str,
	/// The mods folder is `mods` next to the player dir (the launcher's `<game>/mods` beside `<game>/player`) instead of
	/// `<install root>/mods`. `RAC_MODS_DIR` overrides both.
	pub beside_player: bool,
}

/// [PC] Art a game hangs in its world (`Pack::world_art`; R&C1: the Ko-fi drawings on the garage wall, rac1/kofi.rs): textured
/// quads in the level pass, alpha blended, depth-tested without depth write, uploaded once (one art set per process).
pub struct WorldArt {
	/// Each sheet's texture and its four corners in the world (uv 0,0 / 1,0 / 1,1 / 0,1).
	pub sheets: Vec<(crate::level::Texture, [[f32; 3]; 4])>,
	/// The colour every sheet is multiplied by.
	pub tint: [f32; 4],
}

/// No game: the Classic picture, no Debug mode, no vegetation, no GS capture, no shop, moby pool or world art, the retail looks.
pub static NONE: Pack = Pack {
	kbm: &crate::input::kbm::generic::TABLE,
	graphics: classic,
	debug_mode: no,
	running: no,
	pc_ext: no,
	veg_for_load: zero,
	ui_sharp: |_| false,
	capture_features: |_| wgpu::Features::empty(),
	capture_window: |_, _, _, _| {},
	capture_submit_lock: own_lock,
	prompt_class: u32::MAX,
	portrait_class: u32::MAX,
	mesh_fallback: (u32::MAX, u32::MAX),
	moby_limit: 0,
	fx_clamped: |_| false,
	fx_glass: |_| false,
	world_art: |_| None,
	looks: &crate::looks::NONE,
	metal_textures: &[],
	metal_subdued_textures: &[],
	metal_subdued_classes: &[],
	smoke: &[],
	smoke_quads: |_| &[],
	shot_lights: &[],
	fire_rows: |_| &[],
	fire_mobys: |_| &[],
	far: (crate::viewer::RETAIL_FAR, crate::viewer::RETAIL_FAR_RAW),
	player: "player",
	mods: ModsRule { game: "rac1", beside_player: false },
	font_patch: None,
	dx12_writer: None,
	fps_setting: || 0,
	note_refresh: |_| {},
	render_scale_pct: || 100,
	reset_sizes: None,
	is_usa: no,
	fur: || None,
	achievements: None,
	ui: None,
	ui_sound: |_| {},
	diag_facts: crate::diag::Facts::default,
	scene_msaa_default: 0,
};

fn classic() -> Graphics {
	Graphics::CLASSIC
}

fn no() -> bool {
	false
}

fn zero() -> u8 {
	0
}

fn own_lock() -> MutexGuard<'static, ()> {
	static L: Mutex<()> = Mutex::new(());
	L.lock().unwrap_or_else(|e| e.into_inner())
}

static PACK: OnceLock<&'static Pack> = OnceLock::new();

/// The pack hands over its part (once per process; the first wins).
pub fn set(p: &'static Pack) {
	let _ = PACK.set(p);
	crate::localization::install();
}

/// The pack's part, or `NONE`.
#[inline]
pub fn get() -> &'static Pack {
	PACK.get().copied().unwrap_or(&NONE)
}
