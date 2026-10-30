//! One field of the worker: the front end (title, menus; the late GS pipe, the capture's modes) and the level-load loop (turbo).

use super::*;

impl Worker {
	pub(super) fn field_front(&mut self, frame_ns: u64) -> bool {
		let mut jobs_rx = self.jobs_rx.take();
		let mut gtx = self.gtx.take();
		let card = &mut self.card;
		let mut m = &mut *self.m;
		let cap = &mut self.cap;
		let dir = self.dir;
		let cold = self.cold;
		let mut booting = self.booting;
		let mut cold_front = self.cold_front;
		let mut pooled = self.pooled;
		let mut transit = self.transit;
		let field = self.field;
		let title_trace = self.title_trace;
		let mut last_mode = self.last_mode;
		let mut front = self.front;
		let mut late_gs = self.late_gs;
		let mut full_on = self.full_on;
		let gs_front = self.gs_front;
		let mut front_n = self.front_n;
		let mut front_ns = self.front_ns;
		let mut loop_n = self.loop_n;
		let mut show_n = self.show_n;
		let mut draw_on = self.draw_on;
		// The title boot over without a front end (it failed): the card is the game's again.
		if card.is_held() && !title_boot::holding() {
			card.release(&mut m, false);
		}
		// The front end replaced the level (or the level replaced it): see title_boot.rs.
		let in_front = if cold_front {
			if title_boot::level_loaded(&m) {
				cold_front = false;
				false
			} else {
				true
			}
		} else {
			title_boot::front_end(&m)
		};
		if in_front && booting {
			booting = false;
			eprintln!("retail: cold boot: the front end is in RAM at field {field}");
		}
		if gs_front && !front && !in_front && gtx.is_none() && title_boot::want_late_gs(&m) {
			late_gs = start_late_gs(&mut m, dir, &mut gtx, &mut jobs_rx);
		}
		if in_front && !front {
			if gtx.is_none() && gs_front {
				late_gs = start_late_gs(&mut m, dir, &mut gtx, &mut jobs_rx);
			}
			// The mirror draws the front end (viewer front_mirror) but the capture keeps getting its GS jobs: they carry the
			// texture uploads the game makes once per boot (cold boot: the HUD / weapon-icon atlas at VRAM block 4096 is
			// uploaded at field 25, before any level) and that no later frame repeats until the pause menu. Its overlays
			// of these fields are dropped below (the front end is the mirror's picture) and not even drawn (DRAW_OFF).
			if let Some(q) = cap.as_ref().and_then(|c| c.tx.as_ref()) {
				let _ = q.send(ee_oracle::gs::Job::Gif(Vec::new(), gscap::DRAW_OFF));
			}
			card.release(&mut m, true);
			if !cold_front {
				title_boot::hook_front_movies(&mut m);
			}
			// The title's Options -> Graphics row (pcext.rs `hook_front`).
			pcext::hook_front(&mut m);
			// And the main menu's Quit Game row (titlequit.rs).
			crate::retail::titlequit::hook_front(&mut m.cpu.mem);
			(pooled, transit, front_n, front_ns) = (u32::MAX, true, 0, 0);
			eprintln!("retail: front end running at field {field} (late GS pipe {late_gs})");
		} else if front && !in_front {
			if late_gs {
				// Back to the level: the mirror's HUD capture takes the jobs again, the front end's pipe ends.
				m.cpu.mem.gsq = cap.as_ref().and_then(|c| c.tx.clone());
				(gtx, jobs_rx, late_gs) = (None, None, false);
				title_boot::set_late(None);
			} else if gtx.is_none() {
				m.cpu.mem.gsq = cap.as_ref().and_then(|c| c.tx.clone());
			}
			// The capture draws again (it only kept the GS state and the VRAM transfers in the front end).
			if let Some(q) = cap.as_ref().and_then(|c| c.tx.as_ref()) {
				if std::mem::replace(&mut full_on, false) {
					let _ = q.send(ee_oracle::gs::Job::Gif(Vec::new(), gscap::FULL_OFF));
				}
				let _ = q.send(ee_oracle::gs::Job::Gif(Vec::new(), gscap::DRAW_ON));
			}
			eprintln!("retail: front end left at field {field} after {front_n} fields");
		}
		if cold.is_some() && !disc::extras_off() {
			pcext::watch_language(&m, field, in_front);
		}
		front = in_front;
		crate::omni::note_front(in_front);
		// Mode 6 of the front end: the display buffer's picture comes from the capture (gscap `full`), see the overlay below.
		let want_full = in_front && !booting && m.rd32(title_boot::front_mode()) == 6;
		if want_full != full_on {
			if let Some(q) = cap.as_ref().and_then(|c| c.tx.as_ref()) {
				let _ = q.send(ee_oracle::gs::Job::Gif(Vec::new(), if want_full { gscap::FULL_ON } else { gscap::FULL_OFF }));
				full_on = want_full;
				(loop_n, show_n, draw_on) = (0, 0, true);
			}
		}
		if front && title_trace {
			let mode = m.rd32(title_boot::front_mode());
			if mode != last_mode {
				eprintln!("title: field {field} front mode {last_mode:#x} -> {mode:#x} crtc {:x?}", crate::gsfull::crtc::Crtc::from_regs(&m.cpu.mem.gs));
				last_mode = mode;
			}
		}
		if front {
			(front_n, front_ns) = (front_n + 1, front_ns + frame_ns);
			if front_n % 100 == 0 {
				eprintln!("retail: front end field {front_n}: ee {:.2} ms/field (last 100)", front_ns as f64 / 1e8);
				front_ns = 0;
			}
		}
		card.after_field(&m);
		self.jobs_rx = jobs_rx;
		self.gtx = gtx;
		self.booting = booting;
		self.cold_front = cold_front;
		self.pooled = pooled;
		self.transit = transit;
		self.last_mode = last_mode;
		self.front = front;
		self.late_gs = late_gs;
		self.full_on = full_on;
		self.front_n = front_n;
		self.front_ns = front_ns;
		self.loop_n = loop_n;
		self.show_n = show_n;
		self.draw_on = draw_on;
		in_front
	}

