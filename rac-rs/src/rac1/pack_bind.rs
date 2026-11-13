//! R&C1's answers to the engine's window and renderer (engine split stage E2 batch 4a, 2026-10-26; the engine side is
//! `engine/pack.rs`). Handed over by main.rs before the window opens.

use crate::pack::Pack;
use crate::retail;

pub static RAC1: Pack = Pack {
	kbm: &crate::kbm_actions::TABLE,
	graphics: retail::pcext::menu_graphics,
	debug_mode,
	running: retail::active,
	pc_ext: retail::pcext::enabled,
	veg_for_load: retail::pcext::veg_for_load,
	ui_sharp: retail::ui_sharp,
	capture_features: retail::gpu_capture_features,
	capture_window: retail::gpu_capture_window,
	capture_submit_lock: retail::gpu_capture_submit_lock,
	prompt_class: crate::game::vendor::PROMPT_CLASS,
	portrait_class: crate::game::vendor::PORTRAIT_CLASS,
	mesh_fallback: (crate::game::BOLT, crate::game::BOLT + 4),
	moby_limit: crate::particles::MOBY_LIMIT,
	fx_clamped,
	fx_glass,
	world_art: crate::kofi::world_art,
	looks: &crate::planet_looks::LOOKS,
	metal_textures: crate::metal_materials::METAL_TEXTURES,
	metal_subdued_textures: crate::metal_equipment::TEXTURES,
	metal_subdued_classes: crate::metal_equipment::CLASSES,
	smoke: crate::smoke_table::ROWS,
	smoke_quads: crate::smoke_table::quad_rows,
	shot_lights: crate::weapon_lights::ROWS,
	fire_rows: crate::fire_look::rows,
	fire_mobys: crate::fire_look::mobys,
	far: (rac_engine::viewer::RETAIL_FAR, rac_engine::viewer::RETAIL_FAR_RAW),
	player: "player",
	// The mods folder of an install, as ever: <install root>/mods.
	mods: crate::pack::ModsRule { game: "rac1", beside_player: false },
	font_patch: Some(font_patch),
	dx12_writer: Some(retail::pcext::renderer_switch::set_dx12),
	fps_setting: retail::pcdisp::fps_setting,
	note_refresh: retail::pcdisp::note_refresh,
	render_scale_pct: retail::pcdisp::render_scale_pct,
	reset_sizes: Some(crate::pack::SizeReset { names: "Upscaler, Render scale and Aspect ratio", reset: retail::pcext::ui_bridge::reset_sizes }),
	is_usa: retail::disc::usa,
	fur: crate::hero_fur::spec,
	achievements: Some(&crate::trophies::PACK),
	ui: Some(&crate::modern_menu::PACK),
	ui_sound: rac_engine::ui::sound::request,
	diag_facts: crate::diag_facts::get,
	scene_msaa_default: SCENE_MSAA_DEFAULT,
};

/// The scene MSAA mode (engine scene_msaa.rs): Off with DLSS/FSR (user 2026-11-08 after test build 13: "Yes, make it default",
/// then "MSAA automatic": no menu row; DLSS / FSR smooth the edges themselves, +5..10 % fps at DLAA). Without a running upscaler 4x.
pub const SCENE_MSAA_DEFAULT: u8 = 1;

/// fx table keys with a clamped variant: the clamped textures of the queue-B and queue-Q2 callbacks.
fn fx_clamped(k: usize) -> bool {
	retail::queue_b::FX_CLAMPED.contains(&k) || retail::queue_q2::FX_CLAMPED.contains(&k)
}

/// fx table keys with a no-mip glass variant (queue_q1 `GLASS_TEX`).
fn fx_glass(k: usize) -> bool {
	retail::queue_q1::GLASS_TEX.contains(&k)
}

/// Button glyphs option (retail/glyphs.rs): the chosen style's cells into a font sheet; the sheet as sent when it patched and
/// `want_orig` is set. Default style + default bindings: nothing to do.
fn font_patch(data: &mut [u128], want_orig: bool) -> Option<Vec<u128>> {
	use retail::glyphs as gl;
	let want = gl::wanted();
	let cells = if gl::is_retail(&want) { None } else { gl::patch().and_then(|p| p.find(data).map(|i| gl::cells_for(&p, i, &want))) };
	let original = crate::translation_probe::patch_texture_words(data, want_orig)
        .or_else(|| (want_orig && cells.is_some()).then(|| data.to_vec()));
	if let Some(cells) = cells.as_ref() { gl::patch_qwords(data, cells); }
	original
}

/// Debug mode: Pause -> Options -> Controls -> Debug, or RAC_DEV_TOOLS=1 / RAC_DEBUG_KEYS=1 (`devtools::NOTICE` lists the keys).
fn debug_mode() -> bool {
	retail::devtools::enabled() || retail::debugkeys::enabled()
}
