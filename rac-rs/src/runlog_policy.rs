//! [PC] The run log's rules that need no operating-system call (GitHub issue #49): whether it is on, which files it is written to,
//! and what becomes of the text of message boxes. `runlog.rs` (the Windows log and its watchdog) and `runlog_linux.rs` use these; the
//! rules are plain functions and are tested on every system.
//!
//! - ON/OFF (Windows). `RAC_RUN_LOG=0` is off, `=1` is on, otherwise the log is on when stderr goes nowhere (the double-clicked game)
//!   and, new, always under Wine (`wine.rs`): Wine hands every process a stderr that nobody reads (Playden on the PS5 shows a loading
//!   screen and collects files). A console or a redirected stderr keeps its stderr on a real Windows, as before. Under Wine
//!   `RAC_RUN_LOG=0` is the way to keep stderr for a terminal.
//! - FILES. Always `<player dir>/log.txt` (the run before it `log.prev.txt`). Under Wine also `<exe folder>/<exe name>_Data/output_log.txt`
//!   (`output_log.prev.txt`), the file Playden copies into its own logs: it is the real file there and `log.txt` a hard link to it (one
//!   file under two names, both rotated at each start). A file system without hard links gets the one file only, with a line in the
//!   log that says so. If the first file cannot be created, the next one is the real file.
//! - MESSAGE BOXES. Their text is also a log line (`message box: ...`) on every system, because a box can be invisible (Playden) or
//!   already closed when a report is written. A box shown before the log starts (the start-up errors: no disc image, no `play.cfg`) is
//!   kept in memory and written right after the log's first lines. `RAC_RUN_LOG=0` switches all of it off: nothing is kept, nothing
//!   written. A start that decides to have no log (a console) drops what was kept.
#[cfg(any(windows, test))]
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// What `RAC_RUN_LOG` says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
	/// Not set (or any other value): the rules decide.
	Auto,
	/// `=1`: on.
	On,
	/// `=0`: off, for the log and for everything that feeds it.
	Off,
}

/// The `RAC_RUN_LOG` value `v` as a setting.
pub fn setting(v: Option<&str>) -> Setting {
	match v {
		Some("0") => Setting::Off,
		Some("1") => Setting::On,
		_ => Setting::Auto,
	}
}

/// `setting` of the process's own environment.
fn env_setting() -> Setting {
	setting(std::env::var("RAC_RUN_LOG").ok().as_deref())
}

/// Whether the Windows run log is on: `has_stderr` = the process has a standard error handle, `wine` = running under Wine.
#[cfg(any(windows, test))]
pub fn wanted(setting: Setting, has_stderr: bool, wine: bool) -> bool {
	match setting {
		Setting::Off => false,
		Setting::On => true,
		Setting::Auto => wine || !has_stderr,
	}
}

/// One log file name and the name its previous run is moved to.
#[cfg(any(windows, test))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
	pub path: PathBuf,
	pub prev: PathBuf,
}

/// The files of the run log, in the order they are tried: `<dir>/log.txt`; under Wine (`wine_data` = the `<exe name>_Data` folder)
/// first `output_log.txt` in that folder and then `<dir>/log.txt`.
#[cfg(any(windows, test))]
pub fn slots(dir: &Path, wine_data: Option<PathBuf>) -> Vec<Slot> {
	let slot = |d: &Path, name: &str, prev: &str| Slot { path: d.join(name), prev: d.join(prev) };
	let normal = slot(dir, "log.txt", "log.prev.txt");
	match wine_data {
		Some(d) => vec![slot(&d, "output_log.txt", "output_log.prev.txt"), normal],
		None => vec![normal],
	}
}

/// A log file made by `open`.
#[cfg(any(windows, test))]
#[derive(Debug)]
pub struct Opened {
	/// The real file (stderr is pointed at it).
	pub file: std::fs::File,
	pub path: PathBuf,
	/// Slots that could not be made (for the log's first lines).
	pub notes: Vec<String>,
	/// Every name the run log has now (the real file's first).
	pub names: Vec<PathBuf>,
}

