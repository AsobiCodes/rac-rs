//! The RivaTuner box's "Yes" (rtss_crash.rs): `renderer=dx12` into the options file through the writer of the game's own option
//! saves. The game keeps the whole file in memory (`ST`) and rewrites all of it on every change, under that lock: a separate
//! read-modify-write of the file from another thread could lose one of the two changes, and the game's next save would write the
//! old renderer back. So the key is set in `ST.opts` and stored from there.
use super::*;
use std::sync::TryLockError;
use std::time::Duration;

/// The crash path (`wait` false) tries the lock this often, 10 ms apart (200 ms in all): another live thread may be inside a save.
/// Never forever: the crashed thread may hold the lock itself.
const TRIES: u32 = 20;

/// Sets `renderer=dx12` in the game's options and writes the file. `wait`: wait for the lock (the watchdog's answer thread);
/// false: the lock is tried for 200 ms at most (the crash filter runs on the crashed thread, which may hold it itself). None: the
/// options are not loaded (`init` did not run, or `RAC_PC_EXT=0`) or the lock stayed busy; the caller writes the file directly then
/// (through its own temp name, so it cannot collide with a save in progress).
pub fn set_dx12(wait: bool) -> Option<std::io::Result<()>> {
	let mut s = None;
	for n in 0..TRIES {
		match ST.try_lock() {
			Ok(g) => s = Some(g),
			Err(TryLockError::Poisoned(p)) => s = Some(p.into_inner()),
			Err(TryLockError::WouldBlock) if wait => s = Some(ST.lock().unwrap_or_else(|e| e.into_inner())),
			Err(TryLockError::WouldBlock) if n + 1 < TRIES => std::thread::sleep(Duration::from_millis(10)),
			Err(TryLockError::WouldBlock) => {}
		}
		if s.is_some() {
			break;
		}
	}
	let mut s = s?;
	let path = s.path.clone()?;
	s.opts.renderer = Some(true);
	Some(s.opts.store(&path))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn yes_goes_through_the_games_own_options() {
		let _turn = super::super::tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
		let d = std::env::temp_dir().join(format!("pcext_renderer_switch_{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&d);
		std::fs::create_dir_all(&d).unwrap();
		let f = d.join("pc_options.txt");
		let mut guard = ST.lock().unwrap_or_else(|e| e.into_inner());
		let before = std::mem::replace(&mut *guard, St { opts: PcOptions::default(), path: Some(f.clone()), last: None, glast: None, lang_seen: None, warned: false });
		// The game changed an option (in memory and on disk) before the answer.
		guard.opts.camera = Some(false);
		guard.save();
		// The lock is held (here by this thread, as in a crash inside the options code): the crash path does not wait for it.
		assert!(set_dx12(false).is_none());
		drop(guard);
		assert!(set_dx12(false).expect("lock free").is_ok());
		let t = std::fs::read_to_string(&f).unwrap();
		assert!(t.contains("renderer=dx12") && t.contains("camera=classic"), "{t}");
		// The game's next save keeps the key (the old file-only writer lost it here).
		let mut guard = ST.lock().unwrap_or_else(|e| e.into_inner());
		guard.opts.strafe = Some(false);
		guard.save();
		let t = std::fs::read_to_string(&f).unwrap();
		assert!(t.contains("renderer=dx12") && t.contains("strafe=off"), "{t}");
		// Not loaded (no path): the caller falls back to the file.
		guard.path = None;
		drop(guard);
		assert!(set_dx12(true).is_none());
		*ST.lock().unwrap_or_else(|e| e.into_inner()) = before;
		let _ = std::fs::remove_dir_all(&d);
	}
}
