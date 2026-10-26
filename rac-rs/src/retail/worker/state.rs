//! The worker thread's state (`Worker`): everything that lives across fields, the thread start and its end.
#![allow(unused_mut)]

use super::*;

pub(super) struct Worker {
	pub(super) jobs_rx: Option<std::sync::mpsc::Receiver<ee_oracle::gs::Job>>,
	pub(super) gtx: Option<std::sync::mpsc::SyncSender<crate::gsfull::pipe::FieldIn>>,
	pub(super) card: memcard::Saver,
	pub(super) shipmouse_state: shipmouse::State,
	pub(super) wperf: WPerf,
	pub(super) smoke: Smoke,
	pub(super) marks: std::collections::VecDeque<u64>,
	pub(super) m: Box<ee_oracle::Machine>,
	pub(super) prx: std::sync::mpsc::Receiver<FieldIn>,
	pub(super) play: Option<playfeed::Feed>,
	pub(super) vush: efx_vu::Shadow,
	pub(super) cap: Option<Capture>,
	pub(super) gsh: Option<Arc<crate::gsfull::pipe::GsFull>>,
	pub(super) pipe: Option<snap_pipe::Pipe>,
	pub(super) picker: Option<efx_vu::Picker>,
	pub(super) par: Option<snap_pipe::Par>,
	pub(super) layers_used: (u32, Vec<crate::anim::ListLayer>),
	pub(super) swap_lag: Option<SwapWords>,
	pub(super) ptx: std::sync::mpsc::Sender<Snap>,
	pub(super) dir: &'static String,
	pub(super) cold: Option<&'static String>,
	pub(super) booting: bool,
	pub(super) cold_front: bool,
	pub(super) pooled: u32,
	pub(super) transit: bool,
	pub(super) field: u32,
	pub(super) noted: (u32, u32),
	pub(super) ov_log: bool,
	pub(super) ov_sync: bool,
	pub(super) title_trace: bool,
	pub(super) last_mode: u32,
	pub(super) movies: u32,
	pub(super) ov_delay: u64,
	pub(super) front: bool,
	pub(super) late_gs: bool,
	pub(super) full_on: bool,
	pub(super) p1_on: bool,
	pub(super) gs_front: bool,
	pub(super) front_n: u32,
	pub(super) front_ns: u64,
	pub(super) last_turbo: bool,
	pub(super) loop_n: u32,
	pub(super) show_n: u32,
	pub(super) draw_on: bool,
	pub(super) show_t: std::time::Instant,
	pub(super) ll_last: std::time::Instant,
	pub(super) ll_idle: u64,
	pub(super) ll_ee: u64,
	pub(super) ll_post: u64,
	pub(super) ll_n: u32,
	pub(super) ll_fmode: u32,
	pub(super) ll_lmode: u32,
	pub(super) ll_seen_front: bool,
	pub(super) ll_ctl: bool,
	pub(super) ll_ctl_field: u32,
	pub(super) ll_stop: Option<u32>,
}

