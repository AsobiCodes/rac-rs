//! Lane fglive: the pure rules of follow.rs (offers per Upscaler x card, in force, why paused, live vs restart, the note).
use super::*;

const OFF: i32 = 0;
const FSR: i32 = up::FFX_VALUE;
const DLSS: i32 = up::DLSS_VALUE;

/// An RTX 50 with every DLL: everything runs.
fn all(_: i32) -> bool {
	true
}
/// An RTX 30 (or DLSS DLLs missing): FSR 2x only.
fn no_dlss_g(v: i32) -> bool {
	v <= 1
}
/// An RTX 40: FSR 2x and DLSS 2x.
fn rtx40(v: i32) -> bool {
	v <= 2
}
/// A PC without FSR frame-generation files and no NVIDIA card: Off only.
fn none(v: i32) -> bool {
	v <= 0
}

fn offers(upscaler: i32, cap: &impl Fn(i32) -> bool) -> Vec<i32> {
	(0..5).filter(|&v| offered_with(v, upscaler, cap)).collect()
}

#[test]
fn rule_a_offers_the_running_upscalers_maker_only() {
	assert_eq!(offers(OFF, &all), [0], "Upscaler Off: Off only (the row is dimmed)");
	assert_eq!(offers(FSR, &all), [0, 1], "FSR: Off, FSR 2x");
	assert_eq!(offers(DLSS, &all), [0, 2, 3, 4], "DLSS: Off, DLSS 2x/3x/4x");
	assert_eq!(offers(DLSS, &rtx40), [0, 2], "as the card allows");
	assert_eq!(offers(DLSS, &no_dlss_g), [0, 1], "a card without DLSS frame generation: FSR 2x with DLSS");
	assert_eq!(offers(FSR, &no_dlss_g), [0, 1]);
	assert_eq!(offers(FSR, &none), [0]);
	assert_eq!(offers(DLSS, &none), [0]);
	assert_eq!((maker_of(0), maker_of(1), maker_of(2), maker_of(4)), (None, Some(Maker::Fsr), Some(Maker::Dlss), Some(Maker::Dlss)));
}

#[test]
fn rule_b_in_force_follows_the_upscaler_without_touching_the_choice() {
	let d = Some(Maker::Dlss);
	assert_eq!(in_force(4, DLSS, &all, d), 4, "DLSS 4x with DLSS in a DLSS run");
	assert_eq!(in_force(4, FSR, &all, d), 0, "Upscaler FSR: paused");
	assert_eq!(why(4, FSR, &all, d), Some(Why::OtherUpscaler));
	assert_eq!(in_force(4, OFF, &all, d), 0, "Upscaler Off: paused");
	assert_eq!(why(4, OFF, &all, d), Some(Why::UpscalerOff));
	assert_eq!(in_force(4, DLSS, &all, d), 4, "back to DLSS: 4x resumes (the stored choice never changed)");
	assert_eq!(in_force(4, DLSS, &rtx40, d), 2, "a 2x card runs 2x");
	assert_eq!(why(0, DLSS, &all, d), Some(Why::ChosenOff));
	assert_eq!(why(4, DLSS, &all, d), None);
	assert_eq!(shown_with(4, FSR, &all), 0, "the row shows Off under the other Upscaler");
	assert_eq!(shown_with(1, DLSS, &no_dlss_g), 1, "FSR 2x with DLSS on an RTX 30");
	assert_eq!(in_force(1, DLSS, &no_dlss_g, Some(Maker::Fsr)), 1);
	assert_eq!(in_force(1, FSR, &all, Some(Maker::Fsr)), 1);
	assert_eq!(in_force(1, FSR, &all, Some(Maker::Fsr)), 1);
}

#[test]
fn rule_c_live_within_the_running_maker_restart_for_another() {
	let (d, f) = (Some(Maker::Dlss), Some(Maker::Fsr));
	for v in 2..=4 {
		assert!(!needs_restart(v, d), "DLSS {v} live in a DLSS run");
		assert!(needs_restart(v, f) && needs_restart(v, None));
	}
	assert!(!needs_restart(0, None) && !needs_restart(0, d), "Off is always live");
	assert!(!needs_restart(1, f) && needs_restart(1, d) && needs_restart(1, None));
	assert_eq!(in_force(2, DLSS, &all, None), 0, "a run started Off: nothing changes until a restart");
	assert_eq!(why(2, DLSS, &all, None), Some(Why::Restart));
	assert_eq!(in_force(2, DLSS, &all, f), 0);
}

