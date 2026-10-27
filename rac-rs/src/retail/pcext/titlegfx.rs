//! The title screen's Graphics page: the same sections, rows and values as the pause menu's Graphics page (`pcctl.rs`).
//!
//! The title's Options list row opens a copy of the retail Video Options page (`pcext::build_title`); this module writes its row
//! table: a "Page" stepper followed by the current section's rows. The front end's value item only flips the row's byte on a
//! press, so one press steps on from the value last applied (`pcext::sync` does that for the rows with more than two values), and
//! the stepper's own byte advances the section here. The mirror draws the page from `rows()` (viewer/front_mirror.rs).
//!
//! The rows themselves come from `pcctl::gfx_section` / `pcctl::gfx_row`, the same definitions the pause page uses, so the two
//! pages cannot drift apart.

use super::*;
use crate::retail::pcctl;

/// The stepper row's byte: the front-end item flips it on a press, `sync` advances the section and puts it back.
pub(super) const STEP: u32 = BASE + 24;
/// The stepper row's value text ("< Picture >"): its string changes with the section, so the mirror refreshes it.
pub(super) const ID_SECTION: u32 = 0x7f24;

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
const TAB_RECS: usize = 14;

/// Writes the current section's table at `TGTAB` (padded, so no record of a longer section stays behind the zero record).
fn write(mem: &mut Mem) {
	let custom = mem.rd8(pcctl::PRESETB).min(3) == 3;
	set(|s| s.2 = custom as u8);
	let mut t = table(section(), custom);
	t.resize(TAB_RECS * 5, 0);
	for (i, w) in t.iter().enumerate() {
		mem.wr32(TGTAB + 4 * i as u32, *w);
	}
}

/// `build_title`: the page opens on its remembered section, with the stepper at rest.
pub(super) fn build(mem: &mut Mem) {
	set(|s| s.1 = true);
	mem.wr8(STEP, 0);
	// The Graphics rows' shared block (the page magic and the HUD, Renderer and Skip intro bytes): on fresh RAM it takes the
	// options file's values, so those rows work on the title too, not only after the pause page has been built once. It also
	// writes the pause page's row table, which the title does not show.
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
			pcctl::GLYPH_ROW => crate::retail::xglyphs::style(),
			_ => v[i],
		};
		let vid = r[2 + byte.min(3) as usize];
		out.push((r[0], if vid != 0 { vid } else { r[2] }));
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
		crate::retail::pcctl::build_gfx(&mut mem, super::GTAB, &ROWS, true, false);
		assert!(crate::retail::pcctl::hud_sharp(&mut mem).is_some(), "the pause page is built");
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
		let page = mem.rd8(crate::retail::pcctl::PRESETB).min(3);
		mem.wr8(crate::retail::pcctl::PRESETB, (page + 1) % 4);
		super::super::sync(&mut mem, u32::MAX, false, &mut s);
		assert_eq!(super::super::presets::detect_now(), 0, "one press from Custom applies Low");
		let after = GROWS.map(|a| mem.rd8(a));
		super::super::sync(&mut mem, u32::MAX, false, &mut s);
		assert_eq!(GROWS.map(|a| mem.rd8(a)), after, "the preset settles after one field");
	}

	/// The title page's build sets the Graphics rows' shared block up on fresh RAM, so the HUD, Renderer and Skip intro rows work
	/// on the title before any level has been visited.
	#[test]
	fn the_title_page_sets_the_shared_rows_up() {
		let _turn = super::super::tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
		let mut mem = Mem::new();
		assert!(crate::retail::pcctl::hud_sharp(&mut mem).is_none(), "fresh RAM has no page");
		build(&mut mem);
		assert_eq!(crate::retail::pcctl::hud_sharp(&mut mem), Some(HUD_SHARP.load(Ordering::Relaxed)), "the HUD row works");
		assert_eq!(crate::retail::pcctl::intro_skip(&mut mem), Some(crate::retail::pcintro::skip()), "the Skip intro row works");
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
		assert_eq!(mem.rd32(TGTAB + 20 * 1), table(1, false)[5], "the table is the new section's");
		for _ in 1..pcctl::gfx_sections() {
			mem.wr8(STEP, 1);
			sync(&mut mem);
		}
		assert_eq!(section(), 0, "wraps back to the first section");
		test_section(0);
		deactivate();
	}
}