	pub(super) fn field_load_loop(&mut self, in_front: bool) -> bool {
		let m = &mut *self.m;
		let cap = &mut self.cap;
		let booting = self.booting;
		let cold_front = self.cold_front;
		let field = self.field;
		let full_on = self.full_on;
		let mut last_turbo = self.last_turbo;
		let mut loop_n = self.loop_n;
		let mut show_n = self.show_n;
		let mut draw_on = self.draw_on;
		let mut show_t = self.show_t;
		// The level-load loop of the front end (`in_load_loop`): no pacing while it runs, the main thread hands the next field
		// at once (`Retail::turbo`), and the GS picture work only for a pair of fields every `TURBO_SHOW_MS`. The loop only
		// waits for the level data (it reads no pad); its picture, a slowly moving starfield, just moves on faster.
		let was_load = loop_n > 0;
		loop_n = if in_front && !booting && full_on && in_load_loop(&m, !cold_front) { loop_n.saturating_add(1) } else { 0 };
		// While the loop runs the capture draws its main-to-display copies as a clean picture (gscap `LOAD_ON`, `Gs::load`):
		// full mode keeps no main buffer, the copy would show stale VRAM. FULL_ON / FULL_OFF clear it on both sides.
		if (loop_n > 0) != was_load {
			if let Some(q) = cap.as_ref().and_then(|c| c.tx.as_ref()) {
				let _ = q.send(ee_oracle::gs::Job::Gif(Vec::new(), if loop_n > 0 { gscap::LOAD_ON } else { gscap::LOAD_OFF }));
			}
		}
		let turbo = loop_n > TURBO_STABLE;
		if turbo != last_turbo {
			crate::loadlog::say(|| format!("field {field}: load loop turbo {}", if turbo { "ON" } else { "off" }));
			(show_n, show_t, last_turbo) = (0, std::time::Instant::now(), turbo);
		}
		// The marker takes effect from the next field's prims: the pair's second field copies the main buffer its first drew.
		let draw = !turbo
			|| if show_n > 0 {
				show_n -= 1;
				true
			} else if show_t.elapsed().as_millis() >= TURBO_SHOW_MS {
				(show_n, show_t) = (1, std::time::Instant::now());
				true
			} else {
				false
			};
		if full_on && draw != draw_on {
			if let Some(q) = cap.as_ref().and_then(|c| c.tx.as_ref()) {
				let _ = q.send(ee_oracle::gs::Job::Gif(Vec::new(), if draw { gscap::DRAW_ON } else { gscap::DRAW_OFF }));
				draw_on = draw;
			}
		}
		if field % 100 == 0 && std::env::var_os("RAC_TITLE_TRACE").is_some() {
			let rw = |a: usize| i32::from_le_bytes(m.cpu.mem.ram[a..a + 4].try_into().unwrap());
			eprintln!("title: ram field {field}: level {} bolts {} slot summaries {:?} autosave {}", rw(0x15ed84), rw(0x15ed98), (0..6).map(|i| rw(0x13d2b0 + i * 0x1c)).collect::<Vec<_>>(), rw(0x13d384));
			eprintln!("title: ram field {field}: front mode {:#x} req level {} req flag {:#x} loader {:#x} pc {:08x} 15f5a4 {:#x}", rw(va(0x15f5e4)), rw(va(0x15f5a0)), rw(va(0x15f5b8)), rw(0x15ee48) & 0xffff, m.cpu.pc, rw(va(0x15f5a4)));
		}
		self.last_turbo = last_turbo;
		self.loop_n = loop_n;
		self.show_n = show_n;
		self.draw_on = draw_on;
		self.show_t = show_t;
		turbo
	}
}
