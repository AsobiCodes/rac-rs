//! [PC] The player's Resolution (v0.5.1, user 2026-11-07; no retail counterpart): Desktop (the default = exactly as before) or one
//! of the monitor's sizes. The menu row (id `ROW`) is a Choice of `SLOTS` values: value 0 is Desktop, values 1.. are the sizes of
//! `list()` in order, named at run time (`ui::live`: "1920x1080"); the slots past the list are withdrawn.
//! - The row's value (`WANT`, saved at once) comes into force (`FORCE`) `SETTLE` after the last change or when the menu closes
//!   (`settle`, called by the host each frame): stepping through the sizes rebuilds nothing until the player stops.
//! - Windowed: the host sizes the window's client area to the size and centres it (`window_size`: a size whose window does not fit
//!   the monitor's work area reads as Desktop there). Desktop leaves the window as it is.
//! - Fullscreen (borderless, no display mode switch): the whole frame, HUD and menus included, is drawn at the size and scaled to the
//!   screen (`frame_size`; gpu/aspect_pass.rs: the box picture has the size, its rectangle is the largest one of its shape on the
//!   screen, the copy filters). Alt-tab and the upscalers (their output = the size) work as before; frame generation pauses.
//! A size the monitor cannot show (bigger than the monitor, or not on its list once its video modes are known) reads as Desktop.
//! The host names the monitor when it changes (`set_monitor`) and its video modes only when the row is shown or a size is saved
//! (`needs_modes`, `set_modes`). Kept in the options file only (`resolution=WxH`, absent = Desktop): no RAM byte, the game and its
//! RAM hash never see it. A game that never calls `init` or the setters keeps Desktop (draws exactly as before).

use crate::pcopts::PcOptions;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Options file key and the menu row's id.
pub const KEY: &str = "resolution";
pub const ROW: &str = "resolution";
/// Values of the row: Desktop + up to `SLOTS - 1` sizes.
pub const SLOTS: usize = 24;
/// The smallest size offered.
const MIN: (u32, u32) = (640, 480);
/// Common 16:9 sizes added when the monitor lists few (a virtual or remote display lists one or two).
const COMMON: [(u32, u32); 7] = [(1280, 720), (1366, 768), (1600, 900), (1920, 1080), (2560, 1440), (3200, 1800), (3840, 2160)];
/// How long the row's value waits after the last change before it comes into force (while the menu stays open).
const SETTLE: Duration = Duration::from_millis(700);

/// The row's value (w << 32 | h; 0 = Desktop): what the options file holds.
static WANT: AtomicU64 = AtomicU64::new(0);
/// The size in force (`settle`).
static FORCE: AtomicU64 = AtomicU64::new(0);
/// `FORCE` as the monitor can show it (`resolve`), refreshed whenever one of its inputs changes: `current` reads only this.
static CURRENT: AtomicU64 = AtomicU64::new(0);
/// The monitor's size and its work area (the screen minus the task bar; 0 = not known yet).
static MONITOR: AtomicU64 = AtomicU64::new(0);
static WORK: AtomicU64 = AtomicU64::new(0);
/// The monitor's video modes are on the list (`set_modes`; false after `set_monitor` until then).
static MODES_KNOWN: AtomicBool = AtomicBool::new(false);
/// The system names no monitor at all (`set_no_monitor`): Desktop only, until `set_monitor`.
static NO_MONITOR: AtomicBool = AtomicBool::new(false);
/// The window is fullscreen (set by the host each frame).
static FULL: AtomicBool = AtomicBool::new(false);
/// A row change waits to come into force: since when (`settle`); `PENDING` = there is one (no lock per frame otherwise).
static PENDING: AtomicBool = AtomicBool::new(false);
static SINCE: Mutex<Option<Instant>> = Mutex::new(None);
/// The sizes of the row's values 1.. (`mode_list`).
static LIST: Mutex<Vec<(u32, u32)>> = Mutex::new(Vec::new());

fn pack(s: Option<(u32, u32)>) -> u64 {
	s.map_or(0, |(w, h)| (w as u64) << 32 | h as u64)
}

fn unpack(v: u64) -> Option<(u32, u32)> {
	(v != 0).then_some(((v >> 32) as u32, v as u32))
}

