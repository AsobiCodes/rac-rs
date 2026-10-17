//! [PC] (speed) Frame cap for the uncapped present modes (Mailbox / Immediate, gpu.rs `present_mode`). Without it the
//! render loop draws as fast as the CPU allows (150-300 frames per second on fast PCs) while the game itself makes 50
//! new pictures a second; with Mailbox every frame past the display's refresh rate is thrown away, but its CPU time
//! (main thread, frame workers, driver threads) is taken from the EE thread that runs the game, which on 6-core CPUs
//! is what sets the game speed. The cap waits at the end of a redraw (before the next one is requested), so input
//! that arrives meanwhile is read by the next frame. Fifo (RAC_VSYNC=1) needs no cap: present already waits.
//! RAC_FPS_CAP=N sets the cap (0 = uncapped as before); the default is the display's refresh rate (at least 50):
//! Kerwan walk bench on 6 cores, game thread fields over 20 ms of 2000: uncapped 160, 2x refresh (340) 30, refresh (170) 10.

use std::time::{Duration, Instant};

pub(super) struct Pace {
	/// Start of the next frame slot.
	next: Option<Instant>,
	/// Frame period; refreshed from the monitor every few seconds (a window can move to another display).
	period: Option<Duration>,
	checked: Option<Instant>,
	env: Option<u32>,
}

impl Pace {
	pub(super) fn new() -> Pace {
		Pace { next: None, period: None, checked: None, env: std::env::var("RAC_FPS_CAP").ok().and_then(|v| v.trim().parse().ok()) }
	}

	fn cap_hz(&self, window: &winit::window::Window) -> u32 {
		if let Some(n) = self.env {
			return n;
		}
		let hz = window.current_monitor().and_then(|m| m.refresh_rate_millihertz()).map_or(60, |mhz| (mhz + 500) / 1000);
		hz.max(50)
	}

	/// End of a redraw: with an uncapped present mode, sleeps until this frame's slot is over.
	pub(super) fn wait(&mut self, window: &winit::window::Window, uncapped: bool) {
		if !uncapped {
			self.next = None;
			return;
		}
		let now = Instant::now();
		if self.checked.is_none_or(|t| now - t > Duration::from_secs(2)) {
			self.checked = Some(now);
			let hz = self.cap_hz(window);
			self.period = (hz > 0).then(|| Duration::from_secs_f64(1.0 / hz as f64));
		}
		let Some(period) = self.period else {
			self.next = None;
			return;
		};
		// Slots follow each other; a frame that ran late starts a new run of slots instead of hurrying to catch up.
		let next = match self.next {
			Some(t) if t + period > now => t + period,
			_ => now,
		};
		self.next = Some(next);
		if next > now {
			std::thread::sleep(next - now);
		}
	}
}
