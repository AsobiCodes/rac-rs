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
