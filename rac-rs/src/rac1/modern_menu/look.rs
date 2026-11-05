//! What the modern pages' hint bars show: the glyph style of the Options screen's Button glyphs row (PlayStation symbols, letters, the
//! Switch layout) and the device a page starts with. The four hosts (`pause_host`, `title_host`, `ship_host`, `vendor_host`) used to
//! start every page with the pad and the PlayStation symbols, so a keyboard player saw pad buttons until the first key press. A page
//! now starts with the last REAL input device the window saw (`retail::prompts`: keyboard and mouse, or the pad) and follows the
//! Button glyphs row at once. Rebinds (`binds::pad_map`) are not mapped here: a rebound pad button still shows its default symbol.
use rac_engine::ui::input::{Device, GlyphSet};

/// Test seam: 0 = the real last device, 1 = the keyboard, 2 = the pad.
#[cfg(test)]
static TEST_START: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Forces the device a page starts with (None = the real one). Tests only.
#[cfg(test)]
pub fn set_start_for_test(keyboard: Option<bool>) {
	let v = match keyboard {
		None => 0,
		Some(true) => 1,
		Some(false) => 2,
	};
	TEST_START.store(v, std::sync::atomic::Ordering::Relaxed);
}

/// True when the player's last real input was the keyboard or the mouse: the device a page opens with.
pub fn start_keyboard() -> bool {
	#[cfg(test)]
	match TEST_START.load(std::sync::atomic::Ordering::Relaxed) {
		1 => return true,
		2 => return false,
		_ => {}
	}
	crate::retail::prompts::host_is_keyboard()
}

/// A key press or a mouse click / wheel turn reached a modern page (scripted taps bypass the window events): the keyboard side is
/// the device now. The four hosts share this; the device is read live (`start_keyboard`), never kept per page.
pub fn note_keyboard() {
	crate::retail::prompts::note_keyboard();
}

/// A new pad button press reached a modern page: the pad is the device now.
pub fn note_pad() {
	crate::retail::prompts::note_pad();
}

/// The hint bar's device for a host whose own last device is `keyboard`.
pub fn device(keyboard: bool) -> Device {
	if keyboard { Device::Keyboard } else { Device::Pad }
}

/// The glyph set the Button glyphs row has now (0 PlayStation, 1 Letters, 2 Letters (Switch layout)).
pub fn glyph_set() -> GlyphSet {
	GlyphSet::from_index(crate::retail::glyphs::byte())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::modern_menu::{pause_host, ship_host, title_host, vendor_host};
	use crate::retail::menu_model::cmd::TEST_LOCK;
	use crate::retail::pcopts::PcOptions;
	use ee_oracle::mem::Mem;

	/// Every page starts with the device the player used last: the keyboard (it used to be the pad), or the pad.
	#[test]
	fn a_page_opens_with_the_last_real_input_device() {
		let _w = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
		for keyboard in [true, false] {
			set_start_for_test(Some(keyboard));
			let want = device(keyboard);
			pause_host::set_up(true);
			assert_eq!(pause_host::device(), want, "pause, keyboard {keyboard}");
			pause_host::set_up(false);
			title_host::set_up(true);
			assert_eq!(title_host::device(), want, "title, keyboard {keyboard}");
			title_host::set_up(false);
			ship_host::set_up(true);
			assert_eq!(ship_host::device(), want, "map, keyboard {keyboard}");
			ship_host::set_up(false);
			vendor_host::set_up(true);
			assert_eq!(vendor_host::device(), want, "shop, keyboard {keyboard}");
			vendor_host::set_up(false);
		}
		set_start_for_test(None);
	}

	/// The device is read live on all four hosts: a key or a mouse click / wheel turn is the keyboard side, a new pad press the pad;
	/// a plain mouse move leaves it to the window-level filter (`prompts::MoveSum`).
	#[test]
	fn the_device_follows_key_mouse_and_pad_live_on_all_four_hosts() {
		use rac_engine::ui::input::{MouseButton, MouseEvent, PadSample};
		let _w = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
		set_start_for_test(None);
		let press = Some(PadSample { buttons: 0x4000, lx: 0x80, ly: 0x80 });
		let (mv, wheel) = (MouseEvent::Move { x: 3.0, y: 3.0 }, MouseEvent::Wheel { lines: 1.0 });
		let click = MouseEvent::Down { button: MouseButton::Left, x: 3.0, y: 3.0 };
		pause_host::set_up(true);
		title_host::set_up(true);
		ship_host::set_up(true);
		vendor_host::set_up(true);
		let hosts: [(&str, fn() -> Device, fn(MouseEvent) -> bool, fn(Option<PadSample>)); 4] = [
			("pause", pause_host::device, pause_host::mouse, pause_host::begin),
			("title", title_host::device, title_host::mouse, title_host::begin),
			("map", ship_host::device, ship_host::mouse, ship_host::begin),
			("shop", vendor_host::device, vendor_host::mouse, vendor_host::begin),
		];
		for (name, device, mouse, begin) in hosts {
			note_pad();
			assert_eq!(device(), Device::Pad, "{name}: the pad");
			mouse(mv);
			assert_eq!(device(), Device::Pad, "{name}: a plain move does not switch");
			mouse(click);
			assert_eq!(device(), Device::Keyboard, "{name}: a click is the keyboard side");
			note_pad();
			mouse(wheel);
			assert_eq!(device(), Device::Keyboard, "{name}: a wheel turn is the keyboard side");
			begin(None); // the first frame primes the pause and title pages: the press that brought them up must not count
			begin(press);
			assert_eq!(device(), Device::Pad, "{name}: a new pad press");
		}
		pause_host::set_up(false);
		title_host::set_up(false);
		ship_host::set_up(false);
		vendor_host::set_up(false);
	}

	/// The very first frame of a page is built before its host is set up: with no state yet the device is the start device too.
	#[test]
	fn the_first_frame_before_set_up_uses_the_start_device() {
		let _w = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
		set_start_for_test(Some(true));
		pause_host::set_up(false);
		title_host::set_up(false);
		ship_host::set_up(false);
		vendor_host::set_up(false);
		assert_eq!(
			(pause_host::device(), title_host::device(), ship_host::device(), vendor_host::device()),
			(Device::Keyboard, Device::Keyboard, Device::Keyboard, Device::Keyboard)
		);
		set_start_for_test(Some(false));
		assert_eq!(pause_host::device(), Device::Pad);
		set_start_for_test(None);
	}

	/// The pages' glyphs follow the Button glyphs row (they were the PlayStation symbols whatever the row said).
	#[test]
	fn the_glyph_set_follows_the_button_glyphs_row() {
		let _w = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
		let _g = crate::retail::pcopts::globals_guard();
		let (mut mem, mut opts) = (Mem::new(), PcOptions::default());
		for (v, want) in [(1, GlyphSet::Letters), (2, GlyphSet::Switch), (0, GlyphSet::PlayStation)] {
			crate::retail::glyphs::set_from_ui(&mut mem, &mut opts, v);
			assert_eq!(glyph_set(), want, "row value {v}");
		}
		crate::retail::glyphs::init(&PcOptions::parse(""));
	}
}
