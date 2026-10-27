//! Mods of a game: the folder `<install root>\mods\` (contract with the game):
//!   mods\active.txt            enabled mod ids, one per line, highest priority first; `#` lines and blank lines are ignored
//!   mods\<id>\mod.json         {"format":1,"id","name","author","version","description","game":"rac1","kind":"textures","serials":[...],"source":"pcsx2"}
//!   mods\<id>\textures\...     for kind "textures": the contents of a PCSX2 pack's replacements folder, subfolders kept
//!   mods\<folder>\omni-mod.json   a script or native mod (format 2); active.txt lists the manifest `id` (the folder name may differ), settings.json is keyed by it, too
//!   mods\settings.json         {"format":1,"mods":{"<mod id>":{"<key>": value}}}: the values of script mods' settings (see `settings`)
//! Unknown kinds (or a newer format) are listed but cannot be enabled ("needs a newer version").
pub mod active;
pub mod detect;
pub mod import;
pub mod keys;
pub mod script;
pub mod settings;

use crate::platform;
pub use active::{Master, Masters};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// What a player may expect from a pack made for the other region's disc (one line; the pack preview uses it, too).
pub const OTHER_REGION_NOTE: &str = "Most world textures still match; menus, HUD and text may stay original.";

/// The kinds this launcher can enable.
pub const KNOWN_KINDS: &[&str] = &["textures", "script", "native"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModInfo {
    #[serde(default = "one")]
    pub format: u32,
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "rac1")]
    pub game: String,
    #[serde(default = "textures")]
    pub kind: String,
    #[serde(default)]
    pub serials: Vec<String>,
    #[serde(default)]
    pub source: String,
}

fn one() -> u32 {
    1
}
fn rac1() -> String {
    "rac1".into()
}
fn textures() -> String {
    "textures".into()
}

/// Folder-safe ids: `[a-z0-9][a-z0-9._-]{0,63}`.
pub fn valid_id(id: &str) -> bool {
    let b = id.as_bytes();
    !b.is_empty() && b.len() <= 64 && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-'))
}

pub fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    let s = s.trim_matches('-').chars().take(48).collect::<String>();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "texture-pack".to_string()
    } else {
        s
    }
}

#[derive(Clone, Debug)]
pub struct ModEntry {
    pub id: String,
    pub dir: PathBuf,
    pub info: Result<ModInfo, String>,
    /// A script or native mod: what its omni-mod.json says (`info` is made from it, kind "script" or "native").
    pub script: Option<script::ScriptMod>,
    /// The description file that was read: `mod.json` or `omni-mod.json`.
    pub manifest_file: &'static str,
}

impl ModEntry {
    pub fn name(&self) -> String {
        match &self.info {
            Ok(i) if !i.name.is_empty() => i.name.clone(),
            _ => self.id.clone(),
        }
    }

    /// A script or native mod (it has an omni-mod.json, readable or not).
    pub fn is_script(&self) -> bool {
        self.manifest_file == script::FILE
    }

    /// Why this mod cannot be switched on (None = it can).
    pub fn cannot_enable(&self, game_id: &str) -> Option<String> {
        match &self.info {
            Err(e) if self.manifest_file == script::FILE => Some(format!("{} has a problem: {e}", script::FILE)),
            Err(e) => Some(format!("mod.json could not be read ({e})")),
            Ok(i) if i.format != 1 => Some("needs a newer version of the launcher".to_string()),
            Ok(i) if !KNOWN_KINDS.contains(&i.kind.as_str()) => Some("needs a newer version".to_string()),
            Ok(i) if i.game != game_id && !self.script.as_ref().is_some_and(|m| m.games.contains_key(game_id)) => Some(format!("made for another game ({})", i.game)),
            Ok(_) => None,
        }
    }

