//! Script and native mods (omni mods): a folder in the mods dir with an `omni-mod.json` (format 2, `"type": "script"` or native).
//! The launcher reads what it shows and edits: name, version, author, description, which games, and the `settings` and `presets`.
//! It checks those by the loader's rules (a mod the game would refuse shows an error line and cannot be switched on); the parts only the
//! game needs (scripts, hooks, plugin files, requires, conflicts) are the game's to judge, so a mod is never blocked here for them.
use super::settings::{parse_presets, parse_settings, Preset, SettingDef};
use serde_json::Value as Json;
use std::collections::BTreeMap;

pub const FILE: &str = "omni-mod.json";
/// compat: the same manifest under OpenRAC's file name, read when a folder has no `omni-mod.json` (as the game's loader does).
pub const FILE_COMPAT: &str = "openrac-mod.json";

/// The manifest of a mod folder: `omni-mod.json`, else (compat) `openrac-mod.json`.
pub fn manifest_in(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    [FILE, FILE_COMPAT].into_iter().map(|f| dir.join(f)).find(|p| p.is_file())
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScriptMod {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    /// `"script"` or `"native"`.
    pub kind: String,
    /// game id -> disc versions, e.g. `rac1` -> `pal`, `ntsc`.
    pub games: BTreeMap<String, Vec<String>>,
    pub settings: Vec<SettingDef>,
    pub presets: Vec<Preset>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Parsed {
    Mod(Box<ScriptMod>),
    /// `"format": 1` is a data pack, not a script mod: not listed here.
    DataPack,
    /// Why the manifest cannot be used (one line; "and N more" when there are several).
    Bad(String),
}

/// `my-mod-2`: lower case letters and digits joined by single hyphens, at most 64 characters (the loader's rule).
pub fn is_mod_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.split('-').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
}

fn one_line(mut errors: Vec<String>) -> String {
    let first = errors.remove(0);
    match errors.len() {
        0 => first,
        1 => format!("{first} (and 1 more problem)"),
        n => format!("{first} (and {n} more problems)"),
    }
}

/// Parses the text of an omni-mod.json. Never panics; a broken file is `Bad`.
pub fn parse(text: &str) -> Parsed {
    let json: Json = match serde_json::from_str(text.trim_start_matches('\u{feff}')) {
        Ok(j) => j,
        Err(e) => return Parsed::Bad(format!("it is not valid JSON ({e})")),
    };
    let Some(obj) = json.as_object() else { return Parsed::Bad("it is not a JSON object".to_string()) };
    match obj.get("format").and_then(Json::as_u64) {
        Some(2) => {}
        Some(1) => return Parsed::DataPack,
        Some(n) if n > 2 => return Parsed::Bad("needs a newer version of the launcher".to_string()),
        _ => return Parsed::Bad("format must be 2".to_string()),
    }
    let mut errors = Vec::new();
    let text_of = |k: &str, errors: &mut Vec<String>| -> String {
        match obj.get(k) {
            None => String::new(),
            Some(Json::String(s)) => s.clone(),
            Some(_) => {
                errors.push(format!("{k}: not text"));
                String::new()
            }
        }
    };
    let id = obj.get("id").and_then(Json::as_str).filter(|s| is_mod_id(s));
    if id.is_none() {
        errors.push("id: lower case letters and digits joined by single hyphens, at most 64 characters".to_string());
    }
    let name = obj.get("name").and_then(Json::as_str).filter(|s| !s.trim().is_empty());
    if name.is_none() {
        errors.push("name: required".to_string());
    }
    let (version, author, description) = (text_of("version", &mut errors), text_of("author", &mut errors), text_of("description", &mut errors));
    let kind = match obj.get("type").and_then(Json::as_str) {
        None if !obj.contains_key("type") => "native",
        Some("native") => "native",
        Some("script") => "script",
        _ => {
            errors.push("type: native (the default) or script".to_string());
            "native"
        }
    };
    let games = games_of(obj.get("games"), &mut errors);
    let settings = parse_settings(obj.get("settings"), &mut errors);
    let presets = parse_presets(obj.get("presets"), &settings, &mut errors);
    if !errors.is_empty() {
        return Parsed::Bad(one_line(errors));
    }
    Parsed::Mod(Box::new(ScriptMod {
        id: id.unwrap_or_default().to_string(),
        name: name.unwrap_or_default().to_string(),
        version,
        author,
        description,
        kind: kind.to_string(),
        games,
        settings,
        presets,
    }))
}

fn games_of(j: Option<&Json>, errors: &mut Vec<String>) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    let Some(o) = j.and_then(Json::as_object).filter(|o| !o.is_empty()) else {
        errors.push("games: an object with at least one game".to_string());
        return out;
    };
    for (g, v) in o {
        match v.as_array().filter(|a| !a.is_empty() && a.iter().all(Json::is_string)) {
            Some(a) => {
                out.insert(g.clone(), a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect());
            }
            None => errors.push(format!("games {g}: a list of at least one disc version (pal, ntsc)")),
        }
    }
    out
}

