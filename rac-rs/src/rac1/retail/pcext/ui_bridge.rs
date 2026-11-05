//! The bridge between the engine's modern Options screen (`rac1/modern_menu`, drawn on the main thread) and the PC option RAM the
//! worker owns. The screen hands over `Change{id, value}` (`set`, any thread, queued in `CMDS`); the worker applies them in
//! `sync` right after `setup` and BEFORE the Preset row and the `GROWS` read (`drain`), and publishes the current values at the
//! end of `sync` (`publish`, read back by `get`). The screen never touches RAM; the classic Graphics / Camera pages keep working on
//! the same bytes, so both menus always agree.
//!
//! The Graphics rows are bytes (`GROWS`). Their UI values: the effect-level rows (ao, shadows, water, particles, sunrays, gi, fog,
//! clouds, fur, fire, smoke) are levels 0..3 (Off or Classic, Low, Medium, High) and the byte is `swap_level(level)`; Culling and
//! Waves are inverted bytes (0 = On); Draw distance is 0 Low, 1 Medium, 2 High with `pcopts::draw_byte`; the rest are plain.
//! THE PLUS-ONE TRAP (pcext.rs `sync`): a multi-value row whose byte differs from `glast` is read as ONE press of the classic page
//! (`last + 1`), so a byte written here is also written to `glast`, and `apply_graphics` runs once here instead.
use super::super::pcdisp as d;
use super::*;
use std::sync::Mutex;

