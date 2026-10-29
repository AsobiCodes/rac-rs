//! The title screen's Graphics page: the same sections, rows and values as the pause menu's Graphics page (`pcctl.rs`).
//!
//! The title's Options list row opens a copy of the retail Video Options page (`pcext::build_title`); this module writes its row
//! table: a "Page" stepper followed by the current section's rows. The front end's value item only flips the row's byte on a
//! press, so one press steps on from the value last applied (`pcext::sync` does that for the rows with more than two values), and
//! the stepper's own byte advances the section here. The mirror draws the page from `rows()` (viewer/front_mirror.rs).
//!
//! The rows themselves come from `pcctl::gfx_section` / `pcctl::gfx_row`, the same definitions the pause page uses, so the two
//! pages cannot drift apart. The Display rows (`pcdisp.rs`) are the pause page's steppers too: their byte flip is read by
//! `pcdisp::field`, which `pcext::sync` runs on the front end while this page is open.

use super::*;
use crate::retail::pcctl;
use crate::retail::pcdisp;

/// The stepper row's byte: the front-end item flips it on a press, `sync` advances the section and puts it back.
pub(super) const STEP: u32 = BASE + 24;
/// The stepper row's value text ("< Picture >"): its string changes with the section, so the mirror refreshes it.
/// 0x7f24 is the culling notice (pcext/cullwarn.rs), so the stepper takes the next free id.
pub(super) const ID_SECTION: u32 = 0x7f25;

/// The section the title page shows (a session choice, like the pause page's), whether this page is the one on screen and
/// whether its Preset row showed Custom at the last table write.
#[cfg(not(test))]
mod state {
	use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
	pub(super) static SEC: AtomicUsize = AtomicUsize::new(0);
	pub(super) static ACTIVE: AtomicBool = AtomicBool::new(false);
	pub(super) static CUSTOM: AtomicU8 = AtomicU8::new(0);
	pub(super) fn get() -> (usize, bool, u8) {
		(SEC.load(Ordering::Relaxed), ACTIVE.load(Ordering::Relaxed), CUSTOM.load(Ordering::Relaxed))
	}
	pub(super) fn set(f: impl FnOnce(&mut (usize, bool, u8))) {
		let mut v = get();
		f(&mut v);
		SEC.store(v.0, Ordering::Relaxed);
		ACTIVE.store(v.1, Ordering::Relaxed);
		CUSTOM.store(v.2, Ordering::Relaxed);
	}
}