/// Moves the previous runs aside and makes the log: the first slot that can be created is the real file, each later slot becomes a
/// hard link to it. None when no slot could be created.
#[cfg(any(windows, test))]
pub fn open(slots: &[Slot]) -> Option<Opened> {
	let mut real: Option<(std::fs::File, PathBuf)> = None;
	let (mut notes, mut names) = (Vec::new(), Vec::new());
	for s in slots {
		if let Some(parent) = s.path.parent() {
			let _ = std::fs::create_dir_all(parent);
		}
		let _ = std::fs::rename(&s.path, &s.prev);
		match &real {
			None => match std::fs::File::create(&s.path) {
				Ok(f) => {
					names.push(s.path.clone());
					real = Some((f, s.path.clone()));
				}
				Err(e) => notes.push(format!("cannot create {} ({e})", s.path.display())),
			},
			Some((_, first)) => match std::fs::hard_link(first, &s.path) {
				Ok(()) => names.push(s.path.clone()),
				Err(e) => {
					// The name must not show the run before (the rename above failed, so the old file is still there) or no run at all.
					let _ = std::fs::write(&s.path, pointer_text(first));
					notes.push(format!("cannot link {} to the log ({e}); the log is only in {}, {} says so", s.path.display(), first.display(), s.path.display()));
				}
			},
		}
	}
	real.map(|(file, path)| Opened { file, path, notes, names })
}

/// What a log name that could not be linked to the real file contains: where this run's log is.
#[cfg(any(windows, test))]
fn pointer_text(real: &Path) -> String {
	format!("The log of this run is in {}\n(this file system has no hard links: this name is not the log itself)\n", real.display())
}

/// Message-box texts kept before the log starts, at most this many (each cut to `MAX_TEXT` characters when it is kept): start-up errors come one at a time.
const MAX_PENDING: usize = 16;
const MAX_TEXT: usize = 8000;

/// Where the run log is in its start: the message boxes are kept until it knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
	/// `runlog::start` has not run yet.
	Waiting,
	/// The log is on: a box's text is written at once.
	Running,
	/// The start found no log to write: a box's text goes nowhere (a terminal shows stderr anyway).
	Declined,
}

/// The texts of message boxes and where the run log is. One process-wide value: `NOTES`.
struct Notes {
	stage: Stage,
	pending: Vec<String>,
}

impl Notes {
	const fn new() -> Notes {
		Notes { stage: Stage::Waiting, pending: Vec::new() }
	}

	/// A message box with `text` opens. `off` = RAC_RUN_LOG=0. The line to write now, if there is one; before the start the text is kept.
	fn note(&mut self, off: bool, text: &str) -> Option<String> {
		if off {
			return None;
		}
		match self.stage {
			Stage::Running => Some(box_line("message box", text)),
			Stage::Waiting => {
				if self.pending.len() < MAX_PENDING {
					self.pending.push(text.chars().take(MAX_TEXT).collect());
				}
				None
			}
			Stage::Declined => None,
		}
	}

	/// `runlog::start` is done: `started` = the log is on. The lines for what was kept (empty when the log is not on).
	fn finish(&mut self, started: bool) -> Vec<String> {
		self.stage = if started { Stage::Running } else { Stage::Declined };
		let kept = std::mem::take(&mut self.pending);
		if started { kept.iter().map(|t| box_line("message box (shown before the run log started)", t)).collect() } else { Vec::new() }
	}
}

static NOTES: Mutex<Notes> = Mutex::new(Notes::new());

/// `label: text`, the text cut to `MAX_TEXT` characters and its later lines indented, so a grep for `message box` finds the start.
fn box_line(label: &str, text: &str) -> String {
	let text: String = text.chars().take(MAX_TEXT).collect();
	format!("{label}: {}", text.trim_end().replace('\n', "\n  "))
}

/// A message box is about to open with `text` (every box: `main.rs` `message_box`, `gscap_guard::message_box`, Linux's
/// `dialog_linux::show`). Called before the box blocks. See the module doc.
pub fn note_message(text: &str) {
	let line = NOTES.lock().unwrap_or_else(|e| e.into_inner()).note(env_setting() == Setting::Off, text);
	if let Some(line) = line {
		eprintln!("{line}");
	}
}

