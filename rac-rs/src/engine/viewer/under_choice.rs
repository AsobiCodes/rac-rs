//! [PC] The player's Underwater look row (GitHub #64; no retail counterpart): R&C1 Graphics -> World -> "Underwater look"
//! (rac1 modern_menu/rows_graphics.rs; set and published by rac1 retail/pcext/ui_bridge_more.rs). On (the default) = the look of
//! underwater.rs at the Water quality; Off = the pass sees a dry camera, so under water the frame is the retail picture (retail's
//! fog words at full strength and its tint sprite; the volumetric fog stays off under water at any setting, prepare.rs
//! `frame_uniforms` reads the raw flag) while the water surface keeps the chosen Water quality. Kept in the options file only
//! (`underwater=on|off`): no RAM byte, so the game and its RAM hash never see it. Read once per frame (gpu/draw/pass_post.rs
//! `post_frame`), so a change applies at the next frame: an Off under water fades the look out like a camera leaving the water
//! (`Under::ramp`). A game that never calls `init` or the setter keeps the default: On.

use crate::pcopts::PcOptions;
use std::sync::atomic::{AtomicBool, Ordering};

/// Options file key of the row; values `on` / `off`.
pub const KEY: &str = "underwater";

static ON: AtomicBool = AtomicBool::new(true);

/// The value of the file key: On unless it reads `off` (case and spaces ignored), so a missing or unknown value is the default.
fn on_of(v: Option<&str>) -> bool {
	!v.is_some_and(|v| v.trim().eq_ignore_ascii_case("off"))
}

/// Start-up: the row from the options file.
pub fn init(opts: &PcOptions) {
	ON.store(on_of(opts.extra_get(KEY)), Ordering::Relaxed);
	if !on() {
		eprintln!("underwater: Underwater look off (options file)");
	}
}

/// The row is On.
pub fn on() -> bool {
	ON.load(Ordering::Relaxed)
}

/// The menu set the row: in force from the next frame and remembered; true = the options file changed.
pub fn set_from_ui(opts: &mut PcOptions, on: bool) -> bool {
	if ON.swap(on, Ordering::Relaxed) == on {
		return false;
	}
	opts.extra_set(KEY, Some(if on { "on" } else { "off" }.to_string()));
	eprintln!("underwater: PC options: Underwater look {}", if on { "on" } else { "off" });
	true
}

/// The camera water flag the underwater look and the water views' sky map see (read once per frame, pass_post.rs): the frame's flag
/// with the row On, a dry camera with it Off. The volumetric fog, clouds, smoke and GI gates keep the raw flag.
pub(crate) fn look(under_flag: bool) -> bool {
	under_flag && on()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn file_values() {
		assert!(on_of(None));
		assert!(on_of(Some("on")));
		assert!(on_of(Some("maybe")), "an unknown value is the default");
		assert!(!on_of(Some("off")));
		assert!(!on_of(Some(" OFF ")));
	}

	#[test]
	fn set_saves_once_and_round_trips() {
		let _guard = crate::pcopts::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
		let mut opts = PcOptions::new();
		init(&opts);
		assert!(on() && look(true) && !look(false));
		assert!(!set_from_ui(&mut opts, true), "already On: nothing written");
		assert_eq!(opts.extra_get(KEY), None);
		assert!(set_from_ui(&mut opts, false));
		assert!(!on() && !look(true));
		assert!(!set_from_ui(&mut opts, false));
		let saved = opts.to_text();
		assert!(saved.contains("underwater=off\n"), "{saved}");
		init(&PcOptions::new());
		assert!(on());
		init(&PcOptions::parse(&saved));
		assert!(!on());
		assert!(set_from_ui(&mut opts, true));
		assert_eq!(PcOptions::parse(&opts.to_text()).extra_get(KEY), Some("on"));
		init(&PcOptions::new());
	}
}
