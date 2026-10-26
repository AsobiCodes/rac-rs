//! Tests of `store.rs` (the installed mods + `active.txt` of the in-game Mods page) and of the reloadable pack slot of `mod.rs`.
//! Everything runs on temporary folders (never the real mods folder, never the process-wide `RAC_MODS_DIR`).
use super::active::{self, Master};
use super::store::{listable_id, Store};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

fn dir(name: &str) -> PathBuf {
	let d = std::env::temp_dir().join(format!("rac_store_{}_{name}", std::process::id()));
	let _ = std::fs::remove_dir_all(&d);
	std::fs::create_dir_all(&d).unwrap();
	d
}

fn add(root: &Path, id: &str, json: &str) {
	std::fs::create_dir_all(root.join(id)).unwrap();
	std::fs::write(root.join(id).join("mod.json"), json).unwrap();
}

fn add_mod(root: &Path, id: &str, name: &str, kind: &str) {
	add(root, id, &format!("{{\"format\":1,\"id\":\"{id}\",\"name\":\"{name}\",\"kind\":\"{kind}\",\"game\":\"rac1\"}}"));
}

fn names(s: &Store) -> Vec<&str> {
	s.mods.iter().map(|m| m.name.as_str()).collect()
}

#[test]
fn the_listable_ids() {
	for ok in ["alpha", "my pack", "Pack-2_x", "a.b"] {
		assert!(listable_id(ok), "{ok}");
	}
	for bad in ["", ".hidden", "..", "a/b", "a\\b", "c:d", "a#b", " lead", "trail ", "leftover.tmp", "nul\0"] {
		assert!(!listable_id(bad), "{bad:?}");
	}
}

#[test]
fn the_store_lists_installed_mods_by_name_and_skips_what_is_not_a_listable_mod() {
	let root = dir("list");
	add_mod(&root, "zeta", "alpha pack", "textures");
	add_mod(&root, "beta", "Beta pack", "script");
	add_mod(&root, "gamma", "Alpha Pack", "native");
	add(&root, "noname", "{\"format\":1,\"game\":\"rac1\"}");
	add_mod(&root, ".hidden", "Hidden", "textures");
	add_mod(&root, "old.tmp", "Tmp", "textures");
	add_mod(&root, "has#hash", "Hash", "textures");
	add(&root, "broken", "{ not json");
	std::fs::create_dir_all(root.join("nojson")).unwrap();
	std::fs::write(root.join("stray.txt"), "x").unwrap();
	let s = Store::load(&root);
	// case-insensitive name order, then id; the name falls back to the id; the kind to "textures"
	assert_eq!(names(&s), vec!["Alpha Pack", "alpha pack", "Beta pack", "noname"], "{:?}", s.mods);
	assert_eq!(s.mods.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["gamma", "zeta", "beta", "noname"]);
	assert_eq!(s.mods[3].kind, "textures");
	assert_eq!(s.master_of("beta"), Some(Master::Mods));
	assert_eq!(s.master_of("zeta"), Some(Master::Textures));
	assert_eq!(s.master_of("broken"), None);
	assert!(s.ids.is_empty() && s.masters.is_on(Master::Textures) && s.masters.is_on(Master::Mods), "no active.txt = nothing switched on, both ticks on");
}

#[test]
fn a_mod_of_another_game_or_format_is_not_listed_but_its_tick_still_holds_it() {
	let root = dir("foreign");
	add_mod(&root, "mine", "Mine", "script");
	add(&root, "other_game", "{\"format\":1,\"name\":\"Other\",\"kind\":\"script\",\"game\":\"rac2\"}");
	add(&root, "future", "{\"format\":2,\"name\":\"Future\",\"kind\":\"native\",\"game\":\"rac1\"}");
	std::fs::write(root.join("active.txt"), format!("{}\n# Load mods: off\n# other_game\nfuture\nmine\n", active::HEADER)).unwrap();
	let s = Store::load(&root);
	assert_eq!(names(&s), vec!["Mine"], "only the rac1 format 1 mod is listed");
	assert_eq!(s.ids, vec!["other_game", "future", "mine"], "held ids and the foreign ones stay in the order");
	assert!(!s.masters.is_on(Master::Mods) && s.masters.is_on(Master::Textures));
	// switching the tick on gives the plain ids back, in place
	let mut s = s;
	s.masters.set(Master::Mods, true);
	assert_eq!(s.render(), format!("{}\nother_game\nfuture\nmine\n", active::HEADER));
}

