//! [PC] Graphics -> Display -> Resolution, the window side (engine viewer/resolution.rs, v0.5.1): every frame the engine learns
//! whether the window is fullscreen (in fullscreen a chosen size is the size the frame is drawn at, gpu/aspect_pass.rs) and lets a
//! row change come into force (`resolution::settle`: 0.7 s after the last step or when the menu closes). The window's monitor is
//! looked at only after a Moved / Resized / ScaleFactorChanged event (`dirty`, handler.rs) and its video modes only when the row
//! asks for them (`resolution::needs_modes`). Windowed, a size in force sets the window's client area once (when it comes into
//! force, at start-up, and after leaving fullscreen once the window has its frame again) and centres the window on its monitor's
//! work area (an off-screen test window is only resized). Desktop leaves the window as it is (exactly as before).

use super::*;
use rac_engine::viewer::resolution;
use std::cell::Cell;

/// A monitor: size, position, work area size, work area position.
type Mon = ((u32, u32), (i32, i32), (u32, u32), (i32, i32));

thread_local! {
	/// MAIN only: the window may be on another monitor (start-up, Moved / Resized / ScaleFactorChanged).
	static DIRTY: Cell<bool> = const { Cell::new(true) };
	/// MAIN only: the window's monitor as last seen.
	static MONITOR: Cell<Option<Mon>> = const { Cell::new(None) };
	/// MAIN only: the windowed size last applied (None = nothing applied since start-up or since fullscreen).
	static APPLIED: Cell<Option<Option<(u32, u32)>>> = const { Cell::new(None) };
	/// MAIN only: the size in force the window was last centred for (None = not since start-up or since fullscreen): the window is
	/// centred only when the size in force changes, never when a drag makes the size fit (again).
	static CENTRED: Cell<Option<Option<(u32, u32)>>> = const { Cell::new(None) };
	/// MAIN only: frames waited after leaving fullscreen for the window to get its frame back (0 = not waiting; `LEAVE_MAX`).
	static LEAVING: Cell<u32> = const { Cell::new(0) };
}

/// At most this many frames wait for the window frame after leaving fullscreen (a frameless window never gets one).
const LEAVE_MAX: u32 = 30;

/// handler.rs: a window event that can mean another monitor.
pub(super) fn dirty() {
	DIRTY.set(true);
}

/// handler.rs: the window moved to a monitor of another scale (DPI): a windowed size in force keeps its physical client size
/// (winit would keep the logical one: 1920x1080 at 100 % became 2880x1620 at 150 %). Desktop: as before.
pub(super) fn scale_changed(event: &mut WindowEvent) {
	if let WindowEvent::ScaleFactorChanged { inner_size_writer, .. } = event {
		if let Some(Some((w, h))) = APPLIED.get() {
			let _ = inner_size_writer.request_inner_size(winit::dpi::PhysicalSize::new(w, h));
		}
	}
}

/// The window's monitor; the primary one when the system names none (Wayland, some remote sessions).
fn monitor(w: &winit::window::Window) -> Option<winit::monitor::MonitorHandle> {
	w.current_monitor().or_else(|| w.primary_monitor())
}

/// The window position that centres an outer size `outer` on an area at `pos` of size `area` (never above or left of it).
fn centred(pos: (i32, i32), area: (u32, u32), outer: (u32, u32)) -> (i32, i32) {
	let off = |m: u32, o: u32| (m.saturating_sub(o) / 2) as i32;
	(pos.0 + off(area.0, outer.0), pos.1 + off(area.1, outer.1))
}

/// The work area (size, position) of monitor `m`: the screen minus the task bar (Windows `GetMonitorInfoW`); elsewhere the screen.
#[cfg(windows)]
pub(super) fn work_area(m: &winit::monitor::MonitorHandle) -> ((u32, u32), (i32, i32)) {
	use winit::platform::windows::MonitorHandleExtWindows;
	#[repr(C)]
	struct MonitorInfo {
		size: u32,
		monitor: [i32; 4],
		work: [i32; 4],
		flags: u32,
	}
	#[link(name = "user32")]
	unsafe extern "system" {
		fn GetMonitorInfoW(monitor: isize, info: *mut MonitorInfo) -> i32;
	}
	let mut mi = MonitorInfo { size: std::mem::size_of::<MonitorInfo>() as u32, monitor: [0; 4], work: [0; 4], flags: 0 };
	// SAFETY: a valid HMONITOR from winit and a MONITORINFO with its size set.
	if unsafe { GetMonitorInfoW(m.hmonitor() as isize, &mut mi) } != 0 {
		let [l, t, r, b] = mi.work;
		if r > l && b > t {
			return (((r - l) as u32, (b - t) as u32), (l, t));
		}
	}
	((m.size().width, m.size().height), (m.position().x, m.position().y))
}

