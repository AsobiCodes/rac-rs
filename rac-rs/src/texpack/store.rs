//! The installed mods and `mods/active.txt` as one store: what the game's own Mods page (Pause -> Options -> Mods, retail/pcmods.rs)
//! lists and switches. Same folder layout and same file format as the launcher (`active.rs`); the page writes the file at every
//! change, the launcher and the next start read it.
use super::active::{self, Kept, Master, Masters};
use super::json::{self, Json};
use super::mods;
use std::path::{Path, PathBuf};

/// One installed mod that can be switched here (`mods/<id>/mod.json` readable, made for rac1; or an `omni-mod.json` (compat: or `openrac-mod.json`): kind
/// "script" / "native").
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
	/// The id `active.txt` lists: a texture pack's folder name, a script / native mod's MANIFEST id (`omni-mod.json` `"id"`; the folder
	/// name when it has none), so a folder like `arsenal-mastery-main` (a GitHub zip) still lists as `arsenal-mastery`.
	pub id: String,
	/// The mod's folder under the mods root.
	pub folder: String,
	/// The mod.json `name` (the id when it has none).
	pub name: String,
	/// The mod.json `kind` ("textures" when it has none, as `mods::parse_mod_json` reads it).
	pub kind: String,
}

pub struct Store {
	pub root: PathBuf,
	/// The mods this page lists, by name.
	pub mods: Vec<Installed>,
	/// active.txt: the ids switched on per mod, highest priority first (ids that are not installed stay, so the order is not lost).
	pub ids: Vec<String>,
	pub masters: Masters,
	kept: Kept,
	/// The kind of every readable mod.json (also those made for another game): a tick holds those back too.
	kinds: Vec<(String, String)>,
}

fn text_field(j: &Json, key: &str) -> String {
	j.get(key).and_then(Json::as_str).unwrap_or("").to_string()
}

/// (name, kind, listable) of a mod.json: a format other than 1 or a game other than rac1 is not listed.
fn read_info(text: &str, id: &str) -> Option<(Installed, bool)> {
	let j = json::parse(text).ok()?;
	if !matches!(j, Json::Obj(_)) {
		return None;
	}
	let kind = j.get("kind").and_then(Json::as_str).unwrap_or("textures").to_string();
	let name = Some(text_field(&j, "name")).filter(|n| !n.is_empty()).unwrap_or_else(|| id.to_string());
	let format_ok = j.get("format").and_then(Json::as_f64).is_none_or(|f| f == 1.0);
	let game_ok = j.get("game").and_then(Json::as_str).is_none_or(|g| g == "rac1");
	Some((Installed { id: id.to_string(), folder: id.to_string(), name, kind }, format_ok && game_ok))
}

/// (name, kind, listable) of an `omni-mod.json` (a script or native mod, no `mod.json`) in folder `folder`: id = the manifest's
/// `"id"` (else the folder name), kind "script", or "native" for `"type": "native"` / a `plugin` table; not listed when its `games`
/// object has no rac1 or its id cannot be a line of active.txt.
fn read_omni_manifest(text: &str, folder: &str) -> Option<(Installed, bool)> {
	let j = json::parse(text).ok()?;
	if !matches!(j, Json::Obj(_)) {
		return None;
	}
	let native = j.get("plugin").is_some() || j.get("type").and_then(Json::as_str) == Some("native");
	let id = Some(text_field(&j, "id")).filter(|n| !n.is_empty()).unwrap_or_else(|| folder.to_string());
	let name = Some(text_field(&j, "name")).filter(|n| !n.is_empty()).unwrap_or_else(|| id.clone());
	let game_ok = listable_id(&id) && match j.get("games") {
		Some(g @ Json::Obj(_)) => g.get("rac1").is_some(),
		_ => true,
	};
	Some((Installed { id, folder: folder.to_string(), name, kind: if native { "native" } else { "script" }.to_string() }, game_ok))
}

/// Whether a folder name can be a listed mod: a single folder name for the game's loader (`mods::id_ok`), not a leftover `.tmp`, and
/// writable into active.txt (a `#` would start a comment, outer spaces are trimmed away).
pub fn listable_id(id: &str) -> bool {
	mods::id_ok(id) && !id.ends_with(".tmp") && !id.contains('#') && id.trim() == id && !id.is_empty()
}

