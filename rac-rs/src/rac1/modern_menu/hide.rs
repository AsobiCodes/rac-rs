//! Which retail screens the modern menus hide. A modern screen (stages B-F: pause menu, title, vendor, ship's maps, planet map) switches its
//! screen on with `set`; while it is on and the menu style is Modern, the retail picture of that screen is not drawn. Nothing
//! sets a switch yet, so every screen is shown as before. The modern Options page (`host`) keeps its own two cases: over the
//! pause menu it hides the retail overlay, over the title it hides the title menu.
use super::{host, style};
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
	Pause,
	Title,
	Vendor,
	Ship,
	/// The planet map (0xb) and its mission list (0xc), v0.5 build 12 (`map_host.rs`).
	Map,
}

impl Screen {
	fn bit(self) -> u8 {
		1 << self as u8
	}
}

/// One bit per screen whose modern page is up (MAIN sets them from the frame that draws the page, MAIN reads them).
static ON: AtomicU8 = AtomicU8::new(0);

/// A modern screen came up (`on`) or went away.
pub fn set(screen: Screen, on: bool) {
	if on {
		ON.fetch_or(screen.bit(), Ordering::Relaxed);
	} else {
		ON.fetch_and(!screen.bit(), Ordering::Relaxed);
	}
}

/// The field each screen's retail menu was last seen closing (u32::MAX = never).
static CLOSED_AT: [AtomicU32; 5] = [const { AtomicU32::new(u32::MAX) }; 5];

/// Keep the hide of `screen` while its retail menu closes (`closing`, snapshot field `field`) and for the one field after: the
/// overlay of that field still holds the retail page [C: pause open/close capture 2026-11-02, mode 3 -> 0 at field 112 with the
/// classic root in both display frames of field 112; I: the overlay is drawn one field after the mode word it goes with].
pub fn closing_hold(screen: Screen, closing: bool, field: u32) -> bool {
	let at = &CLOSED_AT[screen as usize];
	if closing {
		at.store(field, Ordering::Relaxed);
	}
	holds(at.load(Ordering::Relaxed), field)
}

/// Pure core of `closing_hold`: the closing field and the one after it.
fn holds(closed_at: u32, field: u32) -> bool {
	closed_at != u32::MAX && field.checked_sub(closed_at).is_some_and(|d| d <= 1)
}

/// Pure core. `mask` = the screens switched on, `modern` = the menu style, `page_up` = the Options page is up (None = closed,
/// Some(true) = over the title, Some(false) = over the pause menu).
pub fn hidden_in(screen: Screen, mask: u8, modern: bool, page_up: Option<bool>) -> bool {
	let options = match (screen, page_up) {
		(Screen::Title, Some(front)) => front,
		(_, Some(front)) => !front,
		(_, None) => false,
	};
	options || (mask & screen.bit() != 0 && modern)
}

fn page_up() -> Option<bool> {
	if host::hides_retail() {
		Some(false)
	} else if host::hides_title_menu() {
		Some(true)
	} else {
		None
	}
}

/// Is the retail picture of `screen` left out this frame?
pub fn retail_hidden(screen: Screen) -> bool {
	let mask = ON.load(Ordering::Relaxed);
	hidden_in(screen, mask, mask & screen.bit() != 0 && style::modern(), page_up())
}

/// What the hud pass drops: the retail overlay planes (`overlay`) and also the item, popup and portrait panel quads (`targets`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HideSet {
	pub overlay: bool,
	pub targets: bool,
}

impl HideSet {
	/// Pure core of `now`. Every modern screen drops both: the vendor's page shows no item preview (user 2026-11-02: no sprite).
	pub fn from(mask: u8, modern: bool, page_up: Option<bool>) -> HideSet {
		let h = |s| hidden_in(s, mask, modern, page_up);
		let any = h(Screen::Pause) || h(Screen::Ship) || h(Screen::Vendor) || h(Screen::Map);
		HideSet { overlay: any, targets: any }
	}

	pub fn now() -> HideSet {
		let mask = ON.load(Ordering::Relaxed);
		HideSet::from(mask, mask != 0 && style::modern(), page_up())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const ALL: [Screen; 5] = [Screen::Pause, Screen::Title, Screen::Vendor, Screen::Ship, Screen::Map];

	/// Only the pure core is tested: the statics are shared with host tests that toggle `ui::set_open`.
	#[test]
	fn hide_default_shows_every_screen() {
		for s in ALL {
			for modern in [false, true] {
				assert!(!hidden_in(s, 0, modern, None), "{s:?} modern={modern}");
			}
		}
		assert_eq!(HideSet::from(0, true, None), HideSet { overlay: false, targets: false });
		assert_eq!(ON.load(Ordering::Relaxed) & !0b1_1111, 0);
	}

	#[test]
	fn hide_switch_needs_the_modern_style() {
		for s in ALL {
			assert!(!hidden_in(s, s.bit(), false, None), "{s:?} Classic");
			assert!(hidden_in(s, s.bit(), true, None), "{s:?} Modern");
			for other in ALL.into_iter().filter(|o| *o != s) {
				assert!(!hidden_in(other, s.bit(), true, None), "{other:?} with only {s:?} on");
			}
		}
	}

	#[test]
	fn hide_the_options_page_keeps_its_two_cases() {
		for s in ALL {
			assert_eq!(hidden_in(s, 0, true, Some(true)), s == Screen::Title, "{s:?} over the title");
			assert_eq!(hidden_in(s, 0, true, Some(false)), s != Screen::Title, "{s:?} over the pause menu");
		}
		// Over the pause menu the page drops everything the old rule dropped: the overlay and the three panel quads.
		assert_eq!(HideSet::from(0, true, Some(false)), HideSet { overlay: true, targets: true });
		assert_eq!(HideSet::from(0, true, Some(true)), HideSet { overlay: false, targets: false });
	}

	#[test]
	fn hide_holds_the_closing_field_and_the_one_after() {
		assert!(holds(112, 112) && holds(112, 113));
		assert!(!holds(112, 114) && !holds(112, 111) && !holds(u32::MAX, 5), "later, earlier (a new level's count) or never");
	}

	#[test]
	fn hide_every_modern_screen_drops_the_overlay_and_the_panel_quads() {
		assert_eq!(HideSet::from(Screen::Vendor.bit(), true, None), HideSet { overlay: true, targets: true });
		assert_eq!(HideSet::from(Screen::Pause.bit(), true, None), HideSet { overlay: true, targets: true });
		assert_eq!(HideSet::from(Screen::Ship.bit(), true, None), HideSet { overlay: true, targets: true });
		assert_eq!(HideSet::from(Screen::Map.bit(), true, None), HideSet { overlay: true, targets: true });
		assert_eq!(HideSet::from(Screen::Title.bit(), true, None), HideSet { overlay: false, targets: false });
		assert_eq!(HideSet::from(Screen::Pause.bit(), false, None), HideSet { overlay: false, targets: false });
	}
}
