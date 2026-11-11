use super::*;
use rac_engine::viewer::{aspect_box, upscale_choice};

fn page(mem: &mut Mem) {
	let rows: Vec<[u32; 6]> = (0..19u32).map(|i| [0x100 + i, 0x200 + i, 0x300, 0x301, 0, 0]).collect();
	pcctl::build_gfx(mem, super::super::pcext::BASE + 0xaa0, &rows, true, false);
}

fn fresh() -> (Mem, PcOptions) {
	aspect_box::init(&PcOptions::new());
	upscale_choice::init(&PcOptions::new());
	super::super::qs_pause::init(&PcOptions::new());
	super::super::fast_travel::init(&PcOptions::new());
	reset_statics();
	let mut mem = Mem::new();
	page(&mut mem);
	(mem, PcOptions::new())
}

#[test]
fn steps_wrap_on_cross_and_stop_at_the_ends_on_arrows() {
	assert_eq!((step(5, 6, Step::Cross, |_| true), step(0, 6, Step::Left, |_| true), step(5, 6, Step::Right, |_| true)), (0, 0, 5));
	assert_eq!((step(0, 6, Step::Right, |_| true), step(3, 6, Step::Left, |_| true)), (1, 2));
	// DLSS cannot run: FSR then back to Off; nothing else available = stay.
	assert_eq!((step(1, 3, Step::Cross, |v| v != 2), step(0, 3, Step::Right, |v| v != 1), step(1, 3, Step::Right, |v| v == 0)), (0, 2, 1));
}

#[test]
fn the_classic_rows_change_what_the_modern_rows_change() {
	let _g = crate::retail::pcopts::globals_guard();
	let (mut mem, mut opts) = fresh();
	// Aspect: one Cross = 4:3, the same engine state and file key the modern row sets.
	field(&mut mem, 0, [false; 2]); // baseline
	mem.wr8(ASP_B, 1);
	field(&mut mem, 0, [false; 2]);
	assert!(sync(&mut mem, &mut opts) && !sync(&mut mem, &mut opts));
	assert_eq!(aspect_box::current(), aspect_box::Ratio::R4x3);
	let mut modern = PcOptions::new();
	aspect_box::init(&PcOptions::new());
	assert!(aspect_box::set_from_ui(&mut modern, 1));
	assert_eq!(opts.extra_get("aspect"), modern.extra_get("aspect"));
	assert_eq!(text(ID_VASPECT, 0), b"< 4:3 >");
	// Upscaler: Cross = FSR, key `upscaler`.
	mem.wr8(UPS_B, 1);
	field(&mut mem, 0, [false; 2]);
	assert!(sync(&mut mem, &mut opts));
	assert_eq!(opts.extra_get("upscaler"), Some("fsr"));
	assert_eq!(text(ID_VUPS, 0), b"< FSR >");
	assert_eq!(text(ID_VUPS, 2), b"< FSR >");
	// Quick Select pause: the byte is the setting, key `qspause`.
	mem.wr8(QSP_B, 1);
	assert!(sync(&mut mem, &mut opts) && super::super::qs_pause::on());
	assert_eq!(opts.extra_get("qspause"), Some("on"));
	// The modern screen switches it off: the byte follows, no press is read.
	super::super::qs_pause::set_from_ui(&mut PcOptions::new(), false);
	follow_ui(&mut mem);
	assert_eq!(mem.rd8(QSP_B), 0);
	assert!(!sync(&mut mem, &mut opts));
	// Fast travel: the page opens with the byte at the setting (On by default), a press is the setting, key `fasttravel`.
	assert_eq!(mem.rd8(FT_B), 1, "built at the default On");
	mem.wr8(FT_B, 0);
	assert!(sync(&mut mem, &mut opts) && !super::super::fast_travel::on());
	assert_eq!(opts.extra_get("fasttravel"), Some("off"));
	assert!(!sync(&mut mem, &mut opts), "no new press");
	// The modern screen switches it back on: the byte follows, no press is read, Quick Select pause stays.
	super::super::fast_travel::set_from_ui(&mut PcOptions::new(), true);
	follow_ui(&mut mem);
	assert_eq!((mem.rd8(FT_B), mem.rd8(QSP_B)), (1, 0));
	assert!(!sync(&mut mem, &mut opts));
	assert_eq!(text(ID_FT, 0), b"Fast travel");
	aspect_box::init(&PcOptions::new());
	upscale_choice::init(&PcOptions::new());
	super::super::qs_pause::init(&PcOptions::new());
	super::super::fast_travel::init(&PcOptions::new());
}

