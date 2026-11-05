//! Tests of the pure parts of the FidelityFX frame dispatch (ffx_frame.rs).

use super::*;

#[test]
fn version_names_parse() {
	assert_eq!(parse_version("FSR 3.1.5"), Some([3, 1, 5]));
	assert_eq!(parse_version("4.1.2"), Some([4, 1, 2]));
	assert_eq!(parse_version("FSR 4.1 (driver)"), Some([4, 1, 0]));
	assert_eq!(parse_version("upscaler v3.1.4-rc"), Some([3, 1, 4]));
	assert_eq!(parse_version("FSR3"), None);
	assert_eq!(parse_version(""), None);
	assert_eq!(parse_version("..."), None);
}

#[test]
fn the_highest_version_is_pinned() {
	let v = |l: &[(u64, &str)]| l.iter().map(|&(i, n)| (i, n.to_string())).collect::<Vec<_>>();
	assert_eq!(pick_version(&v(&[(7, "FSR 3.1.5"), (9, "FSR 4.1.2"), (8, "FSR 3.1.4")])), Some((9, "FSR 4.1.2")));
	assert_eq!(pick_version(&v(&[(7, "FSR 3.1.5"), (8, "FSR 3.1.4")])), Some((7, "FSR 3.1.5")));
	assert_eq!(pick_version(&v(&[(1, "FSR 3.1.10"), (2, "FSR 3.1.9")])), Some((1, "FSR 3.1.10")));
	// The first of two equal versions (the loader's order); names that do not parse rank below any that do.
	assert_eq!(pick_version(&v(&[(3, "x"), (4, "FSR 3.1.0"), (5, "FSR 3.1.0")])), Some((4, "FSR 3.1.0")));
	assert_eq!(pick_version(&v(&[(3, "custom"), (4, "other")])), Some((3, "custom")));
	assert_eq!(pick_version(&[]), None);
}

#[test]
fn menu_labels() {
	assert_eq!(short_label("FSR 3.1.5"), "FSR 3.1");
	assert_eq!(short_label("FSR 4.1.2"), "FSR 4.1");
	assert_eq!(short_label("custom"), "FSR");
}

#[test]
fn sharpness_from_rcas_stops() {
	assert_eq!(sharpness_of_stops(0.0), 1.0);
	assert!((sharpness_of_stops(super::super::slot::SHARP) - 0.9).abs() < 1e-6);
	assert_eq!(sharpness_of_stops(2.0), 0.0);
	assert_eq!(sharpness_of_stops(5.0), 0.0);
	assert_eq!(sharpness_of_stops(-1.0), 1.0);
}

#[test]
fn history_resets() {
	assert!(!needs_reset(false, false, Some(41), 42));
	assert!(needs_reset(true, false, Some(41), 42), "a cut");
	assert!(needs_reset(false, true, Some(41), 42), "a new context");
	assert!(needs_reset(false, false, None, 42), "the first frame");
	assert!(needs_reset(false, false, Some(40), 42), "a frame without FFX in between");
	assert!(!needs_reset(false, false, Some(u32::MAX), 0), "the frame counter wraps");
}

#[test]
fn frame_times() {
	assert_eq!(frame_ms(Some((0.02, 1)), Some(3.0)), 20.0);
	assert_eq!(frame_ms(Some((0.02, 2)), None), 10.0);
	assert_eq!(frame_ms(Some((0.02, 0)), None), 20.0);
	assert_eq!(frame_ms(None, Some(8.0)), 8.0);
	assert_eq!(frame_ms(None, Some(0.01)), 1.0);
	assert_eq!(frame_ms(None, Some(5000.0)), 100.0);
	assert!((frame_ms(None, None) - 16.666_666).abs() < 1e-3);
}
