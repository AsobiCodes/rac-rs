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

#[cfg(test)]
mod display_tests {
	use super::display_change;
	use winit::window::Fullscreen;

	#[test]
	fn display_mode_switches_only_on_change() {
		assert_eq!(display_change(false, false), None);
		assert_eq!(display_change(true, true), None);
		assert_eq!(display_change(true, false), Some(Some(Fullscreen::Borderless(None))));
		assert_eq!(display_change(false, true), Some(None));
	}
}
