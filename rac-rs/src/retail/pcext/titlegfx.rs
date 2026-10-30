//! The title screen's Graphics page: the same sections, rows and values as the pause menu's Graphics page (`pcctl.rs`).
//!
//! Ported from DeSynkro's rac-rs-linux (commit 8637c4c10e, "title Graphics page = pause page"), adapted to this tree's rows.
//!
//! The title's Options list row opens a copy of the retail Video Options page (`pcext::build_title`); this module writes its row
//! table: a "Page" stepper followed by the current section's rows. The front end's value item only flips the row's byte on a
//! press, so one press steps on from the value last applied (`pcext::sync` does that for the rows with more than two values), and
//! the stepper's own byte advances the section here. The mirror draws the page from `rows()` and `put_texts` (viewer/front_mirror.rs).
//!
//! The rows themselves come from `pcctl::gfx_section` / `pcctl::gfx_row`, the same definitions the pause page uses, so the two
//! pages cannot drift apart. The Display section's steppers (HUD scale, FPS limit, Render scale) step on a press here too
//! (`pcdisp::field`).

use super::*;
use crate::retail::pcctl;

/// The stepper row's byte: the front-end item flips it on a press, `sync` advances the section and puts it back.
pub(super) const STEP: u32 = BASE + 24;

/// The section the title page shows (a session choice, like the pause page's) and whether this page is the one on screen.
#[cfg(not(test))]
mod state {
	use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
	pub(super) static SEC: AtomicUsize = AtomicUsize::new(0);
	pub(super) static ACTIVE: AtomicBool = AtomicBool::new(false);
	pub(super) fn get() -> (usize, bool) {
		(SEC.load(Ordering::Relaxed), ACTIVE.load(Ordering::Relaxed))
	}
	pub(super) fn set(f: impl FnOnce(&mut (usize, bool))) {
		let mut v = get();
		f(&mut v);
		SEC.store(v.0, Ordering::Relaxed);
		ACTIVE.store(v.1, Ordering::Relaxed);
	}
}

/// Tests run in parallel: each thread keeps its own state, so one test's section never moves another's page.
#[cfg(test)]
mod state {
	thread_local! { static T: std::cell::Cell<(usize, bool)> = const { std::cell::Cell::new((0, false)) }; }
	pub(super) fn get() -> (usize, bool) {
		T.with(|t| t.get())
	}
	pub(super) fn set(f: impl FnOnce(&mut (usize, bool))) {
		T.with(|t| {
			let mut v = t.get();
			f(&mut v);
			t.set(v);
		});
	}
}
use state::{get, set};

/// The section the title page shows.
pub(super) fn section() -> usize {
	get().0 % pcctl::gfx_sections()
}

/// Whether the title's Graphics page is the one on screen (the pause page clears this when it is built).
pub(super) fn active() -> bool {
	get().1
}

/// The pause menu's Graphics page was built: this page is no longer the one being pressed.
pub(crate) fn deactivate() {
	set(|s| s.1 = false);
}

/// Test hook: the section the title page shows.
#[cfg(test)]
pub(super) fn test_section(n: usize) {
	set(|s| s.0 = n);
}

/// The title page's copy for `sec`: the stepper, the section's rows and the zero record. A row is `{label, byte, value 0, value 1,
/// flags}` (the front-end value item reads five words; its two values are the first two, the rest step on through `sync`).
fn table(sec: usize) -> Vec<u32> {
	let mut t = vec![pcctl::ID_PAGE, STEP, pcctl::ID_SELG, pcctl::ID_SELG, 0];
	for &i in pcctl::gfx_section(sec) {
		let r = pcctl::gfx_row(i, false); // the value item reads two values: the Custom choice (4th) is not in this table
		t.extend([r[0], r[1], r[2], r[3], 0]);
	}
	t.push(0);
	t
}

/// `TGTAB`'s capacity in 5-word records (the old 13-row table's place, `TGTAB..POOL2`; `gfx_word_tests`).
pub(super) const TAB_RECS: usize = 14;

/// Writes the current section's table at `TGTAB` (padded, so no record of a longer section stays behind the zero record).
fn write(mem: &mut Mem) {
	let mut t = table(section());
	t.resize(TAB_RECS * 5, 0);
	for (i, w) in t.iter().enumerate() {
		mem.wr32(TGTAB + 4 * i as u32, *w);
	}
}

