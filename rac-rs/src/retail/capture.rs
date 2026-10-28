//! The GS capture on its own thread (`Capture`, `CapTake`, `capture_gs_vram`) and the overlay pictures it hands over
//! (`ov_px`, the sharp-HUD window height).

use super::*;

/// The GS capture (oracle gs.rs) on a thread of its own, as `Machine::capture_gs_async` runs it, plus a count of the
/// jobs it has not run yet. The capture needs about 38 ms per field while a pause menu is up (the game restores the
/// grabbed frame into the main buffer every field) but the interpreter needs 1 ms, so left alone the queue grows by
/// 12 fields per second of menu time and the picture shows the menu seconds late: the menu looked dead (2026-10-08).
/// `pace` makes the interpreter thread wait for the capture instead.
pub(super) struct Capture {
	/// Finished overlays, each tagged with the field whose jobs published it (1-based, the worker's `field`).
	pub(super) slot: Arc<Mutex<std::collections::VecDeque<(u64, gscap::Overlay)>>>,
	/// Field-end markers the capture has run (= the last field whose jobs are all drawn).
	pub(super) ends: Arc<AtomicU64>,
	/// Jobs the interpreter has queued / the capture has run.
	queued: Arc<AtomicU64>,
	pub(super) ran: Arc<AtomicU64>,
	/// The capture thread ended (a panic): nothing to wait for.
	pub(super) dead: Arc<AtomicBool>,
	pub(super) lag: usize,
	/// The sender the machine's `gsq` holds while the capture is fed (the field-end markers go through it even while the
	/// front end's GS-full pipe takes the jobs, title_boot.rs).
	pub(super) tx: Option<std::sync::mpsc::Sender<ee_oracle::gs::Job>>,
}

/// How many fields the capture may trail the interpreter by (RAC_RETAIL_GS_LAG=N; 0 = never wait, the old behaviour).
const GS_LAG: usize = 2;

/// The longest the interpreter waits for the capture in one field, ms (a longer wait gives up and resyncs).
const GS_WAIT_MS: u128 = 500;

/// The size of the picture `ov_px` hands over: the sharp HUD's (lane G: the GPU capture drew the overlay again at the window's
/// resolution) or the 512-wide overlay's. Asked before `ov_px`, which takes the pixels.
pub(super) fn ov_dims(o: &gscap::Overlay) -> (u32, u32) {
	o.hd.as_ref().map_or((o.w, o.rows), |h| (h.w, h.rows))
}

/// Rows (flags 1 = right, 2 = left; one entry per row of the overlay, 448 in play) of the last overlay where a GS primitive crosses the scissor edge (widescreen presenter).
pub fn edge_cut_rows() -> Option<Vec<u8>> {
	super::gscap::edge_cut::published()
}

/// What the HUD position's top corner cells must know about an overlay picture (a top help prompt, the hoverboard race's bars, the Gadgetron
/// title letters; `gscap::edge_cut::guard`). It is measured with each picture and travels with it (`Retail::take_overlay`).
pub use super::gscap::edge_cut::guard::Guard as HudGuard;
/// A corner widget's reach inside a `HudGuard` (hud_place.rs tests build guards with it).
#[cfg(test)]
pub use super::gscap::edge_cut::guard::Reach as HudReach;

/// The inner edge of a corner cell and the rows the cells reach at most, as the capture's guard measures them (hud_place.rs keeps `CELL_W` and
/// `CELL_H` equal; tests).
#[cfg(test)]
pub fn gscap_guard_cells() -> (f32, f32) {
	(super::gscap::edge_cut::guard::GUARD_COL, super::gscap::edge_cut::guard::CELL_ROWS)
}

