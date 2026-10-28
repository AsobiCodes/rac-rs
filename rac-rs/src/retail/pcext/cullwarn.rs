//! The Culling warning (v0.4.1): while Graphics -> Culling is Off, the Graphics page's title bar shows a notice in the game's own menu
//! font instead of "Graphics Options": turning Culling off gives no gameplay benefit and puts a heavy load on the hardware. It is a
//! warning, not a lock (the player can leave Off on). Why the title bar: the Graphics pages have no description panel and the item
//! engine cannot add one at run time (a panel's rectangle comes from an animation chosen when the page opens, so an extra panel would
//! be an empty box with Culling On); the title item's text id is a plain RAM word, so `sync` swaps it.
//!
//! Pause page: the notice is up while Culling is Off and the Screen section (the one with the Culling row) is shown. Title page: while
//! Culling is Off (all rows are on one page; the viewer's front-end mirror draws that page and asks `title_header_id`). The text has
//! two lines (byte 1 = new line, like the Buy plate) in a pool of its own (`POOL`, inside the camopts block BASE + 0x1000..0x1800,
//! which prompts.rs' canary skips).
//! Culling Off also needs the marker key `culling_off=kept` in the options file (`pcopts.rs` header); Culling is On after any preset.
use super::*;
use crate::retail::pcctl;

/// The notice's text id (after `ID_CLOUDS` 0x7f23).
pub const ID: u32 = 0x7f24;
/// The text's pool slot (0x100 bytes; the longest text is about 50).
pub(super) const POOL: u32 = BASE + 0x1400;
const POOL_LEN: u32 = 0x100;

/// En Fr De Es It in the game's font bytes (e-acute 0x8c, o-acute 0x9c, u-grave 0xa5; accents are the ones the other texts use). Fr, Es
/// and It start with the language's own word for the Culling row (texts.rs `ID_CULLING`: Masquage, Ocultacion with o-acute,
/// Occultamento). Two lines (byte 1 = new line) of at most 22 letters each: the title page's bar is 203 x 47 (about 383 x 78 pixels at 1280 x 720; the pause page's is 281 x 47), the
/// game's text drawer wraps a line wider than the bar and a third line would be cut off (first try, three long lines, was clipped on
/// the pause page, 2026-10-27). Checked by screenshot on both pages in all five languages (`scratch_v041_cull/runs`). [I] wording,
/// native review wanted.
pub(super) static TEXT: [&[u8]; 5] = [
	b"Culling Off: no gain\x01and heavy PC load.",
	b"Sans masquage : aucun\x01gain, charge \x8clev\x8ce.",
	b"Culling aus: kein\x01Vorteil, hohe Last.",
	b"Sin ocultaci\x9cn: sin\x01ventaja, mucha carga.",
	b"Senza occultamento:\x01solo pi\xa5 carico.",
];

/// The title page's heading text id (the front-end mirror, `frontend.rs` `header`, draws the title page natively and does not read the
/// RAM item): the notice while Culling is Off, else "Graphics Options".
pub fn title_header_id() -> u32 {
	header_id(unpack_gfx(GFX.load(Ordering::Relaxed))[9] == 1, true)
}

/// The notice as the front-end mirror's text entry (id, string in font bytes as chars), like `pcext::texts`.
pub(super) fn text_item(lang: usize) -> (u32, String) {
	(ID, TEXT[lang.min(4)].iter().map(|&b| b as char).collect())
}

/// The title item's text id: the notice while Culling is Off and its row is on show, else the page title.
pub(super) fn header_id(culling_off: bool, row_shown: bool) -> u32 {
	if culling_off && row_shown {
		ID
	} else {
		ID_GRAPHICS_HDR
	}
}

/// Worker, the end of `pcext::sync`: points the title items of the pause and title Graphics pages at the right text. Only pages that
/// hold one of the two ids are touched, so RAM without the page (a savestate, the Options list of another build) is left alone.
pub(super) fn sync(mem: &mut Mem) {
	let off = mem.rd8(CULL) == 1;
	let pause_shown = pcctl::gfx_built(mem) && pcctl::gfx_sec() == pcctl::SEC_SCREEN;
	for (item, shown) in [(GHDR, pause_shown), (TGHDR, true)] {
		let now = mem.rd32(item + 0x34);
		let want = header_id(off, shown);
		if (now == ID_GRAPHICS_HDR || now == ID) && now != want {
			mem.wr32(item + 0x34, want);
		}
	}
}

