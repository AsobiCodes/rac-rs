//! The Skip intro row (Pause / title Options -> Graphics -> System -> Skip intro: Off / On), retail side. Off (default) plays the boot
//! logo movie exactly as retail does; On ends that movie at once (viewer/front_mirror.rs asks `skip()` for the logo movie, the same
//! path the test clock uses), so the game goes on to the legal still and the title screen. Nothing else is cut: the memory card check,
//! the language and the save loading run as always. The row's byte lives in the pcctl block (`pcctl::intro_skip`), `sync` hands a
//! change to the options file (`intro=play|skip`, pcopts.rs). The label is pcctl's `ID_INTRO`, the values are retail's Off / On.
use super::pcopts::PcOptions;
use std::sync::atomic::{AtomicBool, Ordering};

/// The row as last applied (the file's value until the player changes it).
static SKIP: AtomicBool = AtomicBool::new(false);

/// `pcext::init`: the file's choice is the row's start value.
pub fn init(opts: &PcOptions) {
	SKIP.store(opts.intro_skip(), Ordering::Relaxed);
}

/// True = the boot logo movie ends at once.
pub fn skip() -> bool {
	SKIP.load(Ordering::Relaxed)
}

/// Worker, every field after the page patch: a changed row goes into the options file (true = the caller saves).
pub fn sync(mem: &mut ee_oracle::mem::Mem, opts: &mut PcOptions) -> bool {
	let Some(on) = super::pcctl::intro_skip(mem) else { return false };
	if SKIP.swap(on, Ordering::Relaxed) == on {
		return false;
	}
	eprintln!("retail: PC options: Skip intro {}", if on { "On" } else { "Off" });
	opts.intro = Some(on);
	true
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_file_sets_the_start_value_and_a_changed_row_is_saved() {
		init(&PcOptions::parse("intro=skip\n"));
		assert!(skip());
		init(&PcOptions::parse(""));
		assert!(!skip());
		let mut mem = ee_oracle::mem::Mem::new();
		let mut o = PcOptions::default();
		assert!(!sync(&mut mem, &mut o), "page not built yet");
	}
}
