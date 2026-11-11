//! Tests of aspect_box.rs: the rectangle maths, the mouse mapping and the options file round trip.
use super::*;

fn r(x: u32, y: u32, w: u32, h: u32) -> Rect {
	Rect { x, y, w, h }
}

#[test]
fn auto_and_the_same_shape_are_the_whole_window() {
	assert_eq!(fit((1920, 1080), Ratio::Auto), Rect::full((1920, 1080)));
	assert_eq!(fit((1281, 723), Ratio::Auto), Rect::full((1281, 723)));
	assert!(fit((1920, 1080), Ratio::R16x9).is_full((1920, 1080)));
	assert!(fit((1280, 720), Ratio::R16x9).is_full((1280, 720)));
	assert!(fit((1024, 768), Ratio::R4x3).is_full((1024, 768)));
	assert!(fit((1920, 1200), Ratio::R16x10).is_full((1920, 1200)));
	assert!(fit((2560, 1080), Ratio::R21x9).is_full((2560, 1080)), "21:9 = the 2560 x 1080 panel");
	assert!(fit((3840, 1080), Ratio::R32x9).is_full((3840, 1080)));
	assert!(fit((5120, 2160), Ratio::R21x9).is_full((5120, 2160)), "exact: the 5120 x 2160 panel");
	assert!(fit((5120, 1440), Ratio::R32x9).is_full((5120, 1440)), "exact: 32:9 at 5120 x 1440");
	// Within 1 % of the shape: the whole window, no bars (the box pass and ReShade's switch-off for a few pixels of bar).
	assert!(fit((3440, 1440), Ratio::R21x9).is_full((3440, 1440)), "the 3440 x 1440 monitor sold as 21:9 (43:18, 0.8 % off 64:27)");
	assert!(fit((2560, 1080), Ratio::R21x9).is_full((2560, 1080)));
	assert!(fit((1366, 768), Ratio::R16x9).is_full((1366, 768)), "the 1366 x 768 laptop (0.05 % off 16:9)");
	assert!(fit((1360, 768), Ratio::R16x9).is_full((1360, 768)));
	assert!(fit((1920, 1081), Ratio::R16x9).is_full((1920, 1081)), "a one-pixel-off window");
}

/// The 1 % rule stops short of the next shape: these still get bars.
#[test]
fn a_shape_more_than_1_percent_off_gets_bars() {
	assert!(!near((3440, 1440), (16, 9)), "21:9 monitor at 16:9");
	assert!(!near((1920, 1200), (16, 9)), "16:10 at 16:9 (11 % off)");
	assert!(!near((1920, 1080), (16, 10)));
	assert!(!near((1280, 1024), (4, 3)), "5:4 at 4:3 (6 % off)");
	assert!(!near((2560, 1080), (32, 9)));
	assert!(near((1440, 1080), (4, 3)), "exact");
	// The edge: 1 % of ww * b is allowed, one more is not.
	assert!(near((1000, 1010), (1, 1)) && !near((1000, 1011), (1, 1)));
	assert_eq!(fit((1280, 1024), Ratio::R4x3), r(0, 32, 1280, 960));
	assert_eq!(fit((3440, 1440), Ratio::R16x9), r(440, 0, 2560, 1440));
}

#[test]
fn a_wider_window_gets_bars_left_and_right() {
	assert_eq!(fit((1280, 720), Ratio::R4x3), r(160, 0, 960, 720));
	assert_eq!(fit((1920, 1080), Ratio::R4x3), r(240, 0, 1440, 1080));
	assert_eq!(fit((2560, 1080), Ratio::R16x9), r(320, 0, 1920, 1080));
	assert_eq!(fit((1920, 1080), Ratio::R16x10), r(96, 0, 1728, 1080));
}

#[test]
fn a_narrower_window_gets_bars_top_and_bottom() {
	assert_eq!(fit((1280, 720), Ratio::R21x9), r(0, 90, 1280, 540));
	assert_eq!(fit((1920, 1080), Ratio::R21x9), r(0, 135, 1920, 810));
	assert_eq!(fit((1280, 720), Ratio::R32x9), r(0, 180, 1280, 360));
	assert_eq!(fit((1920, 1080), Ratio::R32x9), r(0, 270, 1920, 540));
	assert_eq!(fit((1024, 768), Ratio::R16x9), r(0, 96, 1024, 576));
}

/// Odd sizes: whole pixels, the long side rounded half up, the bars split with the odd pixel on the right / bottom.
#[test]
fn odd_sizes_round_to_whole_pixels() {
	// 721 * 4 / 3 = 961.33 -> 961; (1281 - 961) / 2 = 160
	assert_eq!(fit((1281, 721), Ratio::R4x3), r(160, 0, 961, 721));
	// 1366 x 768 is not quite 16:9 (768 * 16 / 9 = 1365.33) but within 1 %: the whole window, not a 1365-px box with one bar column
	assert_eq!(fit((1366, 768), Ratio::R16x9), Rect::full((1366, 768)));
	// 1001 * 27 / 64 = 422.29 -> 422; (723 - 422) / 2 = 150
	assert_eq!(fit((1001, 723), Ratio::R21x9), r(0, 150, 1001, 422));
	// 3 * 4 / 3 = 4 -> the half up rounding; 5 * 9 / 16 = 2.81 -> 3
	assert_eq!(fit((7, 3), Ratio::R4x3), r(1, 0, 4, 3));
	assert_eq!(fit((5, 5), Ratio::R16x9), r(0, 1, 5, 3));
	for (w, h) in [(1281, 721), (1001, 723), (999, 1999), (3, 1000), (1000, 3), (640, 480), (3440, 1440)] {
		for ratio in ALL {
			let b = fit((w, h), ratio);
			assert!(b.x + b.w <= w && b.y + b.h <= h && b.w >= 1 && b.h >= 1, "{w}x{h} {ratio:?}: {b:?}");
			assert!(b.w == w || b.h == h, "one side is the window's: {w}x{h} {ratio:?} {b:?}");
			assert!(b.x.abs_diff(w - b.w - b.x) <= 1 && b.y.abs_diff(h - b.h - b.y) <= 1, "centred: {w}x{h} {ratio:?} {b:?}");
		}
	}
}

