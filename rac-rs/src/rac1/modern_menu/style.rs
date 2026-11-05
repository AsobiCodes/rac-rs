//! The menu style: Modern (the engine's pages; default when the key is missing) or Classic (the retail-style option pages). The
//! player's key is `menustyle=modern|classic` in pc_options.txt (`PcOptions::extra_*`); `RAC_MENU_STYLE=classic|modern` wins (scripted
//! menu runs pin `classic`, ENV.md). The classic "Menu style" row and the modern one both end in `set_from_ui`.
use rac_engine::pcopts::PcOptions;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

/// The file's choice: true = Classic.
static FILE_CLASSIC: AtomicBool = AtomicBool::new(false);
static ENV: OnceLock<Option<String>> = OnceLock::new();

fn is_classic(v: &str) -> Option<bool> {
	match v.trim().to_ascii_lowercase().as_str() {
		"classic" => Some(true),
		"modern" => Some(false),
		_ => None,
	}
}

/// Pure core: the env value (if any, and valid) beats the file; otherwise the file; Modern when neither says Classic.
pub fn resolve(env: Option<&str>, file_classic: bool) -> bool {
	!env.and_then(is_classic).unwrap_or(file_classic)
}

/// Modern menus on (the default)?
pub fn modern() -> bool {
	let env = ENV.get_or_init(|| std::env::var("RAC_MENU_STYLE").ok());
	resolve(env.as_deref(), FILE_CLASSIC.load(Ordering::Relaxed))
}

/// The player's choice as the file holds it (what both Menu style rows show; `RAC_MENU_STYLE` is not part of it): true = Classic.
pub fn file_classic() -> bool {
	FILE_CLASSIC.load(Ordering::Relaxed)
}

/// `pcext::init`: the file's key.
pub fn init(opts: &PcOptions) {
	FILE_CLASSIC.store(opts.extra_get("menustyle").and_then(is_classic).unwrap_or(false), Ordering::Relaxed);
}

/// The player chose the style (the modern row: value 1 = Classic): remembered in the file's key (Modern = no key).
pub fn set_from_ui(opts: &mut PcOptions, classic: bool) {
	FILE_CLASSIC.store(classic, Ordering::Relaxed);
	opts.extra_set("menustyle", classic.then(|| "classic".to_string()));
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn menustyle_file_roundtrip_and_env_wins() {
		let _g = crate::retail::pcopts::globals_guard();
		assert!(resolve(None, false), "default: Modern");
		assert!(!resolve(None, true), "the file says Classic");
		assert!(!resolve(Some("classic"), false), "env Classic beats a Modern file");
		assert!(resolve(Some("MODERN"), true), "env Modern beats a Classic file");
		assert!(resolve(Some("nonsense"), false) && !resolve(Some("nonsense"), true), "an invalid env value is ignored");
		let mut o = PcOptions::new();
		set_from_ui(&mut o, true);
		assert_eq!(o.extra_get("menustyle"), Some("classic"));
		init(&o);
		assert!(FILE_CLASSIC.load(Ordering::Relaxed));
		set_from_ui(&mut o, false);
		assert_eq!(o.extra_get("menustyle"), None);
	}
}
