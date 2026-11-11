//! [PC] RAC_DIAG (engine/diag.rs): R&C1's facts for the diagnostic record (`Pack::diag_facts`): the loaded level, paused / front
//! end, the hero's position and the camera mode. The host publishes them each frame while RAC_DIAG is on (host/handler/
//! frame_render.rs, read-only copies of what it already holds); the engine reads them once per second. No RAM is read or written.

use rac_engine::diag::Facts;
use std::sync::Mutex;

static FACTS: Mutex<Option<Facts>> = Mutex::new(None);

/// The host's facts of this frame: loaded level index, hero position, (paused, front end) when retail runs.
pub fn publish(level: usize, hero: Option<[f32; 3]>, retail: Option<(bool, bool)>) {
	let f = Facts {
		level: crate::toc::LEVEL_NAMES.get(level).copied().unwrap_or(""),
		level_id: level as i32,
		paused: retail.is_some_and(|r| r.0),
		menu: retail.is_some_and(|r| r.1),
		hero,
		camera: if crate::retail::modcam::on() { "modern" } else { "classic" },
	};
	*FACTS.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
}

/// `Pack::diag_facts`.
pub fn get() -> Facts {
	FACTS.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}
