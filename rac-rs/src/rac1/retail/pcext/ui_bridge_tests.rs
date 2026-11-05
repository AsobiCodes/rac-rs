//! Bridge tests: UI values to bytes, the plus-one trap, and the Preset path against the classic `choose`.
use super::*;

/// Puts the process-wide option state a bridge test changes back to a new player's (`set` + `drain` end in `apply_graphics`: GFX,
/// fog, clouds, fur, fire, smoke, texture filter; also the HUD, display and Underwater look settings). Without it a later test that
/// reads them (`pcext::tests::eudora_graphics_page`: the Preset row reads High or Custom) saw what this one left behind. The caller
/// holds `pcopts::globals_guard`.
pub(in crate::retail::pcext) fn reset_globals() {
	let opts = PcOptions::new();
	let rows = furopt::rows(&opts);
	HUD_SHARP.store(opts.hud_sharp(), Ordering::Relaxed);
	super::super::GFX.store(pack_gfx(swap_rows(rows)), Ordering::Relaxed); // pcext's renderer word (`GFX` here is the bridge's row table)
	(fogopt::set(rows[13]), cloudopt::set(rows[14]), furopt::set(rows[15], rows[16]), fireopt::set(rows[17]), smokeopt::set(rows[18]));
	texfilter::set(opts.texfilter.unwrap_or(true));
	crate::retail::pcdisp::init(&opts);
	rac_engine::viewer::under_choice::init(&opts);
	rac_engine::viewer::aspect_box::init(&opts);
	rac_engine::viewer::upscale_choice::init(&opts);
}

