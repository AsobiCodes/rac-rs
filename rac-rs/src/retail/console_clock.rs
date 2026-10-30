//! The console clock: which runs give the game the host's real date and time, and which keep the fixed clock.
//!
//! A real PS2 has a clock (CDVD RTC in Japan time, plus the time zone and summer time of its settings) and the game stores
//! what it reads in every save, where the Load Game screen shows it as day, month, year. The oracle (ee_oracle::clock)
//! can hand the game exactly that; this file decides when. The game reads the clock only when it saves, but the value goes
//! into game RAM, so a replay, a hash run or a savestate test must see the SAME clock every time: they keep the fixed
//! (zero) clock they always had, and only normal play gets the real one.
//!
//! The rule, in `real_wanted`:
//! - `RAC_REAL_CLOCK=1` forces the real clock, `RAC_REAL_CLOCK=0` forces the fixed one (what a test that checks a save date uses).
//! - Otherwise the clock is fixed when any of the switches in `DETERMINISTIC_RUNS` is set: the ones that script the input, replay or
//!   record a run, start from a savestate, force the test clock or time, or hold a run at a field (the gate, the hash and speed
//!   runs, the smoke runs, the picture checks). Nothing else counts: RAC_MUTE, the update-test and guard-test variables
//!   (RAC_TEST_UPDATE_BASE ...), RAC_RETAIL_HUD / _FMV and the log switches leave a player's run real.
//! - Otherwise (a player starting the game) the clock is real.
//!
//! The oracle's default stays the fixed clock (`ee_oracle::clock::real` is false until `init` runs), so every cargo test that
//! loads a machine, and every tool that does not go through `retail::pcext::init`, keeps it. The run scripts (gate.ps1, hash.ps1,
//! fullcheck.ps1, run.ps1) should also set RAC_REAL_CLOCK=0 so their exactness does not depend on this list.

/// The switches that make a run deterministic (scripted or replayed input, savestate or travel start, test clock and timing, held
/// field): with one of them set the game keeps the fixed clock, so RAM hashes and pictures never depend on the day.
const DETERMINISTIC_RUNS: [&str; 18] = [
	"RAC_RETAIL_REPLAY",
	"RAC_RETAIL_PLAY",
	"RAC_RETAIL_REC",
	"RAC_RETAIL_PAD",
	"RAC_RETAIL_KEYTAP",
	"RAC_RETAIL_TRAVEL",
	"RAC_RETAIL_HOLD",
	"RAC_RETAIL_SMOKE",
	"RAC_RETAIL_SMOKE_RAM",
	"RAC_TEST_CLOCK",
	"RAC_TEST_UNPACED",
	"RAC_TEST_WINDOW",
	"RAC_TEST_STATE",
	"RAC_TEST_PAD",
	"RAC_TEST_TRAVEL",
	"RAC_TEST_FIELDS",
	"RAC_SHOT_FIELD",
	"RAC_SHOT_FRAMES",
];

/// True when the run should get the host's real clock (see the module doc), for the environment `var` reads and `names` lists.
fn real_wanted(var: impl Fn(&str) -> Option<String>, names: &[String]) -> bool {
	match var("RAC_REAL_CLOCK").as_deref() {
		Some("1") => return true,
		Some("0") => return false,
		_ => {}
	}
	!names.iter().any(|k| DETERMINISTIC_RUNS.contains(&k.as_str()))
}

/// Called once before the machine is created (retail::pcext::init): switches the oracle's console clock on for normal play.
pub fn init() {
	let names: Vec<String> = std::env::vars_os().map(|(k, _)| k.to_string_lossy().into_owned()).collect();
	let real = real_wanted(|k| std::env::var(k).ok(), &names);
	ee_oracle::clock::set_real(real);
	eprintln!("retail: console clock {}", if real { "real (host date and time, JST clock with the host time zone)" } else { "fixed (test, replay, scripted run or RAC_REAL_CLOCK=0)" });
}

#[cfg(test)]
mod tests {
	use super::*;

	fn run(env: &[(&str, &str)]) -> bool {
		let names: Vec<String> = env.iter().map(|(k, _)| k.to_string()).collect();
		real_wanted(|k| env.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string()), &names)
	}

	/// A player's run (no test variable) gets the real clock.
	#[test]
	fn normal_play_is_real() {
		assert!(run(&[]));
		assert!(run(&[("RAC_LOCKON_CLASSIC", "1"), ("ORACLE_MCD_DIR", "x")]));
	}

	/// Every kind of replay, recording, scripted-input, savestate and test-clock run keeps the fixed clock, so RAM hashes and pictures never depend on the day.
	#[test]
	fn tests_and_replays_keep_the_fixed_clock() {
		for k in DETERMINISTIC_RUNS {
			assert!(!run(&[(k, "1")]), "{k}");
		}
		// what the gate, hash.ps1, fullcheck.ps1 and run.ps1 set, with their usual company
		assert!(!run(&[("RAC_MUTE", "1"), ("RAC_RETAIL_REPLAY", "x.racpad"), ("RAC_RETAIL_SMOKE", "2000"), ("RAC_TEST_WINDOW", "hidden")]));
		assert!(!run(&[("RAC_RETAIL_PAD", "10-20:4000:0,0"), ("RAC_TEST_CLOCK", "1"), ("RAC_TEST_UNPACED", "1"), ("RAC_MUTE", "1")]));
	}

	/// Variables that are not about a deterministic run (audio mute, the update sandbox, guard tests, logs, HUD and movie workarounds) leave the real clock on.
	#[test]
	fn other_variables_leave_the_real_clock() {
		for k in ["RAC_MUTE", "RAC_TEST_UPDATE_BASE", "RAC_TEST_GAME_VERSION", "RAC_TEST_RTSS_ANSWER", "RAC_RETAIL_HUD", "RAC_RETAIL_FMV", "RAC_RETAIL_OVLOG", "RAC_RETAIL_SNDLOG", "RAC_NO_UPDATE_CHECK"] {
			assert!(run(&[(k, "1")]), "{k}");
		}
		assert!(run(&[("RAC_TEST_UPDATE_BASE", "x"), ("RAC_MUTE", "1")]));
	}

	/// RAC_REAL_CLOCK decides both ways, over the test variables.
	#[test]
	fn the_switch_overrides() {
		assert!(run(&[("RAC_REAL_CLOCK", "1"), ("RAC_TEST_CLOCK", "1")]));
		assert!(!run(&[("RAC_REAL_CLOCK", "0")]));
		assert!(run(&[("RAC_REAL_CLOCK", "other")]));
	}
}