/// `WxH` (`1920x1080`, ` 1920 X 1080 `); None for Desktop, a typo or a size below 640 x 480.
pub fn parse(s: &str) -> Option<(u32, u32)> {
	let s = s.trim().to_ascii_lowercase();
	let (a, b) = s.split_once('x')?;
	let (w, h) = (a.trim().parse::<u32>().ok()?, b.trim().parse::<u32>().ok()?);
	(w >= MIN.0 && h >= MIN.1 && w <= 16384 && h <= 16384).then_some((w, h))
}

/// The row's sizes for a monitor of size `mon` whose video modes are `modes`: no duplicates, none bigger than the monitor or below
/// 640 x 480, sorted by width then height; when the monitor lists fewer than 4, the common 16:9 sizes that fit join them; the
/// monitor's own size is always there; at most `SLOTS - 1` (the smallest go first).
pub fn mode_list(modes: &[(u32, u32)], mon: (u32, u32)) -> Vec<(u32, u32)> {
	let fits = |&(w, h): &(u32, u32)| w >= MIN.0 && h >= MIN.1 && (mon.0 == 0 || (w <= mon.0 && h <= mon.1));
	let mut v: Vec<(u32, u32)> = modes.iter().copied().filter(fits).collect();
	if v.len() < 4 {
		v.extend(COMMON.iter().copied().filter(fits));
	}
	if mon.0 >= MIN.0 && mon.1 >= MIN.1 {
		v.push(mon);
	}
	v.sort_unstable();
	v.dedup();
	let n = v.len();
	if n > SLOTS - 1 {
		v.drain(..n - (SLOTS - 1));
	}
	v
}

/// Start-up: the size from the options file, in force at once; the row offers the common sizes until the host names the monitor.
pub fn init(opts: &PcOptions) {
	let want = opts.extra_get(KEY).and_then(parse);
	WANT.store(pack(want), Ordering::Relaxed);
	FORCE.store(pack(want), Ordering::Relaxed);
	PENDING.store(false, Ordering::Relaxed);
	if let Some((w, h)) = want {
		eprintln!("resolution: {w}x{h} (options file)");
	}
	crate::ui::live::watch(ROW);
	if MONITOR.load(Ordering::Relaxed) == 0 {
		publish(mode_list(&COMMON, (0, 0)));
	}
	refresh();
}

/// The host: the window's monitor is (now) one of size `mon` with the work area `work` (its size minus the task bar; the monitor's
/// size where unknown). Its video modes are not known until `set_modes`: meanwhile the row offers the common sizes that fit.
pub fn set_monitor(mon: (u32, u32), work: (u32, u32)) {
	MONITOR.store(pack(Some(mon)), Ordering::Relaxed);
	WORK.store(pack(Some(work)), Ordering::Relaxed);
	MODES_KNOWN.store(false, Ordering::Relaxed);
	NO_MONITOR.store(false, Ordering::Relaxed);
	publish(mode_list(&COMMON, mon));
	refresh();
}

/// The host: the system names no monitor for the window, not even a primary one (some remote sessions): no size can be checked
/// against a screen, so the row offers Desktop only and a saved size reads as Desktop until `set_monitor`.
pub fn set_no_monitor() {
	MONITOR.store(0, Ordering::Relaxed);
	WORK.store(0, Ordering::Relaxed);
	MODES_KNOWN.store(false, Ordering::Relaxed);
	NO_MONITOR.store(true, Ordering::Relaxed);
	publish(Vec::new());
	refresh();
}

/// Whether the host should name the monitor's video modes now (`set_modes`): not known yet and the row is on screen or a size is
/// saved. Enumerating the modes can take a while, so only then.
pub fn needs_modes() -> bool {
	MONITOR.load(Ordering::Relaxed) != 0
		&& !MODES_KNOWN.load(Ordering::Relaxed)
		&& (WANT.load(Ordering::Relaxed) != 0 || (crate::ui::is_open() && crate::ui::live::seen(ROW)))
}

/// The host: the video modes of the monitor of size `mon` (winit `MonitorHandle::video_modes`).
pub fn set_modes(modes: &[(u32, u32)], mon: (u32, u32)) {
	MONITOR.store(pack(Some(mon)), Ordering::Relaxed);
	if WORK.load(Ordering::Relaxed) == 0 {
		WORK.store(pack(Some(mon)), Ordering::Relaxed);
	}
	MODES_KNOWN.store(true, Ordering::Relaxed);
	let list = mode_list(modes, mon);
	eprintln!("resolution: monitor {}x{}, {} sizes offered", mon.0, mon.1, list.len());
	publish(list);
	refresh();
}