#[test]
fn set_mod_puts_a_new_one_on_top_and_never_twice() {
	let root = dir("setmod");
	add_mod(&root, "a", "A", "textures");
	add_mod(&root, "b", "B", "textures");
	let mut s = Store::load(&root);
	s.set_mod("a", true);
	s.set_mod("b", true);
	s.set_mod("b", true);
	assert_eq!(s.ids, vec!["b", "a"], "a newly switched on mod is the highest priority");
	assert!(s.is_on("a") && s.is_on("b"));
	s.set_mod("a", false);
	s.set_mod("a", false);
	assert_eq!(s.ids, vec!["b"]);
	assert!(!s.is_on("a"));
}

#[test]
fn save_writes_the_launcher_file_and_load_reads_it_back() {
	let root = dir("save");
	add_mod(&root, "tex", "Tex", "textures");
	add_mod(&root, "scr", "Scr", "script");
	let mut s = Store::load(&root);
	s.set_mod("tex", true);
	s.set_mod("scr", true);
	s.masters.set(Master::Textures, false);
	s.save().unwrap();
	let text = std::fs::read_to_string(root.join("active.txt")).unwrap();
	assert_eq!(text, format!("{}\n# Load texture packs: off\nscr\n# tex\n", active::HEADER));
	assert!(!root.join("active.txt.tmp").exists(), "the temporary file is gone");
	let back = Store::load(&root);
	assert_eq!((back.ids.clone(), back.masters), (s.ids.clone(), s.masters));
	assert_eq!(back.render(), text, "a second round trip changes nothing");
	// the game's own reader sees the script mod only
	assert_eq!(super::mods::parse_active(&text), vec!["scr"]);
}

#[test]
fn a_foreign_comment_and_a_bom_crlf_file_survive_a_save() {
	let root = dir("foreign_lines");
	add_mod(&root, "a", "A", "textures");
	std::fs::write(root.join("active.txt"), "\u{feff}# my own note\r\na\r\n").unwrap();
	let mut s = Store::load(&root);
	s.set_mod("a", false);
	s.set_mod("a", true);
	s.save().unwrap();
	let text = std::fs::read_to_string(root.join("active.txt")).unwrap();
	assert_eq!(text, format!("{}\n# my own note\na\n", active::HEADER), "LF, no BOM, the note stays above its id");
}

#[test]
fn a_failed_rename_reports_the_error_and_leaves_no_temporary_file() {
	let root = dir("failsave");
	add_mod(&root, "a", "A", "textures");
	// a folder named active.txt: the rename onto it fails
	std::fs::create_dir_all(root.join("active.txt")).unwrap();
	let mut s = Store::load(&root);
	s.set_mod("a", true);
	assert!(s.save().is_err());
	assert!(!root.join("active.txt.tmp").exists(), "no temporary file is left behind");
}

#[test]
fn save_creates_the_mods_folder_when_it_is_missing() {
	let root = dir("mkdir").join("not_yet");
	let s = Store::load(&root);
	assert!(s.mods.is_empty());
	s.save().unwrap();
	assert_eq!(std::fs::read_to_string(root.join("active.txt")).unwrap(), format!("{}\n", active::HEADER));
}

// ---- the reloadable pack slot (mod.rs) ----

