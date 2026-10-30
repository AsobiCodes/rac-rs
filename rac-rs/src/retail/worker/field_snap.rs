//! One field of the worker: the snapshot for the main thread (effects layer, level and pool changes, overlay), the load log and
//! the hand-over.

use super::*;

impl Worker {
	pub(super) fn field_snap(&mut self, own: Vec<efx_own::Own>) -> Snap {
		let wperf = &mut self.wperf;
		let m = &mut *self.m;
		let vush = &mut self.vush;
		let cap = &mut self.cap;
		let pipe = &mut self.pipe;
		let picker = &mut self.picker;
		let mut par = self.par.take();
		let booting = self.booting;
		let field = self.field;
		let front = self.front;
		let mut p1_on = self.p1_on;
		let t_l4 = std::time::Instant::now();
		// EFFECTS LAYER, VU1 owners: the chain walk reads RAM here; the VIF1 / VU1 feed runs on its own thread while
		// the snapshot is built (retail/efx_vu.rs `prep` / `run`).
		let lv = m.cpu.mem.ram[LEVEL as usize] as u32;
		let efx_on = !front && !booting && efx_draw::enabled();
		let t_prep = std::time::Instant::now();
		let (mut bi, mut work, mut hand) = (None, None, None);
		let mut par_on = false;
		if efx_on {
			if let Some(p) = pipe.as_mut() {
				// The ghost machine gets the RAM the walk reads now; capture + walk run on the picture thread.
				hand = p.hand(&m, &own, lv, field, vush.take_pending());
			} else if let Some(pk) = picker.as_mut() {
				if par.as_mut().is_some_and(|p| p.want()) {
					// Capture + walk run on the helper while snap() below builds the snapshot; both only read the machine.
					par_on = par.as_mut().is_some_and(|p| p.start(&m, &own, lv, pk, vush.take_pending()));
					if !par_on {
						par = None;
					}
				} else {
					let b = efx_draw::capture(&m, lv, &own);
					work = Some(pk.prep(&m, &own, &b, vush.take_pending()));
					bi = Some(b);
				}
			}
		}
		wperf.extra(0, t_prep.elapsed().as_nanos() as u64);
		let t_snap = std::time::Instant::now();
		crate::kofi::note(!front && !booting, lv, m.cpu.mem.ram[crate::kofi::NOVALIS_UNLOCK]); // [PC] Ko-fi drawings: the first Veldin visit (kofi.rs)
		let mut s = if front || booting { front_snap(&m) } else { snap(&m) };
		let want_p1 = !front && !booting && rtt_page(&s);
		if want_p1 != p1_on {
			if let Some(q) = cap.as_ref().and_then(|c| c.tx.as_ref()) {
				let _ = q.send(ee_oracle::gs::Job::Gif(Vec::new(), if want_p1 { gscap::P1_ON } else { gscap::P1_OFF }));
				p1_on = want_p1;
			}
		}
		wperf.extra(1, t_snap.elapsed().as_nanos() as u64);
		// EFFECTS LAYER: the VU1 feed and the owned DIRECT-GIF callbacks this field drew (retail/efx_draw.rs, after the
		// ported ones) are built on the VU1 thread while the EE runs the next field; `Retail::tick` appends them.
		let t_join = std::time::Instant::now();
		if par_on {
			if let Some((b, w)) = par.as_mut().and_then(|p| p.join(&m, field)) {
				bi = Some(b);
				work = Some(w);
			}
		}
		let t_sub = std::time::Instant::now();
		if let Some(h) = hand {
			s.fx_late = Some(FxLate::Later(h.finish(s.fx.is_some())));
		} else if let (Some(w), Some(b), Some(pk)) = (work, bi, picker.as_mut()) {
			s.fx_late = pk.submit(w, b, field, s.fx.is_some()).map(FxLate::Now);
		}
		if efx_on {
			if let Some(p) = par.as_mut() {
				p.done(t_prep.elapsed().as_nanos() as u64);
			}
		}
		wperf.extra(2, t_join.elapsed().as_nanos() as u64);
		wperf.lap(5);
		// After the section timer: the print itself takes ms when stderr is a file.
		if wperf.on && t_l4.elapsed().as_micros() > 2000 {
			let hs = |i: usize| snap_pipe::HS[i].load(std::sync::atomic::Ordering::Relaxed) as f64 / 1e6;
			eprintln!("perf: snap spike field {field}: total {:.2} ms = before {:.2} dispatch {:.2} snap() {:.2} join {:.2} submit {:.2} | par {par_on} | helper wake {:.2} capture {:.2} walk {:.2} | fresh buffers {} KB", t_l4.elapsed().as_secs_f64() * 1e3, t_prep.duration_since(t_l4).as_secs_f64() * 1e3, t_snap.duration_since(t_prep).as_secs_f64() * 1e3, t_join.duration_since(t_snap).as_secs_f64() * 1e3, t_sub.duration_since(t_join).as_secs_f64() * 1e3, t_sub.elapsed().as_secs_f64() * 1e3, hs(0), hs(1), hs(2), efx_vu::FRESH.swap(0, std::sync::atomic::Ordering::Relaxed) / 1024);
		}
		self.par = par;
		self.p1_on = p1_on;
		s
	}