/// The sharp HUD for the window the viewer draws in (`window_h` pixels high), every frame from the render thread: on while the
/// Graphics row HUD is Sharp (`pcext::hud_sharp_now`, pc_options.txt `hud=`), off with RAC_HUD=original or RAC_PC_EXT=0. The GPU
/// capture then draws the overlay a second time at the window's resolution (gscap/draw/gpu.rs, gpu.wgsl k_hd); the 512 picture
/// stays what the game's retail picture is.
pub fn set_hud_target(window_h: u32) {
	static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	let off = *OFF.get_or_init(|| !pcext::enabled() || std::env::var("RAC_HUD").is_ok_and(|v| v.eq_ignore_ascii_case("original")));
	gscap::set_hd_target(window_h, !off && pcext::hud_sharp_now());
}

/// The sharp title / main menu (viewer/hudsharp.rs) for a window `window_h` pixels high: the front end's art quads take the edge-aware
/// sampler. The same switches as `set_hud_target` (Graphics row HUD Sharp; off with RAC_HUD=original or RAC_PC_EXT=0), and the window
/// at least `HD_MIN_PX_PER_ROW` pixels per row of the 448-row menu screen (896 px; below it the nearest picture is as before).
pub fn ui_sharp(window_h: u32) -> bool {
	static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	let off = *OFF.get_or_init(|| !pcext::enabled() || std::env::var("RAC_HUD").is_ok_and(|v| v.eq_ignore_ascii_case("original")));
	!off && pcext::hud_sharp_now() && window_h as f32 >= 448.0 * gscap::HD_MIN_PX_PER_ROW
}

/// An overlay's pixels as RGBA8.
pub(super) fn ov_px(o: &mut gscap::Overlay) -> Vec<[u8; 4]> {
	if let Some(h) = o.hd.take() {
		return h.rgba;
	}
	if !o.rgba.is_empty() {
		// The CPU capture converted the picture itself (gscap `Gs::publish`).
		return std::mem::take(&mut o.rgba);
	}
	o.px[..(o.w * o.rows) as usize].iter().map(|p| p.map(|c| (c * 255.0 + 0.5) as u8)).collect()
}

pub(super) fn capture_gs(m: &mut ee_oracle::Machine, dir: &str) -> Option<Capture> {
	capture_gs_vram(m, ee_oracle::gs::Gs::load_vram(&format!("{dir}/GS.bin"))?, dir)
}

/// Pages of the page-menu engine (level overlay) whose render-to-texture panels hold actors drawn through VU1 (path 1),
/// which the capture draws only in P1 mode (gscap `P1`): the post-game page (id 0x22) with the green robot (class 1957), and
/// the pause pages with 3D previews (decomp/notes/ingame_pause_menu.md "Item previews"): Weapons 3 (Ratchet with the weapon,
/// the weapon, its ammo), Gadgets 4 (Ratchet), Quick Select 5 (the selected weapon), Items 6 (the gold bolt), Weapons help
/// 0x18 (weapon, ammo) and Gadgets help 0x19. Checked against --gsfull; the other pages draw no model (their pictures are
/// plain textures the capture already shows).
pub(super) fn rtt_page(s: &Snap) -> bool {
	s.menu.as_ref().is_some_and(|m| !m.front && matches!(m.id, 3 | 4 | 5 | 6 | 0x18 | 0x19 | 0x22))
}

