//! One field of the worker: the hooks after it, the effects layer's owned packets, the GS-full feed and the capture's field end.

use super::*;

impl Worker {
	pub(super) fn field_post(&mut self, frame_ns: u64) -> Vec<efx_own::Own> {
		let jobs_rx = &mut self.jobs_rx;
		let gtx = &mut self.gtx;
		let wperf = &mut self.wperf;
		let smoke = &mut self.smoke;
		let m = &mut *self.m;
		let gsh = &mut self.gsh;
		let booting = self.booting;
		let field = self.field;
		let front = self.front;
		if !front && !booting && !disc::extras_off() {
			modcam::post_field(&m);
			strafe::post_field(&m);
		}
		smoke.field(field, frame_ns, &m.cpu.mem.ram[..]);
		let own = efx_own::field_end(&mut m.cpu);
		if std::env::var_os("RAC_EFX_LOG").is_some() {
			eprintln!("efxown field {field}: {:x?}", own);
			for o in &own {
				let p = efx_own::walk(&m.cpu, o);
				let mut ids = [0u32; 8];
				p.blocks.iter().for_each(|b| ids[b.3 as usize] += 1);
				let (vc, whole) = efx_own::vif_census(&p.words);
				eprintln!("efxpkt field {field} q{} #{} {:x}: ok {} words {} blocks {} ids {:?} vif mscal/f/cnt/direct/unpack/mpg {:?} whole {}", o.queue, o.idx, o.fnp, p.ok, p.words.len(), p.blocks.len(), ids, vc, whole);
				if vc[3] > 0 && vc[0] + vc[1] + vc[2] == 0 {
					eprintln!("efxgif field {field} {:x}: {:x?}", o.fnp, efx_own::gif_census(&p.words));
				}
			}
		}
		if let (Some(tx), Some(rx)) = (&gtx, &jobs_rx) {
			// One field's graphics jobs and CRTC registers to the GS-full threads; blocks while they are `QUEUE` fields behind.
			let inp = crate::gsfull::pipe::FieldIn { jobs: rx.try_iter().collect(), crtc: crate::gsfull::crtc::Crtc::from_regs(&m.cpu.mem.gs) };
			let t_send = std::time::Instant::now();
			let _ = tx.send(inp);
			if let Some(sh) = &gsh {
				sh.ee.work(frame_ns);
				sh.ee.wait(t_send.elapsed().as_nanos() as u64);
			}
		}
		wperf.lap(3);
		own
	}

	pub(super) fn field_capture_end(&mut self, turbo: bool) {
		let wperf = &mut self.wperf;
		let mut marks = &mut self.marks;
		let cap = &mut self.cap;
		let mut movies = self.movies;
		if let Some(c) = &cap {
			// The field's end for the capture: a picture drawn straight into the display buffer this field
			// (no-present fields, gscap `field_end`) shows now.
			if let Some(q) = &c.tx {
				// A movie went up this field (fmv_bridge: the game thread entered the player hook): its picture
				// replaced the display buffer, so the movie-end fade draws over the movie (gscap `movie_shown`).
				let shown = fmv_bridge::counts().0 + fmv_bridge::waiting() as u32;
				if std::mem::replace(&mut movies, shown) != shown {
					let _ = q.send(ee_oracle::gs::Job::Gif(Vec::new(), gscap::MOVIE_SHOWN));
				}
				let _ = q.send(ee_oracle::gs::Job::Gif(Vec::new(), u64::MAX));
			}
			if !turbo {
				c.pace(&mut marks);
			}
		}
		wperf.lap(4);
		self.movies = movies;
	}
}