    /// A pack for the other region than the installed disc: a warning, not a block.
    pub fn serial_warning(&self, disc_serial: Option<&str>) -> Option<String> {
        let (Ok(i), Some(d)) = (&self.info, disc_serial) else { return None };
        if i.serials.is_empty() || i.serials.iter().any(|s| s.eq_ignore_ascii_case(d)) {
            return None;
        }
        Some(format!("Made for {}; your disc is {}. {}", i.serials.join(", "), d, OTHER_REGION_NOTE))
    }
}

#[derive(Clone, Debug, Default)]
pub struct ModStore {
    pub dir: PathBuf,
    pub entries: Vec<ModEntry>,
    /// active.txt, highest priority first (may name mods that are not installed: kept so the order is not lost).
    pub active: Vec<String>,
    /// Where active.txt is read and written; empty = `<dir>/active.txt`. A profile points it at its own copy.
    pub active_file: PathBuf,
    /// The master ticks of the Mods page ("Load texture packs", "Load mods"); kept in active.txt (see `active`).
    pub masters: Masters,
    /// Lines of active.txt that the launcher did not write: kept and written back.
    kept: active::Kept,
}

/// The tick of mod `id` among `entries`.
fn master_in(entries: &[ModEntry], id: &str) -> Option<Master> {
    let info = entries.iter().find(|e| e.id == id)?.info.as_ref().ok()?;
    Some(Master::of_kind(&info.kind))
}

/// The entry of a folder with an omni-mod.json (None = a format 1 data pack, which is not a script mod and is left to mod.json).
/// The entry id is the manifest's `id` (what active.txt lists and what keys settings.json; the folder name may differ). A broken manifest,
/// or a manifest id another folder already has, shows the problem under the folder name and cannot be switched on.
fn script_entry(folder: &str, dir: &Path, file: &Path, seen: &[ModEntry]) -> Option<ModEntry> {
    let text = match std::fs::read(file) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => return Some(bad_script(folder, dir, e.to_string())),
    };
    match script::parse(&text) {
        script::Parsed::DataPack => None,
        script::Parsed::Bad(why) => Some(bad_script(folder, dir, why)),
        script::Parsed::Mod(m) => {
            if seen.iter().any(|e| e.id == m.id) {
                return Some(bad_script(folder, dir, format!("another mod folder already has the id \"{}\"", m.id)));
            }
            let game = if m.games.contains_key("rac1") { "rac1".to_string() } else { m.games.keys().next().cloned().unwrap_or_default() };
            let info = ModInfo {
                format: 1,
                id: m.id.clone(),
                name: m.name.clone(),
                author: m.author.clone(),
                version: m.version.clone(),
                description: m.description.clone(),
                game,
                kind: m.kind.clone(),
                serials: Vec::new(),
                source: String::new(),
            };
            Some(ModEntry { id: m.id.clone(), dir: dir.to_path_buf(), info: Ok(info), script: Some(*m), manifest_file: script::FILE })
        }
    }
}

fn bad_script(folder: &str, dir: &Path, why: String) -> ModEntry {
    ModEntry { id: folder.to_string(), dir: dir.to_path_buf(), info: Err(why), script: None, manifest_file: script::FILE }
}

pub fn parse_active(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in text.lines() {
        let l = l.trim_start_matches('\u{feff}').trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if !out.iter().any(|x| x == l) {
            out.push(l.to_string());
        }
    }
    out
}

/// A plain active.txt with these ids switched on (every master tick on).
pub fn active_text(active: &[String]) -> String {
    active::render(active, Masters::default(), &[], &|_| None)
}

impl ModStore {
    pub fn load(dir: &Path) -> ModStore {
        Self::load_with(dir, &dir.join("active.txt"))
    }

