//! Where the images of a texture pack are, for folders a player copied by hand (v0.4.1, issue #28).
//!
//! The documented layout is `mods/<pack>/textures/...` (+ `mod.json`). A player who copies a PCSX2 pack into `mods\` gets other
//! shapes: `mods/<pack>/replacements/*.png`, `mods/<serial>/replacements/...`, `mods/replacements/*.png`, a whole PCSX2
//! `textures` folder, or the images loose in the pack folder. The index scans a folder recursively, so what matters is only that the
//! folder (or its `textures` folder) holds files PCSX2 would load (a name `name::parse_file_name` knows). A folder without `mod.json`
//! and without such files is not a pack. This is the one rule both the Mods page (`store.rs`) and the loader (`mods::load_active`) use.
use super::name;
use std::path::{Path, PathBuf};

/// Folders deeper than this under a pack are not looked into for the existence check (the index itself goes 16 deep).
const DEPTH: u32 = 6;
/// Directory entries looked at before giving up on a folder that has no replacement files (a big unrelated folder must not slow the menu).
const BUDGET: usize = 4000;

/// The folder to index for the mod at `folder`: its `textures` folder when there is one, else `folder` itself when it holds
/// replacement files anywhere below it, else None.
pub fn textures_dir(folder: &Path) -> Option<PathBuf> {
	let t = folder.join("textures");
	if t.is_dir() {
		return Some(t);
	}
	has_replacements(folder).then(|| folder.to_path_buf())
}

/// Whether `dir` holds at least one file with a PCSX2 replacement name (png / dds), searched breadth-first within the budget.
pub fn has_replacements(dir: &Path) -> bool {
	let (mut left, mut level) = (BUDGET, vec![dir.to_path_buf()]);
	for _ in 0..=DEPTH {
		let mut next = Vec::new();
		for d in level {
			let Ok(rd) = std::fs::read_dir(&d) else { continue };
			for e in rd.flatten() {
				left = match left.checked_sub(1) {
					Some(l) => l,
					None => return false,
				};
				let Ok(ft) = e.file_type() else { continue };
				if ft.is_dir() {
					next.push(e.path());
				} else if name::parse_file_name(&e.file_name().to_string_lossy()).is_some() {
					return true;
				}
			}
		}
		level = next;
	}
	false
}

/// Loose replacement files directly in the mods folder (`mods\1234-...png`, not inside a pack folder): these cannot be a pack, the
/// player has to put them into a folder of their own. The count, for one log line.
pub fn loose_files(root: &Path) -> usize {
	let Ok(rd) = std::fs::read_dir(root) else { return 0 };
	rd.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_file()) && name::parse_file_name(&e.file_name().to_string_lossy()).is_some()).count()
}

#[cfg(test)]
mod tests {
	use super::*;

	fn tmp(tag: &str) -> PathBuf {
		let d = std::env::temp_dir().join(format!("texpack_layout_{tag}_{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&d);
		std::fs::create_dir_all(&d).unwrap();
		d
	}

	/// A file name PCSX2 loads: TEX0 hash, CLUT hash, 16 hex digits each.
	const IMG: &str = "1-2-00005dd3.dds";

	fn put(root: &Path, rel: &str) {
		let p = root.join(rel);
		std::fs::create_dir_all(p.parent().unwrap()).unwrap();
		std::fs::write(p, b"x").unwrap();
	}

	#[test]
	fn the_shapes_a_player_copies_in_are_found() {
		let root = tmp("shapes");
		put(&root, &format!("documented/textures/{IMG}"));
		put(&root, &format!("pcsx2serial/SCES-50916/replacements/{IMG}"));
		put(&root, &format!("repl/replacements/{IMG}"));
		put(&root, &format!("loose/{IMG}"));
		put(&root, "notes/readme.txt");
		put(&root, "wrongext/a.txt");
		std::fs::create_dir_all(root.join("empty")).unwrap();
		assert_eq!(textures_dir(&root.join("documented")), Some(root.join("documented").join("textures")));
		for f in ["pcsx2serial", "repl", "loose"] {
			assert_eq!(textures_dir(&root.join(f)), Some(root.join(f)), "{f}");
		}
		for f in ["notes", "wrongext", "empty", "missing"] {
			assert_eq!(textures_dir(&root.join(f)), None, "{f}");
		}
		let _ = std::fs::remove_dir_all(&root);
	}

	#[test]
	fn loose_files_in_the_mods_folder_are_counted_not_taken_for_a_pack() {
		let root = tmp("loose");
		put(&root, IMG);
		put(&root, "pack/x.txt");
		assert_eq!(loose_files(&root), 1);
		assert_eq!(textures_dir(&root.join("pack")), None);
		let _ = std::fs::remove_dir_all(&root);
	}

	#[test]
	fn a_huge_folder_without_images_gives_up() {
		let root = tmp("huge");
		for i in 0..(BUDGET + 50) {
			std::fs::write(root.join(format!("f{i}.txt")), b"").unwrap();
		}
		assert!(!has_replacements(&root));
		let _ = std::fs::remove_dir_all(&root);
	}
}