/// The list in force, its names on the row's values and the unused slots withdrawn (only when it changed).
fn publish(list: Vec<(u32, u32)>) {
	if *LIST.lock().unwrap_or_else(|e| e.into_inner()) == list {
		return;
	}
	for i in 1..SLOTS {
		let s = list.get(i - 1);
		match s {
			Some(&(w, h)) => crate::ui::live::set(ROW, i as i32, format!("{w}x{h}")),
			None => crate::ui::live::clear(ROW, i as i32),
		}
		crate::ui::live::offer(ROW, i as i32, s.is_some());
	}
	*LIST.lock().unwrap_or_else(|e| e.into_inner()) = list;
}

/// The sizes the row offers (values 1..).
pub fn list() -> Vec<(u32, u32)> {
	LIST.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// `s` as the monitor can show it: None (Desktop) when bigger than the monitor or, once its modes are known, not on the list.
fn resolve(s: Option<(u32, u32)>) -> Option<(u32, u32)> {
	let s = s.filter(|_| !NO_MONITOR.load(Ordering::Relaxed))?;
	let mon = unpack(MONITOR.load(Ordering::Relaxed));
	let fits = mon.is_none_or(|m| s.0 <= m.0 && s.1 <= m.1);
	let listed = !MODES_KNOWN.load(Ordering::Relaxed) || LIST.lock().unwrap_or_else(|e| e.into_inner()).contains(&s);
	(fits && listed).then_some(s)
}

fn refresh() {
	CURRENT.store(pack(resolve(unpack(FORCE.load(Ordering::Relaxed)))), Ordering::Relaxed);
}

/// The size in force (None = Desktop). One atomic load.
pub fn current() -> Option<(u32, u32)> {
	unpack(CURRENT.load(Ordering::Relaxed))
}

/// The row's value: the index of the row's size on the list + 1, 0 = Desktop (or a size the monitor cannot show).
pub fn index() -> i32 {
	resolve(unpack(WANT.load(Ordering::Relaxed))).and_then(|s| list().iter().position(|&x| x == s)).map_or(0, |i| i as i32 + 1)
}

/// The menu set the row to `v`: true = the options file changed. The size comes into force later (`settle`).
pub fn set_from_ui(opts: &mut PcOptions, v: i32) -> bool {
	let size = usize::try_from(v).ok().filter(|&i| i > 0).and_then(|i| list().get(i - 1).copied());
	// WANT, PENDING and SINCE change together under SINCE's lock (`settle_at` takes WANT under it): no press is lost to a settle on
	// another thread.
	{
		let mut since = SINCE.lock().unwrap_or_else(|e| e.into_inner());
		if WANT.swap(pack(size), Ordering::Relaxed) == pack(size) {
			return false;
		}
		*since = Some(Instant::now());
		PENDING.store(true, Ordering::Relaxed);
	}
	opts.extra_set(KEY, size.map(|(w, h)| format!("{w}x{h}")));
	eprintln!("resolution: PC options: {}", size.map_or("Desktop".to_string(), |(w, h)| format!("{w}x{h}")));
	true
}

/// The host, each frame: a row change comes into force `SETTLE` after the last one, or at once when the menu is closed.
pub fn settle() {
	if PENDING.load(Ordering::Relaxed) {
		settle_at(Instant::now(), crate::ui::is_open());
	}
}

/// Under SINCE's lock (`set_from_ui`): the WANT taken into force is the one whose PENDING is cleared.
fn settle_at(now: Instant, menu_open: bool) {
	let since = SINCE.lock().unwrap_or_else(|e| e.into_inner());
	if !PENDING.load(Ordering::Relaxed) || (menu_open && since.is_some_and(|t| now.saturating_duration_since(t) < SETTLE)) {
		return;
	}
	PENDING.store(false, Ordering::Relaxed);
	FORCE.store(WANT.load(Ordering::Relaxed), Ordering::Relaxed);
	drop(since);
	refresh();
}

/// The host: the window is fullscreen now (`frame_size` applies only then).
pub fn set_fullscreen(on: bool) {
	FULL.store(on, Ordering::Relaxed);
}

pub fn fullscreen() -> bool {
	FULL.load(Ordering::Relaxed)
}

/// The client size of a window whose frame (title bar + borders) adds `frame`: `current` while that window fits the monitor's
/// work area (None = Desktop: the window stays as it is).
pub fn window_size(frame: (u32, u32)) -> Option<(u32, u32)> {
	window_size_of(current(), unpack(WORK.load(Ordering::Relaxed)), frame)
}

fn window_size_of(want: Option<(u32, u32)>, work: Option<(u32, u32)>, frame: (u32, u32)) -> Option<(u32, u32)> {
	want.filter(|&(w, h)| work.is_none_or(|a| w.saturating_add(frame.0) <= a.0 && h.saturating_add(frame.1) <= a.1))
}

/// The size the whole frame is drawn at in a window of size `real`: Some only while fullscreen with a size other than the window's
/// that fits in it (`frame_size_of`).
pub fn frame_size(real: (u32, u32)) -> Option<(u32, u32)> {
	frame_size_of(fullscreen(), current(), real)
}

fn frame_size_of(full: bool, want: Option<(u32, u32)>, real: (u32, u32)) -> Option<(u32, u32)> {
	want.filter(|&(w, h)| full && (w, h) != real && w <= real.0 && h <= real.1)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_file_value() {
		assert_eq!(parse("1920x1080"), Some((1920, 1080)));
		assert_eq!(parse(" 1280 X 720 "), Some((1280, 720)));
		assert_eq!(parse("desktop"), None);
		assert_eq!(parse("320x240"), None, "below 640x480");
		assert_eq!(parse("1920x"), None);
	}

	#[test]
	fn the_mode_list() {
		let modes = [(2560, 1440), (1920, 1080), (1920, 1080), (1280, 720), (800, 600), (640, 480), (320, 200), (3840, 2160), (1024, 768)];
		let l = mode_list(&modes, (2560, 1440));
		assert_eq!(l, vec![(640, 480), (800, 600), (1024, 768), (1280, 720), (1920, 1080), (2560, 1440)], "deduplicated, sorted, nothing bigger than the monitor");
		let few = mode_list(&[(1920, 1080)], (1920, 1080));
		assert_eq!(few, vec![(1280, 720), (1366, 768), (1600, 900), (1920, 1080)], "the common 16:9 sizes that fit join a short list");
		let many: Vec<(u32, u32)> = (0..40).map(|i| (640 + i * 40, 480 + i * 20)).collect();
		let l = mode_list(&many, (4000, 2000));
		assert_eq!(l.len(), SLOTS - 1);
		assert_eq!(*l.last().unwrap(), (4000, 2000), "the monitor's own size stays");
	}

	#[test]
	fn the_frame_size_only_fullscreen_and_fitting() {
		assert_eq!(frame_size_of(true, Some((1920, 1080)), (2560, 1440)), Some((1920, 1080)));
		assert_eq!(frame_size_of(false, Some((1920, 1080)), (2560, 1440)), None, "windowed: the window has the size");
		assert_eq!(frame_size_of(true, None, (2560, 1440)), None, "Desktop");
		assert_eq!(frame_size_of(true, Some((2560, 1440)), (2560, 1440)), None, "the screen's own size: the plain path");
		assert_eq!(frame_size_of(true, Some((3840, 2160)), (2560, 1440)), None, "too big: Desktop");
	}

	#[test]
	fn a_window_must_fit_the_work_area() {
		let work = Some((2560, 1392));
		assert_eq!(window_size_of(Some((1920, 1080)), work, (16, 39)), Some((1920, 1080)));
		assert_eq!(window_size_of(Some((2560, 1440)), work, (16, 39)), None, "the monitor's own size does not fit a window: Desktop");
		assert_eq!(window_size_of(Some((2560, 1353)), work, (0, 39)), Some((2560, 1353)), "exactly the work area");
		assert_eq!(window_size_of(Some((1280, 720)), None, (16, 39)), Some((1280, 720)), "work area not known yet");
	}

	/// Every static this module owns and the row's run-time labels / offers, as they were before a test.
	struct Saved([u64; 5], [bool; 4], Vec<(u32, u32)>, bool);

	fn save() -> Saved {
		let a = [&WANT, &FORCE, &CURRENT, &MONITOR, &WORK].map(|s| s.load(Ordering::Relaxed));
		let b = [&MODES_KNOWN, &FULL, &PENDING, &NO_MONITOR].map(|s| s.load(Ordering::Relaxed));
		Saved(a, b, list(), crate::ui::live::watching(ROW))
	}

	fn restore(s: Saved) {
		for (st, v) in [&WANT, &FORCE, &CURRENT, &MONITOR, &WORK].into_iter().zip(s.0) {
			st.store(v, Ordering::Relaxed);
		}
		for (st, v) in [&MODES_KNOWN, &FULL, &PENDING, &NO_MONITOR].into_iter().zip(s.1) {
			st.store(v, Ordering::Relaxed);
		}
		*SINCE.lock().unwrap_or_else(|e| e.into_inner()) = None;
		if !s.3 {
			crate::ui::live::unwatch(ROW); // `init` watches the row
		}
		*LIST.lock().unwrap_or_else(|e| e.into_inner()) = Vec::new(); // publish always writes the labels again below
		if s.2.is_empty() {
			for i in 1..SLOTS {
				crate::ui::live::clear(ROW, i as i32);
				crate::ui::live::offer(ROW, i as i32, true);
			}
		} else {
			publish(s.2);
		}
	}

	#[test]
	fn the_row_round_trip() {
		let _g = crate::pcopts::globals_guard();
		let saved = save();
		let mut o = PcOptions::parse("resolution=1600x900\n");
		set_monitor((2560, 1440), (2560, 1400));
		set_modes(&[(1280, 720), (1600, 900), (1920, 1080), (2560, 1440)], (2560, 1440));
		init(&o);
		assert_eq!((current(), index()), (Some((1600, 900)), 2), "the saved size is in force at once");
		assert!(set_from_ui(&mut o, 3));
		assert_eq!(o.extra_get(KEY), Some("1920x1080"));
		assert_eq!((current(), index()), (Some((1600, 900)), 3), "the row moved, the size in force waits");
		let t = SINCE.lock().unwrap().unwrap();
		settle_at(t + Duration::from_millis(300), true);
		assert_eq!(current(), Some((1600, 900)), "the menu is open and the change is new");
		settle_at(t + SETTLE, true);
		assert_eq!(current(), Some((1920, 1080)), "in force after the wait");
		assert!(set_from_ui(&mut o, 0));
		settle_at(Instant::now(), false);
		assert_eq!((o.extra_get(KEY), current(), index()), (None, None, 0), "at once when the menu is closed");
		set_monitor((1280, 720), (1280, 680));
		set_modes(&[(1280, 720)], (1280, 720));
		assert!(set_from_ui(&mut o, 1));
		init(&PcOptions::parse("resolution=1920x1080\n"));
		assert_eq!(current(), None, "a size the monitor cannot show reads as Desktop");
		restore(saved);
	}

	#[test]
	fn no_monitor_offers_desktop_only() {
		let _g = crate::pcopts::globals_guard();
		let saved = save();
		init(&PcOptions::parse("resolution=1280x720
"));
		set_no_monitor();
		assert_eq!((list(), current(), index()), (Vec::new(), None, 0), "no sizes, the saved one reads as Desktop");
		assert_eq!(window_size((16, 39)), None, "the window stays as it is");
		set_monitor((1920, 1080), (1920, 1040));
		assert_eq!(current(), Some((1280, 720)), "a monitor again: the saved size comes back");
		assert!(!list().is_empty());
		restore(saved);
	}

	#[test]
	fn no_press_is_lost_to_a_settle() {
		let _g = crate::pcopts::globals_guard();
		let saved = save();
		set_monitor((2560, 1440), (2560, 1400));
		set_modes(&[(1280, 720), (1600, 900), (1920, 1080), (2560, 1440)], (2560, 1440));
		init(&PcOptions::parse(""));
		let presses = std::thread::spawn(|| {
			let mut o = PcOptions::parse("");
			for i in 0..400 {
				set_from_ui(&mut o, 1 + i % 3);
			}
			set_from_ui(&mut o, 2);
		});
		while !presses.is_finished() {
			settle_at(Instant::now(), false);
		}
		presses.join().unwrap();
		settle_at(Instant::now(), false);
		assert_eq!(current(), Some((1600, 900)), "the last press is in force");
		assert!(!PENDING.load(Ordering::Relaxed));
		restore(saved);
	}
}