#[cfg(not(windows))]
pub(super) fn work_area(m: &winit::monitor::MonitorHandle) -> ((u32, u32), (i32, i32)) {
	((m.size().width, m.size().height), (m.position().x, m.position().y))
}

/// handler.rs `resumed`: the saved windowed size for the new window (no second resize at the first frame), when it fits the
/// primary monitor's work area with room for a window frame.
pub(super) fn start_size(el: &ActiveEventLoop) -> Option<winit::dpi::PhysicalSize<u32>> {
	let (w, h) = crate::pcopts::PcOptions::load(&crate::pcopts::file()).extra_get(resolution::KEY).and_then(resolution::parse)?;
	let (work, _) = work_area(&el.primary_monitor()?);
	(w + 64 <= work.0 && h + 64 <= work.1).then(|| winit::dpi::PhysicalSize::new(w, h))
}

impl App {
	/// Each frame after the Display mode switch (frame_start.rs).
	pub(super) fn display_res_frame(&mut self) {
		let Some(g) = &self.gpu else { return };
		resolution::settle();
		if DIRTY.replace(false) {
			match monitor(&g.window) {
				Some(m) => {
					let (work, wpos) = work_area(&m);
					let key = ((m.size().width, m.size().height), (m.position().x, m.position().y), work, wpos);
					if MONITOR.get() != Some(key) {
						MONITOR.set(Some(key));
						resolution::set_monitor(key.0, work);
					}
				}
				// No monitor at all: the row offers Desktop only.
				None => {
					MONITOR.set(None);
					resolution::set_no_monitor();
				}
			}
		}
		if resolution::needs_modes() {
			if let Some(m) = monitor(&g.window) {
				let modes: Vec<(u32, u32)> = m.video_modes().map(|v| (v.size().width, v.size().height)).collect();
				resolution::set_modes(&modes, (m.size().width, m.size().height));
			}
		}
		if self.fullscreen {
			resolution::set_fullscreen(true);
			APPLIED.set(None);
			CENTRED.set(None);
			LEAVING.set(0);
			return;
		}
		// Leaving fullscreen: the frame follows the window at once (no box); only the size below waits until the window has its
		// title bar and borders again, so it is applied once, with the real frame, after the system's own resize.
		if resolution::fullscreen() {
			resolution::set_fullscreen(false);
			LEAVING.set(1);
		}
		let (inner, outer) = (g.window.inner_size(), g.window.outer_size());
		if LEAVING.get() > 0 {
			if inner == outer && LEAVING.get() < LEAVE_MAX {
				LEAVING.set(LEAVING.get() + 1);
				return;
			}
			LEAVING.set(0);
		}
		let frame = (outer.width.saturating_sub(inner.width), outer.height.saturating_sub(inner.height));
		let want = resolution::window_size(frame);
		if APPLIED.get() == Some(want) {
			return;
		}
		APPLIED.set(Some(want));
		let Some((w, h)) = want else { return };
		if (inner.width, inner.height) != (w, h) {
			let _ = g.window.request_inner_size(winit::dpi::PhysicalSize::new(w, h));
		}
		let skip = super::display::test_window();
		eprintln!("resolution: window {w}x{h}{}", if skip { " (test window: resized only)" } else { "" });
		let recentre = CENTRED.replace(Some(resolution::current())) != Some(resolution::current());
		if let (false, true, Some((_, _, work, wpos))) = (skip, recentre, MONITOR.get()) {
			let (x, y) = centred(wpos, work, (w + frame.0, h + frame.1));
			g.window.set_outer_position(winit::dpi::PhysicalPosition::new(x, y));
		}
	}
}

#[cfg(test)]
mod tests {
	use super::centred;

	#[test]
	fn the_window_is_centred_on_its_monitor() {
		assert_eq!(centred((0, 0), (2560, 1440), (1296, 759)), (632, 340));
		assert_eq!(centred((-1920, 0), (1920, 1080), (1296, 759)), (-1608, 160), "a monitor left of the main one");
		assert_eq!(centred((0, 0), (1280, 720), (1296, 759)), (0, 0), "bigger than the monitor: its corner");
	}
}