/// The capture thread on a given 4 MB VRAM (a cold boot has no GS.bin: empty VRAM, attached from the first field).
pub(super) fn capture_gs_vram(m: &mut ee_oracle::Machine, vram: Vec<u8>, vu_dir: &str) -> Option<Capture> {
	let mut c = Capture { tx: None, slot: Arc::new(Mutex::new(Default::default())), ends: Arc::new(AtomicU64::new(0)), queued: Arc::new(AtomicU64::new(0)), ran: Arc::new(AtomicU64::new(0)), dead: Arc::new(AtomicBool::new(false)), lag: std::env::var("RAC_RETAIL_GS_LAG").ok().and_then(|v| v.parse().ok()).unwrap_or(GS_LAG) };
	// The machine's sender feeds a relay that counts the jobs, then the capture thread.
	let (tx, rx) = std::sync::mpsc::channel::<ee_oracle::gs::Job>();
	let (tx2, rx2) = std::sync::mpsc::channel::<ee_oracle::gs::Job>();
	let (queued, ran, dead2, out, ends) = (c.queued.clone(), c.ran.clone(), c.dead.clone(), c.slot.clone(), c.ends.clone());
	let vu_dir = vu_dir.to_string();
	std::thread::Builder::new()
		.name("gs-relay".into())
		.spawn(move || {
			while let Ok(j) = rx.recv() {
				queued.fetch_add(1, Ordering::SeqCst);
				if tx2.send(j).is_err() {
					break;
				}
			}
		})
		.ok()?;
	std::thread::Builder::new()
		.name("gs".into())
		.spawn(move || {
			struct Flag(Arc<AtomicBool>);
			impl Drop for Flag {
				fn drop(&mut self) {
					self.0.store(true, Ordering::SeqCst);
				}
			}
			let _flag = Flag(dead2);
			hot::thread();
			// rac-rs's multi-threaded copy of the oracle capture (retail/gscap.rs): same pixels, menus 4x faster.
			let mut g = gscap::Gs::new(vram);
			g.set_vu1_dir(&vu_dir);
			// RAC_GSCAP_OVHASH=1 (debug): one line per published overlay with an FNV hash of its pixels (GPU against CPU path, old against new kernels).
			let ovhash = std::env::var_os("RAC_GSCAP_OVHASH").is_some();
			// RAC_GSCAP_OVHASH8=1 (debug): one line per published overlay with an FNV hash of the RGBA8 picture the main thread gets.
			let ovhash8 = std::env::var_os("RAC_GSCAP_OVHASH8").is_some();
			let mut ovn = 0u64;
			// [PC] (speed) Lazy readback (gscap `set_lazy`): a published overlay is collected while the next job records, and `ends`
			// (END jobs whose overlays are all out) waits for it. RAC_GS_ASYNC=0 switches it off.
			let lazy = std::env::var("RAC_GS_ASYNC").map(|v| v != "0").unwrap_or(true);
			g.set_lazy(lazy);
			let mut ends_seen = 0u64;
			let mut put = |tag: u64, mut o: gscap::Overlay| {
				if ovhash {
					let h = o.px.iter().flatten().fold(0xcbf2_9ce4_8422_2325u64, |h, v| (h ^ v.to_bits() as u64).wrapping_mul(0x0100_0000_01b3));
					// With the sharp HUD (GPU capture, hd pass) the line ends with the hash of that picture too (Vulkan against DX12 parity).
					let hd = o.hd.as_ref().map_or(String::new(), |d| format!(" hd {:016x}", d.rgba.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, p| (h ^ u32::from_le_bytes(*p) as u64).wrapping_mul(0x0100_0000_01b3))));
					eprintln!("ovhash {ovn} {h:016x} rows {} frozen {}{hd}", o.rows, o.frozen);
				}
				if ovhash8 {
					// The sharp HUD (lane G) keeps its own picture: the hash is of the 512 picture then, and `hd` stays for the viewer.
					let hd = o.hd.take();
					let px8 = ov_px(&mut o);
					let h = px8.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, p| (h ^ u32::from_le_bytes(*p) as u64).wrapping_mul(0x0100_0000_01b3));
					eprintln!("ovhash8 {ovn} {h:016x} rows {} frozen {} disp {}", o.rows, o.frozen, o.disp);
					o.rgba = px8;
					o.hd = hd;
				}
				if ovhash || ovhash8 {
					ovn += 1;
				}
				if let Ok(mut s) = out.lock() {
					s.push_back((tag, o));
				}
			};
			loop {
				let j = match rx2.try_recv() {
					Ok(j) => j,
					Err(std::sync::mpsc::TryRecvError::Empty) => {
						// Nothing queued: the picture in flight is collected now (the main thread may be waiting for it).
						g.gpu_finish(true);
						while let Some((tag, o)) = g.ready.pop_front() {
							put(tag, o);
						}
						ends.store(g.pending_tag().map_or(ends_seen, |t| (t - 1).min(ends_seen)), Ordering::SeqCst);
						match rx2.recv() {
							Ok(j) => j,
							Err(_) => break,
						}
					}
					Err(_) => break,
				};
				let end = matches!(&j, ee_oracle::gs::Job::Gif(q, u64::MAX) if q.is_empty());
				let tag0 = ends_seen + 1;
				g.run_oracle(j);
				if end {
					ends_seen += 1;
				}
				g.gpu_finish(false);
				while let Some((tag, o)) = g.ready.pop_front() {
					put(tag, o);
				}
				if let Some(o) = g.done.take() {
					put(tag0, o);
				}
				ends.store(g.pending_tag().map_or(ends_seen, |t| (t - 1).min(ends_seen)), Ordering::SeqCst);
				ran.fetch_add(1, Ordering::SeqCst);
			}
			// The stream ended: the last picture still being read back goes out first.
			g.gpu_finish(true);
			while let Some((tag, o)) = g.ready.pop_front() {
				put(tag, o);
			}
			ends.store(ends_seen, Ordering::SeqCst);
		})
		.ok()?;
	c.tx = Some(tx.clone());
	m.cpu.mem.gsq = Some(tx);
	Some(c)
}