#[test]
fn tiny_windows_keep_one_pixel() {
	assert_eq!(fit((1, 1), Ratio::R32x9), r(0, 0, 1, 1));
	assert_eq!(fit((1, 100), Ratio::R21x9), r(0, 49, 1, 1));
	assert_eq!(fit((100, 1), Ratio::R4x3), r(49, 0, 1, 1));
	assert_eq!(fit((0, 0), Ratio::Auto), Rect::full((0, 0)), "Auto keeps the window as it is");
	let b = fit((0, 0), Ratio::R4x3);
	assert!(b.w >= 1 && b.h >= 1, "never a zero picture: {b:?}");
}

#[test]
fn the_mouse_maps_into_the_box_and_the_bars_are_outside() {
	let b = fit((1280, 720), Ratio::R4x3); // 160, 0, 960 x 720
	assert_eq!(to_box(b, 160.0, 0.0), (0.0, 0.0));
	assert_eq!(to_box(b, 640.0, 360.0), (480.0, 360.0), "the window centre is the box centre");
	let (x, y) = to_box(b, 1119.5, 719.5);
	assert!(inside(b, x, y), "the last pixel");
	for (wx, wy) in [(159.9, 300.0), (1120.0, 300.0), (0.0, 0.0), (1279.0, 719.0)] {
		let (x, y) = to_box(b, wx, wy);
		assert!(!inside(b, x, y), "on a bar: {wx}, {wy}");
	}
	let b = fit((1280, 720), Ratio::R21x9); // 0, 90, 1280 x 540
	assert_eq!(to_box(b, 10.0, 100.0), (10.0, 10.0));
	let (x, y) = to_box(b, 640.0, 89.0);
	assert!(!inside(b, x, y), "the top bar");
	let (x, y) = to_box(b, 640.0, 630.0);
	assert!(!inside(b, x, y), "the bottom bar");
	let full = Rect::full((1280, 720));
	assert_eq!(to_box(full, 12.5, 7.0), (12.5, 7.0), "Auto: the window pixels unchanged");
}

#[test]
fn file_values_parse_and_name() {
	for v in ALL {
		assert_eq!(Ratio::parse(v.name()), v);
		assert_eq!(Ratio::from_index(v as i32), v);
	}
	assert_eq!(Ratio::parse(""), Ratio::Auto);
	assert_eq!(Ratio::parse("wide"), Ratio::Auto, "an unknown value is the default");
	assert_eq!(Ratio::parse(" 21X9 "), Ratio::R21x9);
	assert_eq!(Ratio::parse("16/10"), Ratio::R16x10);
	assert_eq!(Ratio::from_index(-1), Ratio::Auto);
	assert_eq!(Ratio::from_index(6), Ratio::Auto);
}

#[test]
fn set_saves_once_and_round_trips() {
	let _guard = crate::pcopts::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	let mut opts = PcOptions::new();
	init(&opts);
	assert_eq!(current(), Ratio::Auto, "a missing key is Auto (older options files)");
	assert!(!set_from_ui(&mut opts, 0), "already Auto: nothing written");
	assert_eq!(opts.extra_get(KEY), None);
	assert!(set_from_ui(&mut opts, Ratio::R21x9 as i32));
	assert_eq!(current(), Ratio::R21x9);
	assert!(!set_from_ui(&mut opts, Ratio::R21x9 as i32));
	let saved = opts.to_text();
	assert!(saved.contains("aspect=21:9\n"), "{saved}");
	init(&PcOptions::new());
	assert_eq!(current(), Ratio::Auto);
	init(&PcOptions::parse(&saved));
	assert_eq!(current(), Ratio::R21x9);
	init(&PcOptions::parse("aspect=nonsense\n"));
	assert_eq!(current(), Ratio::Auto);
	init(&PcOptions::parse(&saved));
	assert!(set_from_ui(&mut opts, 0), "21:9 back to Auto");
	assert_eq!(PcOptions::parse(&opts.to_text()).extra_get(KEY), Some("auto"));
	init(&PcOptions::new());
}

#[test]
fn the_resolution_picture() {
	// No chosen size: exactly `fit`.
	assert_eq!(picture((2560, 1440), Ratio::Auto, None), ((2560, 1440), Rect::full((2560, 1440))));
	assert_eq!(picture((2560, 1440), Ratio::R4x3, None), ((1920, 1440), r(320, 0, 1920, 1440)));
	// 1920 x 1080 on a 2560 x 1440 screen: drawn at 1920 x 1080, scaled over the whole screen.
	assert_eq!(picture((2560, 1440), Ratio::Auto, Some((1920, 1080))), ((1920, 1080), Rect::full((2560, 1440))));
	// 1280 x 1024 (5:4) on 16:9: bars left and right, the picture keeps its shape.
	assert_eq!(picture((2560, 1440), Ratio::Auto, Some((1280, 1024))), ((1280, 1024), r(380, 0, 1800, 1440)));
	// With an Aspect ratio too: the shape's box inside the chosen size.
	assert_eq!(picture((2560, 1440), Ratio::R4x3, Some((1920, 1080))), ((1440, 1080), r(320, 0, 1920, 1440)));
}