// ---- the title's Options list hook (moved out of pcext.rs for the size caps) ----

pub(super) const TITLE_CODE: [(u32, u32); 11] = [
	(0x3c02_0000, 0xffff_0000), // lui v0, hi(region)
	(0x8c42_0000, 0xffff_0000), // lw v0, lo(region)(v0)
	(0x5040_0004, !0),          // beql v0, zero, +4
	(0x3c02_0000, 0xffff_0000), // lui v0, hi(lists)
	(0x3c02_0000, 0xffff_0000), // lui v0, hi(lists)
	(0x1000_0002, !0),          // b +2
	(0x2442_0000, 0xffff_0000), // addiu v0, v0, lo(PAL list)
	(0x2442_0000, 0xffff_0000), // addiu v0, v0, lo(NTSC list)
	(0xac82_0034, !0),          // sw v0, 0x34(a0)
	(0x03e0_0008, !0),          // jr ra
	(0x0000_102d, !0),          // move v0, zero
];

/// The front end runs interpreted (no recompiled code: title_boot.rs `hook_front_movies`), so the Options hook is an oracle host
/// stub on every copy of `TITLE_CODE` in the code range, applied only while RAM still holds that function. Called when the
/// front end starts (cold boot title in the boot ELF, frontbin after Quit Game).
pub fn hook_front(m: &mut ee_oracle::Machine) {
	if !enabled() {
		return;
	}
	let ram = &m.cpu.mem.ram;
	let rd = |a: usize| u32::from_le_bytes(ram[a..a + 4].try_into().unwrap());
	let found: Vec<u32> = (0x10_0000..0x40_0000 - 4 * TITLE_CODE.len()).step_by(4).filter(|&a| TITLE_CODE.iter().enumerate().all(|(k, &(v, mask))| rd(a + 4 * k) & mask == v)).map(|a| a as u32).collect();
	for a in found {
		let sig = [0, 1, 2, 3].map(|k| m.rd32(a + 4 * k));
		m.cpu.add_host_stub(a, "title Options list (pcext)", sig, title_options);
		eprintln!("retail: PC options: title Options hook at {a:#x}");
	}
}

/// The title's Options page (host stub on its list set-up, `hook_front`): the retail rows of the disc's region and a Graphics row
/// that opens a copy of the Video Options page reading the Graphics rows (`ROWS`: the bytes of the pause menu's Graphics
/// page), all in reserved RAM. The region is set up first (`setup`), so the rows show the remembered values. The instruction
/// count advances as retail's would (9 instructions with the PAL rows, 8 with the NTSC ones).
fn title_options(c: &mut Cpu) -> bool {
	let item = c.g(4) as u32;
	let Ok(mut s) = ST.lock() else { return false };
	let Some(n) = build_title(&mut c.mem, c.pc, item, &mut s) else { return false };
	c.s(2, 0);
	c.icount += n;
	c.mem.now = c.icount;
	true
}

