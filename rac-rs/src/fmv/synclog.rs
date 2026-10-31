//! [PC] RAC_FMV_SYNC_LOG=<file>: one CSV row per `Player::poll` (wall time, the audio clock, the queues, the frame shown) so a movie's
//! audio / video offset can be read off over the whole movie without listening (agents run muted). Off unless the variable is set.
//!
//! Columns: `t_ms` wall ms since the movie started, `played` audio frames consumed by the device callbacks, `pulls` device callbacks that
//! visited the stream, `abuf` audio frames queued, `vq` decoded video frames queued, `clock` the 90 kHz clock the frames are due against,
//! `shown` PTS of the frame shown at this poll (blank = none due), `lag_ms` how far that frame is behind the clock (0 = exactly on it),
//! `tref` / `ct` / `ysum` / `rff` the shown frame's MPEG temporal reference, coding type (1 I, 2 P, 3 B), an 8 x 8 grid of luma block means in hex (tells its
//! picture from another decoder's) and repeat_first_field; `dev_ms` the output device's latency (audio.rs `out_latency`); `comp_ms` the output latency the clock subtracts (the picture follows what is HEARD); `mode` a = audio clock, w = wall clock (audio given up), p = audio clock held (starved).

use std::io::Write;
use std::time::Instant;

pub struct SyncLog {
	out: std::io::BufWriter<std::fs::File>,
	t0: Instant,
}

/// What one poll saw.
pub struct Row {
	pub played: u64,
	pub pulls: u64,
	pub abuf: usize,
	pub vq: usize,
	pub clock: u64,
	pub shown: Option<u64>,
	pub frame: Option<(u16, u8, String, bool)>,
	pub dev_ms: f64,
	/// The output latency the clock already allows for (`Player::lat_ms`), 0 on a build without compensation.
	pub comp_ms: f64,
	pub mode: char,
}

/// The picture's identity for the log: the means of an 8 x 8 grid of luma blocks, two hex digits each.
pub fn thumb(y: &[u8], w: usize, h: usize) -> String {
	let mut s = String::with_capacity(128);
	for gy in 0..8 {
		for gx in 0..8 {
			let (x0, x1, y0, y1) = (gx * w / 8, (gx + 1) * w / 8, gy * h / 8, (gy + 1) * h / 8);
			let mut sum = 0u64;
			for row in y0..y1 {
				sum += y[row * w + x0..row * w + x1].iter().map(|&v| v as u64).sum::<u64>();
			}
			let n = ((x1 - x0) * (y1 - y0)).max(1) as u64;
			s.push_str(&format!("{:02x}", (sum / n) as u8));
		}
	}
	s
}

impl SyncLog {
	/// Opens (appends to) the file named by RAC_FMV_SYNC_LOG, or None when the variable is not set; `what` heads the movie's section.
	pub fn open(what: &str) -> Option<SyncLog> {
		let path = std::env::var("RAC_FMV_SYNC_LOG").ok().filter(|p| !p.is_empty())?;
		let f = std::fs::OpenOptions::new().create(true).append(true).open(path).ok()?;
		let mut out = std::io::BufWriter::new(f);
		let _ = writeln!(out, "# movie {what}");
		let _ = writeln!(out, "t_ms,played,pulls,abuf,vq,clock,shown,lag_ms,dev_ms,mode,tref,ct,thumb,rff,comp_ms");
		Some(SyncLog { out, t0: Instant::now() })
	}

	pub fn row(&mut self, r: &Row) {
		let t = self.t0.elapsed().as_secs_f64() * 1000.0;
		let (shown, lag) = match r.shown {
			Some(p) => (p.to_string(), format!("{:.1}", (r.clock as f64 - p as f64) / 90.0)),
			None => (String::new(), String::new()),
		};
		let fr = r.frame.as_ref().map_or(",,,".to_string(), |(t, c, y, rff)| format!("{t},{c},{y},{}", *rff as u8));
		let _ = writeln!(self.out, "{t:.1},{},{},{},{},{},{shown},{lag},{:.1},{},{fr},{:.1}", r.played, r.pulls, r.abuf, r.vq, r.clock, r.dev_ms, r.mode, r.comp_ms);
	}
}

impl Drop for SyncLog {
	fn drop(&mut self) {
		let _ = self.out.flush();
	}
}

/// [PC] Test-only delays that force the conditions a player's PC produces (off unless set; ENV.md):
/// `RAC_FMV_TEST_STALL=<start_ms>:<len_ms>[,...]` blocks the polling thread for `len_ms` once the movie has run `start_ms` (a render hitch, a dragged
/// window, a slow present); `RAC_FMV_TEST_DECODE_MS=<n>` adds `n` ms to every video packet the worker decodes (a slow CPU or disc).
pub struct Knobs {
	stalls: Vec<(u128, u64)>,
	next: usize,
}

impl Knobs {
	pub fn from_env() -> Knobs {
		let mut stalls: Vec<(u128, u64)> = std::env::var("RAC_FMV_TEST_STALL")
			.ok()
			.map(|v| v.split(',').filter_map(|p| p.split_once(':')).filter_map(|(a, b)| Some((a.trim().parse().ok()?, b.trim().parse().ok()?))).collect())
			.unwrap_or_default();
		stalls.sort();
		Knobs { stalls, next: 0 }
	}

	/// No delays (the test rig).
	pub fn off() -> Knobs {
		Knobs { stalls: Vec::new(), next: 0 }
	}

	/// Called at the top of every poll: sleeps once when the next stall is due.
	pub fn stall(&mut self, since: Instant) {
		if let Some(&(at, len)) = self.stalls.get(self.next) {
			if since.elapsed().as_millis() >= at {
				self.next += 1;
				eprintln!("fmv: TEST stall of the polling thread, {len} ms (RAC_FMV_TEST_STALL)");
				std::thread::sleep(std::time::Duration::from_millis(len));
			}
		}
	}
}

/// RAC_FMV_TEST_DECODE_MS (0 = off).
pub fn test_decode_ms() -> u64 {
	std::env::var("RAC_FMV_TEST_DECODE_MS").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(0)
}
