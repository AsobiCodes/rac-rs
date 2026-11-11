use super::*;
use rac_engine::viewer::fgen_follow as fl;

fn page(mem: &mut Mem) {
	let rows: Vec<[u32; 6]> = (0..19u32).map(|i| [0x100 + i, 0x200 + i, 0x300, 0x301, 0, 0]).collect();
	pcctl::build_gfx(mem, super::super::pcext::BASE + 0xaa0, &rows, true, false);
}

/// The engine choice from `file` (`framegen=` value), FSR offered or not, and a fresh page.
fn fresh(file: Option<&str>, fsr: bool) -> (Mem, PcOptions) {
	let mut opts = PcOptions::new();
	if let Some(v) = file {
		opts.extra_set(fg::KEY, Some(v.to_string()));
	}
	fg::init(&opts);
	fg::set_runs(fg::FSR_VALUE, fsr);
	// Lane fglive: the Upscaler FSR (FSR 2x fits it) and a run started with the file's maker (its values apply at once).
	rac_engine::viewer::upscale_choice::set_kind(&mut opts, rac_engine::viewer::upscale_choice::FFX_VALUE);
	fl::set_running(fl::maker_of(fg::index(file) as i32));
	reset_statics();
	let mut mem = Mem::new();
	page(&mut mem);
	(mem, opts)
}

#[test]
fn the_row_has_off_and_fsr_2x_in_the_engines_order_and_five_labels() {
	assert_eq!(NAMES.len() + 1, fg::KINDS.len(), "one name per engine value after Off");
	assert_eq!(NAMES[fg::FSR_VALUE as usize - 1], "FSR 2x");
	let modern = crate::modern_menu::rows_upscale::FG_VALUES;
	assert_eq!(modern.len(), fg::KINDS.len(), "the modern row has the same values");
	assert!(modern[fg::FSR_VALUE as usize].iter().all(|s| *s == NAMES[0]));
	for (i, v) in fg::DLSS_VALUES.into_iter().enumerate() {
		let name = format!("DLSS {}x", i + 2);
		assert_eq!((NAMES[v as usize - 1], fg::KINDS[v as usize]), (name.as_str(), format!("dlss{}", i + 2).as_str()));
		assert!(modern[v as usize].iter().all(|s| *s == name), "the modern row names it the same in all 5 languages");
	}
	for l in 0..5 {
		assert!(!text(ID_FG, l).is_empty() && !NOT_AVAILABLE[l].is_empty());
	}
	assert_eq!(text(ID_FG, 0), b"Frame Generation");
	assert_eq!(text(ID_FG, 1), b"G\x8cn\x8cration d'images");
	assert!(text(0x7f9d, 0).is_empty());
}

#[test]
fn cross_steps_the_choice_into_the_options_file_and_says_restart() {
	let _g = crate::retail::pcopts::globals_guard();
	let (mut mem, mut opts) = fresh(None, true);
	assert_eq!(text(ID_VFG, 0), b"< Off >");
	field(&mut mem, u32::MAX, [false; 2]); // first look: byte seen, no step
	mem.wr8(FG_B, 1); // retail's Cross
	field(&mut mem, u32::MAX, [false; 2]);
	assert_eq!(text(ID_VFG, 0), b"< FSR 2x - restart >", "pending already shows");
	assert!(sync(&mut opts), "the options file changed");
	assert_eq!((opts.extra_get(fg::KEY), fg::chosen()), (Some("fsr"), 1));
	assert_eq!(text(ID_VFG, 2), b"< FSR 2x - Neustart >");
	mem.wr8(FG_B, 0); // Cross again wraps to Off = the running value: no restart
	field(&mut mem, u32::MAX, [false; 2]);
	assert!(sync(&mut opts));
	assert_eq!((opts.extra_get(fg::KEY), text(ID_VFG, 1)), (Some("off"), b"< Non >".to_vec()));
	assert!(!sync(&mut opts), "nothing pending");
}

#[test]
fn a_file_that_chose_fsr_shows_it_without_restart_and_no_dlls_read_not_available() {
	let _g = crate::retail::pcopts::globals_guard();
	let (_mem, mut opts) = fresh(Some("fsr"), true);
	assert_eq!(text(ID_VFG, 0), b"< FSR 2x >");
	let (mut mem2, _) = fresh(Some("fsr"), false);
	assert_eq!(text(ID_VFG, 0), b"< Not available >");
	assert_eq!(text(ID_VFG, 3), b"< No disponible >");
	field(&mut mem2, u32::MAX, [false; 2]);
	mem2.wr8(FG_B, 1);
	field(&mut mem2, u32::MAX, [false; 2]);
	assert!(!sync(&mut opts), "Cross on a row with nothing to choose changes nothing");
	fg::set_runs(fg::FSR_VALUE, true);
}