impl Store {
	/// Reads `root` (the mods folder): every `<id>/mod.json` and `active.txt` (missing = nothing switched on).
	pub fn load(root: &Path) -> Store {
		let (mut mods, mut kinds) = (Vec::new(), Vec::new());
		if let Ok(rd) = std::fs::read_dir(root) {
			for e in rd.flatten() {
				let id = e.file_name().to_string_lossy().to_string();
				if !e.path().is_dir() || !listable_id(&id) {
					continue;
				}
				let read = |f: &str| std::fs::read(e.path().join(f)).ok().map(|b| String::from_utf8_lossy(&b).into_owned());
				let script = || crate::omni::modset::manifest_file(&e.path()).and_then(|p| std::fs::read_to_string(p).ok());
				let bare = || super::layout::textures_dir(&e.path()).map(|_| (Installed { id: id.clone(), folder: id.clone(), name: id.clone(), kind: "textures".to_string() }, true));
				let info = read("mod.json").and_then(|t| read_info(&t, &id)).or_else(|| script().and_then(|t| read_omni_manifest(&t, &id)));
				// No mod.json and no manifest: a PCSX2 pack copied in by hand is still a texture pack (`mods.rs` loads it the same way).
				let info = info.or_else(|| if read("mod.json").is_none() { bare() } else { None });
				let Some((m, listed)) = info else { continue };
				kinds.push((m.id.clone(), m.kind.clone()));
				if listed {
					mods.push(m);
				}
			}
		}
		let loose = super::layout::loose_files(root);
		if loose > 0 {
			eprintln!("mods: {loose} texture files lie loose in {}: put them into a folder of their own: <pack name>/textures/", root.display());
		}
		// Two folders with one manifest id (a copy next to the original): the first folder in name order is the mod, one line says so.
		mods.sort_by(|a: &Installed, b: &Installed| a.folder.cmp(&b.folder));
		let mut seen: Vec<String> = Vec::new();
		mods.retain(|m| {
			let fresh = !seen.contains(&m.id);
			if !fresh {
				eprintln!("mods: folder {} has the id {} of an earlier folder: ignored", m.folder, m.id);
			}
			seen.push(m.id.clone());
			fresh
		});
		mods.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
		let text = std::fs::read(root.join("active.txt")).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
		let mut s = Store { root: root.to_path_buf(), mods, ids: Vec::new(), masters: Masters::default(), kept: Vec::new(), kinds };
		let p = active::parse(&text, &|id| s.master_of(id));
		(s.ids, s.masters, s.kept) = (p.ids, p.masters, p.kept);
		s
	}

	/// The tick that governs mod `id` (None = not installed or its mod.json cannot be read).
	pub fn master_of(&self, id: &str) -> Option<Master> {
		self.kinds.iter().find(|(i, _)| i == id).map(|(_, k)| Master::of_kind(k))
	}

	/// Whether mod `id` is switched on per mod (a held mod counts: its tick is a separate switch).
	pub fn is_on(&self, id: &str) -> bool {
		self.ids.iter().any(|i| i == id)
	}

	/// Switches mod `id` on (at the top = highest priority, as the launcher does) or off.
	pub fn set_mod(&mut self, id: &str, on: bool) {
		match (on, self.is_on(id)) {
			(true, false) => self.ids.insert(0, id.to_string()),
			(false, true) => self.ids.retain(|i| i != id),
			_ => {}
		}
	}

	/// The file text as the launcher would write it.
	pub fn render(&self) -> String {
		active::render(&self.ids, self.masters, &self.kept, &|id| self.master_of(id))
	}

	/// Writes `active.txt` (temporary file + rename, like the launcher).
	pub fn save(&self) -> std::io::Result<()> {
		std::fs::create_dir_all(&self.root)?;
		let (p, tmp) = (self.root.join("active.txt"), self.root.join("active.txt.tmp"));
		std::fs::write(&tmp, self.render())?;
		std::fs::rename(&tmp, p).inspect_err(|_| {
			let _ = std::fs::remove_file(&tmp);
		})
	}
}