/// `runlog::start` has finished (`started`: the log is on): writes the boxes shown before it, or drops them.
pub fn finish(started: bool) {
	let lines = NOTES.lock().unwrap_or_else(|e| e.into_inner()).finish(started);
	for l in lines {
		eprintln!("{l}");
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Today's Windows rule is kept for a real Windows; Wine is on whatever stderr it has; the switch beats everything.
	#[test]
	fn the_log_is_on_when_stderr_goes_nowhere_or_under_wine() {
		// A real Windows (wine = false): as before this change.
		assert!(wanted(Setting::Auto, false, false), "double-clicked: no stderr");
		assert!(!wanted(Setting::Auto, true, false), "console or redirected stderr: unchanged, no log");
		assert!(wanted(Setting::On, true, false), "RAC_RUN_LOG=1 forces it");
		assert!(!wanted(Setting::Off, false, false), "RAC_RUN_LOG=0 turns it off");
		// Wine: the stderr is Wine's own and nobody reads it.
		assert!(wanted(Setting::Auto, true, true), "Wine with a stderr: on by default");
		assert!(wanted(Setting::Auto, false, true));
		assert!(!wanted(Setting::Off, true, true), "RAC_RUN_LOG=0 beats Wine");
		assert!(wanted(Setting::On, true, true));
	}

	#[test]
	fn the_switch_reads_zero_and_one_only() {
		assert_eq!(setting(None), Setting::Auto);
		assert_eq!(setting(Some("0")), Setting::Off);
		assert_eq!(setting(Some("1")), Setting::On);
		assert_eq!(setting(Some("")), Setting::Auto);
		assert_eq!(setting(Some("2")), Setting::Auto);
	}

	#[test]
	fn the_files_are_log_txt_and_under_wine_playdens_file_first() {
		let (dir, data) = (PathBuf::from("/p/player"), PathBuf::from("/p/rac-rs_Data"));
		assert_eq!(slots(&dir, None), vec![Slot { path: dir.join("log.txt"), prev: dir.join("log.prev.txt") }]);
		assert_eq!(
			slots(&dir, Some(data.clone())),
			vec![Slot { path: data.join("output_log.txt"), prev: data.join("output_log.prev.txt") }, Slot { path: dir.join("log.txt"), prev: dir.join("log.prev.txt") }]
		);
	}

	fn scratch(name: &str) -> PathBuf {
		let d = std::env::temp_dir().join(format!("rac_runlog_policy_{name}_{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&d);
		std::fs::create_dir_all(&d).unwrap();
		d
	}

	#[test]
	fn a_normal_start_makes_log_txt_and_keeps_the_run_before() {
		let d = scratch("normal");
		std::fs::write(d.join("log.txt"), "old run").unwrap();
		let o = open(&slots(&d, None)).expect("log");
		assert_eq!(o.path, d.join("log.txt"));
		assert_eq!(std::fs::read_to_string(d.join("log.prev.txt")).unwrap(), "old run");
		assert_eq!(std::fs::read_to_string(d.join("log.txt")).unwrap(), "", "a fresh file");
		assert!(o.notes.is_empty());
		assert!(!d.join("output_log.txt").exists(), "no Playden file off Wine");
		let _ = std::fs::remove_dir_all(&d);
	}

	/// Under Wine the same bytes are readable under both names, and each start moves both runs aside.
	#[test]
	fn under_wine_both_names_show_the_same_log() {
		use std::io::Write;
		let d = scratch("wine");
		let (player, data) = (d.join("player"), d.join("rac-rs_Data"));
		for run in 0..2 {
			let mut o = open(&slots(&player, Some(data.clone()))).expect("log");
			assert_eq!(o.path, data.join("output_log.txt"), "Playden's file is the real one");
			if !o.notes.is_empty() {
				eprintln!("no hard links on this file system: {:?}", o.notes);
				let _ = std::fs::remove_dir_all(&d);
				return;
			}
			writeln!(o.file, "run {run}").unwrap();
			o.file.flush().unwrap();
			assert_eq!(o.names, vec![data.join("output_log.txt"), player.join("log.txt")]);
			assert_eq!(std::fs::read_to_string(data.join("output_log.txt")).unwrap(), format!("run {run}\n"));
			assert_eq!(std::fs::read_to_string(player.join("log.txt")).unwrap(), format!("run {run}\n"), "the second name is the same file");
		}
		assert_eq!(std::fs::read_to_string(data.join("output_log.prev.txt")).unwrap(), "run 0\n");
		assert_eq!(std::fs::read_to_string(player.join("log.prev.txt")).unwrap(), "run 0\n");
		let _ = std::fs::remove_dir_all(&d);
	}

	/// An unusable first place hands the real file to the next one; an unusable later place only costs a note.
	#[test]
	fn an_unusable_place_is_skipped_or_noted() {
		let d = scratch("skip");
		let blocker = d.join("blocker");
		std::fs::write(&blocker, "a file where a folder should be").unwrap();
		let o = open(&slots(&d.join("player"), Some(blocker.join("rac-rs_Data")))).expect("falls back to log.txt");
		assert_eq!(o.path, d.join("player").join("log.txt"));
		assert_eq!(o.notes.len(), 1, "{:?}", o.notes);
		assert!(o.notes[0].contains("output_log.txt"));
		let both = [Slot { path: d.join("a.txt"), prev: d.join("a.prev.txt") }, Slot { path: blocker.join("b.txt"), prev: blocker.join("b.prev.txt") }];
		let o = open(&both).expect("a.txt");
		assert_eq!(o.path, d.join("a.txt"));
		assert_eq!(o.names, vec![d.join("a.txt")]);
		assert!(o.notes[0].contains("cannot link"), "{:?}", o.notes);
		assert!(open(&[Slot { path: blocker.join("c.txt"), prev: blocker.join("c.prev.txt") }]).is_none(), "nothing can be made: no log");
		let _ = std::fs::remove_dir_all(&d);
	}

	/// Boxes before the start are kept and written once, in order, after the log's first lines; later boxes are written at once.
	#[test]
	fn a_name_that_cannot_be_linked_says_where_the_log_is() {
		let d = scratch("nolink");
		// b.prev.txt is a folder, so the run before (b.txt) cannot be moved aside and the link finds the old file in its way.
		std::fs::write(d.join("b.txt"), "the run before").unwrap();
		std::fs::create_dir_all(d.join("b.prev.txt").join("x")).unwrap();
		let both = [Slot { path: d.join("a.txt"), prev: d.join("a.prev.txt") }, Slot { path: d.join("b.txt"), prev: d.join("b.prev.txt") }];
		let o = open(&both).expect("a.txt");
		assert_eq!(o.names, vec![d.join("a.txt")]);
		assert!(o.notes[0].contains("cannot link") && o.notes[0].contains("says so"), "{:?}", o.notes);
		let b = std::fs::read_to_string(d.join("b.txt")).unwrap();
		assert!(!b.contains("the run before") && b.contains("a.txt"), "b.txt names the real log, not the run before: {b}");
		let _ = std::fs::remove_dir_all(&d);
	}

	#[test]
	fn boxes_before_the_start_are_kept_and_flushed() {
		let mut n = Notes::new();
		assert_eq!(n.note(false, "no disc image\nrun Setup"), None);
		assert_eq!(n.note(false, "second"), None);
		let lines = n.finish(true);
		assert_eq!(lines, vec!["message box (shown before the run log started): no disc image\n  run Setup", "message box (shown before the run log started): second"]);
		assert_eq!(n.note(false, "later"), Some("message box: later".into()), "a running log gets it at once");
		assert!(n.finish(true).is_empty(), "nothing is written twice");
	}

	/// RAC_RUN_LOG=0 keeps nothing and writes nothing, at any stage.
	#[test]
	fn the_off_switch_covers_the_boxes_too() {
		let mut n = Notes::new();
		assert_eq!(n.note(true, "before"), None);
		assert!(n.pending.is_empty(), "nothing is buffered");
		assert!(n.finish(true).is_empty(), "so nothing is flushed when the log runs anyway");
		assert_eq!(n.note(true, "after"), None);
	}

	/// A start that has no log drops what it kept, and boxes after it go nowhere (a console shows stderr itself).
	#[test]
	fn a_start_without_a_log_drops_the_boxes() {
		let mut n = Notes::new();
		n.note(false, "kept for a moment");
		assert!(n.finish(false).is_empty());
		assert!(n.pending.is_empty());
		assert_eq!(n.note(false, "later"), None);
		assert!(n.pending.is_empty(), "and nothing piles up");
	}

	#[test]
	fn the_pending_boxes_are_capped_and_a_long_text_is_cut() {
		let mut n = Notes::new();
		for i in 0..40 {
			n.note(false, &format!("box {i}"));
		}
		assert_eq!(n.finish(true).len(), MAX_PENDING);
		let long = "x".repeat(MAX_TEXT * 2);
		assert_eq!(box_line("message box", &long).len(), "message box: ".len() + MAX_TEXT);
		// The kept text is cut when it is kept (16 boxes of any size stay small), not only when it is written.
		let mut n = Notes::new();
		n.note(false, &long);
		assert_eq!(n.pending[0].chars().count(), MAX_TEXT);
		assert_eq!(n.finish(true)[0].chars().count(), "message box (shown before the run log started): ".len() + MAX_TEXT);
	}
}
