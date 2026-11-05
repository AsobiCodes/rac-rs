//! Test-window and display-mode helpers of the viewer window, with their test.

use super::*;

/// Agent test runs (RAC_TEST_WINDOW=hidden, or the smoke check's RAC_RETAIL_SMOKE): the window lives off-screen.
pub(super) fn test_window() -> bool {
	std::env::var_os("RAC_TEST_WINDOW").map_or(false, |v| v == "hidden") || std::env::var_os("RAC_RETAIL_SMOKE").is_some()
}

/// Graphics -> Display mode: the `set_fullscreen` argument when the wanted mode (`want`, true = Fullscreen) differs from the
/// window's (`is`): borderless fullscreen on the window's current monitor (no video mode change, alt-tab stays instant), or
/// None = back to the window.
pub(super) fn display_change(want: bool, is: bool) -> Option<Option<winit::window::Fullscreen>> {
	(want != is).then(|| want.then(|| winit::window::Fullscreen::Borderless(None)))
}

/// `WIDTHxHEIGHT` in pixels (RAC_TEST_SIZE, RAC_TEST_FULLSCREEN).
pub(super) fn wxh(v: &str) -> Option<(u32, u32)> {
	let (a, b) = v.split_once('x')?;
	Some((a.trim().parse::<u32>().ok().filter(|&w| w > 0)?, b.trim().parse::<u32>().ok().filter(|&h| h > 0)?))
}

/// [PC] Test switch RAC_TEST_FULLSCREEN=WxH (ENV.md): the off-screen test window cannot go fullscreen, so a Display mode switch
/// resizes it instead: Fullscreen to WxH, Windowed back to RAC_TEST_SIZE (default 1280x720). The proof of the resize path of a
/// Display mode change (player crash 2026-11-03). None: the switch is off (or not a test window).
pub(super) fn test_fullscreen_size(want: bool) -> Option<(u32, u32)> {
	test_fullscreen_of(want, std::env::var("RAC_TEST_FULLSCREEN").ok().as_deref(), std::env::var("RAC_TEST_SIZE").ok().as_deref())
}

fn test_fullscreen_of(want: bool, full: Option<&str>, size: Option<&str>) -> Option<(u32, u32)> {
	let full = wxh(full?)?;
	Some(if want { full } else { size.and_then(wxh).unwrap_or((1280, 720)) })
}

#[cfg(test)]
mod display_tests {
	use super::{display_change, test_fullscreen_of, wxh};
	use winit::window::Fullscreen;

	#[test]
	fn the_test_fullscreen_switch_sizes() {
		assert_eq!(wxh(" 1920 x 1080 "), Some((1920, 1080)));
		assert_eq!(wxh("0x5"), None);
		assert_eq!(wxh("big"), None);
		assert_eq!(test_fullscreen_of(true, None, None), None, "off");
		assert_eq!(test_fullscreen_of(true, Some("x"), None), None, "a typo is off");
		assert_eq!(test_fullscreen_of(true, Some("1920x1080"), None), Some((1920, 1080)));
		assert_eq!(test_fullscreen_of(false, Some("1920x1080"), None), Some((1280, 720)));
		assert_eq!(test_fullscreen_of(false, Some("1920x1080"), Some("1600x900")), Some((1600, 900)));
	}

	#[test]
	fn display_mode_switches_only_on_change() {
		assert_eq!(display_change(false, false), None);
		assert_eq!(display_change(true, true), None);
		assert_eq!(display_change(true, false), Some(Some(Fullscreen::Borderless(None))));
		assert_eq!(display_change(false, true), Some(None));
	}
}