/// `build_title`: the page opens on its remembered section, with the stepper at rest.
pub(super) fn build(mem: &mut Mem) {
	set(|s| s.1 = true);
	mem.wr8(STEP, 0);
	// The Graphics rows' shared block (the page magic and the HUD, Renderer, Skip intro ... bytes): on fresh RAM it takes the options
	// file's values, so those rows work on the title too, not only after the pause page has been built once. It also writes the
	// pause page's row table, which the title does not show.
	pcctl::build_gfx(mem, GTAB, &ROWS, HUD_SHARP.load(Ordering::Relaxed), crate::retail::pcrender::wanted_dx12());
	// The Preset row starts at the preset the rows match, so its first press steps on from there.
	presets::title_open(mem);
	write(mem);
}

/// Worker, every field (`pcext::sync`): a press on the stepper advances the section and rewrites the table; the Display steppers
/// step. In a level (the pause page built) this does nothing.
pub(super) fn sync(mem: &mut Mem) {
	if !active() {
		return;
	}
	if mem.rd8(STEP) != 0 {
		set(|s| s.0 = (section() + 1) % pcctl::gfx_sections());
		mem.wr8(STEP, 0);
		write(mem);
	}
	if section() == pcctl::GFX_DISPLAY {
		crate::retail::pcdisp::field(mem, u32::MAX, [false; 2]);
	}
}

/// The value index (0..3) the row `i` shows now: its byte, from the host's applied state.
fn row_byte(i: usize, v: &[u8; 15]) -> u8 {
	use crate::retail::{pcctl as c, pcdisp, pcinput, pcintro, pcskip};
	match i {
		c::HUD_ROW => !HUD_SHARP.load(Ordering::Relaxed) as u8,
		c::RENDER_ROW => crate::retail::pcrender::wanted_dx12() as u8,
		// The Preset row shows the preset matching the rows (Custom when they match none).
		c::PRESET_ROW => presets::detect_now(),
		c::INTRO_ROW => pcintro::skip() as u8,
		c::SKIPCUT_ROW => !pcskip::skip() as u8,
		c::HUDPOS_ROW => pcdisp::hud_corners() as u8,
		c::WHEEL_ROW => pcinput::wheel_mouse() as u8,
		c::STICK_ROW => pcinput::stick_byte(),
		c::GLYPH_ROW => crate::retail::glyphs::byte(),
		_ if i < v.len() => v[i],
		_ => 0, // the steppers have one value text
	}
}

/// The page as the mirror draws it (`viewer/frontend.rs` `Page::Graphics`): the stepper, then the current section's rows, each
/// with its label and the text id of the value it shows now.
pub(super) fn rows() -> Vec<(u32, u32)> {
	// The title's Mods page is drawn by the same host page (`front_mirror::page_of`).
	if let Some(r) = crate::retail::pcmods::title_rows() {
		return r;
	}
	let mut v = unpack_gfx(GFX.load(Ordering::Relaxed));
	v[3] = FULLSCREEN.load(Ordering::Relaxed) as u8;
	// Fog and clouds are not in the packed word (unpack_gfx leaves their rows 0): their applied levels come from their modules.
	v[13] = swap_level(fogopt::level());
	v[14] = swap_level(cloudopt::level());
	let custom = presets::detect_now() == 3;
	let mut out = vec![(pcctl::ID_PAGE, pcctl::ID_SELG)];
	for &i in pcctl::gfx_section(section()) {
		let r = pcctl::gfx_row(i, custom);
		let vid = r[2 + row_byte(i, &v).min(3) as usize];
		out.push((r[0], if vid != 0 { vid } else { r[2] }));
	}
	out
}

/// Every text the title page's mirror needs that the pause page gets from the game's text hook, in `lang`: the stepper
/// ("< Picture >"), this file's rows and values (`pcctl`), the Display / input rows and their live values (`pcdisp`, `pcinput`).
/// The mirror inserts them into its text map each frame while the page is drawn (the steppers' texts follow the settings).
pub(crate) fn texts(lang: usize) -> Vec<(u32, String)> {
	let mut out = pcctl::title_texts(lang, section());
	for id in crate::retail::pcdisp::ID0..crate::retail::pcdisp::ID_END {
		let t = crate::retail::pcdisp::text(id, lang);
		if !t.is_empty() {
			out.push((id, t.iter().map(|&b| b as char).collect()));
		}
	}
	out
}

