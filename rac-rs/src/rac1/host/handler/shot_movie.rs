//! [PC] TEST ONLY: RAC_SHOT_MOVIE=N[:F] holds a --shot until the Nth movie the game played (1-based, counted over the whole run)
//! has presented F decoded frames (default 30), then lets the shot be taken while that movie is still on screen. Fields stand
//! still while a movie plays (the game thread is parked), so RAC_SHOT_FIELD cannot reach a picture inside a movie.
//! Run without RAC_TEST_CLOCK's front-end shortcuts (the in-level movies play in real time either way).

use std::sync::atomic::{AtomicU32, AtomicBool, Ordering::Relaxed};

static TAKEN: AtomicU32 = AtomicU32::new(0);
static FRAMES: AtomicU32 = AtomicU32::new(0);
static REACHED: AtomicBool = AtomicBool::new(false);

/// (movie number, frames) wanted, from RAC_SHOT_MOVIE.
fn want() -> Option<(u32, u32)> {
	static W: std::sync::OnceLock<Option<(u32, u32)>> = std::sync::OnceLock::new();
	*W.get_or_init(|| std::env::var("RAC_SHOT_MOVIE").ok().and_then(|v| parse(&v)))
}

pub(super) fn parse(v: &str) -> Option<(u32, u32)> {
	let mut it = v.split(':');
	let n = it.next()?.trim().parse().ok()?;
	let f = it.next().map_or(Some(30), |f| f.trim().parse().ok())?;
	Some((n, f))
}

/// A movie was taken (started) by the game.
pub(super) fn started() {
	let n = TAKEN.fetch_add(1, Relaxed) + 1;
	FRAMES.store(0, Relaxed);
	super::movie_hold::reset();
	eprintln!("movie-state: start #{n}");
}

/// The player handed over one decoded frame that was uploaded to the presenter.
pub(super) fn frame_shown() {
	let k = FRAMES.fetch_add(1, Relaxed) + 1;
	if k == 1 {
		eprintln!("movie-state: first frame of #{} shown", TAKEN.load(Relaxed));
	}
	if want().is_some_and(|(n, f)| TAKEN.load(Relaxed) == n && k >= f) {
		REACHED.store(true, Relaxed);
	}
}

/// A movie ended (finished or skipped), the picture stays held.
pub(super) fn ended(skipped: bool) {
	eprintln!("movie-state: end #{} after {} frames{}", TAKEN.load(Relaxed), FRAMES.load(Relaxed), if skipped { " (skipped)" } else { "" });
}

/// RAC_SHOT_MOVIE is set: it replaces the field / travel waits of the shot.
pub(super) fn enabled() -> bool {
	want().is_some()
}

/// The shot must keep waiting (the wanted movie's frame is not shown yet).
pub(super) fn wait() -> bool {
	!REACHED.load(Relaxed)
}

#[cfg(test)]
mod tests {
	#[test]
	fn parses() {
		assert_eq!(super::parse("2"), Some((2, 30)));
		assert_eq!(super::parse("3:12"), Some((3, 12)));
		assert_eq!(super::parse("x"), None);
	}
}