/// Writes a textures mod `id` (an empty textures folder) and an active.txt that lists `lines`.
fn pack_root(name: &str, lines: &str) -> PathBuf {
	let root = dir(name);
	add_mod(&root, "tex1", "Tex 1", "textures");
	std::fs::create_dir_all(root.join("tex1").join("textures")).unwrap();
	std::fs::write(root.join("active.txt"), lines).unwrap();
	root
}

#[test]
fn a_changed_active_txt_rebuilds_the_packs_at_the_next_look_and_an_unchanged_one_does_not() {
	let root = pack_root("refresh", "tex1\n");
	let slot = Mutex::new(None);
	// the first look builds
	assert!(!super::refresh_slot(&slot, Some(&root)), "an empty slot is built by `packs()`, not by the refresh");
	*slot.lock().unwrap() = Some(super::build_at(Some(&root)));
	let ids = |slot: &Mutex<Option<(String, Option<std::sync::Arc<super::Packs>>)>>| -> Option<Vec<String>> {
		slot.lock().unwrap().as_ref().and_then(|(_, p)| p.as_ref().map(|p| p.active.mods.iter().map(|m| m.id.clone()).collect()))
	};
	assert_eq!(ids(&slot), Some(vec!["tex1".to_string()]));
	assert!(!super::refresh_slot(&slot, Some(&root)), "same text: kept");
	// the Mods page switches the pack off (a held id)
	let mut s = Store::load(&root);
	s.set_mod("tex1", true);
	s.masters.set(Master::Textures, false);
	s.save().unwrap();
	assert!(super::refresh_slot(&slot, Some(&root)), "the file changed: rebuilt");
	assert_eq!(ids(&slot), None, "no pack is on any more");
	assert!(!super::refresh_slot(&slot, Some(&root)), "and then it is the same text again");
	// and back on
	s.masters.set(Master::Textures, true);
	s.save().unwrap();
	assert!(super::refresh_slot(&slot, Some(&root)));
	assert_eq!(ids(&slot), Some(vec!["tex1".to_string()]));
}

#[test]
fn a_missing_mods_folder_is_an_empty_text_and_no_packs() {
	let root = dir("nomods").join("absent");
	assert_eq!(super::active_text_at(Some(&root)), "");
	assert_eq!(super::active_text_at(None), "");
	let (t, p) = super::build_at(Some(&root));
	assert!(t.is_empty() && p.is_none());
}

#[test]
fn a_script_mod_is_listed_and_switched_by_its_manifest_id_not_its_folder() {
	let root = dir("script_id");
	// A GitHub zip unpacks as `<repo>-main`; the manifest id is what active.txt lists.
	for (folder, manifest) in [("arsenal-mastery-main", r#"{"id":"arsenal-mastery","name":"Arsenal Mastery"}"#), ("noid", r#"{"name":"No Id"}"#), ("copy-of-am", r#"{"id":"arsenal-mastery","name":"Arsenal Copy"}"#), ("other-game", r#"{"id":"x","games":{"rac2":{}}}"#), ("bad id", r#"{"id":"a#b"}"#)] {
		std::fs::create_dir_all(root.join(folder)).unwrap();
		std::fs::write(root.join(folder).join("omni-mod.json"), manifest).unwrap();
	}
	let mut s = Store::load(&root);
	let listed: Vec<(&str, &str, &str)> = s.mods.iter().map(|m| (m.id.as_str(), m.folder.as_str(), m.kind.as_str())).collect();
	assert_eq!(listed, [("arsenal-mastery", "arsenal-mastery-main", "script"), ("noid", "noid", "script")], "the first folder in name order keeps a duplicated id; no rac1 / unusable id: not listed");
	assert_eq!(s.master_of("arsenal-mastery"), Some(Master::Mods));
	s.set_mod("arsenal-mastery", true);
	s.save().unwrap();
	let text = std::fs::read_to_string(root.join("active.txt")).unwrap();
	assert!(text.lines().any(|l| l == "arsenal-mastery") && !text.contains("-main"), "{text}");
	assert!(Store::load(&root).is_on("arsenal-mastery"));
}
