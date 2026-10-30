//! The main thread's field loop: `tick` hands the pad to the worker and takes the field it finished, the sound of the
//! field, the lookahead depth and the debug-shot field wait.

use super::*;

/// The sound of one field: the SPU2 register writes of the exact 989snd driver port (spumix.rs).
pub type SndBatch = crate::spumix::Batch;

/// The sound events the oracle made in the field it just ran.
pub fn take_sound_of(m: &mut ee_oracle::Machine) -> SndBatch {
	crate::spumix::drain(m)
}

/// [PC] The part of `Retail::lockstep` that the field snapshot decides: the front end, a level-load loop, and the views that draw no
/// world (`frozen_view`: the menus, and the ship's loading / warp substates), or RAC_RUN_AHEAD=0 (`off`). The modes that draw the
/// world (gameplay 0, scenes 2, the vendor 5, the ship's takeoff / landing / flight streams) are not in lockstep.
fn snap_lockstep(s: &Snap, off: bool) -> bool {
	off || s.front || s.frozen || s.turbo
}

/// [PC] Nanoseconds the render thread has waited for the capture thread's overlay (`ov_late`, in `resolve_ov`, usually from the
/// frame's HUD upload) and `tick` for the VU1 thread's effects layer (`fx_late`, only when nothing read it since the tick before:
/// viewer/interp/late.rs waits for it and counts that wait itself) since the viewer last read them (`take_late_waits`;
/// RAC_FRAME_PROFILE spans t_wov / t_wfx: an overlay wait shows in the next tick frame's t_wov, and as a `T` row in RAC_FRAME_CSV).
static WAIT_OV_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static WAIT_FX_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// [PC] The two late waits of `Retail::tick` in milliseconds (overlay, effects layer) since the last call; resets both.
pub fn take_late_waits() -> (f32, f32) {
	use std::sync::atomic::Ordering::Relaxed;
	(WAIT_OV_NS.swap(0, Relaxed) as f32 / 1e6, WAIT_FX_NS.swap(0, Relaxed) as f32 / 1e6)
}

impl Retail {
	/// One PAL field: takes the field the thread finished, then hands it this tick's pad.
	/// The sound requests of the field `tick` just took (each field's list is handed out once).
	pub fn take_sound(&mut self) -> SndBatch {
		let mut b = std::mem::take(&mut self.snap.sound);
		if b.attach.is_none() {
			b.attach = crate::spumix::ATTACH.lock().ok().and_then(|mut a| a.take());
		}
		b
	}



	pub fn tick(&mut self, buttons: u16, sticks: [u8; 4]) {
		// The overlay and the effects layer of the field the last tick took, if nothing has read them since (`resolve_ov`, `resolve_fx`).
		self.resolve_ov();
		let ns = self.resolve_fx();
		WAIT_FX_NS.fetch_add(ns, std::sync::atomic::Ordering::Relaxed);
		self.tick_inner(buttons, sticks);
		// The next field's pad is out (the worker runs it now). The capture thread's overlay of the field just taken is waited for
		// only when it is first read (`resolve_ov`): the frame's HUD upload in `frame_render`, or the next tick. The effects layer
		// of the field just taken (built on the VU1 thread meanwhile) likewise (`resolve_fx`): the frame's queue-A upload
		// (viewer/interp/late.rs `fx_late`), or the next tick. The render thread's work in between (hero, props, upload) overlaps
		// both waits.
	}

	/// [PC] The effects layer of the last field taken is still on the VU1 thread (`resolve_fx` would wait).
	pub fn fx_pending(&self) -> bool {
		self.snap.fx_late.is_some()
	}