/// `texts` into the front end's text map (one call from the mirror's frame). The list is rebuilt only when the language, the
/// section or a live value (`pcdisp::text_key`) changed, and a text already in the map is not inserted again, so a frame
/// where nothing changed allocates nothing.
pub(crate) fn put_texts(text: &mut crate::frontend::Text, lang: usize) {
	static CACHE: std::sync::Mutex<Option<((usize, usize, u64), Vec<(u32, String)>)>> = std::sync::Mutex::new(None);
	let key = (lang, section(), crate::retail::pcdisp::text_key());
	let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
	if c.as_ref().map(|c| c.0) != Some(key) {
		*c = Some((key, texts(lang)));
	}
	for (id, s) in c.iter().flat_map(|c| c.1.iter()) {
		if text.get(*id) != s.as_str() {
			text.insert(*id, s.clone());
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The title's table is the pause page's rows, section by section: the stepper in front, then the section's rows with the
	/// same labels, bytes and value texts, then the zero record; it fits `TGTAB`'s place.
	#[test]
	fn the_title_page_has_the_pause_pages_rows() {
		for sec in 0..pcctl::gfx_sections() {
			let title = table(sec);
			assert_eq!(&title[..5], &[pcctl::ID_PAGE, STEP, pcctl::ID_SELG, pcctl::ID_SELG, 0], "section {sec}: the stepper comes first");
			// Independent of `table()`'s loop: record k is the section's k-th row, word by word.
			let rows = pcctl::gfx_section(sec);
			assert_eq!(title.len(), (1 + rows.len()) * 5 + 1, "section {sec}: stepper, one record per row, zero record");
			for (k, &i) in rows.iter().enumerate() {
				let r = pcctl::gfx_row(i, false);
				let rec = &title[5 + 5 * k..10 + 5 * k];
				assert_eq!((rec[0], rec[1], rec[2], rec[3], rec[4]), (r[0], r[1], r[2], r[3], 0), "section {sec}: row {k}");
			}
			assert_eq!(title.last(), Some(&0), "section {sec}: the zero record ends the table");
			assert!(title.len() <= TAB_RECS * 5);
		}
	}

	/// A pause Graphics page built once leaves its magic in the reserved RAM, so a level visit used to make the title page's
	/// `presets::show` overwrite the Preset byte and reset every preset row to Low on the next field. The in-game-only paths must
	/// stay out of the front end: a row change sticks, and only a change of the Preset byte steps a preset.
	#[test]
	fn a_level_visit_does_not_make_the_title_reset_its_rows() {
		let _turn = super::super::tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
		let mut mem = Mem::new();
		let mut s = St { opts: PcOptions::new(), path: None, last: None, glast: None, lang_seen: None, warned: false };
		// A level visit: the pause Graphics page is built (its magic and bytes stay in the reserved RAM).
		pcctl::build_gfx(&mut mem, GTAB, &ROWS, true, false);
		assert!(pcctl::hud_sharp(&mut mem).is_some(), "the pause page is built");
		// The title Graphics page opens (it takes the shared block over) and the front end runs a field.
		build(&mut mem);
		super::super::sync(&mut mem, u32::MAX, false, &mut s);
		let high = GROWS.map(|a| mem.rd8(a));
		// The player changes a row a preset covers: it must keep the stepped value, not be reset to Low by a preset.
		mem.wr8(AO, 0);
		super::super::sync(&mut mem, u32::MAX, false, &mut s);
		assert_ne!(mem.rd8(AO), 0, "the row change was applied");
		assert_eq!(mem.rd8(WATER), high[4], "no preset reset the other rows");
		assert_eq!(mem.rd8(LIGHT), high[0], "no preset reset the lighting");
		let once = GROWS.map(|a| mem.rd8(a));
		super::super::sync(&mut mem, u32::MAX, false, &mut s);
		assert_eq!(GROWS.map(|a| mem.rd8(a)), once, "a second field leaves the rows as they are");
		// A real Preset press: the byte changes, one step is applied and it settles.
		let page = mem.rd8(pcctl::PRESETB).min(3);
		mem.wr8(pcctl::PRESETB, (page + 1) % 4);
		super::super::sync(&mut mem, u32::MAX, false, &mut s);
		assert_eq!(presets::detect_now(), 0, "one press from Custom applies Low");
		let after = GROWS.map(|a| mem.rd8(a));
		super::super::sync(&mut mem, u32::MAX, false, &mut s);
		assert_eq!(GROWS.map(|a| mem.rd8(a)), after, "the preset settles after one field");
	}

	/// Level -> quit to the title -> press Preset -> change a row -> the next level starts: the pause page's Preset baseline is the
	/// title's value, so the level's first field does not read the title's press as a new one and reset the row (review M1).
	#[test]
	fn a_preset_pressed_on_the_title_is_not_applied_again_in_the_next_level() {
		let _turn = super::super::tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
		// `stale`: the preset the pause page's Preset row last showed in the level. Returns the Preset byte the title ended on and
		// whether the next level's first fields changed any row.
		let run = |stale: u8| -> (u8, bool) {
			let mut mem = Mem::new();
			let mut s = St { opts: PcOptions::new(), path: None, last: None, glast: None, lang_seen: None, warned: false };
			pcctl::build_gfx(&mut mem, GTAB, &ROWS, true, false);
			pcctl::preset_show(&mut mem, &ROWS, GTAB, stale);
			// Back at the title: the page opens and settles, the player presses Preset, then turns Ambient occlusion off.
			build(&mut mem);
			super::super::sync(&mut mem, u32::MAX, false, &mut s);
			let page = mem.rd8(pcctl::PRESETB).min(3);
			mem.wr8(pcctl::PRESETB, (page + 1) % 4);
			super::super::sync(&mut mem, u32::MAX, false, &mut s);
			mem.wr8(AO, 0);
			super::super::sync(&mut mem, u32::MAX, false, &mut s);
			let (rows, ended) = (GROWS.map(|a| mem.rd8(a)), mem.rd8(pcctl::PRESETB));
			// The next level starts: its first fields run `presets::choose`; the title's row change must stay.
			let mut changed = false;
			for _ in 0..3 {
				presets::choose(&mut mem, &mut s);
				changed |= GROWS.map(|a| mem.rd8(a)) != rows;
			}
			assert_eq!(pcctl::preset_chosen(&mut mem), None, "the pause page sees no press (stale {stale})");
			(ended, changed)
		};
		// The stale value must differ from the one the title ends on, or the old code would not show the bug either.
		let stale = if run(0).0 == 0 { 1 } else { 0 };
		let (ended, changed) = run(stale);
		assert_ne!(ended, stale, "the test's stale preset differs from the title's");
		assert!(!changed, "no preset is applied again at the next level start");
	}

	/// The title page's build sets the Graphics rows' shared block up on fresh RAM, so the HUD, Renderer and Skip intro rows work
	/// on the title before any level has been visited.
	#[test]
	fn the_title_page_sets_the_shared_rows_up() {
		let _turn = super::super::tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
		let mut mem = Mem::new();
		assert!(pcctl::hud_sharp(&mut mem).is_none(), "fresh RAM has no page");
		build(&mut mem);
		assert_eq!(pcctl::hud_sharp(&mut mem), Some(HUD_SHARP.load(Ordering::Relaxed)), "the HUD row works");
		assert_eq!(pcctl::intro_skip(&mut mem), Some(crate::retail::pcintro::skip()), "the Skip intro row works");
	}

	/// One press on the stepper advances the section (and the table follows), wrapping at the last one; every section's rows
	/// show a value text and the texts are worded.
	#[test]
	fn the_stepper_advances_the_section_and_every_row_shows_a_value() {
		let _turn = super::super::tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
		test_section(0);
		let mut mem = Mem::new();
		build(&mut mem);
		assert_eq!(section(), 0);
		assert_eq!(mem.rd8(STEP), 0, "the stepper starts at rest");
		mem.wr8(STEP, 1);
		sync(&mut mem);
		assert_eq!(section(), 1);
		assert_eq!(mem.rd8(STEP), 0, "the stepper is back at rest");
		assert_eq!(mem.rd32(TGTAB + 20 * 1), table(1)[5], "the table is the new section's");
		let words = texts(0);
		for sec in 0..pcctl::gfx_sections() {
			test_section(sec);
			let words = texts(0);
			let get = |id: u32| words.iter().find(|w| w.0 == id).map(|w| w.1.clone());
			assert!(get(pcctl::ID_SELG).is_some_and(|s| s.starts_with("< ") && s.ends_with(" >")), "section {sec}: stepper text");
			for (label, value) in rows().into_iter().skip(1) {
				let known = |id: u32| get(id).is_some() || NEW_IDS.contains(&id) || id == 0x4f5a || id == 0x4f5b;
				assert!(known(label) && known(value), "section {sec}: label {label:#x} / value {value:#x} has a text");
			}
		}
		assert!(words.iter().any(|w| w.0 == pcctl::ID_PAGE && w.1 == "Page"));
		test_section(1);
		for _ in 1..pcctl::gfx_sections() {
			mem.wr8(STEP, 1);
			sync(&mut mem);
		}
		assert_eq!(section(), 0, "wraps back to the first section");
		test_section(0);
		deactivate();
	}
}