	pub(super) fn field_levels(&mut self, mut s: &mut Snap, turbo: bool) {
		let wperf = &mut self.wperf;
		let mut m = &mut *self.m;
		let mut layers_used = &mut self.layers_used;
		let mut swap_lag = &mut self.swap_lag;
		let booting = self.booting;
		let mut pooled = self.pooled;
		let mut transit = self.transit;
		let field = self.field;
		let mut noted = self.noted;
		let front = self.front;
		s.turbo = turbo;
		if !front && !booting {
			lag_layers(&mut s, &mut layers_used);
			if let Some(fx) = s.fx.as_mut() {
				lag_swap(fx, &mut swap_lag);
			}
		}
		if !front && !booting && (s.level, s.mode) != noted {
			noted = (s.level, s.mode);
			rec::note(field, s.level, s.mode);
		}
		if !front && !booting && s.level != pooled {
			if !arrived(&s, &m) {
				transit = true;
				// [PC] The pools as the moby table holds them now (it fills during the last fields of the landing): the level
				// prefetch builds the prop slots for them ahead of the arrival (`pools_hint`). Read only. Only a table that looks
				// real (`HINT_SLOTS`, `HINT_CLASS`).
				let (ts, te) = table(s.level);
				let (lo, hi) = (m.rd32(ts) & 0x01ff_ffff, m.rd32(te) & 0x01ff_ffff);
				let p = if lo <= hi && hi - lo <= HINT_SLOTS * 0x100 { count_pools(s.level, |a| m.rd32(a), |a| m.rd16(a)) } else { Vec::new() };
				if p.last().is_some_and(|c| c.0 < HINT_CLASS) {
					if let Ok(mut h) = POOLS_HINT.lock() {
						*h = Some((s.level, p));
					}
				}
			} else if transit {
				s.pools = Some(count_pools(s.level, |a| m.rd32(a), |a| m.rd16(a)));
				if let Ok(mut h) = POOLS_HINT.lock() {
					*h = None;
				}
				fmv_bridge::check_level(&m, s.level);
				(pooled, transit) = (s.level, false);
			}
		}
		s.sound = take_sound_of(&mut m);
		wperf.lap(6);
		self.pooled = pooled;
		self.transit = transit;
		self.noted = noted;
	}

	pub(super) fn field_overlay(&mut self, s: &mut Snap) {
		let wperf = &mut self.wperf;
		let cap = &mut self.cap;
		let booting = self.booting;
		let field = self.field;
		let ov_log = self.ov_log;
		let ov_sync = self.ov_sync;
		let ov_delay = self.ov_delay;
		let front = self.front;
		let full_on = self.full_on;
		// In-game the overlay is taken later, on the main thread (`OvLate`): the worker goes on to the next field while the
		// capture thread finishes this one. The front end and the debug dumps take it here.
		let late = !front && !booting && !ov_sync;
		if let (true, Some(c)) = (late, cap.as_ref()) {
			let (h, f, done) = (c.handle(), field as u64, c.late(field as u64, ov_delay));
			s.ov_late = Some(OvLate(Box::new(move || {
				// Marks the take run (capture.rs `LateTakes`) after it; dropped with the closure if it never runs.
				let _done = done;
				h.take(f, ov_delay).map(|mut o| {
					let (w, rows) = ov_dims(&o);
					(w, rows, ov_px(&mut o), o.disp, o.guard)
				})
			})));
		}
		let taken = if late { None } else { cap.as_ref().and_then(|c| c.take(field as u64, ov_delay)) };
		wperf.lap(7);
		if let Some(mut o) = taken.filter(|_| (!front && !booting) || full_on) {
			let (ow, orows) = ov_dims(&o);
			let px = ov_px(&mut o);
			// RAC_RETAIL_OVLOG=1: one line per overlay (the field it arrived in, FNV of its pixels): does the menu change?
			let h = if ov_log {
				px.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, p| (h ^ u32::from_le_bytes(*p) as u64).wrapping_mul(0x0100_0000_01b3))
			} else {
				0
			};
			if ov_log {
				eprintln!("retail: overlay field {field} frozen {} hash {h:016x}", o.frozen);
			}
			if let Ok(path) = std::env::var("RAC_RETAIL_OVDUMP") {
				let mut f = format!("P6 {} {} 255
", ow, orows).into_bytes();
				for p in &px {
					let p: &[u8; 4] = p;
					f.extend(p[..3].iter().zip([0.25f32, 0.25, 0.25]).map(|(c, _)| (*c as f32 + p[3] as f32 * 0.3) as u8));
				}
				let _ = std::fs::write(&path, f);
			}
			s.ov = Some((ow, orows, px, o.guard));
			s.ov_disp = o.disp;
		}
	}

