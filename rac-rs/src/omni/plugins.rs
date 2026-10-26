//! The loader's plugins list, built from the files the player's switches write: `mods/active.txt` (which script / native mods are on,
//! in which order; the launcher's format, `texpack/active.rs`, with the "Load mods" master tick) and `mods/settings.json`
//! (`{"format":1,"mods":{"<id>":{"<key>": value}}}`, written by the launcher and by the game's Mods pages).
//!
//! - An explicit `mods/plugins.json` wins over both (tools and test launchers use it; the loader reads it as it is).
//! - Otherwise a mod is loaded only when its folder name is a plain line of `active.txt` and the "Load mods" tick is on. A mod with NO
//!   line is OFF: the same rule as a texture pack (`texpack::mods::load_active` loads only what the file lists), so a folder dropped
//!   into `mods/` never starts by itself. No `active.txt` at all = nothing on.
//! - Each loaded mod gets its values from `settings.json` as they stand (unknown ids and keys stay in the file; the loader types them by
//!   the manifest and falls back to the default for a missing, mistyped or out-of-range one).
//!
//! `settings.json` is read and written here whole (as an ordered JSON value), so a write changes one key and keeps everything else.
use super::json::{self, Json};
use super::load::ModDir;
use crate::texpack::active::{self, Master};
use std::path::Path;

pub const SETTINGS_FILE: &str = "settings.json";

/// What `omni::load::start` hands the loader.
#[derive(Debug, PartialEq)]
pub struct Plan {
	/// The plugins.json text.
	pub text: String,
	/// Manifest ids of the mods that are switched on in it, in load order.
	pub enabled: Vec<String>,
	/// `plugins.json` was there and was used as it is.
	pub explicit: bool,
}

/// `settings.json` as a JSON object (missing or unreadable = a fresh `{"format":1,"mods":{}}`).
pub fn read_settings(dir: &Path) -> Json {
	let parsed = std::fs::read(dir.join(SETTINGS_FILE)).ok().and_then(|b| json::parse(String::from_utf8_lossy(&b).trim_start_matches('\u{feff}')).ok());
	match parsed {
		Some(j @ Json::Obj(_)) => j,
		_ => Json::Obj(vec![("format".into(), Json::Num(1.0)), ("mods".into(), Json::Obj(Vec::new()))]),
	}
}

/// The values stored for mod `id`.
pub fn values_of<'a>(doc: &'a Json, id: &str) -> &'a [(String, Json)] {
	doc.get("mods").and_then(|m| m.get(id)).map_or(&[], Json::entries)
}

/// Mod `id`'s value for `key`.
pub fn value_of<'a>(doc: &'a Json, id: &str, key: &str) -> Option<&'a Json> {
	doc.get("mods").and_then(|m| m.get(id)).and_then(|m| m.get(key))
}

fn obj_mut<'a>(j: &'a mut Json, key: &str) -> &'a mut Json {
	let Json::Obj(v) = j else { unreachable!("callers pass objects") };
	if !v.iter().any(|(k, _)| k == key) {
		v.push((key.to_string(), Json::Obj(Vec::new())));
	}
	let slot = &mut v.iter_mut().find(|(k, _)| k == key).expect("just inserted").1;
	if !matches!(slot, Json::Obj(_)) {
		*slot = Json::Obj(Vec::new());
	}
	slot
}

/// Sets mod `id`'s `key` to `value` (everything else stays, in its order).
pub fn set_value(doc: &mut Json, id: &str, key: &str, value: Json) {
	if !matches!(doc, Json::Obj(_)) {
		*doc = Json::Obj(Vec::new());
	}
	if doc.get("format").is_none() {
		if let Json::Obj(v) = doc {
			v.insert(0, ("format".into(), Json::Num(1.0)));
		}
	}
	let m = obj_mut(obj_mut(doc, "mods"), id);
	if let Json::Obj(v) = m {
		match v.iter_mut().find(|(k, _)| k == key) {
			Some(slot) => slot.1 = value,
			None => v.push((key.to_string(), value)),
		}
	}
}

/// Writes `settings.json` (temporary file + rename).
pub fn write_settings(dir: &Path, doc: &Json) -> std::io::Result<()> {
	std::fs::create_dir_all(dir)?;
	let (p, tmp) = (dir.join(SETTINGS_FILE), dir.join("settings.json.tmp"));
	std::fs::write(&tmp, doc.to_text() + "\n")?;
	std::fs::rename(&tmp, p).inspect_err(|_| {
		let _ = std::fs::remove_file(&tmp);
	})
}

/// The enabled ids of an explicit plugins.json text (entries with `"enabled"` other than false).
fn explicit_ids(text: &str) -> Vec<String> {
	let Ok(j) = json::parse(text) else { return Vec::new() };
	let list = j.get("mods").map_or(&[][..], Json::arr);
	list.iter().filter(|m| m.get("enabled").is_none_or(|e| *e != Json::Bool(false))).filter_map(|m| m.get("id").and_then(Json::str).map(str::to_string)).collect()
}

