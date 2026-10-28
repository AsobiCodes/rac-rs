//! The mods contract for texture packs: `<install root>/mods/active.txt` lists the enabled mod ids (one per line, highest priority
//! first, `#` starts a comment) and `mods/<id>/mod.json` describes each one:
//! `{"format":1,"id","name","author","version","description","game":"rac1","kind":"textures","serials":[...],"source":"pcsx2",
//! "mip_names":"single|chain|both"}`; the images sit in `mods/<id>/textures/` (a PCSX2 `replacements` folder: subfolders allowed).
//! A pack is never refused for a serial mismatch (names are content hashes; a USA pack matches most of a PAL disc).

use super::json::{self, Json};
use std::path::{Path, PathBuf};

/// Which mip naming a pack uses (DESIGN.md 1.5): every level its own name ("single", the real USA pack), the hash of the chain from
/// the drawn level up ("chain", current PCSX2 with Hardware Mipmapping) or both (default; single is tried first).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MipNames {
	Single,
	Chain,
	Both,
}

/// One enabled textures mod.
#[derive(Clone, Debug)]
pub struct TexMod {
	pub id: String,
	pub name: String,
	pub author: String,
	pub version: String,
	pub description: String,
	pub serials: Vec<String>,
	pub mip_names: MipNames,
	/// `mods/<id>/textures`.
	pub textures: PathBuf,
}

/// The enabled textures mods in priority order, and what was skipped or odd (for the log).
#[derive(Debug, Default)]
pub struct Active {
	pub mods: Vec<TexMod>,
	pub notes: Vec<String>,
}

/// The ids of `active.txt`: trimmed lines, blank lines and `#` comments (whole-line or after the id) dropped, duplicates kept
/// once (first position wins).
pub fn parse_active(text: &str) -> Vec<String> {
	let mut ids: Vec<String> = Vec::new();
	for line in text.trim_start_matches('\u{feff}').lines() {
		let id = line.split('#').next().unwrap_or("").trim();
		if !id.is_empty() && !ids.iter().any(|i| i == id) {
			ids.push(id.to_string());
		}
	}
	ids
}

/// An id is a single folder name: no separators, no `..`, not hidden.
pub(super) fn id_ok(id: &str) -> bool {
	!id.starts_with('.') && !id.contains(['/', '\\', ':', '\0']) && id != ".."
}

fn text_field(j: &Json, key: &str) -> String {
	j.get(key).and_then(Json::as_str).unwrap_or("").to_string()
}

/// Reads a mod.json. `kind` other than "textures" and `game` other than "rac1" are reported as Err("not ..."); other problems are
/// real errors. Missing optional fields are empty.
pub fn parse_mod_json(text: &str, id: &str, dir: &Path) -> Result<TexMod, String> {
	let j = json::parse(text)?;
	if !matches!(j, Json::Obj(_)) {
		return Err("mod.json is not an object".into());
	}
	if let Some(game) = j.get("game").and_then(Json::as_str) {
		if game != "rac1" {
			return Err(format!("not a rac1 mod (game \"{game}\")"));
		}
	}
	if let Some(kind) = j.get("kind").and_then(Json::as_str) {
		if kind != "textures" {
			return Err(format!("not a textures mod (kind \"{kind}\")"));
		}
	}
	let mip_names = match j.get("mip_names").and_then(Json::as_str) {
		None | Some("both") => MipNames::Both,
		Some("single") => MipNames::Single,
		Some("chain") => MipNames::Chain,
		Some(o) => return Err(format!("unknown mip_names \"{o}\"")),
	};
	let serials = j.get("serials").and_then(Json::as_array).map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default();
	let declared = text_field(&j, "id");
	Ok(TexMod {
		id: if declared.is_empty() { id.to_string() } else { declared },
		name: text_field(&j, "name"),
		author: text_field(&j, "author"),
		version: text_field(&j, "version"),
		description: text_field(&j, "description"),
		serials,
		mip_names,
		textures: dir.join("textures"),
	})
}

/// The mods root of this run: `RAC_MODS_DIR` or `<install root>/mods`.
pub fn mods_root() -> Option<PathBuf> {
	if let Some(d) = std::env::var_os("RAC_MODS_DIR") {
		return Some(PathBuf::from(d));
	}
	crate::install_root().ok().map(|r| r.join("mods"))
}

