// SPDX-License-Identifier: GPL-3.0-only
//! Optional, language-independent external FMV frame filter. No NVIDIA SDK is linked into the game.
use super::mpeg2::Frame;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

#[derive(Clone)]
struct Config {
	executable: PathBuf,
	scale: u32,
	quality: u32,
}
fn config() -> Option<Config> {
	if !cfg!(windows) || std::env::var("RAC_OMNI").ok().as_deref() == Some("0") {
		return None;
	}
	let force = std::env::var("RAC_RTX_VIDEO").unwrap_or_default();
	if force == "0" {
		return None;
	}
	let root = crate::texpack::mods::mods_root()?;
	let active = std::fs::read_to_string(root.join("active.txt")).unwrap_or_default();
	if force != "1" && !mod_enabled(&active) {
		return None;
	}
	let dir = root.join("rac-rtx-video");
	let text = std::fs::read_to_string(dir.join("rtx-video.ini")).unwrap_or_default();
	let setting = |key: &str, default: u32| {
		text.lines()
			.filter_map(|s| s.split_once('='))
			.find(|(k, _)| k.trim() == key)
			.and_then(|(_, v)| v.trim().parse().ok())
			.unwrap_or(default)
	};
	// Read once per movie, never on the render path. Game/launcher controls persist here.
	let doc = crate::omni::plugins::read_settings(&root);
	let value = |key: &str, default: u32| {
		crate::omni::plugins::value_of(&doc, "rac-rtx-video", key)
			.and_then(|v| v.as_num())
			.filter(|n| n.is_finite() && n.fract() == 0. && *n >= 1. && *n <= 4.)
			.map(|n| n as u32)
			.unwrap_or_else(|| setting(key, default).clamp(1, 4))
	};
	let scale = value("scale", 4);
	let quality = value("quality", 4);
	Some(Config {
		executable: dir.join("rac-rtx-video-worker.exe"),
		scale,
		quality,
	})
}
fn mod_enabled(text: &str) -> bool {
	use crate::texpack::active::{self, Master};
	let parsed = active::parse(text, &|id| (id == "rac-rtx-video").then_some(Master::Mods));
	parsed.masters.is_on(Master::Mods)
		&& parsed
			.ids
			.iter()
			.any(|id| id.split('#').next().unwrap_or("").trim() == "rac-rtx-video")
}
fn valid_input(f: &Frame, w: u32, h: u32) -> bool {
	let n = w as usize * h as usize;
	f.width == w && f.height == h && f.y.len() == n && f.cb.len() == n / 4 && f.cr.len() == n / 4
}
fn word(r: &mut impl Read) -> Result<u32, String> {
	let mut b = [0; 4];
	r.read_exact(&mut b).map_err(|e| e.to_string())?;
	Ok(u32::from_le_bytes(b))
}
fn marker(r: &mut impl Read) -> Result<[u8; 4], String> {
	let mut m = [0; 4];
	r.read_exact(&mut m).map_err(|e| e.to_string())?;
	if &m == b"FAIL" {
		let n = word(r)? as usize;
		if n > 4096 {
			return Err("oversized helper error".into());
		}
		let mut b = vec![0; n];
		r.read_exact(&mut b).map_err(|e| e.to_string())?;
		return Err(String::from_utf8_lossy(&b).into_owned());
	}
	Ok(m)
}