/// Tests run in parallel: each thread keeps its own state, so one test's section never moves another's page.
#[cfg(test)]
mod state {
	thread_local! { static T: std::cell::Cell<(usize, bool, u8)> = const { std::cell::Cell::new((0, false, 0)) }; }
	pub(super) fn get() -> (usize, bool, u8) {
		T.with(|t| t.get())
	}
	pub(super) fn set(f: impl FnOnce(&mut (usize, bool, u8))) {
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
fn table(sec: usize, custom: bool) -> Vec<u32> {
	let mut t = vec![pcctl::ID_PAGE, STEP, ID_SECTION, ID_SECTION, 0];
	for &i in pcctl::gfx_section(sec) {
		let r = pcctl::gfx_row(i, custom);
		t.extend([r[0], r[1], r[2], r[3], 0]);
	}
	t.push(0);
	t
}

/// The pause page's row table (`pcext::GTAB`): `build_gfx` writes it; the title does not show it.
const PAUSE_TABLE: u32 = super::GTAB;

/// `TGTAB`'s capacity in 5-word records: the old 13-row table's place, `TGTAB..POOL2` (`gfx_word_tests`).
pub(super) const TAB_RECS: usize = 14;

/// Writes the current section's table at `TGTAB` (padded, so no record of a longer section stays behind the zero record).
fn write(mem: &mut Mem) {
	let custom = mem.rd8(pcctl::PRESETB).min(3) == 3;
	set(|s| s.2 = custom as u8);
	let mut t = table(section(), custom);
	t.resize(TAB_RECS * 5, 0);
	for (i, w) in t.iter().enumerate() {
		mem.wr32(super::TGTAB + 4 * i as u32, *w);
	}
}

/// `build_title`: the page opens on its remembered section, with the stepper at rest.
pub(super) fn build(mem: &mut Mem) {
	set(|s| s.1 = true);
	mem.wr8(STEP, 0);
	// The Graphics rows' shared block (the page magic and the HUD, Renderer, Skip intro and Button glyphs bytes): on fresh RAM it
	// takes the options file's values, so those rows work on the title too, not only after the pause page has been built once. It
	// also writes the pause page's row table, which the title does not show.
	crate::retail::pcctl::build_gfx(mem, PAUSE_TABLE, &super::ROWS, HUD_SHARP.load(Ordering::Relaxed), crate::retail::pcrender::wanted_dx12());
	// The Preset row starts at the preset the rows match, so its first press steps on from there.
	super::presets::title_open(mem);
	write(mem);
}

/// Worker, every field (`pcext::sync`): a press on the stepper advances the section and rewrites the table; the Preset row's
/// 4th choice follows the Custom state. In a level (the pause page built) this does nothing.
pub(super) fn sync(mem: &mut Mem) {
	if !active() {
		return;
	}
	let custom = mem.rd8(pcctl::PRESETB).min(3) == 3;
	let custom_changed = get().2 != custom as u8;
	if mem.rd8(STEP) != 0 {
		set(|s| s.0 = (section() + 1) % pcctl::gfx_sections());
		mem.wr8(STEP, 0);
		write(mem);
	} else if custom_changed {
		write(mem);
	}
}

/// The page as the mirror draws it (`viewer/frontend.rs` `Page::Graphics`): the stepper, then the current section's rows, each
/// with its label and the text id of the value it shows now.
pub(super) fn rows() -> Vec<(u32, u32)> {
	// The title's Mods page is drawn by the same host page (`front_mirror::page_of`).
	if let Some(r) = crate::retail::pcmods::title_rows() {
		return r;
	}
	let sec = section();
	let mut v = unpack_gfx(GFX.load(Ordering::Relaxed));
	v[3] = FULLSCREEN.load(Ordering::Relaxed) as u8;
	// Fog and clouds are not in the packed word (unpack_gfx leaves their rows 0): their applied levels come from their modules.
	v[13] = super::swap_level(super::fogopt::level());
	v[14] = super::swap_level(super::cloudopt::level());
	let mut out = vec![(pcctl::ID_PAGE, ID_SECTION)];
	for &i in pcctl::gfx_section(sec) {
		let r = pcctl::gfx_row(i, super::presets::detect_now() == 3);
		let byte = match i {
			pcctl::HUD_ROW => HUD_SHARP.load(Ordering::Relaxed) as u8,
			pcctl::RENDER_ROW => crate::retail::pcrender::wanted_dx12() as u8,
			// The Preset row shows the preset matching the rows (Custom when they match none).
			pcctl::PRESET_ROW => super::presets::detect_now(),
			pcctl::INTRO_ROW => crate::retail::pcintro::skip() as u8,
			pcctl::SKIPCUT_ROW => !crate::retail::pcskip::skip() as u8,
			pcctl::GLYPH_ROW => crate::retail::xglyphs::style(),
			// The Display rows are not in the packed word: the position comes from its own byte's state, and the three steppers
			// show their dynamic text id (both choices are the same id).
			pcctl::HUDPOS_ROW => pcdisp::hud_corners() as u8,
			pcctl::HUDSCALE_ROW | pcctl::FPS_ROW | pcctl::RSCALE_ROW => 0,
			_ => v[i],
		};
		let vid = match i {
			pcctl::HUDSCALE_ROW | pcctl::FPS_ROW | pcctl::RSCALE_ROW => r[2],
			_ => {
				let id = r[2 + byte.min(3) as usize];
				if id != 0 {
					id
				} else {
					r[2]
				}
			}
		};
		out.push((r[0], vid));
	}
	out
}

/// The stepper's text in `lang` (`< Picture >` and so on): the mirror inserts it into its text map while the page is drawn.
pub(super) fn texts(lang: usize) -> Vec<(u32, String)> {
	let name = pcctl::gfx_section_name(lang, section());
	let mut s = String::from("< ");
	s.extend(name.iter().map(|&b| b as char));
	s.push_str(" >");
	vec![(ID_SECTION, s)]
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The title's table is the pause page's rows, section by section: the stepper in front, then the section's rows with the
	/// same labels, bytes and value texts, then the zero record.
	#[test]
	fn the_title_page_has_the_pause_pages_rows() {
		for sec in 0..pcctl::gfx_sections() {
			let title = table(sec, false);
			assert_eq!(&title[..5], &[pcctl::ID_PAGE, STEP, ID_SECTION, ID_SECTION, 0], "section {sec}: the stepper comes first");
			let mut want: Vec<u32> = Vec::new();
			for &i in pcctl::gfx_section(sec) {
				let r = pcctl::gfx_row(i, false);
				want.extend([r[0], r[1], r[2], r[3], 0]);
			}
			want.push(0);
			assert_eq!(&title[5..], &want[..], "section {sec}: the title shows the pause page's rows");
			assert!(title.len() <= TAB_RECS * 5, "section {sec} fits TGTAB");
		}
	}

	/// Every Graphics section, including Display and System (Start skips cutscenes, Button glyphs), is on the title page, and the
	/// rows the title shows the values of are the ones the pause page applies.
	#[test]
	fn the_title_page_has_every_section() {
		let mut seen: Vec<usize> = (0..pcctl::gfx_sections()).flat_map(|s| pcctl::gfx_section(s).iter().copied()).collect();
		seen.sort_unstable();
		assert_eq!(pcctl::gfx_sections(), 5);
		assert!(seen.contains(&pcctl::SKIPCUT_ROW) && seen.contains(&pcctl::GLYPH_ROW));
		assert!(seen.contains(&pcctl::HUDPOS_ROW) && seen.contains(&pcctl::HUDSCALE_ROW) && seen.contains(&pcctl::FPS_ROW) && seen.contains(&pcctl::RSCALE_ROW));
	}

	/// The title's Display section draws from the same values the pause page shows: the position's two texts and the three
	/// dynamic stepper ids.
	#[test]
	fn the_display_rows_show_the_applied_values() {
		test_section(3);
		let rows = rows();
		let value = |label: u32| rows.iter().find(|r| r.0 == label).map(|r| r.1);
		assert!(matches!(value(pcdisp::ID_HUDPOS), Some(pcdisp::ID_P43 | pcdisp::ID_CORNERS)), "the HUD position row shows one of its two values");
		assert_eq!(value(pcdisp::ID_HUDSCALE), Some(pcdisp::ID_VSCALE));
		assert_eq!(value(pcdisp::ID_FPS), Some(pcdisp::ID_VFPS));
		assert_eq!(value(pcdisp::ID_RSCALE), Some(pcdisp::ID_VRS));
		assert_eq!(value(pcctl::ID_PAGE), Some(ID_SECTION), "the stepper leads");
		test_section(0);
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
		crate::retail::pcctl::build_gfx(&mut mem, super::super::GTAB, &super::super::ROWS, true, false);
		assert!(crate::retail::pcctl::hud_sharp(&mut mem).is_some(), "the pause page is built");
		// The title Graphics page opens (it takes the shared block over) and the front end runs a field.
		build(&mut mem);
		super::super::sync(&mut mem, u32::MAX, false, &mut s);
		let high = super::super::GROWS.map(|a| mem.rd8(a));
		// The player changes a row a preset covers: it must keep the stepped value, not be reset to Low by a preset.
		mem.wr8(super::super::AO, 0);
		super::super::sync(&mut mem, u32::MAX, false, &mut s);
		assert_ne!(mem.rd8(super::super::AO), 0, "the row change was applied");
		assert_eq!(mem.rd8(super::super::WATER), high[4], "no preset reset the other rows");
		assert_eq!(mem.rd8(super::super::LIGHT), high[0], "no preset reset the lighting");
		let once = super::super::GROWS.map(|a| mem.rd8(a));
		super::super::sync(&mut mem, u32::MAX, false, &mut s);
		assert_eq!(super::super::GROWS.map(|a| mem.rd8(a)), once, "a second field leaves the rows as they are");
		// A real Preset press: the byte changes, one step is applied and it settles.
		let page = mem.rd8(crate::retail::pcctl::PRESETB).min(3);
		mem.wr8(crate::retail::pcctl::PRESETB, (page + 1) % 4);
		super::super::sync(&mut mem, u32::MAX, false, &mut s);
		assert_eq!(super::super::presets::detect_now(), 0, "one press from Custom applies Low");
		let after = super::super::GROWS.map(|a| mem.rd8(a));
		super::super::sync(&mut mem, u32::MAX, false, &mut s);
		assert_eq!(super::super::GROWS.map(|a| mem.rd8(a)), after, "the preset settles after one field");
	}

	/// The title page's build sets the Graphics rows' shared block up on fresh RAM, so the HUD, Renderer, Skip intro and Button
	/// glyphs rows work on the title before any level has been visited.
	#[test]
	fn the_title_page_sets_the_shared_rows_up() {
		let _turn = super::super::tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
		let mut mem = Mem::new();
		assert!(crate::retail::pcctl::hud_sharp(&mut mem).is_none(), "fresh RAM has no page");
		build(&mut mem);
		assert_eq!(crate::retail::pcctl::hud_sharp(&mut mem), Some(HUD_SHARP.load(Ordering::Relaxed)), "the HUD row works");
		assert_eq!(crate::retail::pcctl::intro_skip(&mut mem), Some(crate::retail::pcintro::skip()), "the Skip intro row works");
		assert_eq!(mem.rd8(super::super::GLYPHS), crate::retail::xglyphs::style(), "the Button glyphs row works");
	}

	/// One press on the stepper advances the section (and the table follows), wrapping at the last one.
	#[test]
	fn the_stepper_advances_the_section() {
		test_section(0);
		let mut mem = Mem::new();
		build(&mut mem);
		assert_eq!(section(), 0);
		assert_eq!(mem.rd8(STEP), 0, "the stepper starts at rest");
		mem.wr8(STEP, 1);
		sync(&mut mem);
		assert_eq!(section(), 1);
		assert_eq!(mem.rd8(STEP), 0, "the stepper is back at rest");
		assert_eq!(mem.rd32(super::super::TGTAB + 20 * 1), table(1, false)[5], "the table is the new section's");
		for _ in 1..pcctl::gfx_sections() {
			mem.wr8(STEP, 1);
			sync(&mut mem);
		}
		assert_eq!(section(), 0, "wraps back to the first section");
		test_section(0);
		deactivate();
	}
}

