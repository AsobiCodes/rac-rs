//! `load`, part 1: the machine (cold boot from the disc, or a savestate), the PC hooks on the retail functions, the recompiled
//! code and the replay file.
#![allow(unused_mut)]

use super::*;

pub(super) fn open_machine() -> Option<(&'static String, Option<&'static String>, ee_oracle::Machine)> {
	let dir = STATE.get()?;
	crate::loadlog::say(|| "retail: load starts".into());
	// The window's main loop runs here; the worker and the GS capture mark their own threads.
	hot::process();
	hot::thread();
	gscap::gpu_prestart(hot::idle);
	let cold = COLD.get();
	pcext::init(cold.is_some());
	prompts::init();
	// The picture thread's mirror (snap_pipe.rs) copies only the RAM pages written since its last sync: the live RAM is write-watched.
	if snap_pipe::enabled() {
		ee_oracle::mem::watch::watch_next();
	}
	let mut m = match cold {
		Some(iso) => {
			let stubs = std::env::var("ORACLE_STUBS").unwrap_or_else(|_| stubs_path());
			match ee_oracle::Machine::cold(iso, &stubs) {
				Ok(m) => m,
				Err(e) => {
					eprintln!("retail: cold boot failed: {e}");
					return None;
				}
			}
		}
		None => ee_oracle::Machine::load(dir),
	};
	crate::loadlog::say(|| "retail: machine ready".into());
	Some((dir, cold, m))
}

pub(super) fn register_hooks(mut m: &mut ee_oracle::Machine, cold: Option<&String>) -> u32 {
	// The player's own memory card (retail/memcard.rs): a save made in an earlier session, before the first card call.
	memcard::restore(&mut m);
	m.record_sound();
	// The oracle driver's state before its first field: running voices of an adopted savestate, core mix registers.
	*crate::spumix::ATTACH.lock().unwrap() = Some(crate::spumix::attach_state(&mut m));
	let level = m.level();
	// The recompiled retail functions (ee_oracle rec/); ORACLE_REC=0 interprets everything.
	ee_game::register();
	crate::loadlog::say(|| "retail: recompiled code registered".into());
	// Movies play natively (retail/fmv_bridge.rs): the retail player function is replaced by a hook.
	fmv_bridge::reset();
	fmv_bridge::register();
	if cold.is_some() {
		title_boot::hook_boot_movies(&mut m);
	}
	// --wide (GS-full only): the wrapping hands of the field-of-view function (gsfull/wide.rs).
	if crate::gsfull::wide::enabled() && !disc::extras_off() {
		if gsfull_on() {
			crate::gsfull::wide::register();
			eprintln!("retail: 16:9 view (wide field of view, HUD squeezed to the corners)");
		} else {
			eprintln!("retail: --wide needs --gsfull (the mirror renderer has its own widescreen); ignored");
		}
	}
	// --modern-cam / RAC_MODERN_CAM=1 (retail/modcam.rs): off registers nothing (retail bit-exact).
	if modcam::enabled() {
		modcam::register();
		strafe::register();
		aim::register();
		eprintln!("retail: modern camera on (F7 toggles, RMB aims)");
	}
	// PC options rows (retail/pcext.rs): registered after modcam's hands; RAC_PC_EXT=0 registers nothing.
	if pcext::enabled() {
		pcext::register();
	}
	if !disc::extras_off() {
		vendor::register();
	}
	// A loaded savestate's level and draw queues are known now: its first field gets the effects layer too.
	if cold.is_none() {
		efx_own::prime(&mut m.cpu);
	}
	level
}

pub(super) fn begin_rec(m: &mut ee_oracle::Machine, dir: &String, level: u32) -> Option<Arc<rec::Replay>> {
	let rec = m.enable_rec();
	eprintln!("retail: {dir} level {level}, {rec} recompiled functions");
	rec::begin(dir, level, rec);
	let replay = rec::replay();
	if let Some(r) = &replay {
		let norm = |a: &str| a.replace('\\', "/").trim_end_matches('/').to_ascii_lowercase();
		if norm(&r.state) != norm(dir) {
			eprintln!("retail: replay: the file starts from {} but this run loaded {dir}; it will not replay exactly", r.state);
		}
	}
	replay
}