#[test]
fn the_note_says_live_or_restart_or_why_not() {
	let (d, f) = (Some(Maker::Dlss), Some(Maker::Fsr));
	assert_eq!(note(4, OFF, &all, d, false, false), Note::NeedsUpscaler);
	assert_eq!(note(4, DLSS, &all, d, false, false), Note::AtOnce, "DLSS run, DLSS values: at once");
	assert_eq!(note(0, DLSS, &all, d, false, false), Note::AtOnce);
	assert_eq!(note(0, DLSS, &all, None, false, false), Note::Restart, "a run started Off: every value needs a restart");
	assert_eq!(note(2, DLSS, &all, None, false, false), Note::Restart);
	assert_eq!(note(4, FSR, &all, d, false, false), Note::Restart, "FSR 2x in a DLSS run");
	assert_eq!(note(0, FSR, &all, f, false, false), Note::AtOnce, "FSR run: Off <-> FSR 2x at once");
	assert_eq!(note(1, FSR, &all, f, false, false), Note::AtOnce);
	assert_eq!(note(0, DLSS, &none, None, true, false), Note::NoCard, "nothing but Off on a card without DLSS-G");
	assert_eq!(note(0, DLSS, &no_dlss_g, f, true, false), Note::AtOnce, "RTX 30 with FSR 2x on offer: no 'not supported'");
	assert_eq!(note(0, FSR, &none, None, false, true), Note::Missing);
	// Review 7: Upscaler FSR on an RTX 20/30 with the FSR files missing: the missing files, not "not supported".
	assert_eq!(note(0, FSR, &none, None, true, true), Note::Missing);
	assert_eq!(note(0, DLSS, &none, None, true, true), Note::NoCard, "Upscaler DLSS: the card");
}

/// Review 2: the Frame Generation row's fix button sets the Upscaler of the chosen / started maker.
#[test]
fn the_fix_button_picks_the_frame_generations_maker() {
	let (d, f) = (Some(Maker::Dlss), Some(Maker::Fsr));
	for v in 2..=4 {
		assert_eq!(upscaler_for(v, f, &|_| true), DLSS, "chosen DLSS {v}x: DLSS (whatever the run started)");
	}
	assert_eq!(upscaler_for(1, d, &|_| true), FSR, "chosen FSR 2x: FSR");
	assert_eq!(upscaler_for(0, d, &|_| true), DLSS, "nothing chosen: the started maker");
	assert_eq!(upscaler_for(0, f, &|_| true), FSR);
	assert_eq!(upscaler_for(0, None, &|_| true), FSR, "nothing chosen or started: the first upscaler this PC runs");
	assert_eq!(upscaler_for(0, None, &|v| v != FSR), DLSS, "no FidelityFX here: DLSS");
}

/// Review 8: follow reads the upscaler the slot really runs: RAC_UPSCALE over the option, a refused slot = Off.
#[test]
fn follow_uses_the_upscaler_in_force() {
	use crate::viewer::upscale::slot;
	assert_eq!(up::value_of_setting(slot::parse(Some("dlss"))) as i32, DLSS, "RAC_UPSCALE=dlss with the option Off");
	assert_eq!(up::value_of_setting(slot::parse(Some("fsr:performance"))) as i32, FSR);
	assert_eq!(up::value_of_setting(slot::parse(Some("off"))) as i32, OFF);
	assert_eq!(up::value_of_setting(slot::active_of(slot::parse(Some("dlss")), true)) as i32, OFF, "pipe_guard refused the slot: Off");
	// The upscaler in force decides rule A: DLSS 4x runs under RAC_UPSCALE=dlss, pauses under a refused slot.
	let d = Some(Maker::Dlss);
	assert_eq!(in_force(4, up::value_of_setting(slot::parse(Some("dlss"))) as i32, &all, d), 4);
	assert_eq!(why(4, up::value_of_setting(None) as i32, &all, d), Some(Why::UpscalerOff));
}

#[test]
fn the_test_steps_parse() {
	assert_eq!(parse_steps("300:fg=2; 600:up=1;x;9:zz=1;700:fg=0"), [(300, false, 2), (600, true, 1), (700, false, 0)]);
	assert!(parse_steps("").is_empty());
}