    /// The mods in `dir`, switched on/off and ordered by the `active_file` (a profile's list).
    pub fn load_with(dir: &Path, active_file: &Path) -> ModStore {
        let text = std::fs::read(active_file).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        let mut entries = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if !p.is_dir() {
                    continue;
                }
                let id = e.file_name().to_string_lossy().to_string();
                if id.starts_with('.') || id.ends_with(".tmp") {
                    continue;
                }
                let (mj, oj) = (p.join("mod.json"), script::manifest_in(&p));
                if let Some(oj) = oj {
                    if let Some(entry) = script_entry(&id, &p, &oj, &entries) {
                        entries.push(entry);
                        continue;
                    }
                }
                if !mj.is_file() {
                    continue;
                }
                let info = std::fs::read(&mj).map_err(|e| e.to_string()).and_then(|b| serde_json::from_slice::<ModInfo>(&b).map_err(|e| e.to_string()));
                entries.push(ModEntry { id, dir: p, info, script: None, manifest_file: "mod.json" });
            }
        }
        entries.sort_by(|a, b| a.name().to_lowercase().cmp(&b.name().to_lowercase()));
        let parsed = active::parse(&text, &|id| master_in(&entries, id));
        ModStore { dir: dir.to_path_buf(), entries, active: parsed.ids, active_file: active_file.to_path_buf(), masters: parsed.masters, kept: parsed.kept }
    }

    /// The master tick that governs mod `id` (None = not installed, or its mod.json cannot be read).
    pub fn master_of(&self, id: &str) -> Option<Master> {
        master_in(&self.entries, id)
    }

    pub fn master_on(&self, m: Master) -> bool {
        self.masters.is_on(m)
    }

    /// Switches a master tick. The per-mod switches are not touched: `save` writes the ids of that kind as `# <id>` lines (off) or as ids (on).
    pub fn set_master(&mut self, m: Master, on: bool) {
        self.masters.set(m, on);
    }

    /// (installed mods governed by tick `m`, how many of them are switched on per mod).
    pub fn master_counts(&self, m: Master) -> (usize, usize) {
        let of_kind = self.entries.iter().filter(|e| master_in(std::slice::from_ref(*e), &e.id) == Some(m));
        of_kind.fold((0, 0), |(n, on), e| (n + 1, on + self.is_enabled(&e.id) as usize))
    }

    /// True when the game will load this mod: switched on per mod, and its master tick is on (ids that are not installed count as loaded).
    pub fn is_loaded(&self, id: &str) -> bool {
        self.is_enabled(id) && self.master_of(id).map(|m| self.master_on(m)).unwrap_or(true)
    }

    pub fn save(&self) -> std::io::Result<()> {
        let p = if self.active_file.as_os_str().is_empty() { self.dir.join("active.txt") } else { self.active_file.clone() };
        std::fs::create_dir_all(p.parent().unwrap_or(&self.dir))?;
        let tmp = p.with_extension("txt.tmp");
        std::fs::write(&tmp, active::render(&self.active, self.masters, &self.kept, &|id| self.master_of(id)))?;
        std::fs::rename(&tmp, p)
    }

    pub fn is_enabled(&self, id: &str) -> bool {
        self.active.iter().any(|a| a == id)
    }

    pub fn get(&self, id: &str) -> Option<&ModEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// The list as shown: enabled mods in priority order, then the others by name.
    pub fn display_order(&self) -> Vec<&ModEntry> {
        let mut v: Vec<&ModEntry> = self.active.iter().filter_map(|a| self.get(a)).collect();
        v.extend(self.entries.iter().filter(|e| !self.is_enabled(&e.id)));
        v
    }

    /// Switch on (at the top = highest priority) or off. Err = the reason it cannot be enabled.
    pub fn set_enabled(&mut self, id: &str, on: bool, game_id: &str) -> Result<(), String> {
        if on {
            let e = self.get(id).ok_or_else(|| "mod not found".to_string())?;
            if let Some(why) = e.cannot_enable(game_id) {
                return Err(why);
            }
            if !self.is_enabled(id) {
                self.active.insert(0, id.to_string());
            }
        } else {
            self.active.retain(|a| a != id);
        }
        Ok(())
    }

    /// The switched-on mods that sit in the same list on the page as `id` (script and native mods, or texture packs), in priority order.
    pub fn group_of(&self, id: &str) -> Vec<&String> {
        let m = self.master_of(id);
        self.active.iter().filter(|a| self.master_of(a) == m).collect()
    }

    /// Moves an enabled mod one place up (towards higher priority) or down, past the next mod of its own list.
    pub fn move_by(&mut self, id: &str, delta: i32) {
        let group: Vec<String> = self.group_of(id).into_iter().cloned().collect();
        let Some(i) = group.iter().position(|a| a == id) else { return };
        let j = i as i32 + delta;
        if j < 0 || j as usize >= group.len() {
            return;
        }
        let other = group[j as usize].clone();
        let (a, b) = (self.active.iter().position(|x| x == id), self.active.iter().position(|x| *x == other));
        if let (Some(a), Some(b)) = (a, b) {
            self.active.swap(a, b);
        }
    }

    /// To the Recycle Bin, and out of active.txt.
    pub fn remove(&mut self, id: &str) -> Result<(), String> {
        let dir = self.get(id).map(|e| e.dir.clone()).ok_or_else(|| "mod not found".to_string())?;
        platform::recycle(&dir)?;
        self.active.retain(|a| a != id);
        self.entries.retain(|e| e.id != id);
        Ok(())
    }

    /// A folder name for a new mod that does not exist yet.
    pub fn unique_id(&self, base: &str) -> String {
        let base = if valid_id(base) { base.to_string() } else { slug(base) };
        if self.get(&base).is_none() && !self.dir.join(&base).exists() {
            return base;
        }
        (2..).map(|n| format!("{base}-{n}")).find(|c| self.get(c).is_none() && !self.dir.join(c).exists()).unwrap_or(base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("racl_mods_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_mod(dir: &Path, id: &str, kind: &str, serials: &str, game: &str) {
        let d = dir.join(id);
        std::fs::create_dir_all(d.join("textures")).unwrap();
        std::fs::write(d.join("mod.json"), format!(r#"{{"format":1,"id":"{id}","name":"Mod {id}","author":"A","version":"1.2","description":"d","game":"{game}","kind":"{kind}","serials":[{serials}],"source":"pcsx2"}}"#)).unwrap();
    }

    #[test]
    fn active_txt_parse_and_write() {
        assert_eq!(parse_active("# c\n\nb\n  a  \nb\n\u{feff}\r\n"), vec!["b", "a"]);
        let t = active_text(&["x".into(), "y".into()]);
        assert!(t.starts_with('#') && t.ends_with("x\ny\n"));
        assert_eq!(parse_active(&t), vec!["x", "y"]);
    }

    #[test]
    fn store_loads_toggles_orders_and_saves() {
        let d = scratch("store");
        write_mod(&d, "alpha", "textures", r#""SCES-50916""#, "rac1");
        write_mod(&d, "beta", "textures", "", "rac1");
        write_mod(&d, "gamma", "meshes", "", "rac1");
        write_mod(&d, "delta", "textures", "", "rac2");
        std::fs::create_dir_all(d.join("nomod")).unwrap();
        std::fs::write(d.join("active.txt"), "# x\nbeta\nghost\n").unwrap();
        let mut s = ModStore::load(&d);
        assert_eq!(s.entries.len(), 4, "folders without mod.json are not mods");
        assert_eq!(s.active, vec!["beta", "ghost"]);
        assert!(s.set_enabled("alpha", true, "rac1").is_ok());
        assert_eq!(s.active, vec!["alpha", "beta", "ghost"], "new ones go on top");
        assert!(s.set_enabled("gamma", true, "rac1").unwrap_err().contains("needs a newer version"));
        assert!(s.set_enabled("delta", true, "rac1").unwrap_err().contains("another game"));
        s.move_by("alpha", 1);
        assert_eq!(s.active, vec!["beta", "alpha", "ghost"]);
        s.move_by("beta", -1);
        assert_eq!(s.active, vec!["beta", "alpha", "ghost"], "already first");
        s.save().unwrap();
        let s2 = ModStore::load(&d);
        assert_eq!(s2.active, s.active, "dangling ids are kept");
        let order: Vec<&str> = s2.display_order().iter().map(|e| e.id.as_str()).collect();
        assert_eq!(order, vec!["beta", "alpha", "delta", "gamma"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    fn read(p: &Path) -> String {
        std::fs::read_to_string(p).unwrap()
    }

    #[test]
    fn master_ticks_round_trip_and_the_per_mod_switches_survive() {
        let d = scratch("master");
        write_mod(&d, "alpha", "textures", "", "rac1");
        write_mod(&d, "beta", "textures", "", "rac1");
        write_mod(&d, "gamma", "scripts", "", "rac1");
        let on = format!("{}\n# my note\ngamma\nbeta\nalpha\n# last words\n", active::HEADER);
        std::fs::write(d.join("active.txt"), &on).unwrap();
        let mut s = ModStore::load(&d);
        assert_eq!(s.active, vec!["gamma", "beta", "alpha"]);
        assert!(s.master_on(Master::Textures) && s.master_on(Master::Mods));
        assert_eq!(s.master_counts(Master::Textures), (2, 2));
        assert_eq!(s.master_counts(Master::Mods), (1, 1));
        s.save().unwrap();
        assert_eq!(read(&d.join("active.txt")), on, "nothing touched: the file is the same");

        // Load texture packs OFF: the packs become `# id` lines, the script mod stays, the game sees only gamma
        s.set_master(Master::Textures, false);
        s.save().unwrap();
        let off = read(&d.join("active.txt"));
        assert_eq!(parse_active(&off), vec!["gamma"], "{off}");
        assert!(off.contains("# Load texture packs: off\n") && off.contains("# beta\n# alpha\n"), "{off}");
        let mut s2 = ModStore::load(&d);
        assert!(!s2.master_on(Master::Textures) && s2.master_on(Master::Mods));
        assert_eq!(s2.active, vec!["gamma", "beta", "alpha"], "per-mod switches and priority are remembered");
        assert!(s2.is_enabled("alpha") && !s2.is_loaded("alpha") && s2.is_loaded("gamma"));
        assert_eq!(s2.master_counts(Master::Textures), (2, 2), "still two packs switched on per pack");

        // a per-mod switch changed while the tick is off is remembered, too
        assert!(s2.set_enabled("beta", false, "rac1").is_ok());
        s2.active.swap(0, 1); // the order changed while the tick is off, too
        s2.save().unwrap();
        let s3 = ModStore::load(&d);
        assert_eq!(s3.active, vec!["alpha", "gamma"]);
        assert_eq!(parse_active(&read(&d.join("active.txt"))), vec!["gamma"]);

        // ON: the packs come back as plain ids, in place, and the lines nobody wrote are still there
        let mut s4 = s3;
        s4.set_master(Master::Textures, true);
        s4.save().unwrap();
        let back = read(&d.join("active.txt"));
        assert_eq!(back, format!("{}\nalpha\n# my note\ngamma\n# last words\n", active::HEADER), "a note stays above the mod it sat above: {back}");
        assert_eq!(parse_active(&back), vec!["alpha", "gamma"]);

        // both ticks off, then both on
        let mut s5 = ModStore::load(&d);
        s5.set_master(Master::Textures, false);
        s5.set_master(Master::Mods, false);
        s5.save().unwrap();
        let both = read(&d.join("active.txt"));
        assert!(parse_active(&both).is_empty(), "{both}");
        let mut s6 = ModStore::load(&d);
        assert_eq!(s6.active, vec!["alpha", "gamma"]);
        s6.set_master(Master::Textures, true);
        s6.set_master(Master::Mods, true);
        s6.save().unwrap();
        assert_eq!(read(&d.join("active.txt")), back);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn unknown_lines_and_unknown_ids_in_active_txt_are_kept_by_every_save() {
        let d = scratch("masterkeep");
        write_mod(&d, "alpha", "textures", "", "rac1");
        std::fs::write(d.join("active.txt"), "# something a newer launcher wrote\nghost\n# about alpha\nalpha\n# future: 1\n").unwrap();
        let mut s = ModStore::load(&d);
        s.set_master(Master::Textures, false);
        s.save().unwrap();
        let t = read(&d.join("active.txt"));
        for line in ["# something a newer launcher wrote", "ghost", "# about alpha", "# alpha", "# future: 1"] {
            assert!(t.lines().any(|l| l == line), "{line:?} missing from\n{t}");
        }
        let mut s2 = ModStore::load(&d);
        s2.set_master(Master::Textures, true);
        s2.save().unwrap();
        let t2 = read(&d.join("active.txt"));
        assert_eq!(t2, format!("{}\n# something a newer launcher wrote\nghost\n# about alpha\nalpha\n# future: 1\n", active::HEADER));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn serial_warning_for_the_other_region() {
        let d = scratch("serial");
        write_mod(&d, "alpha", "textures", r#""SCES-50916""#, "rac1");
        let s = ModStore::load(&d);
        let e = s.get("alpha").unwrap();
        let w = e.serial_warning(Some("SCUS-97199")).unwrap();
        assert_eq!(w, "Made for SCES-50916; your disc is SCUS-97199. Most world textures still match; menus, HUD and text may stay original.");
        assert_eq!(w.lines().count(), 1);
        assert!(e.serial_warning(Some("SCES-50916")).is_none());
        assert!(e.serial_warning(None).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn broken_mod_json_is_listed_but_cannot_be_enabled() {
        let d = scratch("broken");
        std::fs::create_dir_all(d.join("bad")).unwrap();
        std::fs::write(d.join("bad/mod.json"), "{ nope").unwrap();
        let mut s = ModStore::load(&d);
        assert_eq!(s.entries.len(), 1);
        assert!(s.set_enabled("bad", true, "rac1").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    fn write_script(dir: &Path, folder: &str, text: &str) {
        let d = dir.join(folder);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(script::FILE), text).unwrap();
    }

    const ARSENAL: &str = include_str!("../../tests/data/arsenal-mastery.omni-mod.json");

    #[test]
    fn a_mod_made_for_openrac_is_listed_and_omni_mod_json_wins() {
        let d = scratch("compat");
        // compat: OpenRAC's manifest file name, alone and next to an omni-mod.json
        for (folder, file, id) in [("old", script::FILE_COMPAT, "old-id"), ("both", script::FILE_COMPAT, "compat-id"), ("both", script::FILE, "new-id")] {
            std::fs::create_dir_all(d.join(folder)).unwrap();
            std::fs::write(d.join(folder).join(file), ARSENAL.replace("arsenal-mastery", id)).unwrap();
        }
        let s = ModStore::load(&d);
        let mut ids: Vec<&str> = s.entries.iter().map(|e| e.id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, ["new-id", "old-id"]);
        assert!(s.get("old-id").unwrap().is_script());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn script_mods_are_listed_by_their_manifest_id_next_to_texture_packs() {
        let d = scratch("script");
        write_mod(&d, "pack", "textures", "", "rac1");
        write_script(&d, "arsenal-mastery", ARSENAL);
        write_script(&d, "renamed-folder", &ARSENAL.replace("arsenal-mastery", "other-id").replace("Arsenal Mastery", "Other"));
        write_script(&d, "broken", "{ nope");
        write_script(&d, "datapack", "{\"format\":1,\"id\":\"dp\",\"name\":\"DP\"}");
        write_script(&d, "twin", ARSENAL); // same id as arsenal-mastery
        let mut s = ModStore::load(&d);
        let ids: Vec<&str> = s.entries.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.contains(&"arsenal-mastery") && ids.contains(&"other-id") && !ids.contains(&"renamed-folder") && ids.contains(&"pack"), "{ids:?}");
        assert!(ids.contains(&"broken") && !ids.contains(&"datapack"), "a format 1 data pack with no mod.json is not a mod here: {ids:?}");
        let a = s.get("arsenal-mastery").unwrap();
        assert_eq!(a.script.as_ref().unwrap().settings.len(), 13);
        assert_eq!(s.get("other-id").unwrap().dir.file_name().unwrap(), "renamed-folder", "active.txt and settings.json use the manifest id, the folder is only where it lives");
        assert_eq!(a.info.as_ref().unwrap().kind, "script");
        assert_eq!(a.cannot_enable("rac1"), None);
        assert!(a.cannot_enable("rac2").unwrap().contains("another game"));
        let b = s.get("broken").unwrap();
        assert!(b.cannot_enable("rac1").unwrap().starts_with("omni-mod.json has a problem"), "{:?}", b.cannot_enable("rac1"));
        assert_eq!(b.name(), "broken", "a broken manifest shows under its folder name");
        let twin = s.entries.iter().find(|e| e.id == "twin").unwrap();
        assert!(twin.info.as_ref().unwrap_err().contains("already has the id"));
        // on/off through active.txt with the manifest id; the Load mods tick governs it
        assert!(s.set_enabled("arsenal-mastery", true, "rac1").is_ok());
        assert!(s.set_enabled("broken", true, "rac1").is_err());
        assert_eq!(s.master_of("arsenal-mastery"), Some(Master::Mods));
        s.save().unwrap();
        assert!(read(&d.join("active.txt")).lines().any(|l| l == "arsenal-mastery"));
        s.set_master(Master::Mods, false);
        s.save().unwrap();
        assert!(parse_active(&read(&d.join("active.txt"))).is_empty());
        assert!(ModStore::load(&d).is_enabled("arsenal-mastery") && !ModStore::load(&d).is_loaded("arsenal-mastery"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn priority_moves_stay_inside_the_mods_own_list() {
        let d = scratch("groups");
        write_mod(&d, "t1", "textures", "", "rac1");
        write_mod(&d, "t2", "textures", "", "rac1");
        write_script(&d, "arsenal-mastery", ARSENAL);
        write_script(&d, "other-id", &ARSENAL.replace("arsenal-mastery", "other-id"));
        std::fs::write(d.join("active.txt"), "t1
arsenal-mastery
t2
other-id
").unwrap();
        let mut s = ModStore::load(&d);
        s.move_by("t1", 1);
        assert_eq!(s.active, vec!["t2", "arsenal-mastery", "t1", "other-id"], "t1 swaps with t2, not with the script mod between them");
        s.move_by("other-id", -1);
        assert_eq!(s.active, vec!["t2", "other-id", "t1", "arsenal-mastery"]);
        s.move_by("other-id", -1);
        assert_eq!(s.active, vec!["t2", "other-id", "t1", "arsenal-mastery"], "already first in its list");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn ids() {
        assert!(valid_id("my-pack_1.0") && !valid_id("My Pack") && !valid_id("") && !valid_id("-x") && !valid_id("../x"));
        assert_eq!(slug("HD Textures (v2)!"), "hd-textures-v2");
        assert_eq!(slug("???"), "texture-pack");
        let d = scratch("uid");
        std::fs::create_dir_all(d.join("pack")).unwrap();
        let s = ModStore { dir: d.clone(), ..Default::default() };
        assert_eq!(s.unique_id("pack"), "pack-2");
        assert_eq!(s.unique_id("New Pack"), "new-pack");
        let _ = std::fs::remove_dir_all(&d);
    }
}