struct Session {
	tx: Option<mpsc::SyncSender<Frame>>,
	rx: mpsc::Receiver<Result<Frame, String>>,
	child: Arc<Mutex<Child>>,
	first: bool,
}
impl Session {
	fn start(cfg: &Config, w: u32, h: u32) -> Result<Self, String> {
		if w < 2
			|| h < 2 || w > 4096
			|| h > 4096
			|| (w | h) & 1 != 0
			|| w * cfg.scale > 4096
			|| h * cfg.scale > 4096
		{
			return Err("unsupported video dimensions".into());
		}
		let mut cmd = Command::new(&cfg.executable);
		cmd.stdin(Stdio::piped())
			.stdout(Stdio::piped())
			.stderr(Stdio::inherit());
		#[cfg(windows)]
		{
			use std::os::windows::process::CommandExt;
			cmd.creation_flags(0x08000000);
		}
		let mut child = cmd
			.spawn()
			.map_err(|e| format!("{}: {e}", cfg.executable.display()))?;
		let mut input = child.stdin.take().ok_or("no helper input")?;
		let mut output = child.stdout.take().ok_or("no helper output")?;
		let child = Arc::new(Mutex::new(child));
		let owned = child.clone();
		let cfg = cfg.clone();
		let (tx, requests) = mpsc::sync_channel::<Frame>(1);
		let (answers, rx) = mpsc::channel();
		std::thread::spawn(move || {
			let mut run = || -> Result<(), String> {
				input.write_all(b"RRV1").map_err(|e| e.to_string())?;
				for n in [w, h, cfg.scale, cfg.quality] {
					input
						.write_all(&n.to_le_bytes())
						.map_err(|e| e.to_string())?
				}
				input.flush().map_err(|e| e.to_string())?;
				if marker(&mut output)? != *b"OKAY" {
					return Err("unsupported helper protocol".into());
				}
				let trace = std::env::var_os("RAC_RTX_TRACE").is_some();
				let mut sequence = 0usize;
				while let Ok(mut frame) = requests.recv() {
					if !valid_input(&frame, w, h) {
						return Err("frame dimensions or plane lengths changed".into());
					}
					if trace && sequence < 4 {
						eprintln!(
							"rtx-video trace input {sequence}: {}x{}, planes {}/{}/{}",
							frame.width,
							frame.height,
							frame.y.len(),
							frame.cb.len(),
							frame.cr.len()
						);
					}
					for plane in [&frame.y, &frame.cb, &frame.cr] {
						input.write_all(plane).map_err(|e| e.to_string())?
					}
					input.flush().map_err(|e| e.to_string())?;
					if trace && sequence < 4 {
						eprintln!("rtx-video trace input {sequence} sent; waiting for reply");
					}
					if marker(&mut output)? != *b"FRAM" {
						return Err("invalid frame reply".into());
					}
					let ow = word(&mut output)?;
					let oh = word(&mut output)?;
					let _worker_us = word(&mut output)?;
					if ow != w * cfg.scale || oh != h * cfg.scale {
						return Err("unexpected output dimensions".into());
					}
					let count = ow as usize * oh as usize;
					frame.y.resize(count, 0);
					frame.cb.resize(count / 4, 0);
					frame.cr.resize(count / 4, 0);
					for plane in [&mut frame.y, &mut frame.cb, &mut frame.cr] {
						output.read_exact(plane).map_err(|e| e.to_string())?
					}
					frame.width = ow;
					frame.height = oh;
					if trace && sequence < 4 {
						eprintln!(
							"rtx-video trace output {sequence}: {ow}x{oh}, worker {_worker_us} us"
						);
					}
					sequence += 1;
					if answers.send(Ok(frame)).is_err() {
						break;
					}
				}
				Ok(())
			};
			if let Err(e) = run() {
				let _ = answers.send(Err(e));
			}
			drop(input);
			drop(output);
			// Do not hold the child mutex across a blocking wait: cancellation must be
			// able to kill a helper stuck in the driver or a malformed reply.
			let deadline = Instant::now() + Duration::from_secs(2);
			loop {
				let Ok(mut child) = owned.lock() else { break };
				match child.try_wait() {
					Ok(Some(_)) | Err(_) => break,
					Ok(None) if Instant::now() >= deadline => {
						let _ = child.kill();
						let _ = child.wait();
						break;
					}
					Ok(None) => {}
				}
				drop(child);
				std::thread::sleep(Duration::from_millis(10));
			}
		});
		Ok(Self {
			tx: Some(tx),
			rx,
			child,
			first: true,
		})
	}
	fn kill(&self) {
		if let Ok(mut child) = self.child.lock() {
			let _ = child.kill();
		}
	}
	fn process(&mut self, f: &Frame, stop: &AtomicBool) -> Result<Frame, String> {
		self.tx
			.as_ref()
			.ok_or("helper closed")?
			.send(f.clone())
			.map_err(|_| "helper input disconnected")?;
		let deadline = Instant::now() + Duration::from_secs(if self.first { 30 } else { 3 });
		loop {
			if stop.load(Ordering::Relaxed) {
				self.kill();
				return Err("movie cancelled".into());
			}
			match self.rx.recv_timeout(Duration::from_millis(50)) {
				Ok(result) => {
					self.first = false;
					return result;
				}
				Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
				Err(e) => {
					self.kill();
					return Err(format!("helper timeout/disconnect: {e}"));
				}
			}
		}
	}
}
impl Drop for Session {
	fn drop(&mut self) {
		self.tx.take();
	}
}

