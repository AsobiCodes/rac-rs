//! `Retail::load`: starts the interpreter on its worker thread (and the GS capture, sound, recording and replay it
//! feeds) and drops it again; the worker's pacing constants.
//! `load` is a list of steps; the steps live in children, one per job:
//! - machine.rs: load: machine, PC hooks, recompiled code, replay
//! - gs_setup.rs: load: first snapshot, GS capture and GS-full pipes
//! - feeds.rs: load: travel request, channels, recorded pad, title boot
//! - spawn.rs: load: starting the worker thread, its field loop
//! - state.rs: Worker: the thread's state, its start and end
//! - field_run.rs: field: pad and hooks, the EE runs the field
//! - field_front.rs: field: front end, level-load loop (turbo)
//! - field_gs.rs: field: hooks after the field, GS-full feed, capture end
//! - field_snap.rs: field: snapshot, level/pool changes, overlay, load log, hand-over

use super::*;

mod machine;
mod gs_setup;
mod feeds;
mod spawn;
mod state;
mod field_run;
mod field_front;
mod field_gs;
mod field_snap;
use self::machine::*;
use self::gs_setup::*;
use self::feeds::*;
use self::spawn::*;
use self::state::*;

/// Snap `n` carries the overlay published by field `n - OV_DELAY`'s jobs (RAC_RETAIL_OV_DELAY=N). Before, each snap took
/// whatever overlay the capture thread had finished last: one field early or late by thread timing, and an overlay
/// replaced before a snap took it never showed (the one-field HUD fade-out frames were lost).
const OV_DELAY: u64 = 0;

/// The front end's load loop runs unpaced (the turbo) once this many of its fields in a row were drawn as usual (the fade-in).
const TURBO_STABLE: u32 = 8;

/// In the turbo two fields are drawn every this many ms of wall time (the first draws the loop's picture into the main buffer,
/// the second copies it to the display buffer), the others not: the loading starfield moves on a few times a second.
const TURBO_SHOW_MS: u128 = 100;

/// Every this many fields the worker has the game thread unwind its recompiled frames at the vblank wait
/// (`Cpu::rec_unwind`, the same path as a table rebuild; the game goes on interpreting from the wait, RAM unchanged).
/// Compiled code nests a Rust frame per retail call and some retail paths never return through them: on Novalis the
/// 'ee' thread grew ~9.5 KB a field and overflowed its 256 MB stack at field ~28000 (2026-10-12). Before the O2 probe
/// fix the frequent table rebuilds unwound those frames as a side effect; now nothing else does.
/// [PC] (speed) 10, was 250: fights leak ~17-25 KB of frames a field and the unwinder walks the whole stack (~0.43 ms a MB
/// on Windows), so every 250th field took a 1.6-5.5 ms extra step (a hitch every 5 s); every 10th field it is ~0.15 ms.
const UNWIND_FIELDS: u32 = 10;

/// oracle/stubs.txt for the cold boot: the first one from the exe's folder upwards (an install, or a test build copied elsewhere,
/// keeps working after it was moved or its build folder is gone), else the one of the source tree the exe was built from. The
/// oracle skips a missing file silently, and a cold boot without it hangs on the title screen (seen 2026-10-15), so that is logged.
fn stubs_path() -> String {
	let exe = std::env::current_exe().ok();
	let near = exe.as_deref().and_then(|e| e.parent()).and_then(|d| d.ancestors().map(|a| a.join("oracle").join("stubs.txt")).find(|p| p.is_file()));
	if let Some(p) = near {
		return p.to_string_lossy().into_owned();
	}
	let built = concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/stubs.txt");
	if !std::path::Path::new(built).is_file() {
		eprintln!("retail: oracle/stubs.txt found neither above the exe nor at {built}: the cold boot will hang on the title screen");
	}
	built.into()
}

impl Retail {
	pub fn load() -> Option<Retail> {
		let (dir, cold, mut m) = open_machine()?;
		let level = register_hooks(&mut m, cold);
		crate::omni::init(&mut m.cpu);
		let replay = begin_rec(&mut m, dir, level);
		let (first, swap_lag, layers_used) = first_snapshot(&m, cold);
		let Gfx { vush, pipe, picker, par, jobs_rx, cap, gs, gtx, gshared } = gs_setup(&mut m, dir, cold);
		let travel = travel_request(&replay);
		let Feeds { tx, prx, play, ptx, rx, gsh, tb, hold_card, booting, cold_front } = open_feeds(dir, cold, &gshared);
		let worker = spawn_worker(ThreadIn { level, m, hold_card, gtx, jobs_rx, prx, booting, play, vush, cold_front, cold, dir, cap, gsh, pipe, picker, par, layers_used, swap_lag, ptx })?;
		crate::loadlog::say(|| "retail: load done, game thread started".into());
		let cam = first.cam.unwrap_or(([0.0; 3], 0.0, 0.0, 0.63));
		Some(Retail { tx, worker: Some(worker), replay, replay_ended: false, rx, inflight: 0, taken: 0, ahead: 1, frame_avg: 0.0, waited: 0.0, early: None, movie_req: None, movie_mask: 0, movie_rule: fmv_bridge::Skip::Start, movie_prev: 0, pad_last: 0, kb_rs: [0; 2], snap: first, free: HashMap::new(), order: None, held: HashMap::new(), prop_fx: HashMap::new(), bound: false, level, travel, cam, cam_prev: cam, cam_up: [0.0, 0.0, 1.0], cam_up_prev: [0.0, 0.0, 1.0], failed: false, stop: None, fields: 0, script: pad_script(), keytap: keytap_script(), movies: 0, taps: HashMap::new(), cap_hold: (Vec::new(), 0), mouse_taps: MouseTaps::default(), pad_seen: 0, ms: 0.0, ms_avg: 0.0, gs, gsfull: gshared, aspect: 4.0 / 3.0, mm: mouse_menu::State::default(), mk: menu_keys::Layer::default(), grow: None, tb })
	}
}

impl Drop for Retail {
	fn drop(&mut self) {
		// A game thread waiting in the movie hook must come out (it skips every later movie).
		fmv_bridge::abort();
		// While recording, wait for the thread's last field so a following load's `!LOAD` line comes after it.
		if rec::recording() {
			drop(std::mem::replace(&mut self.tx, std::sync::mpsc::channel().0));
			if let Some(w) = self.worker.take() {
				let _ = w.join();
			}
		}
	}
}