/// `title_options` on the memory; Some(the instructions retail runs). Every address comes from the function's own code at `pc`
/// (the region word, both row lists) and the Video page the PAL list names, checked against the retail texts, so the same body
/// runs on both discs. None when a check fails: the retail function then runs (no Graphics row). The page's slots, items and
/// row table are rebuilt on every entry to Options, the list records only when the rows changed (their third word is the
/// menu's own).
pub(super) fn build_title(mem: &mut Mem, pc: u32, item: u32, s: &mut St) -> Option<u64> {
	let ok = |a: u32| (0x10_0000..0x200_0000).contains(&a);
	let imm = |w: u32| w as u16 as i16 as i32 as u32;
	// lui v0,hi; lw v0,lo(v0) (the region word); ...; lui v0,hi; ...; addiu v0,v0,pal; addiu v0,v0,ntsc.
	let (w0, w1, w3, w6, w7) = (mem.rd32(pc), mem.rd32(pc + 4), mem.rd32(pc + 0xc), mem.rd32(pc + 0x18), mem.rd32(pc + 0x1c));
	let region = ((w0 & 0xffff) << 16).wrapping_add(imm(w1));
	let (pal, ntsc) = (((w3 & 0xffff) << 16).wrapping_add(imm(w6)), ((w3 & 0xffff) << 16).wrapping_add(imm(w7)));
	if !ok(item) || !ok(region) || !ok(pal) || !ok(ntsc) {
		return None;
	}
	let rows = |mem: &mut Mem, a: u32| -> Vec<[u32; 3]> { (0..5).map(|i| [mem.rd32(a + 12 * i), mem.rd32(a + 12 * i + 4), mem.rd32(a + 12 * i + 8)]).take_while(|r| r[0] != 0).collect() };
	let page = |rs: &[[u32; 3]], id: u32| rs.iter().find(|r| r[0] == id | 3 << 16).map(|r| r[1]).filter(|&p| ok(p));
	let is_pal = mem.rd32(region) != 0;
	let shown = if is_pal { pal } else { ntsc };
	let mut recs = rows(mem, shown);
	let (Some(video), Some(sound)) = (page(&rows(mem, pal), FRONT_VIDEO), page(&recs, FRONT_SOUND)) else { return None };
	if recs.len() > 3 || recs.iter().any(|r| r[0] >> 16 != 3) || mem.rd32(video + 0x38) != mem.rd32(sound + 0x38) {
		return None;
	}
	let items: Vec<u32> = (0..14).map(|k| mem.rd32(video + 0x44 + 4 * k)).filter(|&a| ok(a)).collect();
	let hdr = items.iter().copied().find(|&it| mem.rd32(it + 0x34) == FRONT_VIDEO_HDR);
	let val = items.iter().copied().find(|&it| {
		let t = mem.rd32(it + 0x34);
		ok(t) && mem.rd32(t) == FRONT_VIDEO_ROW
	});
	let (Some(hdr), Some(val)) = (hdr, val) else { return None };
	setup(mem, s);
	recs.push([ID_GRAPHICS | 3 << 16, TGPAGE, 0]);
	let seq = mem.rd32(sound + 0xc);
	let trecs = crate::retail::pcmods::title_extend(mem, [video, hdr, val], seq, &mut recs, super::TRECS);
	let same = recs.iter().enumerate().all(|(i, r)| mem.rd32(trecs + 12 * i as u32) == r[0] && mem.rd32(trecs + 12 * i as u32 + 4) == r[1]) && mem.rd32(trecs + 12 * recs.len() as u32) == 0;
	if !same {
		for (i, r) in recs.iter().chain([&[0u32; 3]]).enumerate() {
			for (k, w) in r.iter().enumerate() {
				mem.wr32(trecs + 12 * i as u32 + 4 * k as u32, *w);
			}
		}
	}
	// The page: Video Options' own (slot sequences, parent = Options, id, items) with our header and value items. The value
	// panel (slot 3) plays the Sound page's slot-3 sequence: its rect (47,125,419,164) holds the 13 rows, Video's holds one.
	copy_words(mem, video, TGPAGE, 0x88);
	for o in (0x88..0x100).step_by(4) {
		mem.wr32(TGPAGE + o, 0);
	}
	mem.wr32(TGPAGE + 0xc, seq);
	for o in (0x40..0x80).step_by(4) {
		match mem.rd32(TGPAGE + o) {
			a if a == hdr => mem.wr32(TGPAGE + o, TGHDR),
			a if a == val => mem.wr32(TGPAGE + o, TGVAL),
			_ => {}
		}
	}
	copy_words(mem, hdr, TGHDR, 0x58);
	mem.wr32(TGHDR + 0x34, ID_GRAPHICS_HDR);
	copy_words(mem, val, TGVAL, 0x40);
	mem.wr32(TGVAL + 0x34, TGTAB);
	mem.wr32(TGVAL + 0x38, 0);
	mem.wr32(TGVAL + 0x3c, 0);
	// The page's rows: the stepper and the section's rows, the same definitions the pause page uses (titlegfx.rs).
	titlegfx::build(mem);
	mem.wr32(item + 0x34, trecs);
	Some(if is_pal { 9 } else { 8 })
}