impl ScriptMod {
    pub fn has_settings(&self) -> bool {
        !self.settings.is_empty()
    }

    /// "Script mod" / "Native mod".
    pub fn kind_label(&self) -> &'static str {
        if self.kind == "script" {
            "Script mod"
        } else {
            "Native mod"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARSENAL: &str = include_str!("../../tests/data/arsenal-mastery.omni-mod.json");

    #[test]
    fn the_real_arsenal_mastery_manifest_reads() {
        let Parsed::Mod(m) = parse(ARSENAL) else { panic!("{:?}", parse(ARSENAL)) };
        assert_eq!((m.id.as_str(), m.name.as_str(), m.version.as_str(), m.author.as_str()), ("arsenal-mastery", "Arsenal Mastery", "0.3.0", "rac-rs"));
        assert_eq!(m.kind, "script");
        assert!(m.description.starts_with("Ratchet & Clank 3 style weapon levels"));
        assert_eq!(m.games["rac1"], vec!["pal", "ntsc"]);
        assert_eq!(m.settings.len(), 13);
        assert_eq!(m.presets.len(), 4);
        assert!(m.settings.iter().find(|s| s.key == "start_level").unwrap().advanced);
        for k in ["nanotech_xp_rate", "weapon_damage", "enemy_damage"] {
            let s = m.settings.iter().find(|s| s.key == k).unwrap();
            assert!(s.advanced, "{k} is in the Advanced group");
            assert_eq!(s.kind, crate::mods::settings::Kind::Float { min: Some(0.25), max: Some(4.0), step: Some(0.05) }, "{k}");
            assert_eq!(s.default, Json::from(1.0), "{k}");
        }
        assert_eq!(m.kind_label(), "Script mod");
    }

    #[test]
    fn broken_manifests_are_a_line_not_a_crash() {
        assert!(matches!(parse("{ nope"), Parsed::Bad(m) if m.contains("not valid JSON")));
        assert!(matches!(parse("[]"), Parsed::Bad(_)));
        assert!(matches!(parse(""), Parsed::Bad(_)));
        assert!(matches!(parse("{\"format\":2}"), Parsed::Bad(m) if m.contains("more problem")));
        assert!(matches!(parse("{\"format\":9}"), Parsed::Bad(m) if m.contains("newer")));
        assert!(matches!(parse("{\"id\":\"x\"}"), Parsed::Bad(m) if m.contains("format")));
        assert_eq!(parse("{\"format\":1,\"id\":\"x\"}"), Parsed::DataPack);
        let bad_setting = ARSENAL.replace("\"default\": 1.0, \"min\": 0.5", "\"default\": 9.0, \"min\": 0.5");
        assert!(matches!(parse(&bad_setting), Parsed::Bad(m) if m.contains("out of range")), "{:?}", parse(&bad_setting));
        let bad_preset = ARSENAL.replace("\"rebalance\": \"off\"", "\"rebalance\": \"nope\"");
        assert!(matches!(parse(&bad_preset), Parsed::Bad(m) if m.contains("one of the choices")));
        let bad_id = ARSENAL.replace("\"id\": \"arsenal-mastery\"", "\"id\": \"Arsenal Mastery\"");
        assert!(matches!(parse(&bad_id), Parsed::Bad(m) if m.starts_with("id:")));
    }

    #[test]
    fn a_bom_and_a_missing_type_are_fine() {
        let t = "\u{feff}{\"format\":2,\"id\":\"n\",\"name\":\"N\",\"version\":\"1.0.0\",\"games\":{\"rac1\":[\"pal\"]},\"plugin\":{\"windows-x64\":\"n.dll\"}}";
        let Parsed::Mod(m) = parse(t) else { panic!() };
        assert_eq!((m.kind.as_str(), m.kind_label()), ("native", "Native mod"));
        assert!(!m.has_settings() && m.author.is_empty());
    }

    #[test]
    fn mod_ids() {
        assert!(is_mod_id("arsenal-mastery") && is_mod_id("a1") && !is_mod_id("A") && !is_mod_id("a--b") && !is_mod_id("-a") && !is_mod_id("a_b") && !is_mod_id(""));
    }
}