#[test]
fn the_row_sits_in_the_screen_section_after_renderer_with_its_own_byte_and_ids() {
	let scr = pcctl::gfx_section(pcctl::SEC_SCREEN);
	let at = scr.iter().position(|&r| r == pcctl::FG_ROW).expect("Screen has the row");
	#[cfg(not(target_os = "linux"))]
	assert_eq!(scr[at - 1], pcctl::RENDER_ROW);
	assert!(scr.len() <= 7, "Graphics sections hold at most seven settings");
	assert_eq!(pcctl::gfx_row(pcctl::FG_ROW, false), [ID_FG, FG_B, ID_VFG, ID_VFG, 0, 0]);
	assert!(pcctl::RANGE.contains(&FG_B) && FG_B == CBASE + 0x2e);
	assert!(super::super::pcdisp::serves(ID_FG) && super::super::pcdisp::serves(ID_VFG));
}

#[test]
fn the_title_graphics_text_key_follows_the_row_value_and_availability() {
	let _g = crate::retail::pcopts::globals_guard();
	let _ = fresh(None, true);
	let off = super::super::pcdisp::text_key();
	PEND.store(fg::FSR_VALUE as i8, Ordering::Relaxed);
	let fsr = super::super::pcdisp::text_key();
	assert_ne!(off, fsr, "a new row value refreshes the classic Graphics page's texts");
	fg::set_runs(fg::FSR_VALUE, false);
	assert_ne!(fsr, super::super::pcdisp::text_key(), "FSR withdrawn refreshes them too");
	reset_statics();
}

/// Lane fglive: the row follows the Upscaler: Off = "Needs an upscaler" and no step; DLSS with DLSS frame generation offered =
/// a chosen FSR 2x shows Off (the file keeps it) and steps to DLSS 2x, which needs a restart in a run started with FSR.
#[test]
fn the_classic_row_follows_the_upscaler() {
	use rac_engine::viewer::upscale_choice as up;
	let _g = crate::retail::pcopts::globals_guard();
	let (mut mem, mut opts) = fresh(Some("fsr"), true);
	up::set_kind(&mut opts, 0);
	assert_eq!(text(ID_VFG, 0), b"< Needs an upscaler >");
	field(&mut mem, u32::MAX, [false; 2]);
	mem.wr8(FG_B, 1);
	field(&mut mem, u32::MAX, [false; 2]);
	assert!(!sync(&mut opts), "locked while the Upscaler is Off");
	assert_eq!(opts.extra_get(fg::KEY), Some("fsr"), "the file keeps the choice");
	for v in fg::DLSS_VALUES {
		fg::set_runs(v, true);
	}
	up::set_kind(&mut opts, up::DLSS_VALUE);
	assert_eq!(text(ID_VFG, 0), b"< Off >", "FSR 2x does not follow DLSS: in force Off");
	mem.wr8(FG_B, 0);
	field(&mut mem, u32::MAX, [false; 2]);
	assert!(sync(&mut opts));
	assert_eq!((opts.extra_get(fg::KEY), text(ID_VFG, 0)), (Some("dlss2"), b"< DLSS 2x - restart >".to_vec()));
	up::set_kind(&mut opts, up::FFX_VALUE);
	for v in fg::DLSS_VALUES {
		fg::set_runs(v, false);
	}
	reset_statics();
}

/// Review 6: the default options (Upscaler Off, Frame Generation Off) publish the fork point's text and key (base_v051_tb12
/// pcfgen.rs: "< Off >", or "< Not available >" with nothing this PC runs; key started << 1 | available), so the classic Graphics
/// page writes the same EE RAM bytes as before lane fglive (fullcheck `menu`).
#[test]
fn the_default_options_read_as_at_the_fork_point() {
	use rac_engine::viewer::upscale_choice as up;
	let _g = crate::retail::pcopts::globals_guard();
	for v in fg::DLSS_VALUES {
		fg::set_runs(v, false);
	}
	#[rustfmt::skip]
	let fork_off: [&[u8]; 5] = [b"< Off >", b"< Non >", b"< Aus >", b"< No >", b"< No >"];
	#[rustfmt::skip]
	let fork_na: [&[u8]; 5] = [b"< Not available >", b"< Indisponible >", b"< Nicht vorhanden >", b"< No disponible >", b"< Non disponibile >"];
	for (fsr, fork, key) in [(true, fork_off, 1u64), (false, fork_na, 0)] {
		let (mut mem, mut opts) = fresh(None, fsr);
		fl::set_running(None);
		up::set_kind(&mut opts, 0);
		for l in 0..5 {
			assert_eq!(text(ID_VFG, l), fork[l].to_vec(), "language {l}, FSR offered {fsr}");
		}
		assert_eq!(text_key(), key);
		// No step while the Upscaler is Off: Cross and Right leave the file and the text alone.
		field(&mut mem, u32::MAX, [false; 2]);
		mem.wr8(FG_B, 1);
		field(&mut mem, u32::MAX, [false; 2]);
		assert!(!sync(&mut opts));
		assert_eq!(text(ID_VFG, 0), fork[0].to_vec());
	}
	up::set_kind(&mut PcOptions::new(), up::FFX_VALUE);
	reset_statics();
}