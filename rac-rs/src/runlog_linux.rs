//! [PC] Linux: the run log. The same job as the first half of `runlog.rs` on Windows: when the game's stderr goes nowhere (started from
//! the application menu, no terminal), it goes to `<player dir>/log.txt` instead (the run before it: `log.prev.txt`), so a freeze or a
//! panic leaves a trace. There is no watchdog thread here (that part reads Windows' window state).
//!
//! "Goes nowhere" is a character device that is not a terminal (/dev/null; Rust's start-up also turns a closed stderr into /dev/null)
//! or a socket (the journal, which a desktop launcher uses). A terminal, a pipe (`| tee`) and a file (`2>file`) keep their stderr, so a run started from a shell is unchanged.
//! RAC_RUN_LOG=1 forces the file, RAC_RUN_LOG=0 turns it off (the same switch as on Windows).

const S_IFMT: u32 = 0o170000;
const S_IFCHR: u32 = 0o020000;
const S_IFSOCK: u32 = 0o140000;

/// Whether a stderr with this file mode (`None`: fstat failed) and terminal flag goes nowhere. A descriptor 2 that was closed when
/// the process started is reopened on /dev/null by Rust's start-up code, so it shows up as a character device; `None` is left alone
/// (redirecting would hand the log file the descriptor number 2 and dropping it would close stderr).
pub fn goes_nowhere(mode: Option<u32>, is_tty: bool) -> bool {
	match mode {
		None => false,
		Some(m) => !is_tty && matches!(m & S_IFMT, S_IFCHR | S_IFSOCK),
	}
}

/// Sends stderr into `<dir>/log.txt` when it goes nowhere (or RAC_RUN_LOG=1); true when it did.
#[cfg(target_os = "linux")]
pub fn redirect(dir: &std::path::Path) -> bool {
	use std::os::fd::AsRawFd;
	let force = std::env::var("RAC_RUN_LOG").ok();
	if force.as_deref() == Some("0") {
		return false;
	}
	if force.as_deref() != Some("1") {
		// SAFETY: fstat writes one `stat` struct we own; isatty only reads the descriptor.
		let (mode, tty) = unsafe {
			let mut st: libc::stat = std::mem::zeroed();
			let ok = libc::fstat(2, &mut st) == 0;
			(if ok { Some(st.st_mode as u32) } else { None }, libc::isatty(2) == 1)
		};
		if !goes_nowhere(mode, tty) {
			return false;
		}
	}
	let _ = std::fs::create_dir_all(dir);
	let path = dir.join("log.txt");
	let _ = std::fs::rename(&path, dir.join("log.prev.txt"));
	let Ok(f) = std::fs::File::create(&path) else { return false };
	// SAFETY: dup2 copies the open log file onto descriptor 2; the File closes its own descriptor when it drops, descriptor 2 stays.
	unsafe { libc::dup2(f.as_raw_fd(), 2) >= 0 }
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Only a stderr with no place to read it counts: /dev/null, the journal's socket. A failed fstat leaves it alone.
	#[test]
	fn stderr_goes_nowhere_only_without_a_reader() {
		assert!(!goes_nowhere(None, false), "fstat failed: left alone");
		assert!(goes_nowhere(Some(S_IFCHR | 0o666), false), "/dev/null");
		assert!(goes_nowhere(Some(S_IFSOCK | 0o755), false), "the journal's socket");
		assert!(!goes_nowhere(Some(S_IFCHR | 0o620), true), "a terminal");
		assert!(!goes_nowhere(Some(0o010000 | 0o600), false), "a pipe (| tee)");
		assert!(!goes_nowhere(Some(0o100000 | 0o644), false), "a file (2>file)");
	}
}