/// The test lock, and `reset_globals` when the test ends (also when it panics).
struct Globals(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

fn globals() -> Globals {
	Globals(crate::retail::pcopts::globals_guard())
}

impl Drop for Globals {
	fn drop(&mut self) {
		reset_globals();
	}
}

fn fresh() -> (Mem, St) {
	let mut mem = Mem::new();
	let s = St { opts: PcOptions::new(), path: None, last: None, glast: None, lang_seen: None, warned: false };
	let g = swap_rows(PcOptions::new().graphics_rows());
	for (a, v) in GROWS.iter().zip(g) {
		mem.wr8(*a, v);
	}
	(mem, St { glast: Some(g), ..s })
}

#[test]
fn encodings_round_trip() {
	for (i, (_, enc)) in GFX.iter().enumerate() {
		let n = (rows::row_values(i) as i32).max(2);
		for ui in 0..n {
			assert_eq!(gfx_ui(*enc, gfx_byte(*enc, ui)), ui, "row {i} value {ui}");
		}
	}
}

/// A four-value row set from the screen: the byte AND `glast` hold it, so the classic sync does not read a press (+1).
#[test]
fn bridge_set_updates_byte_and_glast_no_plus_one() {
	let _g = globals();
	let (mut mem, mut s) = fresh();
	set("ao", 3);
	drain(&mut mem, &mut s);
	assert_eq!(mem.rd8(GROWS[1]), swap_level(3), "High is byte 1");
	assert_eq!(s.glast.map(|g| g[1]), Some(1), "glast follows the byte");
	assert_eq!(publish_ui("ao", &mut mem), Some(3));
	set("ao", 0);
	drain(&mut mem, &mut s);
	assert_eq!((mem.rd8(GROWS[1]), s.glast.map(|g| g[1])), (0, Some(0)));
}

/// R2-4: a value above a row's last (Vegetation has three) is held to the last one in RAM and `glast`, so the next field reads no
/// press of the classic page (before: byte 3, `glast` 2 -> one press -> Off).
#[test]
fn a_value_above_the_rows_last_is_held_to_it() {
	let _g = globals();
	let (mut mem, mut s) = fresh();
	set("vegetation", 3);
	drain(&mut mem, &mut s);
	assert_eq!((mem.rd8(GROWS[8]), s.glast.map(|g| g[8])), (2, Some(2)), "Lush, the row's last value");
	let mut g = GROWS.map(|a| mem.rd8(a));
	rows::step_multi(&mut g, s.glast.as_ref(), |_, _| {});
	assert_eq!(g[8], 2, "no plus-one to Off on the next field");
}

fn publish_ui(id: &str, mem: &mut Mem) -> Option<i32> {
	publish(mem);
	get(id)
}

/// The UI's preset (one `set` per row, then the `preset_also` settings) leaves the same Graphics bytes and the same texture filter
/// as the classic Preset row's `apply_preset`, also from a player who had the filter on Classic.
#[test]
fn bridge_preset_writes_same_bytes_as_classic_choose() {
	let _g = globals();
	let also = crate::modern_menu::menu().preset_also;
	assert!(also.iter().any(|(id, _)| *id == "texfilter"), "the modern presets send the texture filter");
	for (p, def) in crate::modern_menu::PRESETS.iter().enumerate() {
		let (mut a, mut sa) = fresh();
		sa.opts.texfilter = Some(false);
		texfilter::set(false);
		for (id, v) in def.sets.iter().chain(also) {
			set(id, *v);
		}
		drain(&mut a, &mut sa);
		let filter_a = texfilter::get();
		let (mut b, mut sb) = fresh();
		sb.opts.texfilter = Some(false);
		texfilter::set(false);
		presets::apply_preset(&mut b, &mut sb, p as u8);
		let (ga, gb) = (GROWS.map(|x| a.rd8(x)), GROWS.map(|x| b.rd8(x)));
		assert_eq!(ga, gb, "preset {p}");
		assert_eq!(sa.glast, Some(ga));
		assert_eq!(sa.opts.texfilter, sb.opts.texfilter, "preset {p}: texture filter in the options");
		assert_eq!(filter_a, texfilter::get(), "preset {p}: texture filter applied");
		assert_eq!(sa.opts.texfilter, Some(true), "preset {p}: every preset has texfilter = smooth");
	}
}

/// The steppers and the position reach the options file although the classic Graphics page was never built on this RAM.
#[test]
fn stepper_rows_reach_the_options_without_the_classic_page() {
	let _g = globals();
	let (mut mem, mut s) = fresh();
	assert!(!crate::retail::pcctl::gfx_built(&mut mem));
	set("hudscale", 120);
	set("fps", 2);
	set("rscale", 3);
	set("hudpos", 1);
	drain(&mut mem, &mut s);
	assert!(crate::retail::pcdisp::sync(&mut mem, &mut s.opts), "a modern change makes the caller save");
	assert_eq!(s.opts.extra_get("hudpos"), Some("corners"));
	assert_eq!(s.opts.extra_get("fps"), Some("60"));
	assert_eq!(s.opts.extra_get("renderscale"), Some("125"));
	assert!(s.opts.extra_get("hudscale").is_some());
	assert!(!crate::retail::pcdisp::sync(&mut mem, &mut s.opts), "nothing changed since: no second save");
	crate::retail::pcdisp::init(&PcOptions::new());
}

/// A7: the Left / Right and Up / Down rows write the retail Camera Options byte (0 = Reversed, 1 = Normal: what the retail toggle
/// writes) and only from a whole retail record in the row copy: a wrong or missing copy writes nothing anywhere.
#[test]
fn the_camera_invert_rows_write_only_the_retail_byte_from_a_whole_record() {
	let _g = globals();
	let (mut mem, mut s) = fresh();
	let published = |mem: &mut Mem| {
		let mut v = Vec::new();
		more::publish(mem, &mut v);
		let find = |id: &str| v.iter().find(|(k, _)| *k == id).map(|(_, x)| *x);
		(find("leftright"), find("updown"))
	};
	mem.wr8(0x15ede0, 1); // the game's boot default: Normal
	mem.wr8(0x15eddc, 1);
	// No row copy yet: nothing is written.
	assert_eq!(more::set(&mut mem, &mut s, "leftright", 1), Some(false));
	assert_eq!((mem.rd8(0x15ede0), mem.rd8(0x15eddc)), (1, 1));
	assert_eq!(published(&mut mem), (Some(0), Some(0)), "no copy: both read Normal");
	// The retail records in the copy.
	for (k, rec) in more::INVERT.iter().enumerate() {
		for (i, w) in rec.iter().enumerate() {
			mem.wr32(TAB + 24 * k as u32 + 4 * i as u32, *w);
		}
	}
	assert_eq!(more::set(&mut mem, &mut s, "leftright", 1), Some(false), "no options file change");
	assert_eq!((mem.rd8(0x15ede0), mem.rd8(0x15eddc)), (0, 1), "Left / Right Reversed = byte 0, Up / Down untouched");
	assert_eq!(published(&mut mem), (Some(1), Some(0)));
	more::set(&mut mem, &mut s, "updown", 1);
	assert_eq!((mem.rd8(0x15ede0), mem.rd8(0x15eddc)), (0, 0));
	more::set(&mut mem, &mut s, "leftright", 0);
	more::set(&mut mem, &mut s, "updown", 0);
	assert_eq!((mem.rd8(0x15ede0), mem.rd8(0x15eddc)), (1, 1), "Normal = byte 1");
	// A record that is not the retail one (here: the byte pointer points elsewhere) writes nothing, not even at that address.
	mem.wr32(TAB + 4, 0x12_3456);
	more::set(&mut mem, &mut s, "leftright", 1);
	assert_eq!((mem.rd8(0x12_3456), mem.rd8(0x15ede0)), (0, 1), "no write through a wrong pointer");
	// Nor when a choice text id differs.
	mem.wr32(TAB + 4, 0x15ede0);
	mem.wr32(TAB + 24 + 8, 0x1234);
	more::set(&mut mem, &mut s, "updown", 1);
	assert_eq!(mem.rd8(0x15eddc), 1, "a record with other choice texts is left alone");
}

/// After a field's `drain`, the values the screen reads back equal what the renderer's `Graphics` verdict holds.
#[test]
fn values_published_after_sync_match_graphics() {
	let _g = globals();
	let (mut mem, mut s) = fresh();
	for (id, v) in [("lighting", 1), ("ao", 2), ("shadows", 1), ("water", 3), ("particles", 2), ("sunrays", 3), ("gi", 1), ("vegetation", 2), ("draw", 0)] {
		set(id, v);
	}
	set("culling", 0);
	set("waves", 0);
	drain(&mut mem, &mut s);
	publish(&mut mem);
	let g = menu_graphics();
	let want = [("lighting", g.lighting as i32), ("ao", g.ao_q as i32), ("shadows", g.shadows_q as i32), ("water", g.water_q as i32), ("particles", g.particles_q as i32), ("sunrays", g.sun_q as i32), ("gi", g.gi_q as i32), ("vegetation", g.veg as i32), ("draw", g.draw as i32), ("culling", g.culling as i32), ("waves", g.waves as i32)];
	for (id, v) in want {
		assert_eq!(get(id), Some(v), "published {id} equals the renderer's value");
	}
	assert_eq!((g.ao_q, g.water_q, g.draw, g.culling, g.waves), (2, 3, 0, false, false));
}

/// GitHub #64: the Underwater look row lives in the options file and the engine only (`under_choice`): no Graphics byte, no `glast`
/// change; it is published back and survives a save and a load.
#[test]
fn the_underwater_row_is_options_file_only() {
	use rac_engine::viewer::under_choice;
	let _g = globals();
	let (mut mem, mut s) = fresh();
	let (bytes, glast) = (GROWS.map(|a| mem.rd8(a)), s.glast);
	assert_eq!(publish_ui("underwater", &mut mem), Some(1), "On by default");
	set("underwater", 0);
	drain(&mut mem, &mut s);
	assert!(!under_choice::on());
	assert_eq!((GROWS.map(|a| mem.rd8(a)), s.glast), (bytes, glast), "no Graphics byte changes");
	assert_eq!(s.opts.extra_get("underwater"), Some("off"));
	assert_eq!(publish_ui("underwater", &mut mem), Some(0));
	under_choice::init(&PcOptions::parse(&s.opts.to_text()));
	assert!(!under_choice::on(), "the file keeps Off");
	set("underwater", 1);
	drain(&mut mem, &mut s);
	assert!(under_choice::on());
	assert_eq!(s.opts.extra_get("underwater"), Some("on"));
}

/// GitHub #45: the Aspect ratio row lives in the options file and the engine only (`aspect_box`): no RAM write at all (the whole
/// EE RAM is the same), no `glast` change; it is published back and survives a save and a load; a file without the key is Auto.
#[test]
fn the_aspect_row_is_options_file_only() {
	use rac_engine::viewer::aspect_box::{self, Ratio};
	let _g = globals();
	let (mut mem, mut s) = fresh();
	let (ram, glast) = (mem.ram.clone(), s.glast);
	assert_eq!(publish_ui("aspect", &mut mem), Some(0), "Auto by default");
	set("aspect", Ratio::R21x9 as i32);
	drain(&mut mem, &mut s);
	assert_eq!(aspect_box::current(), Ratio::R21x9);
	assert!(mem.ram == ram && s.glast == glast, "no RAM byte changes");
	assert_eq!(s.opts.extra_get("aspect"), Some("21:9"));
	assert_eq!(publish_ui("aspect", &mut mem), Some(Ratio::R21x9 as i32));
	aspect_box::init(&PcOptions::parse(&s.opts.to_text()));
	assert_eq!(aspect_box::current(), Ratio::R21x9, "the file keeps 21:9");
	aspect_box::init(&PcOptions::parse("hud=on\n"));
	assert_eq!(aspect_box::current(), Ratio::Auto, "an older file without the key is Auto");
	aspect_box::init(&PcOptions::parse(&s.opts.to_text()));
	set("aspect", 0);
	drain(&mut mem, &mut s);
	assert_eq!(s.opts.extra_get("aspect"), Some("auto"));
}

/// Pack `reset_sizes` (engine gpu_errors.rs escalation) in a level: FSR, Render scale 200 % and 21:9 go back to Off, 100 % and
/// Auto at once (no field needed), the options in memory say so, and a later `drain` keeps them.
#[test]
fn the_size_reset_puts_the_size_rows_back_to_their_defaults() {
	use rac_engine::viewer::aspect_box::{self, Ratio};
	use rac_engine::viewer::upscale_choice;
	let _g = globals();
	let (mut mem, mut s) = fresh();
	set("upscaler", 1);
	set("rscale", crate::retail::pcdisp::RS_LIST.iter().position(|&p| p == 200).unwrap() as i32);
	set("aspect", Ratio::R21x9 as i32);
	drain(&mut mem, &mut s);
	assert_eq!((upscale_choice::kind(), crate::retail::pcdisp::render_scale_pct(), aspect_box::current()), (1, 200, Ratio::R21x9));
	reset_sizes_in(&mut s);
	assert_eq!((upscale_choice::kind(), crate::retail::pcdisp::render_scale_pct(), aspect_box::current()), (0, 100, Ratio::Auto));
	assert_eq!((s.opts.extra_get("upscaler"), s.opts.extra_get("aspect"), s.opts.extra_get("renderscale")), (Some("off"), Some("auto"), None));
	drain(&mut mem, &mut s);
	assert_eq!((upscale_choice::kind(), crate::retail::pcdisp::render_scale_pct(), aspect_box::current()), (0, 100, Ratio::Auto));
}

/// `ST` swapped for a test's and put back when the test ends (also when it panics).
struct StSwap(Option<St>);

impl StSwap {
	fn new(st: St) -> StSwap {
		StSwap(Some(std::mem::replace(&mut *ST.lock().unwrap_or_else(|e| e.into_inner()), st)))
	}
}

impl Drop for StSwap {
	fn drop(&mut self) {
		if let Some(before) = self.0.take() {
			*ST.lock().unwrap_or_else(|e| e.into_inner()) = before;
		}
	}
}

/// Review R15 M1: the size reset at boot or on the title, where no field drains the screen's queue (no `pre_field`; `front_field`
/// only after the title's Options page): applied at once under the options lock and pc_options.txt rewritten, other keys kept, the
/// modern screen's values updated, and the next start reads the defaults. Before `init` read the file it reports false and
/// changes nothing (`init` would put the old sizes back); the engine asks again.
#[test]
fn the_size_reset_works_at_boot_and_on_the_title() {
	use crate::retail::pcdisp;
	use rac_engine::viewer::aspect_box::{self, Ratio};
	use rac_engine::viewer::upscale_choice;
	let _g = globals();
	let dir = std::env::temp_dir().join(format!("pcext_size_reset_{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	let file = dir.join("pc_options.txt");
	std::fs::write(&file, "camera=classic
upscaler=fsr
renderscale=200
aspect=21:9
").unwrap();
	let opts = PcOptions::load(&file);
	(upscale_choice::init(&opts), pcdisp::init(&opts), aspect_box::init(&opts));
	let big = || (upscale_choice::kind(), pcdisp::render_scale_pct(), aspect_box::current());
	assert_eq!(big(), (1, 200, Ratio::R21x9));
	let _st = StSwap::new(St { opts, path: None, last: None, glast: None, lang_seen: None, warned: false });
	reset_for_test();
	publish(&mut Mem::new());
	if enabled() {
		assert!(!reset_sizes(), "the file was not read yet");
		assert_eq!(big(), (1, 200, Ratio::R21x9), "nothing changed");
	}
	ST.lock().unwrap_or_else(|e| e.into_inner()).path = Some(file.clone());
	assert!(reset_sizes());
	assert_eq!(big(), (0, 100, Ratio::Auto), "in force for the next frame");
	let t = std::fs::read_to_string(&file).unwrap();
	assert!(t.contains("upscaler=off") && t.contains("aspect=auto") && !t.contains("renderscale") && t.contains("camera=classic"), "{t}");
	let rs = pcdisp::RS_LIST.iter().position(|&p| p == 100).unwrap() as i32;
	assert_eq!((get("upscaler"), get("rscale"), get("aspect")), (Some(0), Some(rs), Some(0)), "the modern screen shows them");
	reset_for_test();
	let again = PcOptions::load(&file);
	(upscale_choice::init(&again), pcdisp::init(&again), aspect_box::init(&again));
	assert_eq!(big(), (0, 100, Ratio::Auto), "the next start reads the defaults");
	let _ = std::fs::remove_dir_all(&dir);
}