/// Loads the enabled textures mods under `root` (the folder holding `active.txt`). No `active.txt` is an empty result, not an error.
pub fn load_active(root: &Path) -> Active {
	let mut out = Active::default();
	let Ok(text) = std::fs::read_to_string(root.join("active.txt")) else { return out };
	for id in parse_active(&text) {
		if !id_ok(&id) {
			out.notes.push(format!("mod id \"{id}\" skipped (not a folder name)"));
			continue;
		}
		let dir = root.join(&id);
		let md = match std::fs::read_to_string(dir.join("mod.json")) {
			Ok(t) => parse_mod_json(&t, &id, &dir),
			Err(_) if super::layout::textures_dir(&dir).is_some() => {
				out.notes.push(format!("mod \"{id}\": no mod.json, treated as a textures mod"));
				parse_mod_json("{}", &id, &dir)
			}
			Err(_) => Err("no mod.json".to_string()),
		};
		// A pack whose images are not in `textures/` (a PCSX2 `replacements` folder copied in as it is): index the folder that holds them.
		let md = md.map(|mut m| {
			if !m.textures.is_dir() {
				m.textures = super::layout::textures_dir(&dir).unwrap_or(m.textures);
			}
			m
		});
		match md {
			Ok(m) if m.textures.is_dir() => out.mods.push(m),
			Ok(m) => out.notes.push(format!("mod \"{}\": no textures folder at {}", m.id, m.textures.display())),
			Err(e) => out.notes.push(format!("mod \"{id}\" skipped: {e}")),
		}
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn active_txt_comments_blanks_and_duplicates() {
		let t = "\u{feff}# enabled mods, best first\nhd-pack  # the HD pack\n\n  other\r\nhd-pack\n#off\n";
		assert_eq!(parse_active(t), ["hd-pack", "other"]);
		assert!(parse_active("").is_empty());
	}

	#[test]
	fn mod_json_fields_and_defaults() {
		let t = r#"{"format":1,"id":"hd","name":"HD Remaster","author":"Someone","version":"1.2","description":"d","game":"rac1","kind":"textures","serials":["SCUS-97199"],"source":"pcsx2","mip_names":"single"}"#;
		let m = parse_mod_json(t, "folder", Path::new("m/folder")).unwrap();
		assert_eq!((m.id.as_str(), m.name.as_str(), m.version.as_str(), m.mip_names), ("hd", "HD Remaster", "1.2", MipNames::Single));
		assert_eq!(m.serials, ["SCUS-97199"]);
		assert_eq!(m.textures, Path::new("m/folder").join("textures"));
		let d = parse_mod_json("{}", "folder", Path::new("x")).unwrap();
		assert_eq!((d.id.as_str(), d.mip_names), ("folder", MipNames::Both));
	}

	#[test]
	fn other_kinds_and_games_are_skipped_not_serials() {
		assert!(parse_mod_json(r#"{"kind":"audio"}"#, "a", Path::new("x")).unwrap_err().contains("not a textures mod"));
		assert!(parse_mod_json(r#"{"game":"rac2"}"#, "a", Path::new("x")).unwrap_err().contains("not a rac1 mod"));
		assert!(parse_mod_json(r#"{"mip_names":"x"}"#, "a", Path::new("x")).is_err());
		// A serial that is not the disc's is fine.
		assert!(parse_mod_json(r#"{"serials":["SLUS-00000"]}"#, "a", Path::new("x")).is_ok());
		assert!(parse_mod_json("[1]", "a", Path::new("x")).is_err());
	}

	#[test]
	fn load_active_reads_the_folder_layout() {
		let root = std::env::temp_dir().join(format!("texpack_mods_{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(root.join("a/textures")).unwrap();
		std::fs::create_dir_all(root.join("bare/textures")).unwrap();
		std::fs::create_dir_all(root.join("snd")).unwrap();
		std::fs::write(root.join("a/mod.json"), r#"{"kind":"textures","name":"A"}"#).unwrap();
		std::fs::write(root.join("snd/mod.json"), r#"{"kind":"audio"}"#).unwrap();
		std::fs::write(root.join("active.txt"), "a\nsnd\nbare\n../evil\nmissing\n").unwrap();
		let act = load_active(&root);
		assert_eq!(act.mods.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["a", "bare"]);
		assert_eq!(act.notes.len(), 4, "{:?}", act.notes);
		assert!(load_active(&root.join("nothing")).mods.is_empty());
		let _ = std::fs::remove_dir_all(&root);
	}
}
