//! The held last picture of a retail movie (`MOVIE_HOLD`, handler.rs): when it must give way (GitHub #74).
//!
//! Retail keeps a movie's last picture up until its main loop presents again (`Retail::holds`, the no-present word). A flight to
//! another planet that follows a movie (Aridia -> Eudora, Veldin -> Novalis) runs in retail's travel loop, which presents through
//! the warp view the host draws itself, not through that word: the word stayed set for the whole flight, so the faded-to-black movie
//! picture covered the warp for ~14 s (black screen, only the flight's sound). A movie-end fade that has run out (the picture is fully
//! black) while the game is travelling to another level has nothing left to show: the hold ends and the host's own picture shows.

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

/// The movie-end fade reached black (tint alpha 0) while this hold was up.
static FADED: AtomicBool = AtomicBool::new(false);

/// A hold begins or ends: forget the fade.
pub(super) fn reset() {
	FADED.store(false, Relaxed);
}

/// This field's overlay tint of the held picture (`Retail::movie_tint`): remembers a finished fade.
pub(super) fn note_tint(t: Option<[f32; 4]>) {
	if faded(t) {
		FADED.store(true, Relaxed);
	}
}

/// A tint that shows none of the picture (the fade ran out).
pub(super) fn faded(t: Option<[f32; 4]>) -> bool {
	t.is_some_and(|t| t[3] <= 0.0)
}

/// The fade-out rule: the movie-end fade ran out and retail flies to another level (`travelling` is asked only after the fade).
pub(super) fn fade_ends(travelling: impl FnOnce() -> bool) -> bool {
	FADED.load(Relaxed) && travelling()
}

/// One game field (handler/frame_ticks.rs): true when the held picture ends now (the caller clears the presenter).
/// Nothing is read while no picture is held; then the old order, lazily (`overlay_covers` waits for the GS capture overlay):
/// retail presents again, an overlay / menu page / GS front end covers it, or the flight rule above.
pub(super) fn field(r: &mut crate::retail::Retail, hold: &AtomicBool) -> bool {
	if !hold.load(Relaxed) {
		return false;
	}
	note_tint(r.movie_tint());
	// `travel_target`: retail's level word names another level than the loaded one, set by retail's own travel code (the whole flight).
	let end = !r.holds() || r.overlay_covers() || r.menu_presents() || r.gs_front_drops_movie() || fade_ends(|| r.travel_target().is_some());
	let was = end && hold.swap(false, Relaxed);
	if was {
		reset();
	}
	was
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn rules() {
		reset();
		// Fade not run out: travelling is not even asked.
		assert!(!fade_ends(|| panic!("asked before the fade ran out")));
		note_tint(Some([0.0, 0.0, 0.0, 0.5]));
		assert!(!fade_ends(|| panic!("asked before the fade ran out")));
		// Fade ran out, not travelling: stays; travelling: ends (the warp shows).
		note_tint(Some([0.0, 0.0, 0.0, 0.0]));
		assert!(!fade_ends(|| false));
		assert!(fade_ends(|| true));
		// A new hold forgets the fade.
		reset();
		assert!(!fade_ends(|| true));
	}
}
