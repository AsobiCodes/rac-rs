// The rac-rs exe: the game (crate rac_game, ../src) plus the recompiled retail code (crate ee-game, generated on the player's PC).

// Public build (cargo feature `windows_gui`, set by setup.ps1): no console window next to the game. Dev builds and test tools keep theirs.
#![cfg_attr(all(windows, feature = "windows_gui"), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
	rac_game::set_register(ee_game::register);
	rac_game::main()
}