impl Worker {
	pub(super) fn start(inp: ThreadIn) -> Worker {
		let ThreadIn { level, m, hold_card, gtx, jobs_rx, prx, booting, play, vush, cold_front, cold, dir, cap, gsh, pipe, picker, par, layers_used, swap_lag, ptx } = inp;
		let m = Box::new(m);
		hot::thread();
		// Level travel: the level whose pools were counted, and whether a non-gameplay field (the travel
		// sequence and loader, game mode 6) ran since the level byte changed.
		let (mut pooled, mut transit) = (level, false);
		let (mut field, mut noted) = (0u32, (level, m.rd32(va(0x15f5a4))));
		let ov_log = std::env::var("RAC_RETAIL_OVLOG").is_ok();
		let ov_sync = ov_log || std::env::var_os("RAC_RETAIL_OVDUMP").is_some() || std::env::var_os("RAC_OV_SYNC").is_some();
		let title_trace = std::env::var_os("RAC_TITLE_TRACE").is_some();
		let mut last_mode = u32::MAX;
		let mut marks = std::collections::VecDeque::new();
		let mut movies = fmv_bridge::counts().0;
		let ov_delay = std::env::var("RAC_RETAIL_OV_DELAY").ok().and_then(|v| v.parse().ok()).unwrap_or(OV_DELAY);
		let mut smoke = Smoke::from_env();
		let mut wperf = WPerf::from_env();
		let mut shipmouse_state = shipmouse::State::default();
		wperf.sample_pc(&m.cpu.pc as *const u32 as usize);
		let mut card = memcard::Saver::new(&m);
		if hold_card {
			card.hold(&m);
		}
		// The front end (frontbin, title_boot.rs): its picture comes from a GS-full pipe started when Quit Game is
		// confirmed (unless --gsfull runs one all along); `late_gs` says that one is ours.
		let (mut gtx, mut jobs_rx) = (gtx, jobs_rx);
		let (mut front, mut late_gs) = (false, false);
		// The capture draws the display buffer in full (gscap FULL_ON): the front end's mode 6 (New Game intro: caption
		// screens, fades around the movies), whose picture is not the title scene the mirror draws.
		let mut full_on = false;
		// P1 mode of the capture (gscap `P1`): a menu page with a VU1-drawn render-to-texture actor is up.
		let mut p1_on = false;
		// RAC_FRONT_GS=1: the front end shows the GS's own picture (debug / -GsFull references); else the mirror draws it.
		let gs_front = title_boot::gs_picture();
		let (mut front_n, mut front_ns) = (0u32, 0u64);
		let mut last_turbo = false;
		// The turbo starts once the load loop has run `TURBO_STABLE` fields in a row (`loop_n`); in it the capture draws a pair of
		// fields every `TURBO_SHOW_MS` (`show_t`, `show_n`: fields of the pair still to draw) and skips the prims of the others
		// (`draw_on`: the capture's DRAW_ON / DRAW_OFF state in full mode, which FULL_ON resets to drawing).
		let (mut loop_n, mut show_n, mut draw_on, mut show_t) = (0u32, 0u32, true, std::time::Instant::now());
		// RAC_LOAD_LOG (loadlog.rs): per 50 fields the average of the EE time, the rest of the worker's field and the time it
		// waited for the next field to be handed to it; and the mode changes of the front end / the level.
		let (mut ll_last, mut ll_idle, mut ll_ee, mut ll_post, mut ll_n) = (std::time::Instant::now(), 0u64, 0u64, 0u64, 0u32);
		let (mut ll_fmode, mut ll_lmode, mut ll_seen_front, mut ll_ctl, mut ll_ctl_field) = (u32::MAX, u32::MAX, false, false, 0u32);
		// RAC_LOAD_STOP=N (tests): exit N fields after the first gameplay field, with the RAM hash.
		let ll_stop: Option<u32> = std::env::var("RAC_LOAD_STOP").ok().and_then(|v| v.parse().ok());
		Worker { jobs_rx, gtx, card, shipmouse_state, wperf, smoke, marks, m, prx, play, vush, cap, gsh, pipe, picker, par, layers_used, swap_lag, ptx, dir, cold, booting, cold_front, pooled, transit, field, noted, ov_log, ov_sync, title_trace, last_mode, movies, ov_delay, front, late_gs, full_on, p1_on, gs_front, front_n, front_ns, last_turbo, loop_n, show_n, draw_on, show_t, ll_last, ll_idle, ll_ee, ll_post, ll_n, ll_fmode, ll_lmode, ll_seen_front, ll_ctl, ll_ctl_field, ll_stop }
	}

	pub(super) fn finish(&mut self) {
		let card = &mut self.card;
		let m = &mut *self.m;
		card.flush(&m);
	}
}
