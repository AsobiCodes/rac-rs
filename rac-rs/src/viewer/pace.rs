//! [PC] (speed) Frame cap. Without it the render loop draws as fast as the CPU allows (150-300 frames per second on fast PCs)
//! while the game itself makes 50 new pictures a second; with Mailbox every frame past the display's refresh rate is thrown
//! away, but its CPU time (main thread, frame workers, driver threads) is taken from the EE thread that runs the game, which on
//! 6-core CPUs is what sets the game speed. The cap waits at the end of a redraw (before the next one is requested), so input
//! that arrives meanwhile is read by the next frame.
//! The limit is the FPS limit row of Pause -> Options -> Graphics -> Display (retail/pcdisp.rs: Display / 30 / 60 / 120 / 144 / Off;
//! Display = the display's refresh rate, at least 50) or RAC_FPS_CAP=N (0 = uncapped; the env switch wins).
//! - Mailbox / Immediate (gpu.rs `present_mode`): every limit applies.
//! - Fifo (RAC_VSYNC=1, or a surface that offers only Fifo): present already waits for the refresh. A limit at or above the refresh
//!   rate changes nothing. Below it, when refresh / limit (exact refresh rate) is within 2 % of a whole number k >= 2 a frame is spaced
//!   k refresh intervals (steady: 30 on 59.94 Hz is exactly 2 vblanks); otherwise the plain 1/limit period, so the average is the limit
//!   and a frame rate never goes above what the player set (144 Hz with 60 is not 72).
//! - Lockstep modes (menus, title, pause, travel, level loads, movies: `Retail::lockstep`) run one game field per frame, so a late
//!   frame there loses game time for good: the limit does not apply at all, pacing is what limit Off gives (the field clock, see
//!   handler/frame_start.rs `accum`). Gameplay, scenes, the vendor and the ship streams run ahead (`Retail::depth`) and keep the
//!   limit (30 fps drawing, 50 fields a second).
//! Kerwan walk bench on 6 cores, game thread fields over 20 ms of 2000: uncapped 160, 2x refresh (340) 30, refresh (170) 10.

use std::time::{Duration, Instant};

pub(super) struct Pace {
	/// Start of the next frame slot.
	next: Option<Instant>,
	/// Frame period; refreshed from the monitor every few seconds (a window can move to another display).
	period: Option<Duration>,
	checked: Option<Instant>,
	env: Option<u32>,
	/// What the period was made for: the FPS limit row's value, a Fifo swapchain, lockstep mode.
	made_for: (u32, bool, bool),
}

/// The frame cap in Hz before the present mode and lockstep are looked at: the env switch, else the FPS limit row (`setting`: 0
/// Display, `u32::MAX` Off, else the Hz); 0 = no cap. `hz` = the display's refresh rate.
fn limit_hz(env: Option<u32>, setting: u32, hz: u32) -> u32 {
	if let Some(n) = env {
		return n;
	}
	match setting {
		0 => hz.max(50),
		u32::MAX => 0,
		n => n,
	}
}

/// The frame period for the limit `n` Hz (0 = none), or None when nothing needs to sleep. `lockstep`: one game field per frame, so
/// no limit applies (same as Off). `fifo`: present waits for the refresh (`mhz`, exact, in mHz): none for a limit at or above it; else
/// k whole refresh intervals when refresh / n is within 2 % of a whole k >= 2, else 1/n (never faster than the limit).
fn period_for(n: u32, mhz: u32, fifo: bool, lockstep: bool) -> Option<Duration> {
	if n == 0 || lockstep {
		return None;
	}
	if !fifo {
		return Some(Duration::from_secs_f64(1.0 / n as f64));
	}
	if n as u64 * 1000 >= mhz as u64 {
		return None;
	}
	let r = mhz.max(1) as f64 / (n as f64 * 1000.0);
	let k = r.round();
	if k >= 2.0 && (r - k).abs() <= 0.02 * k {
		return Some(Duration::from_secs_f64(k * 1000.0 / mhz as f64));
	}
	Some(Duration::from_secs_f64(1.0 / n as f64))
}

impl Pace {
	pub(super) fn new() -> Pace {
		Pace { next: None, period: None, checked: None, made_for: (0, false, false), env: std::env::var("RAC_FPS_CAP").ok().and_then(|v| v.trim().parse().ok()) }
	}

