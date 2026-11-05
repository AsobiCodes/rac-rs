//! `load`, part 2: the first snapshot (the game before its first field) and the GS capture / GS-full pipes with the effects
//! layer's VU1 feed and picture thread.
#![allow(unused_mut)]

use super::*;

pub(super) fn first_snapshot(m: &ee_oracle::Machine, cold: Option<&String>) -> (Snap, Option<SwapWords>, (u32, Vec<crate::anim::ListLayer>)) {
	// A cold boot starts in the boot ELF (logos): no level yet, the front end's (blank) snapshot until the front end runs.
	let mut first = if cold.is_some() { front_snap(&m) } else { snap(&m) };
	let mut swap_lag = None;
	if let Some(fx) = first.fx.as_mut() {
		lag_swap(fx, &mut swap_lag);
	}
	// The hero layer list retail's next joint evaluation uses (see `lag_layers`).
	let mut layers_used = (first.level, first.parts.layers.clone());
	(first, swap_lag, layers_used)
}

pub(super) struct Gfx {
	pub(super) vush: efx_vu::Shadow,
	pub(super) pipe: Option<snap_pipe::Pipe>,
	pub(super) picker: Option<efx_vu::Picker>,
	pub(super) par: Option<snap_pipe::Par>,
	pub(super) jobs_rx: Option<std::sync::mpsc::Receiver<ee_oracle::gs::Job>>,
	pub(super) cap: Option<Capture>,
	pub(super) gs: bool,
	pub(super) gtx: Option<std::sync::mpsc::SyncSender<crate::gsfull::pipe::FieldIn>>,
	pub(super) gshared: Option<Arc<crate::gsfull::pipe::GsFull>>,
}

pub(super) fn gs_setup(mut m: &mut ee_oracle::Machine, dir: &String, cold: Option<&String>) -> Gfx {
	// RAC_RETAIL_HUD=0 keeps rac-rs's own HUD.
	// RAC_GSFULL=1 / --gsfull: the whole GS output is drawn instead (no mirror overlay capture).
	// A cold boot has no savestate dir for the pipe's start VRAM and VU1 memory: RAC_GSFULL_STATE names one (reference shots),
	// else the pipe starts blank ("": the boot uploads its own VU1 programs and draws everything it shows).
	let gs_dir = if cold.is_some() { std::env::var("RAC_GSFULL_STATE").unwrap_or_default() } else { dir.clone() };
	let gsf = if gsfull_on() { crate::gsfull::pipe::start(&gs_dir) } else { None };
	// EFFECTS LAYER, VU1 owners (retail/efx_vu.rs): the VIF1 + VU1 model the field's jobs pass through.
	let mut vush = efx_vu::Shadow::new();
	// The picture half (capture + chain walk) lives on its own thread (snap_pipe.rs) or, RAC_SNAP_PIPE=0, inline on the worker.
	let mut pipe = if snap_pipe::enabled() { snap_pipe::Pipe::new(efx_vu::Picker::new(&gs_dir), snap_pipe::check().then(|| efx_vu::Picker::new(&gs_dir))) } else { None };
	let mut picker = if pipe.is_none() { Some(efx_vu::Picker::new(&gs_dir)) } else { None };
	// Without the ghost pipe: capture + walk run on a helper thread beside `snap()` when that is faster (snap_pipe.rs `Par`).
	let mut par = if pipe.is_none() { snap_pipe::Par::new(snap_pipe::check().then(|| efx_vu::Picker::new(&gs_dir))) } else { None };
	let jobs_rx = gsf.as_ref().map(|_| {
		let (jtx, jrx) = std::sync::mpsc::channel::<ee_oracle::gs::Job>();
		m.cpu.mem.gsq = Some(jtx);
		jrx
	});
	if gsf.is_some() {
		eprintln!("retail: GS-full display: the game's own picture, {} raster threads", crate::gsfull::pipe::threads());
	} else if gsfull_on() {
		eprintln!("retail: GS-full display unavailable (needs GS.bin, vu1Memory.bin and vu1MicroMem.bin in {dir})");
	}
	let cap = if gsf.is_some() || std::env::var("RAC_RETAIL_HUD").is_ok_and(|v| v == "0") {
		None
	} else if cold.is_some() {
		capture_gs_vram(&mut m, vec![0u8; 4 << 20], &gs_dir)
	} else {
		capture_gs(&mut m, dir)
	};
	if cap.is_none() && gsf.is_none() {
		eprintln!("retail: no GS capture (rac-rs HUD)");
	}
	crate::loadlog::say(|| "retail: GS capture ready".into());
	let gs = cap.is_some() || gsf.is_some();
	let (gtx, gshared) = match gsf {
		Some((tx, sh)) => (Some(tx), Some(sh)),
		None => (None, None),
	};
	Gfx { vush, pipe, picker, par, jobs_rx, cap, gs, gtx, gshared }
}
