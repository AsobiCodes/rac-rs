//! The GS threads of a game whose world the engine draws (R&C2; from lane rc2's window): like `pipe::start` (a VIF1 + VU1
//! stage, then the software GS and the CRTC read), with the HUD overlay mode per field (overlay.rs): an overlay field runs
//! only the effect programs and the HUD, and hands out the see-through HUD picture and the effects' depth, which the host lays
//! over the engine's world picture.
use crate::gscap::{self, Hd};
use crate::gsfull::crtc::{Crtc, present, present_overlay};
use crate::gsfull::gs::Gs;
use crate::gsfull::overlay::{OvlField, raster};
use crate::gsfull::pipe::{Pipe, threads};
use ee_oracle::gs::Job;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Mutex};

/// One EE field: its number (the host's count, for `GsPipe::done`), its graphics jobs, the CRTC registers at its end, and
/// whether the GS draws it as a HUD overlay.
pub struct FieldIn {
	pub field: u64,
	pub jobs: Vec<Job>,
	pub crtc: Crtc,
	pub overlay: bool,
}

/// One presented picture: the shown buffer, or (overlay) the premultiplied HUD and per pixel the effects' view depth in
/// metres (0 = always in front; empty = none).
pub struct Frame {
	pub w: u32,
	pub h: u32,
	pub rgba: Vec<u8>,
	pub overlay: bool,
	pub depth: Vec<f32>,
	/// The HUD again at the window's resolution (the GPU capture, gscap/rc2.rs; RGBA8, alpha = how much of the picture below shows
	/// through): `rgba` then holds the effects only. None: `rgba` has the HUD too.
	pub hd: Option<Hd>,
}

#[derive(Default)]
pub struct GsPipe {
	slot: Mutex<Option<Frame>>,
	/// The number of the newest field both stages are through (its picture, if it drew one, is in the slot or was taken).
	pub done: AtomicU64,
	/// A thread ended (a panic or the channel closed).
	pub dead: AtomicBool,
}

impl GsPipe {
	/// The newest finished picture (each is handed out once).
	pub fn take(&self) -> Option<Frame> {
		self.slot.lock().ok()?.take()
	}
}

/// Marks the shared state dead when a stage's thread ends.
struct Flag(Arc<GsPipe>);
impl Drop for Flag {
	fn drop(&mut self) {
		self.0.dead.store(true, Ordering::SeqCst);
	}
}

/// Starts the two stages (cold boot: blank GS local memory). The sender takes one `FieldIn` per EE field; one with no jobs
/// draws nothing. `sharp`: the GPU capture draws the HUD of overlay fields again at the window's resolution (`Frame::hd`; the
/// caller has called `gscap::gpu_expect_window` before, and `gscap::set_hd_target` follows the window every redraw).
pub fn start(sharp: bool) -> Option<(SyncSender<FieldIn>, Arc<GsPipe>)> {
	let mut pipe = Pipe::new("")?;
	let shared = Arc::new(GsPipe::default());
	let (tx, rx) = sync_channel::<FieldIn>(1);
	let (ftx, frx) = sync_channel::<(Option<OvlField>, bool, u64)>(1);
	let sh = shared.clone();
	std::thread::Builder::new()
		.name("gsfull-vu1".into())
		.stack_size(64 << 20)
		.spawn(move || {
			let _flag = Flag(sh);
			while let Ok(inp) = rx.recv() {
				// A field with no jobs draws nothing; it still passes the GS stage for `done`.
				let f = (!inp.jobs.is_empty()).then(|| pipe.process_rc2(inp.jobs, inp.crtc, inp.overlay));
				if ftx.send((f, inp.overlay, inp.field)).is_err() {
					break;
				}
			}
		})
		.ok()?;
	let sh = shared.clone();
	std::thread::Builder::new()
		.name("gsfull-gs".into())
		.stack_size(64 << 20)
		.spawn(move || {
			let _flag = Flag(sh.clone());
			let mut g = Gs::new(vec![0u8; crate::gsfull::gs_mem::VRAM_SIZE], threads());
			// The GPU capture's twin of local memory (gscap/rc2.rs): fed every field once it is attached, dropped when there is no GPU for it.
			let mut cap = sharp.then(|| gscap::Gs::new(vec![0u8; crate::gsfull::gs_mem::VRAM_SIZE]));
			// Rows of the HUD buffer (its scissor) in the last overlay field: the sharp scale of the next one.
			let mut rows = 448;
			// The sharp scale follows these rows from the first overlay field on (before it, `set_hd_target` assumes the PAL frame's 512).
			if sharp {
				gscap::hd_expect_rows(rows);
			}
			while let Ok((f, overlay, field)) = frx.recv() {
				let Some(f) = f else {
					sh.done.store(field, Ordering::SeqCst);
					continue;
				};
				if cap.as_ref().is_some_and(|c| !c.rc2_live()) {
					cap = None;
				}
				let sharp = overlay && cap.as_ref().is_some_and(|c| c.rc2_hd_ready(rows));
				g.set_hud_skip(sharp);
				// The capture takes the field on its own thread beside the software GS (same qwords; neither changes what the
				// other draws), so its CPU work and the GPU round trip hide behind the software raster. No thread: in turn.
				let r = std::thread::scope(|s| {
					let c = cap.as_mut().map(|c| std::thread::Builder::new().name("gsfull-cap".into()).stack_size(16 << 20).spawn_scoped(s, || c.rc2_field(&f, overlay, rows, sharp)));
					raster(&mut g, &f, overlay);
					crate::gsfull::texreq::serve(&mut g);
					c.map(|j| j.map(|j| j.join()))
				});
				let hd = match r {
					Some(Ok(Ok(hd))) => hd,
					// The capture panicked: the software GS draws the HUD again from the next field on.
					Some(Ok(Err(_))) => {
						cap = None;
						None
					}
					_ => cap.as_mut().and_then(|c| c.rc2_field(&f, overlay, rows, sharp)),
				};
				let crtc = f.f.crtc;
				let frame = if overlay {
					let (p, depth) = present_overlay(&mut g, &crtc);
					let (cw, ch) = g.overlay_clip();
					if cw > 0 && ch > 0 {
						// 512 GS pixels wide is what the sharp scale assumes (anything else: no sharp picture).
						rows = if cw == 512 { ch.clamp(64, 512) } else { 0 };
						if rows != 0 {
							gscap::hd_expect_rows(rows);
						}
					}
					Frame { w: p.w, h: p.h, rgba: p.rgba, overlay, depth, hd }
				} else {
					let p = present(&mut g, &crtc);
					Frame { w: p.w, h: p.h, rgba: p.rgba, overlay, depth: Vec::new(), hd: None }
				};
				if let Ok(mut s) = sh.slot.lock() {
					*s = Some(frame);
				}
				sh.done.store(field, Ordering::SeqCst);
			}
		})
		.ok()?;
	Some((tx, shared))
}
