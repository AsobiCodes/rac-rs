//! [PC] Dev-only tuning without a rebuild: `RAC_WGSL_DIR` (shader modules read from a folder) and `RAC_TUNE` (named values read by
//! `devtune::f32`). Off unless those environment variables are set; players never set them.
// [PC] Dev-only tuning without a rebuild. Off unless the env vars are set (players never set them).
// RAC_WGSL_DIR=DIR: each shader module reads DIR/<label>.wgsl when that file exists; otherwise it writes its built-in source
// there (so the first run fills DIR) and uses the built-in one. Edit the file and restart the game: no build.
// RAC_TUNE="NAME=1.5,OTHER=2": `devtune::f32("NAME", default)` answers the value. For lane experiments only (read it at init
// or once per frame, not per vertex); a merge into dev turns the final values back into constants.
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::OnceLock;

/// The WGSL source of shader module `label`: `src`, unless RAC_WGSL_DIR holds an override file for it.
pub fn wgsl<'a>(label: &str, src: Cow<'a, str>) -> Cow<'a, str> {
	dump(label, &src);
	let Some(dir) = std::env::var_os("RAC_WGSL_DIR") else {
		return src;
	};
	let name: String = label.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
	let path = std::path::Path::new(&dir).join(format!("{name}.wgsl"));
	match std::fs::read_to_string(&path) {
		Ok(text) => {
			eprintln!("devtune: shader '{label}' from {}", path.display());
			Cow::Owned(text)
		}
		Err(_) => {
			let _ = std::fs::create_dir_all(&dir);
			match std::fs::write(&path, src.as_bytes()) {
				Ok(()) => eprintln!("devtune: shader '{label}' written to {} (edit it, restart)", path.display()),
				Err(e) => eprintln!("devtune: shader '{label}': cannot write {}: {e}", path.display()),
			}
			src
		}
	}
}

/// RAC_WGSL_DUMP=DIR: writes every shader module's source (each variant the game builds) to DIR/<label>_<hash>.wgsl, for the
/// FXC check (tools/fxc_check.py). Costs one env lookup per shader build, nothing per frame.
pub fn dump(label: &str, src: &str) {
	let Some(dir) = std::env::var_os("RAC_WGSL_DUMP") else {
		return;
	};
	let name: String = label.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
	let h = src.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
	let _ = std::fs::create_dir_all(&dir);
	let _ = std::fs::write(std::path::Path::new(&dir).join(format!("{name}_{h:016x}.wgsl")), src.as_bytes());
}

/// A lane tuning number: NAME's value in RAC_TUNE, else `default`.
#[allow(dead_code)]
pub fn f32(name: &str, default: f32) -> f32 {
	static TUNE: OnceLock<HashMap<String, f32>> = OnceLock::new();
	let m = TUNE.get_or_init(|| {
		let mut m = HashMap::new();
		for kv in std::env::var("RAC_TUNE").unwrap_or_default().split(',').filter(|s| !s.trim().is_empty()) {
			match kv.split_once('=').and_then(|(k, v)| v.trim().parse::<f32>().ok().map(|x| (k.trim().to_string(), x))) {
				Some((k, x)) => {
					m.insert(k, x);
				}
				None => eprintln!("devtune: RAC_TUNE part {kv:?} is not NAME=number"),
			}
		}
		if !m.is_empty() {
			eprintln!("devtune: RAC_TUNE {m:?}");
		}
		m
	});
	m.get(name).copied().unwrap_or(default)
}