/// The text hook: the notice's string for `lang` goes into its pool slot, its address into v0.
pub(super) fn text_hook(c: &mut Cpu, lang: usize) -> bool {
	let s = TEXT[lang.min(4)];
	for (i, b) in s.iter().enumerate() {
		c.mem.wr8(POOL + i as u32, *b);
	}
	c.mem.wr8(POOL + s.len() as u32, 0);
	c.s(2, POOL as u64);
	true
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Same rules as `pcext::tests::texts_fit_the_font_and_are_nonempty` (printable ASCII, the game's accent bytes 0x80..0xb0, the new
	/// line 1), plus: two lines, each short enough for the title bar, and the whole text inside its pool slot.
	#[test]
	fn the_texts_fit_the_font_and_the_title_bar() {
		for (l, s) in TEXT.iter().enumerate() {
			assert!(s.iter().all(|&b| (0x20..0x7f).contains(&b) || (0x80..=0xb0).contains(&b) || b == 1), "language {l}");
			let lines: Vec<&[u8]> = s.split(|&b| b == 1).collect();
			assert_eq!(lines.len(), 2, "language {l}: two lines");
			assert!(lines.iter().all(|x| !x.is_empty() && x.len() <= 22), "language {l}: a line of at most 22 letters");
			assert!((s.len() as u32) < POOL_LEN);
		}
		assert!(POOL >= BASE + 0x1380 && POOL + POOL_LEN <= camopts::CO_POOL_END, "the pool is clear of POOL3, the row copy and the end of the block");
		assert!(!NEW_IDS.contains(&ID));
	}

	/// The notice is chosen exactly while Culling is Off and its row is on show; and it is swapped on the page's title item only.
	#[test]
	fn the_notice_is_up_while_culling_is_off_and_its_row_is_shown() {
		assert_eq!(header_id(true, true), ID);
		assert_eq!(header_id(true, false), ID_GRAPHICS_HDR);
		assert_eq!(header_id(false, true), ID_GRAPHICS_HDR);
		let mut mem = Mem::new();
		// RAM without the pages: nothing is written.
		mem.wr8(CULL, 1);
		sync(&mut mem);
		assert_eq!((mem.rd32(GHDR + 0x34), mem.rd32(TGHDR + 0x34)), (0, 0));
		// The title page holds its title: Off brings the notice, On takes it away.
		mem.wr32(TGHDR + 0x34, ID_GRAPHICS_HDR);
		sync(&mut mem);
		assert_eq!(mem.rd32(TGHDR + 0x34), ID);
		mem.wr8(CULL, 0);
		sync(&mut mem);
		assert_eq!(mem.rd32(TGHDR + 0x34), ID_GRAPHICS_HDR);
	}

	/// The pause page: the notice swaps in only while the Screen section (the one that shows the Culling row) is shown, a step to
	/// another section puts the title back, and so does Culling On. `SEC_SCREEN` really is the section with the Culling row.
	#[test]
	fn the_pause_title_is_swapped_and_restored() {
		let mut mem = Mem::new();
		pcctl::build_gfx(&mut mem, GTAB, &ROWS, false, false);
		mem.wr32(GHDR + 0x34, ID_GRAPHICS_HDR);
		mem.wr8(CULL, 1);
		let has_culling_row = |mem: &mut Mem| (0..8).any(|i| mem.rd32(GTAB + 24 * i) == ID_CULLING);
		pcctl::test_gfx_section(&mut mem, 0, &ROWS, GTAB);
		sync(&mut mem);
		assert_eq!((mem.rd32(GHDR + 0x34), has_culling_row(&mut mem)), (ID_GRAPHICS_HDR, false), "Picture section: no Culling row, no notice");
		pcctl::test_gfx_section(&mut mem, pcctl::SEC_SCREEN, &ROWS, GTAB);
		assert!(has_culling_row(&mut mem), "SEC_SCREEN shows the Culling row");
		sync(&mut mem);
		assert_eq!(mem.rd32(GHDR + 0x34), ID, "Screen section with Culling Off: the notice");
		pcctl::test_gfx_section(&mut mem, 1, &ROWS, GTAB);
		sync(&mut mem);
		assert_eq!(mem.rd32(GHDR + 0x34), ID_GRAPHICS_HDR, "another section: the title is back");
		pcctl::test_gfx_section(&mut mem, pcctl::SEC_SCREEN, &ROWS, GTAB);
		sync(&mut mem);
		assert_eq!(mem.rd32(GHDR + 0x34), ID);
		mem.wr8(CULL, 0);
		sync(&mut mem);
		assert_eq!(mem.rd32(GHDR + 0x34), ID_GRAPHICS_HDR, "Culling On: the title is back");
		// A word that holds neither id (another build's RAM) is never touched.
		mem.wr8(CULL, 1);
		mem.wr32(GHDR + 0x34, 0x1234);
		sync(&mut mem);
		assert_eq!(mem.rd32(GHDR + 0x34), 0x1234);
	}

	/// The text lookup hand (`pcext::text_hook`, the one retail's lookup entries are patched to) routes 0x7f24 to the notice in every
	/// language word (0 En, 2 Fr, 3 De, 4 Es, 5 It) and returns a pointer into the notice's pool slot.
	#[test]
	fn the_text_lookup_returns_the_notice_in_every_language() {
		let mut c = Cpu::new(Mem::new());
		for (word, l) in [(0u32, 0usize), (2, 1), (3, 2), (4, 3), (5, 4)] {
			c.mem.wr32(LANG_WORD, word);
			c.s(4, ID as u64);
			assert!(super::super::text_hook(&mut c));
			let p = c.g(2) as u32;
			assert!((POOL..POOL + POOL_LEN).contains(&p), "pool pointer {p:#x}");
			let mut got = Vec::new();
			while c.mem.rd8(p + got.len() as u32) != 0 {
				got.push(c.mem.rd8(p + got.len() as u32));
			}
			assert_eq!(got, TEXT[l], "language word {word}");
		}
	}
}
