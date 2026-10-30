//! `rac-rs omni-host-abi`: prints which OmniLoader interface this game's host serves, as one line of JSON on stdout, and exits at once
//! (nothing else starts: no window, no disc, no run log). The launcher runs it before it installs a newer loader library, so it only
//! installs one whose ABI major equals this host's (omni/cabi.rs `ABI_VERSION`) and whose features this host can serve. A game
//! before v0.4.2 does not know the word; the launcher then assumes major 1 (every game since v0.4.0 serves 1.6).
//!
//! `{"omni_host":{"abi":"1.6","size":360,"lib":"omniloader.dll"}}`: `abi` = major.minor, `size` = the `OmniHost` size announced to a loader
//! of that minor, `lib` = the file name the game looks for next to the exe.

use super::cabi::{host_size_for, ABI_VERSION, HOST_ABI_MINOR};

/// The line the subcommand prints.
pub fn line() -> String {
	format!("{{\"omni_host\":{{\"abi\":\"{}.{}\",\"size\":{},\"lib\":\"{}\"}}}}", ABI_VERSION, HOST_ABI_MINOR, host_size_for(HOST_ABI_MINOR), super::load::lib_name())
}

/// True (after printing the line) when the first program argument is `omni-host-abi`.
pub fn asked(args: &[String]) -> bool {
	if args.first().map(String::as_str) != Some("omni-host-abi") {
		return false;
	}
	println!("{}", line());
	true
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::omni::cabi::{HOST_SIZE_V1_4, HOST_SIZE_V1_6};

	#[test]
	fn the_line_names_the_abi_the_size_and_the_library() {
		let j = crate::omni::json::parse(&line()).expect("one line of JSON");
		let h = j.get("omni_host").expect("omni_host");
		assert_eq!(h.get("abi").and_then(|v| v.str()), Some("1.6"));
		assert_eq!(h.get("size").and_then(|v| v.as_num()), Some(HOST_SIZE_V1_6 as f64));
		assert_eq!(h.get("lib").and_then(|v| v.str()), Some(if cfg!(windows) { "omniloader.dll" } else { "libomniloader.so" }));
		assert!(!line().contains('\n'));
	}

	#[test]
	fn the_minor_constant_and_the_size_table_agree() {
		// Adding an ABI step means a new size constant, an arm in `host_size_for` and a new HOST_ABI_MINOR: all three move together.
		assert_eq!(HOST_ABI_MINOR, 6);
		assert_eq!(host_size_for(HOST_ABI_MINOR), HOST_SIZE_V1_6);
		assert_eq!(host_size_for(HOST_ABI_MINOR + 1), HOST_SIZE_V1_6, "a loader newer than the host gets the host's own size");
		assert_ne!(host_size_for(HOST_ABI_MINOR), host_size_for(HOST_ABI_MINOR - 2));
		assert_eq!(host_size_for(4), HOST_SIZE_V1_4);
	}

	#[test]
	fn only_the_exact_word_is_taken() {
		assert!(!asked(&[]));
		assert!(!asked(&["view".to_string(), "omni-host-abi".to_string()]));
		assert!(!asked(&["omni-host-abi2".to_string()]));
	}
}