	/// [PC] Waits for the VU1 thread's effects layer of the last field taken if it is still out (`Snap::fx_late`) and puts it on
	/// the end of `fx.qa` / `fx.pools` (after the ported quads). `fx()` does NOT wait: the viewer calls this (viewer/interp/late.rs
	/// `fx_late`) before its first read of `qa` / `qb` / `pools` in a frame (the upload phase) and before every tick, and `tick`
	/// calls it too. A new reader of those lists earlier in the frame (hero, props, HUD) must call `fx_late` first, or it sees
	/// them without the layer and unblended. Returns the wait in ns.
	pub fn resolve_fx(&mut self) -> u64 {
		let Some(l) = self.snap.fx_late.take() else {
			return 0;
		};
		let t_w = std::time::Instant::now();
		let w = crate::runlog::waiting("the effects (VU1) thread's layer of the last field");
		let got = l.get();
		drop(w);
		let ns = t_w.elapsed().as_nanos() as u64;
		efx_vu::prof_wait(ns);
		if let (Some((qa, pools)), Some(fx)) = (got, self.snap.fx.as_mut()) {
			fx.qa.extend(qa);
			fx.pools.extend(pools);
		}
		ns
	}

	/// [PC] Waits for the GS capture thread's overlay of the last field taken if it is still pending (`Snap::ov_late`) and stores
	/// it in `ov` / `ov_disp`. Every reader of those calls this first (`tick`, `present.rs`), so each sees what it saw when `tick`
	/// waited right after taking the field; only the moment the render thread blocks moved.
	pub(crate) fn resolve_ov(&mut self) {
		if let Some(l) = self.snap.ov_late.take() {
			let t_ov = std::time::Instant::now();
			let w = crate::runlog::waiting("the GS capture thread's overlay of the last field");
			let got = (l.0)();
			drop(w);
			WAIT_OV_NS.fetch_add(t_ov.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
			if let Some((w, rows, px, disp, guard)) = got {
				self.snap.ov = Some((w, rows, px, guard));
				self.snap.ov_disp = disp;
			}
		}
	}

	fn tick_inner(&mut self, buttons: u16, sticks: [u8; 4]) {
		if self.failed {
			return;
		}
		self.waited = 0.0;
		if self.inflight > 0 {
			let t_wait = std::time::Instant::now();
			let w = crate::runlog::waiting("the game thread's next field (retail)");
			// The game thread can park in the movie hook (fmv_bridge) in the middle of a field: wait for the field in
			// 2 ms steps and hand a movie request to the viewer instead of blocking; while the movie plays, return at once.
			let got = loop {
				if let Some(s) = self.early.take() {
					break Ok(s);
				}
				match self.rx.try_recv() {
					Ok(s) => break Ok(s),
					Err(std::sync::mpsc::TryRecvError::Disconnected) => break Err(()),
					Err(std::sync::mpsc::TryRecvError::Empty) => {}
				}
				if let Some(q) = fmv_bridge::take_request() {
					self.movie_req = Some(q);
					return;
				}
				if fmv_bridge::waiting() {
					return;
				}
				match self.rx.recv_timeout(std::time::Duration::from_millis(2)) {
					Ok(s) => break Ok(s),
					Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
					Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break Err(()),
				}
			};
			drop(w);
			self.waited = t_wait.elapsed().as_secs_f32() * 1000.0;
			match got {
				Ok(mut s) => {
					// Several ticks can run per frame: the travel pools wait for `switch`, the overlay for `take_overlay`.
					if s.pools.is_none() {
						s.pools = self.snap.pools.take();
					}
					if s.ov.is_none() {
						s.ov = self.snap.ov.take();
					}
					if let Some(e) = &s.err {
						eprintln!("retail: {e}");
						self.failed = true;
						self.stop = Some(e.clone());
					}
					self.ms = s.ms;
					self.ms_avg += (s.ms - self.ms_avg) * 0.125;
					self.pad_seen |= s.pad.iter().fold(0, |a, p| a | p);
					self.taken += 1;
					s.field = self.taken;
					// "Keep the last" (orbs / fx / shadows are None while frozen) is decided on the SHOWN snap: `native_delay` queues
					// 2 fields in modes != 0, so handing the old shown snap's pieces to the newest queued snap (as before) left the
					// shown one without them for 2 of every 3 fields (pause menu: the sea / water flickered every third field).
					let mut old = std::mem::take(&mut self.snap);
					self.snap = native_delay(s);
					if self.snap.orbs.is_none() {
						self.snap.orbs = old.orbs.take();
					}
					if self.snap.fx.is_none() {
						self.snap.fx = old.fx.take();
					}
					if self.snap.shadows.is_none() {
						self.snap.shadows = old.shadows.take();
					}
					// The front end replaced the level: whatever level comes next is a new one for rac-rs (`switch`), also the same number.
					if self.snap.front && self.level != u32::MAX {
						self.level = u32::MAX;
						self.unbind();
					}
				}
				Err(_) => self.failed = true,
			}
			self.inflight -= 1;
		}
		// The game thread is in the movie hook (its field already ended through the vblank callback): hold the next
		// field until the movie ends, so no `Machine::frame` waits on its 20 s watchdog meanwhile.
		if let Some(q) = fmv_bridge::take_request() {
			self.movie_req = Some(q);
		}
		if fmv_bridge::waiting() {
			return;
		}
		self.cam_prev = self.cam;
		self.cam_up_prev = self.cam_up;
		// Scenes and the ship streams write the render camera too (scene_camera_eval FUN_00289608: eye 0x166ec0, rows
		// 0x1670d0, tan 0x16cbf0 = Eudora's `camera_globals`); outside gameplay only while rac-rs shows retail's level.
		OCC_MASK.lock().map(|mut g| *g = None).ok();
		if let Some(c) = self.snap.cam.filter(|_| !self.snap.hold && (self.snap.mode == 0 || self.snap.level == self.level)) {
			OCC_MASK.lock().map(|mut g| *g = self.snap.occ_mask).ok();
			self.cam = c;
			self.cam_up = self.snap.cam_up;
		}
		// [PC] Run-ahead (`ahead`): hand the thread fields until it holds `depth` of them (one in lockstep).
		let want = self.depth();
		while self.inflight < want && !self.failed {
			// RAC_RETAIL_HOLD=N (shots): run no field past N, so a normal and a --gsfull shot show the same retail field.
			if std::env::var("RAC_RETAIL_HOLD").ok().and_then(|v| v.parse::<u64>().ok()).is_some_and(|n| self.fields as u64 >= n) {
				return;
			}
			// Debug shots (RAC_SHOT_FIELD = N): the game stops after field N, so the shot shows exactly that field.
			if std::env::var("RAC_SHOT_FIELD").ok().and_then(|v| v.parse::<u32>().ok()).is_some_and(|f| self.fields >= f + SHOT_ALLOW.load(std::sync::atomic::Ordering::Relaxed)) {
				return;
			}
			// [PC] DEV ONLY: the dev tools' planet warp (devtools.rs, RAC_DEV_TOOLS=1) is the same travel request, in gameplay only.
			devtools::feed_field(self.fields);
			let warp_ok = self.gameplay() && !self.snap.hold && self.snap.level == self.level;
			// The field's final input: a replayed file, or the keys with the RAC_RETAIL_PAD script ORed in. The worker
			// records exactly this (rec.rs), so a replay is field-exact whatever the window's frame timing was.
			let fin = if let Some(r) = &self.replay {
				r.fields.get(self.fields as usize).copied().unwrap_or_else(|| {
					if !std::mem::replace(&mut self.replay_ended, true) {
						eprintln!("retail: replay ended at field {} (neutral pad from here)", self.fields);
					}
					FieldIn { buttons: 0, sticks: [0x80; 4], travel: None, mc: modcam::Input::default(), omni: Default::default() }
				})
			} else {
				let (mut buttons, mut sticks) = (buttons, sticks);
				// The pad's R2 is the aim's or retail's by the mode and body form of the field just taken (`native_delay` above): the
				// pause menu's R2 scanner gets it from its first field, and leaving the pause gives retail no crouch (prompts.rs `route_r2`).
				prompts::route_r2(&mut buttons, self.pad_gameplay(), self.pad_ratchet());
				// The keys that skipped a movie stay away from the game until released.
				self.movie_mask &= buttons;
				buttons &= !self.movie_mask;
				for &(a, b, bt, st, rs) in &self.script {
					if (a..b).contains(&self.fields) {
						buttons |= bt;
						(sticks[2], sticks[3]) = (st[0], st[1]);
						if let Some(r) = rs {
							(sticks[0], sticks[1]) = (r[0], r[1]);
						}
					}
				}
				let travel = self.travel.filter(|&(_, f)| f == self.fields).map(|(l, _)| l).or_else(|| devtools::take_warp(warp_ok, self.level));
				FieldIn { buttons, sticks, travel, mc: modcam::Input { keys: if modcam::enabled() { self.kb_rs } else { [0; 2] }, ..modcam::take_ui(self.fields) }, omni: crate::omni::input::take(|fx, fy| self.picture_point(fx, fy)) }
			};
			// [PC] DEV ONLY: under a replay (tests) the warp joins the replayed field; with the tools off a replay is untouched.
			let fin = if self.replay.is_some() && fin.travel.is_none() && devtools::enabled() { FieldIn { travel: devtools::take_warp(warp_ok, self.level), ..fin } } else { fin };
			self.fields += 1;
			if self.tx.send(fin).is_err() {
				break;
			}
			self.inflight += 1;
		}
	}

	/// [PC] The next tick takes its field without waiting: the thread already finished it (kept in `early`), or the tick
	/// waits for nothing (no field handed out, a movie, a stopped game).
	pub fn field_ready(&mut self) -> bool {
		if self.failed || self.inflight == 0 || self.early.is_some() || fmv_bridge::waiting() {
			return true;
		}
		match self.rx.try_recv() {
			Ok(s) => {
				self.early = Some(s);
				true
			}
			Err(std::sync::mpsc::TryRecvError::Disconnected) => true,
			Err(std::sync::mpsc::TryRecvError::Empty) => false,
		}
	}

	/// [PC] The worker is in lockstep (`depth` is 1 and stays 1): outside the modes that draw the world (the front end, menus, movies,
	/// level loads and travel, the title boot) and with RAC_RUN_AHEAD=0. A frame runs exactly one field there, so the frame rate is
	/// the game speed (viewer/pace.rs applies no frame cap there).
	pub fn lockstep(&self) -> bool {
		static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
		let off = *OFF.get_or_init(|| std::env::var("RAC_RUN_AHEAD").is_ok_and(|v| v == "0"));
		snap_lockstep(&self.snap, off) || self.snap.level != self.level || self.movie_req.is_some() || fmv_bridge::waiting() || self.tb.as_ref().is_some_and(|t| t.active())
	}

	/// [PC] The run-ahead depth for this tick (`ahead`). 1 in lockstep (`lockstep`); else the fields that end while one frame draws, up to
	/// 3: 1 while frames come at least once a field (a fast PC is in lockstep as before), raised at once when frames slow
	/// down, lowered when they are fast again. The modes: gameplay 0, scenes 2, the vendor 5 and the ship streams 6 (takeoff,
	/// landing, flights). Before, only 0 and 2 ran ahead: with 1 field in flight a frame longer than a field (a slow GPU, 4K)
	/// made one game step per frame in the vendor and in Clank's boarding / leaving the ship, so the animation ran at the
	/// frame rate while the sound (real time) did not (GitHub issue 37).
	pub(super) fn depth(&mut self) -> u32 {
		if self.lockstep() {
			self.ahead = 1;
			return 1;
		}
		let f = disc::field_dt() * 1000.0;
		let up = ((self.frame_avg / f).ceil() as u32).clamp(1, 3);
		if up > self.ahead {
			self.ahead = up;
		} else if self.ahead > 1 && self.frame_avg < (self.ahead - 1) as f32 * f * 0.8 {
			self.ahead -= 1;
		}
		self.ahead
	}

	/// [PC] The window's frame interval of this redraw (ms), for the run-ahead depth.
	pub fn set_frame_ms(&mut self, ms: f32) {
		self.frame_avg += (ms.min(100.0) - self.frame_avg) * 0.1;
	}

	/// A movie the game asked for (once): the viewer plays it and calls `end_movie` when it ended or was skipped.
	/// Fields handed to the game so far (logs).
	pub fn field_count(&self) -> u32 {
		self.fields
	}

	/// The front end is in its level-load loop and no movie is parked: the viewer may run fields without waiting for the 50 Hz step.
	pub fn turbo(&self) -> bool {
		self.snap.turbo && !self.failed && !fmv_bridge::waiting()
	}

	/// The error that stopped the interpreter, once (the viewer shows it in the title bar).
	pub fn take_stop(&mut self) -> Option<String> {
		self.stop.take()
	}

	/// Debug shots: RAC_SHOT_FIELD = N holds the capture until N retail fields were sent (same field in normal and gsfull).
	pub fn shot_field_wait(&self) -> bool {
		// Field N = the snap of the Nth field is shown (and in --gsfull the frame of that field is rasterised).
		std::env::var("RAC_SHOT_FIELD").ok().and_then(|v| v.parse::<u32>().ok()).is_some_and(|f| { let f = f + SHOT_ALLOW.load(std::sync::atomic::Ordering::Relaxed); self.snap.field < f || self.gsfull.is_some() && GSF_SHOWN.load(std::sync::atomic::Ordering::Relaxed) < f as u64 })
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The snapshot the worker makes of a field in `mode` / ship substate `ship` (snapshot.rs sets `frozen` this way).
	fn snap(mode: u32, ship: u32) -> Snap {
		Snap { mode, ship, frozen: frozen_view(mode, ship), ..Default::default() }
	}

	/// Issue 37: the vendor (5) and the ship streams (6) ran in lockstep, one game step per frame whenever a frame took longer
	/// than a field (4K): the animation fell behind the sound. Every view that draws the world runs ahead; the menus, the front
	/// end, level loads and the ship's loading / warp substates do not.
	#[test]
	fn the_worker_runs_ahead_in_every_world_view() {
		let ahead: Vec<(u32, u32)> = (0..16).flat_map(|m| (0..10).map(move |sh| (m, sh))).filter(|&(m, sh)| (m == 6 || sh == 0) && !snap_lockstep(&snap(m, sh), false)).collect();
		// Gameplay 0, scenes 2, the vendor 5, and mode 6 in its takeoff (0), flight (3), flight (7) and landing (8) streams.
		assert_eq!(ahead, vec![(0, 0), (2, 0), (5, 0), (6, 0), (6, 3), (6, 7), (6, 8)]);
		// Mode 6 loading / warp substates, the menus (3) and every other mode stay in lockstep.
		assert!([1, 2, 4, 5, 6, 9].iter().all(|&sh| snap_lockstep(&snap(6, sh), false)));
		assert!([1, 3, 4, 7, 8, 9, 10, 15].iter().all(|&m| snap_lockstep(&snap(m, 0), false)));
	}

	/// The other lockstep reasons: the front end, a level-load loop, RAC_RUN_AHEAD=0 (a world view changes nothing there).
	#[test]
	fn front_end_load_loops_and_the_off_switch_force_lockstep() {
		let world = snap(0, 0);
		assert!(!snap_lockstep(&world, false));
		assert!(snap_lockstep(&world, true));
		assert!(snap_lockstep(&Snap { front: true, ..snap(0, 0) }, false));
		assert!(snap_lockstep(&Snap { turbo: true, ..snap(0, 0) }, false));
	}
}
