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

/// [PC] perf (lane A7): `line` to stderr from a log thread, so the caller (the render thread) does not wait for the write
/// (a console or a redirected stderr took ~2 ms per line in a prop pool growth). Lines keep their order among themselves.
pub fn later(line: String) {
	static TX: OnceLock<Option<std::sync::mpsc::Sender<String>>> = OnceLock::new();
	let tx = TX.get_or_init(|| {
		let (tx, rx) = std::sync::mpsc::channel::<String>();
		std::thread::Builder::new().name("log".into()).spawn(move || {
			for l in rx {
				eprintln!("{l}");
			}
		}).ok().map(|_| tx)
	});
	match tx {
		Some(tx) => {
			if let Err(e) = tx.send(line) {
				eprintln!("{}", e.0);
			}
		}
		None => eprintln!("{line}"),
	}
}