pub(super) struct Filter {
	cfg: Option<Config>,
	session: Option<Session>,
	failed: bool,
	frames: u64,
	total_ms: f64,
	max_ms: f64,
}
impl Filter {
	pub fn new() -> Self {
		Self {
			cfg: config(),
			session: None,
			failed: false,
			frames: 0,
			total_ms: 0.,
			max_ms: 0.,
		}
	}
	pub fn prime_frames(&self) -> u64 {
		if self.frames > 0 && !self.failed {
			12
		} else {
			3
		}
	}
	pub fn apply(&mut self, frame: &mut Frame, stop: &AtomicBool) {
		let Some(cfg) = self.cfg.clone().filter(|_| !self.failed) else {
			return;
		};
		let start = Instant::now();
		let result = (|| {
			if self.session.is_none() {
				self.session = Some(Session::start(&cfg, frame.width, frame.height)?)
			}
			self.session.as_mut().unwrap().process(frame, stop)
		})();
		match result {
			Ok(enhanced) => {
				if self.frames == 0 {
					eprintln!(
						"rtx-video: active {}x{} -> {}x{}, quality {}; original audio and PTS retained",
						frame.width, frame.height, enhanced.width, enhanced.height, cfg.quality
					);
				} else {
					let ms = start.elapsed().as_secs_f64() * 1000.;
					self.total_ms += ms;
					self.max_ms = self.max_ms.max(ms);
				}
				*frame = enhanced;
				self.frames += 1;
			}
			Err(e) => {
				if !stop.load(Ordering::Relaxed) {
					eprintln!("rtx-video: {e}; original video fallback");
				}
				self.failed = true;
				if let Some(s) = self.session.take() {
					s.kill();
				}
			}
		}
	}
}
impl Drop for Filter {
	fn drop(&mut self) {
		if self.frames > 1 {
			eprintln!(
				"rtx-video: {} frames; host round trip mean {:.3} ms, max {:.3} ms (excluding model startup)",
				self.frames,
				self.total_ms / (self.frames - 1) as f64,
				self.max_ms
			);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	fn frame() -> Frame {
		Frame {
			width: 4,
			height: 2,
			y: vec![16; 8],
			cb: vec![128; 2],
			cr: vec![128; 2],
			pts: Some(12345),
			picture_structure: 3,
			top_field_first: true,
			progressive: false,
			repeat_first_field: true,
			aspect_ratio_information: 2,
			frame_rate_code: 3,
			temporal_reference: 17,
			coding_type: 1,
		}
	}
	#[test]
	fn active_switch_respects_comments_bom_and_master() {
		assert!(mod_enabled("\u{feff}# enabled\r\nrac-rtx-video # VSR\r\n"));
		assert!(!mod_enabled("# rac-rtx-video\n"));
		assert!(!mod_enabled("# Load mods: off\nrac-rtx-video\n"));
		assert!(mod_enabled("# Load texture packs: off\nrac-rtx-video\n"));
		assert!(!mod_enabled(""));
	}
	#[test]
	fn helper_errors_are_bounded_and_truncation_is_reported() {
		assert_eq!(
			marker(&mut b"FAIL\x03\0\0\0bad".as_slice()).unwrap_err(),
			"bad"
		);
		assert_eq!(
			marker(&mut b"FAIL\xff\xff\xff\xff".as_slice()).unwrap_err(),
			"oversized helper error"
		);
		assert!(marker(&mut b"FAIL\x04\0\0\0bad".as_slice()).is_err());
		assert!(marker(&mut b"FRA".as_slice()).is_err());
	}
	#[test]
	fn invalid_or_changed_planes_cannot_desynchronize_the_pipe() {
		let mut f = frame();
		assert!(valid_input(&f, 4, 2));
		assert!(!valid_input(&f, 2, 4));
		f.cr.pop();
		assert!(!valid_input(&f, 4, 2));
	}
	#[test]
	fn missing_helper_keeps_original_pixels_and_all_frame_metadata() {
		let mut f = frame();
		let before = format!("{f:?}");
		let mut filter = Filter {
			cfg: Some(Config {
				executable: std::env::temp_dir().join("rac-rtx-video-missing-test-helper.exe"),
				scale: 4,
				quality: 4,
			}),
			session: None,
			failed: false,
			frames: 0,
			total_ms: 0.,
			max_ms: 0.,
		};
		filter.apply(&mut f, &AtomicBool::new(false));
		assert_eq!(format!("{f:?}"), before);
		assert!(filter.failed);
		assert_eq!(filter.prime_frames(), 3);
		filter.apply(&mut f, &AtomicBool::new(false));
		assert_eq!(format!("{f:?}"), before);
	}
	#[test]
	fn disabled_filter_does_no_processing() {
		let mut f = frame();
		let before = format!("{f:?}");
		let mut filter = Filter {
			cfg: None,
			session: None,
			failed: false,
			frames: 0,
			total_ms: 0.,
			max_ms: 0.,
		};
		filter.apply(&mut f, &AtomicBool::new(false));
		assert_eq!(format!("{f:?}"), before);
		assert_eq!(filter.prime_frames(), 3);
		filter.frames = 1;
		assert_eq!(filter.prime_frames(), 12);
		filter.failed = true;
		assert_eq!(filter.prime_frames(), 3);
	}
}
