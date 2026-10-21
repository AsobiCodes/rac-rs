// The game as a library (crate rac_game): src/main.rs is its root, included as it is. The exe (package rac-rs-exe in exe/)
// links it with the recompiled retail code, crate ee-game, which this crate does NOT depend on: so it compiles at the same
// time as the 427 generated crates instead of after them, and a full build ends with a short link instead of ~100 s of
// this crate alone. Every `ee_game::register()` here reaches the real one through `register` below, set by the exe first thing.

// `ee_game::...` in every module resolves to this crate's root (`register` below), so no call site changes.
extern crate self as ee_game;

include!("main.rs");

static REGISTER: std::sync::OnceLock<fn()> = std::sync::OnceLock::new();

/// The exe passes `ee_game::register` here before it calls `main`.
pub fn set_register(f: fn()) {
	let _ = REGISTER.set(f);
}

/// `ee_game::register()`: registers the recompiled retail code (the function the exe passed in).
#[cfg(not(test))]
pub fn register() {
	(REGISTER.get().expect("ee_game::register was not passed in (exe/src/main.rs calls rac_game::set_register first)"))()
}

/// Unit tests have no exe: they use ee-game directly (a dev-dependency, so it does not hold up the normal build).
#[cfg(test)]
pub fn register() {
	ee_game_for_tests::register()
}