	pub(super) fn field_end(&mut self, mut s: Snap, t: std::time::Instant, r: Result<(), String>, frame_ns: u64) -> std::ops::ControlFlow<()> {
		let wperf = &mut self.wperf;
		let m = &mut *self.m;
		let ptx = &mut self.ptx;
		let booting = self.booting;
		let field = self.field;
		let front = self.front;
		let mut ll_last = self.ll_last;
		let mut ll_idle = self.ll_idle;
		let mut ll_ee = self.ll_ee;
		let mut ll_post = self.ll_post;
		let mut ll_n = self.ll_n;
		let mut ll_fmode = self.ll_fmode;
		let mut ll_lmode = self.ll_lmode;
		let mut ll_seen_front = self.ll_seen_front;
		let mut ll_ctl = self.ll_ctl;
		let mut ll_ctl_field = self.ll_ctl_field;
		let ll_stop = self.ll_stop;
		s.ms = t.elapsed().as_secs_f32() * 1000.0;
		if crate::loadlog::on() {
			let el = t.elapsed().as_nanos() as u64;
			(ll_ee, ll_post, ll_n) = (ll_ee + frame_ns, ll_post + el.saturating_sub(frame_ns), ll_n + 1);
			let (fm, lm) = (m.rd32(title_boot::front_mode()), m.rd32(va(0x15f5a4)));
			if front && fm != ll_fmode {
				crate::loadlog::say(|| format!("field {field}: front mode {ll_fmode:#x} -> {fm:#x}"));
				ll_fmode = fm;
			}
			if !front && !booting && lm != ll_lmode {
				crate::loadlog::say(|| format!("field {field}: level {} mode {ll_lmode:#x} -> {lm:#x} loader {:#x}", m.rd32(0x15ed84), m.rd32(0x15ee48) & 0xffff));
				ll_lmode = lm;
			}
			ll_seen_front |= front;
			if ll_seen_front && !front && !ll_ctl && lm == 0 {
				ll_ctl = true;
				ll_ctl_field = field;
				crate::loadlog::say(|| format!("field {field}: FIRST GAMEPLAY FIELD (level {} mode 0) ram {:016x}", m.rd32(0x15ed84), rec::hash_ram(&m.cpu.mem.ram[..])));
			}
			if let (true, Some(n)) = (ll_ctl, ll_stop) {
				if field >= ll_ctl_field + n {
					crate::loadlog::say(|| format!("field {field}: STOP ram {:016x}", rec::hash_ram(&m.cpu.mem.ram[..])));
					crate::gscap_guard::quit();
					crate::omni::flush_log();
					std::process::exit(0);
				}
			}
			if ll_n == 50 {
				let f = |v: u64| v as f64 / 1e6 / 50.0;
				crate::loadlog::say(|| format!("worker field {field} front {front} fmode {fm:#x} lmode {lm:#x} level {}: per field ee {:.1} ms, rest {:.1} ms, waited {:.1} ms (loader {:#x})", m.rd32(0x15ed84), f(ll_ee), f(ll_post), f(ll_idle), m.rd32(0x15ee48) & 0xffff));
				(ll_ee, ll_post, ll_idle, ll_n) = (0, 0, 0, 0);
			}
		}
		wperf.lap(8);
		wperf.field(field);
		// RAC_PERF_LOG=1 only: after the section timers (its eprintln takes up to 3.5 ms when stderr is a file or pipe, which
		// showed up as a 100-field `snap` max spike when it ran inside the snap section).
		snap_pipe::report(field);
		ll_last = std::time::Instant::now();
		s.err = r.err();
		let stop = s.err.is_some();
		self.ll_last = ll_last;
		self.ll_idle = ll_idle;
		self.ll_ee = ll_ee;
		self.ll_post = ll_post;
		self.ll_n = ll_n;
		self.ll_fmode = ll_fmode;
		self.ll_lmode = ll_lmode;
		self.ll_seen_front = ll_seen_front;
		self.ll_ctl = ll_ctl;
		self.ll_ctl_field = ll_ctl_field;
		if ptx.send(s).is_err() || stop {
			return std::ops::ControlFlow::Break(());
		}
		std::ops::ControlFlow::Continue(())
	}
}