impl Capture {
	/// After a field: waits until the capture has run every job of the field `GS_LAG` fields back.
	pub(super) fn pace(&self, marks: &mut std::collections::VecDeque<u64>) {
		if self.lag == 0 {
			return;
		}
		marks.push_back(self.queued.load(Ordering::SeqCst));
		if marks.len() <= self.lag {
			return;
		}
		let target = marks.pop_front().unwrap_or(0);
		let t = std::time::Instant::now();
		while self.ran.load(Ordering::SeqCst) < target && !self.dead.load(Ordering::SeqCst) {
			if t.elapsed().as_millis() > GS_WAIT_MS {
				eprintln!("retail: the GS capture did not finish within {GS_WAIT_MS} ms; not waiting for it");
				marks.clear();
				return;
			}
			std::thread::sleep(std::time::Duration::from_micros(300));
		}
		crate::viewer::framecsv::wait('G', t);
	}

	/// The overlay for snap `field`: waits until field `field - delay` is drawn, then hands out the newest overlay
	/// published up to that field (older ones of the same window are what the display showed before it).
	pub(super) fn take(&self, field: u64, delay: u64) -> Option<gscap::Overlay> {
		self.handle().take(field, delay)
	}

	/// The parts `take` needs, to run it on another thread.
	pub(super) fn handle(&self) -> CapTake {
		CapTake { ends: self.ends.clone(), dead: self.dead.clone(), slot: self.slot.clone() }
	}
}

/// `Capture::take`'s shared state (see `OvLate`).
pub(super) struct CapTake {
	pub(super) ends: Arc<AtomicU64>,
	pub(super) dead: Arc<AtomicBool>,
	pub(super) slot: Arc<Mutex<std::collections::VecDeque<(u64, gscap::Overlay)>>>,
}

impl CapTake {
	pub(super) fn take(&self, field: u64, delay: u64) -> Option<gscap::Overlay> {
		let want = field.saturating_sub(delay);
		let t = std::time::Instant::now();
		while self.ends.load(Ordering::SeqCst) < want && !self.dead.load(Ordering::SeqCst) && t.elapsed().as_millis() <= GS_WAIT_MS {
			std::thread::sleep(std::time::Duration::from_micros(100));
		}
		crate::viewer::framecsv::wait('T', t);
		let mut q = self.slot.lock().ok()?;
		let mut last = None;
		while q.front().is_some_and(|(f, _)| *f <= want) {
			last = q.pop_front().map(|(_, o)| o);
		}
		last
	}
}
