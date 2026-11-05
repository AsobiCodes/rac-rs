//! The game's install folder and the window title (stage E3: from the crate root src/main.rs, so the engine and the
//! packs reach them).

/// The setup script the messages name: Setup.cmd on Windows, setup.sh on Linux. A macro (not a const) so that `concat!` makes it one
/// literal with the rest of the message: on Windows the strings in the exe are exactly the ones from before the Linux port.
#[cfg(target_os = "linux")]
#[macro_export]
macro_rules! setup_name {
	() => {
		"setup.sh"
	};
}
#[cfg(not(target_os = "linux"))]
#[macro_export]
macro_rules! setup_name {
	() => {
		"Setup.cmd"
	};
}

/// The install root: the first folder from the exe's folder upwards that holds `generated/play.cfg`. Err is the text for a message box.
pub fn install_root() -> Result<std::path::PathBuf, String> {
	let exe = std::env::current_exe().map_err(|e| format!(concat!("Cannot find the game folder ({}). Run ", setup_name!(), " first."), e))?;
	exe.parent()
		.and_then(|d| d.ancestors().find(|d| d.join("generated").join("play.cfg").is_file()))
		.map(|d| d.to_path_buf())
		.ok_or_else(|| concat!("Run ", setup_name!(), " first: the game has not been set up yet (generated/play.cfg not found).").to_string())
}

/// [PC] The window title: "rac-rs v0.2.1" from the install root's `generated/built_version.txt` (setup.ps1 writes it after building
/// this exe, so an update whose files arrived but whose build has not finished still shows the old version), else its VERSION file
/// (the release the player downloaded), else "rac-rs" (dev and test runs outside an install).
pub fn app_name() -> &'static str {
	static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
	NAME.get_or_init(|| {
		let read = |r: &std::path::Path, f: &str| std::fs::read_to_string(r.join(f)).ok().filter(|v| !v.trim().is_empty());
		let ver = install_root().ok().and_then(|r| read(&r, "generated/built_version.txt").or_else(|| read(&r, "VERSION"))).unwrap_or_default();
		let ver = ver.trim_start_matches('\u{feff}').trim();
		if ver.is_empty() || ver.len() > 24 || !ver.chars().all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c)) { "rac-rs".into() } else { format!("rac-rs v{ver}") }
	})
}