/// The folders whose manifest id an earlier folder (name order) already has: ignored, the first one is the mod (`found` is in folder order).
pub fn duplicates(found: &[ModDir]) -> Vec<&ModDir> {
	found.iter().enumerate().filter(|(i, f)| found[..*i].iter().any(|g| g.id == f.id)).map(|(_, f)| f).collect()
}

/// The ids of `active.txt` that are installed script / native mods, in file order; `Load mods` off = none. A line is a mod's MANIFEST id
/// (`omni-mod.json` `"id"`), not its folder name (a GitHub zip unpacks as `arsenal-mastery-main`); two folders with one id: the first
/// in folder-name order is the mod.
pub fn active_ids(dir: &Path, found: &[ModDir]) -> Vec<String> {
	let text = std::fs::read(dir.join("active.txt")).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
	let installed = |id: &str| found.iter().any(|f| f.id == id);
	let p = active::parse(&text, &|id| installed(id).then_some(Master::Mods));
	if !p.masters.is_on(Master::Mods) {
		return Vec::new();
	}
	let mut out: Vec<String> = Vec::new();
	for id in p.ids {
		if installed(&id) && !out.contains(&id) {
			out.push(id);
		}
	}
	out
}

/// plugins.json text for `ids` in this order, all enabled, with the values `settings.json` holds for each.
pub fn build(ids: &[String], doc: &Json) -> String {
	let list: Vec<Json> = ids
		.iter()
		.map(|id| {
			let values: Vec<(String, Json)> = values_of(doc, id).iter().filter(|(_, v)| *v != Json::Null).cloned().collect();
			Json::Obj(vec![("id".into(), Json::Str(id.clone())), ("enabled".into(), Json::Bool(true)), ("settings".into(), Json::Obj(values))])
		})
		.collect();
	Json::Obj(vec![("format".into(), Json::Num(1.0)), ("mods".into(), Json::Arr(list))]).to_text()
}

