//! RAC_LOAD_LOG=1: wall-clock stamps of the load path (boot, title, Load Game, the retail loader, rac-rs's own level load).
//! Every line is `load: +SECONDS.mmm text`, seconds since the first call (main calls `init` first thing).
use std::sync::OnceLock;
use std::time::Instant;

static T0: OnceLock<Instant> = OnceLock::new();
static ON: OnceLock<bool> = OnceLock::new();

pub fn init() {
	T0.get_or_init(Instant::now);
}

pub fn on() -> bool {
	*ON.get_or_init(|| std::env::var("RAC_LOAD_LOG").is_ok_and(|v| v != "0" && !v.is_empty()))
}

/// Seconds since `init`.
pub fn secs() -> f64 {
	T0.get_or_init(Instant::now).elapsed().as_secs_f64()
}

pub fn say(text: impl FnOnce() -> String) {
	if on() {
		eprintln!("load: +{:.3} {}", secs(), text());
	}
}