#[test]
fn texts_are_short_font_bytes_in_five_languages() {
	let _g = crate::retail::pcopts::globals_guard();
	let (_m, _o) = fresh();
	for l in 0..5 {
		for id in IDS {
			let s = text(id, l);
			assert!(!s.is_empty() && s.len() <= 22, "{id:#x} {l}: {s:?}");
			assert!(s.iter().all(|&b| (0x20..0x7f).contains(&b) || (0x80..=0xb0).contains(&b)) && !s.contains(&b'%'));
		}
	}
	assert_eq!(text(ID_VUPS, 1), b"< Non >");
	assert!(text(0x7fc5, 0).is_empty());
}

#[test]
fn the_rows_sit_on_the_pages_of_the_modern_tabs() {
	let disp = pcctl::gfx_section(pcctl::GFX_DISPLAY);
	assert_eq!(&disp[disp.len() - 2..], &[pcctl::ASP_ROW, pcctl::UPS_ROW]);
	// The modern Game tab's Gameplay group: its own last section, Quick Select pause then Fast travel.
	let play = pcctl::gfx_section(pcctl::gfx_sections() - 1);
	assert_eq!(play, [pcctl::QSP_ROW, pcctl::FT_ROW]);
	assert!(disp.len() <= 7);
	assert_eq!(pcctl::gfx_row(pcctl::QSP_ROW, false)[2..4], [0x4f5b, 0x4f5a]);
	assert_eq!(pcctl::gfx_row(pcctl::FT_ROW, false)[..4], [ID_FT, FT_B, 0x4f5b, 0x4f5a]);
}

/// Every PC text id has ONE owner. `pcext::text_hook` tries its owners in a fixed order and the first range that matches wins, so
/// two owners claiming one id make the later one read the earlier one's text (the Mods row once read "Aspect ratio"). This lists
/// what each owner claims over the whole PC id space (0x7000..0x8000) and fails on any overlap, then checks the new ids by name.
#[test]
fn no_two_text_owners_claim_one_id() {
	use super::super::{pcctl, pcdisp, pcext, pcmods};
	// camopts.rs / lockstr.rs (private to pcext): ID_LOCKON 0x7f80 ..= ID_LPCT 0x7f88.
	let camopts = |id: u32| (0x7f80..=0x7f88).contains(&id);
	let ctl = |id: u32| (0x7f40..pcctl::ID_LAST).contains(&id) || (pcctl::ID_DBG0..pcctl::ID_DBG_END).contains(&id);
	let new_ids: Vec<u32> = pcext::texts(0).map(|(id, _)| id).filter(|&id| id != pcext::cullwarn::ID).collect(); // texts() also lists the Culling notice (cullwarn)
	let owners: [(&str, &dyn Fn(u32) -> bool); 6] = [
		("camopts", &camopts),
		("pcctl", &ctl),
		("pcdisp pool", &|id| pcdisp::serves(id)),
		("cullwarn", &|id| id == pcext::cullwarn::ID),
		("pcmods", &|id| (pcmods::ID_FIRST..pcmods::ID_END).contains(&id)),
		("pcext new texts", &|id| new_ids.contains(&id)),
	];
	for id in 0x7000..0x8000u32 {
		let hit: Vec<&str> = owners.iter().filter(|(_, f)| f(id)).map(|(n, _)| *n).collect();
		assert!(hit.len() <= 1, "text id {id:#x} is claimed by {hit:?}");
	}
	for id in IDS {
		assert!(pcdisp::serves(id) && id < pcmods::ID_FIRST, "{id:#x}: pcmore's ids sit in pcdisp's pool, below the Mods page's");
		assert!((0x7f98..=0x7f9f).contains(&id) && !ctl(id) && !camopts(id), "{id:#x}"); // 0x7f9e / 0x7f9f: pcfgen.rs
	}
	// The title page's text map (titlegfx::texts) must not carry an id of the Mods page either.
	for lang in 0..5 {
		for (id, _) in pcext::titlegfx::texts(lang) {
			assert!(!(pcmods::ID_FIRST..pcmods::ID_END).contains(&id), "{id:#x}: the title page's pause texts must not shadow the Mods page");
		}
	}
	assert_eq!(IDS.iter().map(|&i| text(i, 0).is_empty()).collect::<Vec<_>>(), [false; 8]);
}