/// The plan for the mods folder `dir` holding the mod folders `found`.
pub fn plan(dir: &Path, found: &[ModDir]) -> Plan {
	if let Ok(text) = std::fs::read_to_string(dir.join("plugins.json")) {
		return Plan { enabled: explicit_ids(&text), text, explicit: true };
	}
	let enabled = active_ids(dir, found);
	Plan { text: build(&enabled, &read_settings(dir)), enabled, explicit: false }
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::texpack::active::HEADER;
	use std::path::PathBuf;

	fn dir(name: &str) -> PathBuf {
		let d = std::env::temp_dir().join(format!("rac_plugins_{}_{name}", std::process::id()));
		let _ = std::fs::remove_dir_all(&d);
		std::fs::create_dir_all(&d).unwrap();
		d
	}

	fn md(folder: &str, id: &str) -> ModDir {
		ModDir { folder: folder.into(), id: id.into(), native: false }
	}

	fn ids_of(p: &Plan) -> Vec<&str> {
		p.enabled.iter().map(String::as_str).collect()
	}

	#[test]
	fn a_mod_is_on_only_with_a_line_and_the_mods_tick() {
		let d = dir("onoff");
		let found = [md("alpha", "alpha"), md("beta", "beta")];
		assert!(plan(&d, &found).enabled.is_empty(), "no active.txt: nothing on");
		std::fs::write(d.join("active.txt"), format!("{HEADER}\nbeta\n")).unwrap();
		assert_eq!(ids_of(&plan(&d, &found)), ["beta"], "alpha has no line: off");
		std::fs::write(d.join("active.txt"), format!("{HEADER}\nbeta\nalpha\nghost\nbeta\n")).unwrap();
		assert_eq!(ids_of(&plan(&d, &found)), ["beta", "alpha"], "file order, ghosts and repeats dropped");
		std::fs::write(d.join("active.txt"), format!("{HEADER}\n# alpha\nbeta\n")).unwrap();
		assert_eq!(ids_of(&plan(&d, &found)), ["beta"], "a commented line is off");
		// The tick: "Load mods" off holds every script mod back (their lines become `# id`).
		std::fs::write(d.join("active.txt"), format!("{HEADER}\n# Load mods: off\n# beta\n# alpha\n")).unwrap();
		let p = plan(&d, &found);
		assert!(p.enabled.is_empty() && !p.explicit);
		assert_eq!(p.text, r#"{"format":1,"mods":[]}"#);
		// A texture-pack tick off does not hold script mods.
		std::fs::write(d.join("active.txt"), format!("{HEADER}\n# Load texture packs: off\nalpha\n")).unwrap();
		assert_eq!(ids_of(&plan(&d, &found)), ["alpha"]);
	}

	#[test]
	fn a_line_is_the_manifest_id_not_the_folder_name() {
		let d = dir("folder");
		let found = [md("arsenal-mastery-main", "arsenal-mastery")];
		std::fs::write(d.join("active.txt"), format!("{HEADER}\narsenal-mastery\n")).unwrap();
		assert_eq!(ids_of(&plan(&d, &found)), ["arsenal-mastery"], "the folder is arsenal-mastery-main, the line is the manifest id");
		std::fs::write(d.join("active.txt"), format!("{HEADER}\narsenal-mastery-main\n")).unwrap();
		assert!(plan(&d, &found).enabled.is_empty(), "the folder name is no id: off");
		// settings.json is keyed by the manifest id as well.
		std::fs::write(d.join("active.txt"), format!("{HEADER}\narsenal-mastery\n")).unwrap();
		std::fs::write(d.join("settings.json"), r#"{"format":1,"mods":{"arsenal-mastery":{"perks":false}}}"#).unwrap();
		let j = json::parse(&plan(&d, &found).text).unwrap();
		assert_eq!(j.get("mods").unwrap().arr()[0].get("settings").unwrap().get("perks"), Some(&Json::Bool(false)));
	}

	#[test]
	fn two_folders_with_one_id_the_first_in_name_order_wins() {
		let found = [md("am-copy", "arsenal-mastery"), md("am-main", "arsenal-mastery"), md("other", "other")];
		let dup: Vec<&str> = duplicates(&found).iter().map(|f| f.folder.as_str()).collect();
		assert_eq!(dup, ["am-main"]);
		let d = dir("dup");
		std::fs::write(d.join("active.txt"), format!("{HEADER}\narsenal-mastery\n")).unwrap();
		assert_eq!(ids_of(&plan(&d, &found)), ["arsenal-mastery"], "listed once");
	}

	#[test]
	fn plugins_json_overrides_everything() {
		let d = dir("explicit");
		let found = [md("alpha", "alpha"), md("beta", "beta")];
		std::fs::write(d.join("active.txt"), format!("{HEADER}\nalpha\n")).unwrap();
		let text = r#"{"format":1,"mods":[{"id":"beta","enabled":true,"settings":{"x":1}},{"id":"alpha","enabled":false}]}"#;
		std::fs::write(d.join("plugins.json"), text).unwrap();
		let p = plan(&d, &found);
		assert_eq!((p.text.as_str(), ids_of(&p), p.explicit), (text, vec!["beta"], true));
	}

	#[test]
	fn settings_reach_the_list_typed_as_stored() {
		let d = dir("settings");
		let found = [md("alpha", "alpha"), md("beta", "beta")];
		std::fs::write(d.join("active.txt"), format!("{HEADER}\nalpha\nbeta\n")).unwrap();
		let s = r#"{"format":1,"mods":{"alpha":{"rate":1.5,"mode":"hard","on":false,"n":3,"gone":null},"other":{"k":true}}}"#;
		std::fs::write(d.join("settings.json"), s).unwrap();
		let p = plan(&d, &found);
		let j = json::parse(&p.text).unwrap();
		let m = j.get("mods").unwrap().arr();
		assert_eq!(m.len(), 2);
		let a = m[0].get("settings").unwrap();
		assert_eq!(a.get("rate"), Some(&Json::Num(1.5)));
		assert_eq!(a.get("mode"), Some(&Json::Str("hard".into())));
		assert_eq!(a.get("on"), Some(&Json::Bool(false)));
		assert_eq!(a.get("n"), Some(&Json::Num(3.0)));
		assert!(a.get("gone").is_none(), "null is not passed");
		assert_eq!(m[1].get("settings"), Some(&Json::Obj(vec![])), "beta has no values: the manifest defaults");
		assert!(!p.text.contains("other"), "settings of a mod that is not listed stay in the file only");
	}

	#[test]
	fn settings_json_round_trips_and_keeps_unknown_ids_and_keys() {
		let d = dir("round");
		assert_eq!(read_settings(&d).to_text(), r#"{"format":1,"mods":{}}"#);
		let s = r#"{"format":1,"future":"kept","mods":{"zed":{"a":1},"alpha":{"rate":2,"mystery":[1,2]}}}"#;
		std::fs::write(d.join("settings.json"), s).unwrap();
		let mut doc = read_settings(&d);
		set_value(&mut doc, "alpha", "rate", Json::Num(0.5));
		set_value(&mut doc, "alpha", "new", Json::Bool(true));
		set_value(&mut doc, "fresh", "k", Json::Str("v".into()));
		write_settings(&d, &doc).unwrap();
		let back = std::fs::read_to_string(d.join("settings.json")).unwrap();
		assert_eq!(back.trim(), r#"{"format":1,"future":"kept","mods":{"zed":{"a":1},"alpha":{"rate":0.5,"mystery":[1,2],"new":true},"fresh":{"k":"v"}}}"#);
		assert!(!d.join("settings.json.tmp").exists());
		assert_eq!(value_of(&read_settings(&d), "alpha", "rate"), Some(&Json::Num(0.5)));
		// A broken file is replaced by a fresh document, not a crash.
		std::fs::write(d.join("settings.json"), "{ nope").unwrap();
		assert_eq!(read_settings(&d).to_text(), r#"{"format":1,"mods":{}}"#);
		let mut doc = Json::Null;
		set_value(&mut doc, "a", "k", Json::Num(1.0));
		assert_eq!(doc.to_text(), r#"{"format":1,"mods":{"a":{"k":1}}}"#);
	}
}