/// Changes from the screen, not yet applied.
static CMDS: Mutex<Vec<(String, i32)>> = Mutex::new(Vec::new());
/// The values as the worker last published them (`publish`).
static VALUES: Mutex<Vec<(&'static str, i32)>> = Mutex::new(Vec::new());
/// The worker's other list (`publish` builds in it and swaps it with `VALUES`).
static SPARE: Mutex<Vec<(&'static str, i32)>> = Mutex::new(Vec::new());

/// How a Graphics row's UI value maps to its byte.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Enc {
	Plain,
	Level,
	Inverted,
	Draw,
}

/// The Graphics rows in `GROWS` order: the screen's row id and the encoding. A test keeps the ids equal to the rows table.
const GFX: [(&str, Enc); 19] = [
	("lighting", Enc::Plain),
	("ao", Enc::Level),
	("shadows", Enc::Level),
	("displaymode", Enc::Plain),
	("water", Enc::Level),
	("particles", Enc::Level),
	("sunrays", Enc::Level),
	("smoothing", Enc::Plain),
	("vegetation", Enc::Plain),
	("culling", Enc::Inverted),
	("gi", Enc::Level),
	("waves", Enc::Inverted),
	("draw", Enc::Draw),
	("fog", Enc::Level),
	("clouds", Enc::Level),
	("fur", Enc::Level),
	("furstyle", Enc::Plain),
	("fire", Enc::Level),
	("smoke", Enc::Level),
];

fn gfx_byte(enc: Enc, ui: i32) -> u8 {
	let v = ui.clamp(0, 3) as u8;
	match enc {
		Enc::Plain => v,
		Enc::Level => swap_level(v),
		Enc::Inverted => 1 - v.min(1),
		Enc::Draw => pcopts::draw_byte(v),
	}
}

fn gfx_ui(enc: Enc, byte: u8) -> i32 {
	(match enc {
		Enc::Plain => byte,
		Enc::Level => swap_level(byte.min(3)),
		Enc::Inverted => 1 - byte.min(1),
		Enc::Draw => pcopts::draw_level(byte),
	}) as i32
}

/// Any thread: the screen changed row `id` to `v`.
pub fn set(id: &str, v: i32) {
	if let Ok(mut c) = CMDS.lock() {
		c.push((id.to_string(), v));
	}
}

/// [PC] Pack `reset_sizes` (engine gpu_errors.rs: lasting wgpu errors; main thread, on any screen): Upscaler Off, Render scale
/// 100 %, Aspect ratio Auto, in force from the next frame and saved, here under the options lock. Not through the worker's queue:
/// `drain` runs only in a level or once the title's Options page was opened, so a reset queued at boot or on the title never
/// happened (review R15 M1). None of the three has a RAM byte (Upscaler and Aspect ratio live in the engine, Render scale in
/// pcdisp's value), so the worker has nothing to write. false: the options file was not read yet (`init` would put the old sizes
/// back); the engine asks again next frame.
pub fn reset_sizes() -> bool {
	let mut s = ST.lock().unwrap_or_else(|e| e.into_inner());
	if s.path.is_none() && enabled() {
		return false;
	}
	reset_sizes_in(&mut s);
	true
}

/// `reset_sizes` on the options (the caller holds `ST`): the three rows, the file (no path: nothing to save, the extension is
/// off) and the values the modern screen shows (published at once: on the title the worker may not publish for a while).
fn reset_sizes_in(s: &mut St) {
	let rs = d::RS_LIST.iter().position(|&p| p == 100).unwrap_or(0) as i32;
	rac_engine::viewer::upscale_choice::set_kind(&mut s.opts, 0);
	rac_engine::viewer::aspect_box::set_from_ui(&mut s.opts, 0);
	d::set_from_ui(2, rs);
	s.opts.extra_set("renderscale", None);
	s.save();
	if let Ok(mut v) = VALUES.lock() {
		for (id, x) in v.iter_mut() {
			match *id {
				"upscaler" | "aspect" => *x = 0,
				"rscale" => *x = rs,
				_ => {}
			}
		}
	}
}

/// Any thread: the row's value as of the worker's last `publish` (None before the first one or for an unknown id).
pub fn get(id: &str) -> Option<i32> {
	VALUES.lock().ok()?.iter().find(|(k, _)| *k == id).map(|(_, v)| *v)
}

/// Worker, in `sync` after `setup`: applies the queued changes. The Graphics rows go in as one batch (bytes + `glast`, one
/// `apply_graphics`), the others through their module's setter.
pub(super) fn drain(mem: &mut Mem, s: &mut St) {
	let cmds = match CMDS.lock() {
		Ok(mut c) if !c.is_empty() => std::mem::take(&mut *c),
		_ => return,
	};
	let mut g = s.glast.unwrap_or_else(|| GROWS.map(|a| mem.rd8(a)));
	let mut gfx = false;
	for (id, v) in cmds {
		match GFX.iter().position(|(k, _)| *k == id) {
			Some(i) => {
				// Held to the row's last value (R2-4): `sync` holds the byte to it too, and a byte above it in RAM but not in `glast`
				// would read as a press of the classic page next field (the plus-one trap above).
				let b = gfx_byte(GFX[i].1, v).min(super::rows::row_values(i) - 1);
				mem.wr8(GROWS[i], b);
				g[i] = b;
				gfx = true;
			}
			None => other(mem, s, &id, v),
		}
	}
	if gfx {
		s.glast = Some(g);
		apply_graphics(s, g, true);
	}
}

/// The rows that are not Graphics bytes.
fn other(mem: &mut Mem, s: &mut St, id: &str, v: i32) {
	let b = v.clamp(0, 255) as u8;
	match id {
		"camera" => mem.wr8(CAM, b.min(2)),
		"strafe" => {
			// Raised / Shoulder / Steady / Off: Steady is byte Raised + its flag (modcam/steady.rs `row_set`: Off keeps the choice before
			// it), the classic row keeps its three values.
			let (byte, save) = super::super::modcam::steady::row_set(&mut s.opts, b);
			if save {
				s.save();
			}
			mem.wr8(STRAFE, byte)
		}
		"refill" => mem.wr8(BUYALL, b.min(1)),
		"lockon" => mem.wr8(camopts::LOCKON, b.min(2)),
		"lockon_strength" => lockstr::lockstr_set_from_ui(s, b),
		"metal_reflect" => super::super::pcmrefl::set_from_ui(b.min(100)),
		"camspeed" => camopts::set_camspeed(mem, s, b),
		"mousespeed" => mousespeed::set_from_ui(s, b),
		"metal" => {
			if super::super::pcctl::gfx_built(mem) {
				mem.wr8(super::super::pcctl::METAL_B, b.min(1));
			}
			if super::super::pcmetal::set_from_ui(&mut s.opts, b != 0) {
				s.save();
			}
		}
		"supporters" => {
			if super::super::pcctl::gfx_built(mem) {
				mem.wr8(super::super::pcsupp::SUPP_B, b.min(1));
			}
			if super::super::pcsupp::set_from_ui(&mut s.opts, b != 0) {
				s.save();
			}
		}
		"hud" => hud(mem, s, v == 0),
		"hudpos" => {
			if super::super::pcctl::gfx_built(mem) {
				mem.wr8(d::POS_B, b.min(1));
			}
			d::set_from_ui(3, v);
		}
		"hudscale" => d::set_from_ui(0, v),
		"fps" => d::set_from_ui(1, v),
		"rscale" => d::set_from_ui(2, v),
		"menustyle" => {
			crate::modern_menu::style::set_from_ui(&mut s.opts, v != 0);
			super::super::pcctl::set_ui_byte(mem, super::super::pcctl::UiByte::MenuStyle, v != 0);
			s.save();
		}
		_ => match more::set(mem, s, id, v) {
			Some(true) => s.save(),
			Some(false) => {}
			None => eprintln!("retail: modern menu: unknown row {id:?}"),
		},
	}
}

/// The HUD row: Sharp = true. Applied and remembered here even when the classic page was never built (`sync` would see the byte).
fn hud(mem: &mut Mem, s: &mut St, sharp: bool) {
	super::super::pcctl::set_hud(mem, sharp);
	if HUD_SHARP.swap(sharp, Ordering::Relaxed) != sharp {
		eprintln!("retail: PC options: HUD {}", if sharp { "Sharp" } else { "Original" });
		s.opts.hud = Some(sharp);
		s.save();
	}
}

/// Worker, the end of `sync`: the rows' current values for `get`. Runs every field of every level, so it allocates nothing once warm
/// (R2-5): the list is built in the spare Vec and swapped with `VALUES`, the old one becomes the next spare.
pub(super) fn publish(mem: &mut Mem) {
	let mut v = SPARE.lock().map(|mut s| std::mem::take(&mut *s)).unwrap_or_default();
	v.clear();
	v.reserve(48);
	for (i, (id, enc)) in GFX.iter().enumerate() {
		v.push((id, gfx_ui(*enc, mem.rd8(GROWS[i]))));
	}
	let sharp = super::super::pcctl::hud_sharp(mem).unwrap_or_else(|| HUD_SHARP.load(Ordering::Relaxed));
	let pos = |list: &[u32], x: u32| list.iter().position(|&l| l == x).unwrap_or(0) as i32;
	v.extend([
		("camera", mem.rd8(CAM).min(2) as i32),
		("strafe", super::super::modcam::steady::ui_value(mem.rd8(STRAFE).min(2), super::super::modcam::steady::on()) as i32),
		("refill", (mem.rd8(BUYALL) != 0) as i32),
		("lockon", mem.rd8(camopts::LOCKON).min(2) as i32),
		("lockon_strength", lockstr::lockon_strength() as i32),
		("camspeed", camopts::CAMSHOW.load(Ordering::Relaxed) as i32),
		("mousespeed", mousespeed::shown() as i32),
		("metal", super::super::pcmetal::enabled() as i32),
		("metal_reflect", rac_engine::metal_option::reflect_pct() as i32),
		("supporters", super::super::pcsupp::enabled() as i32),
		("hud", !sharp as i32),
		("hudpos", d::hud_corners() as i32),
		("hudscale", d::hud_scale_pct() as i32),
		("fps", pos(&d::FPS_LIST, d::fps_setting())),
		("rscale", pos(&d::RS_LIST.map(|r| r as u32), d::render_scale_pct())),
		("lang", texts::lang_of(mem) as i32),
		("menustyle", crate::modern_menu::style::file_classic() as i32),
	]);
	more::publish(mem, &mut v);
	if let Ok(mut out) = VALUES.lock() {
		std::mem::swap(&mut *out, &mut v);
	}
	if let Ok(mut s) = SPARE.lock() {
		*s = v;
	}
}

/// Test hook: the queue and the published values emptied.
#[cfg(test)]
pub(super) fn reset_for_test() {
	CMDS.lock().unwrap().clear();
	VALUES.lock().unwrap().clear();
}

/// Test door (the caller holds `pcopts::globals_guard`): the values a fresh RAM publishes after the game's start-up from the default
/// options (every module's globals re-initialised from `PcOptions::new()`, then `setup`, like `init` and the first field). The
/// modern table's `def`s must equal these.
#[cfg(test)]
pub(crate) fn defaults_for_test() -> Vec<(&'static str, i32)> {
	use super::super as r;
	let opts = PcOptions::new();
	r::pcrender::init(&opts);
	r::pcintro::init(&opts);
	r::pcskip::init(&opts);
	r::pcdisp::init(&opts);
	r::pcinput::init(&opts);
	r::glyphs::init(&opts);
	r::pcmetal::init(&opts);
	r::pcsupp::init(&opts);
	r::wrench_throw::init(&opts);
	rac_engine::viewer::upscale_choice::init(&opts);
	rac_engine::viewer::under_choice::init(&opts);
	rac_engine::viewer::aspect_box::init(&opts);
	crate::modern_menu::style::init(&opts);
	r::qs_pause::init(&opts);
	r::auto_gadget::init(&opts);
	camopts_init(&opts);
	let mut mem = Mem::new();
	let mut s = St { opts, path: None, last: None, glast: None, lang_seen: None, warned: false };
	setup(&mut mem, &mut s);
	publish(&mut mem);
	VALUES.lock().unwrap().clone()
}

#[path = "ui_bridge_more.rs"]
mod more;

#[cfg(test)]
#[path = "ui_bridge_tests.rs"]
pub(super) mod ui_bridge_tests;