	/// End of a redraw: sleeps until this frame's slot is over. `fifo`: the swapchain is vsynced; `lockstep`: `Retail::lockstep`.
	pub(super) fn wait(&mut self, window: &winit::window::Window, fifo: bool, lockstep: bool) {
		let now = Instant::now();
		// The FPS limit row (Pause -> Options -> Graphics -> Display) is looked at every frame, so a change applies at once.
		let made_for = (crate::retail::pcdisp::fps_setting(), fifo, lockstep);
		if self.checked.is_none_or(|t| now - t > Duration::from_secs(2)) || self.made_for != made_for {
			self.made_for = made_for;
			self.checked = Some(now);
			let mhz = window.current_monitor().and_then(|m| m.refresh_rate_millihertz()).unwrap_or(60000);
			let hz = (mhz + 500) / 1000;
			crate::retail::pcdisp::note_refresh(hz);
			self.period = period_for(limit_hz(self.env, made_for.0, hz), mhz, fifo, lockstep);
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
			super::framecsv::cap(now, now.elapsed().as_secs_f32() * 1000.0);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn ms(d: Option<Duration>) -> Option<f64> {
		d.map(|d| (d.as_secs_f64() * 1e6).round() / 1e3)
	}

	#[test]
	fn limit_comes_from_the_env_then_the_row() {
		assert_eq!(limit_hz(Some(25), 0, 144), 25);
		assert_eq!(limit_hz(Some(0), 30, 144), 0);
		assert_eq!(limit_hz(None, 0, 144), 144);
		assert_eq!(limit_hz(None, 0, 30), 50, "Display never goes below the PAL field rate");
		assert_eq!(limit_hz(None, u32::MAX, 144), 0);
		assert_eq!(limit_hz(None, 30, 144), 30);
	}

	/// Review N1: in lockstep (one game field per frame) a late frame loses game time for good, so no limit applies there, whatever
	/// the row says (30 made travel 49.4 fields a second). Run-ahead modes keep 33.3 ms.
	#[test]
	fn lockstep_ignores_the_limit() {
		for fifo in [false, true] {
			for n in [0, 30, 60, 144] {
				assert_eq!(period_for(n, 60000, fifo, true), None, "limit {n}, fifo {fifo}");
			}
		}
		assert_eq!(ms(period_for(30, 60000, false, false)), Some(33.333));
		assert_eq!(ms(period_for(144, 144000, false, false)), Some(6.944));
		assert_eq!(period_for(0, 60000, false, false), None);
	}

	/// Review N4: on a Fifo swapchain a limit that divides the exact refresh rate (within 2 %) is spaced in whole refresh intervals;
	/// any other limit below the refresh rate keeps the plain 1/limit period (never above the limit); a limit at or above it sleeps not.
	#[test]
	fn fifo_slots_are_whole_refresh_intervals_or_the_plain_period() {
		// (refresh mHz, limit, whole intervals per frame; 0 = no sleep; u64::MAX = plain 1/limit)
		const PLAIN: u64 = u64::MAX;
		let cases: [(u32, u32, u64); 15] = [
			(59940, 30, 2), (59940, 60, 0), (59940, 120, 0),
			(60000, 30, 2), (60000, 60, 0), (60000, 120, 0),
			(120000, 60, 2), (120000, 30, 4),
			(144000, 72, 2), (144000, 60, PLAIN), (144000, 120, PLAIN),
			(165000, 60, PLAIN), (165000, 30, PLAIN), (165000, 120, PLAIN), (144000, 144, 0),
		];
		for (mhz, n, k) in cases {
			let got = period_for(n, mhz, true, false);
			if k == 0 {
				assert_eq!(got, None, "{mhz} mHz limit {n}");
				continue;
			}
			let want = if k == PLAIN { 1.0 / n as f64 } else { k as f64 * 1000.0 / mhz as f64 };
			let got = got.unwrap_or_else(|| panic!("{mhz} mHz limit {n}: no sleep")).as_secs_f64();
			assert!((got - want).abs() < 1e-9, "{mhz} mHz limit {n}: {got} vs {want}");
			assert!(got >= 1.0 / n as f64 - 1e-9, "{mhz} mHz limit {n}: faster than the limit");
		}
		// 30 on 59.94 Hz is exactly two vblanks (33.367 ms), not 33.333 ms.
		assert_eq!(ms(period_for(30, 59940, true, false)), Some(33.367));
		assert_eq!(ms(period_for(60, 144000, true, false)), Some(16.667));
		assert_eq!(period_for(0, 60000, true, false), None);
		// Without Fifo the slot is 1/limit.
		assert_eq!(ms(period_for(30, 59940, false, false)), Some(33.333));
	}
}
