//! Settings of script mods, as the format declares them (`"settings"` and `"presets"` of omni-mod.json), and the file the
//! values live in: `<mods dir>\settings.json` = `{"format":1,"mods":{"<mod id>":{"<key>": value}}}`, written by this launcher and by the
//! game's own Mods page, read by the game at start.
//!
//! The rules are the OmniLoader's (`omniloader/src/settings.rs`): six types, a value that is missing, of the wrong type or out of range is
//! the default, a preset sets several at once. Values are kept as JSON values, typed by the declaration.
use super::keys::{bind_names, bind_problem};
use serde_json::{Map, Value as Json};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub value: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Bool,
    Int { min: Option<i64>, max: Option<i64> },
    Float { min: Option<f64>, max: Option<f64>, step: Option<f64> },
    Choice { choices: Vec<Choice> },
    Str { max_length: Option<usize> },
    /// `group`: keybinds of one group may not share an input (the page warns).
    Keybind { binds: usize, group: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SettingDef {
    pub key: String,
    pub name: String,
    pub description: String,
    pub kind: Kind,
    pub default: Json,
    /// `"group": "advanced"` on a setting that is not a keybind: shown folded away.
    pub advanced: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub values: Map<String, Json>,
}

fn is_int(v: &Json) -> bool {
    v.is_i64() || v.is_u64()
}

fn is_key(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn opt_num(s: &Map<String, Json>, field: &str, int: bool, w: &str, errors: &mut Vec<String>) -> Option<Option<f64>> {
    match s.get(field) {
        None | Some(Json::Null) => Some(None),
        Some(v) if v.is_number() && (!int || is_int(v)) => Some(v.as_f64()),
        Some(_) => {
            errors.push(format!("{w}: {field} is not {}", if int { "an integer" } else { "a number" }));
            None
        }
    }
}

impl SettingDef {
    /// Checks one `settings[i]` entry by the manifest rules; every problem goes to `errors`.
    pub fn parse(i: usize, j: &Json, errors: &mut Vec<String>) -> Option<SettingDef> {
        let mut w = format!("settings[{i}]");
        let Some(s) = j.as_object() else {
            errors.push(format!("{w}: not an object"));
            return None;
        };
        let key = s.get("key").and_then(Json::as_str).filter(|k| is_key(k));
        match key {
            Some(k) => w = format!("setting {k}"),
            None => errors.push(format!("{w}: key must be lower case letters, digits and _")),
        }
        let name = s.get("name").and_then(Json::as_str).filter(|n| !n.is_empty());
        if name.is_none() {
            errors.push(format!("{w}: no name"));
        }
        let description = match s.get("description") {
            None => String::new(),
            Some(Json::String(d)) => d.clone(),
            Some(_) => {
                errors.push(format!("{w}: description is not text"));
                String::new()
            }
        };
        let kind_name = s.get("type").and_then(Json::as_str).unwrap_or("");
        if !["bool", "int", "float", "choice", "string", "keybind"].contains(&kind_name) {
            errors.push(format!("{w}: type must be one of bool, choice, float, int, keybind, string"));
            return None;
        }
        let Some(default) = s.get("default") else {
            errors.push(format!("{w}: no default"));
            return None;
        };
        let before = errors.len();
        let kind = parse_kind(kind_name, s, default, &w, errors)?;
        if errors.len() != before {
            return None;
        }
        let default = match &kind {
            Kind::Float { .. } => Json::from(default.as_f64()?),
            _ => default.clone(),
        };
        let advanced = !matches!(kind, Kind::Keybind { .. }) && s.get("group").and_then(Json::as_str) == Some("advanced");
        Some(SettingDef { key: key?.to_string(), name: name?.to_string(), description, kind, default, advanced })
    }

    /// Why `value` is not one this setting takes, or `None`.
    pub fn value_problem(&self, value: &Json) -> Option<String> {
        let range = |lo: Option<f64>, hi: Option<f64>| -> Option<String> {
            let v = value.as_f64()?;
            (lo.is_some_and(|l| v < l) || hi.is_some_and(|h| v > h)).then(|| "is out of range".to_string())
        };
        match &self.kind {
            Kind::Bool => (!value.is_boolean()).then(|| "is not true or false".to_string()),
            Kind::Int { min, max } => {
                if !is_int(value) {
                    return Some("is not an integer".to_string());
                }
                range(min.map(|v| v as f64), max.map(|v| v as f64))
            }
            Kind::Float { min, max, .. } => {
                if !value.is_number() {
                    return Some("is not a number".to_string());
                }
                range(*min, *max)
            }
            Kind::Choice { choices } => {
                let ok = value.as_str().is_some_and(|v| choices.iter().any(|c| c.value == v));
                (!ok).then(|| "is not one of the choices".to_string())
            }
            Kind::Str { max_length } => match value.as_str() {
                None => Some("is not text".to_string()),
                Some(t) => max_length.filter(|m| t.chars().count() > *m).map(|_| "is longer than max_length".to_string()),
            },
            Kind::Keybind { binds, .. } => match value.as_str() {
                None => Some("is not text".to_string()),
                Some(t) => bind_problem(t, *binds),
            },
        }
    }

    /// The value in the shape this setting stores (a float is always a float).
    pub fn typed(&self, value: &Json) -> Json {
        match &self.kind {
            Kind::Float { .. } => Json::from(value.as_f64().unwrap_or(0.0)),
            _ => value.clone(),
        }
    }

    pub fn is_keybind(&self) -> bool {
        matches!(self.kind, Kind::Keybind { .. })
    }
}

fn parse_kind(kind: &str, s: &Map<String, Json>, d: &Json, w: &str, errors: &mut Vec<String>) -> Option<Kind> {
    match kind {
        "bool" => {
            if d.is_boolean() {
                Some(Kind::Bool)
            } else {
                errors.push(format!("{w}: the default is not true or false"));
                None
            }
        }
        "int" | "float" => parse_number(kind == "int", s, d, w, errors),
        "choice" => parse_choice(s, d, w, errors),
        "keybind" => {
            let binds = match s.get("binds") {
                None => 2,
                Some(v) => match v.as_u64() {
                    Some(n @ (1 | 2)) => n as usize,
                    _ => {
                        errors.push(format!("{w}: binds must be 1 or 2"));
                        return None;
                    }
                },
            };
            let why = match d.as_str() {
                Some(t) => bind_problem(t, binds),
                None => Some("is not text".to_string()),
            };
            if let Some(why) = why {
                errors.push(format!("{w}: the default {why}"));
                return None;
            }
            let group = s.get("group").and_then(Json::as_str).unwrap_or("").to_string();
            Some(Kind::Keybind { binds, group })
        }
        _ => {
            let Some(text) = d.as_str() else {
                errors.push(format!("{w}: the default is not text"));
                return None;
            };
            let max_length = match s.get("max_length") {
                None => None,
                Some(v) => match v.as_u64().filter(|m| *m > 0) {
                    Some(m) => Some(m as usize),
                    None => {
                        errors.push(format!("{w}: max_length must be a positive integer"));
                        return None;
                    }
                },
            };
            if max_length.is_some_and(|m| text.chars().count() > m) {
                errors.push(format!("{w}: the default is longer than max_length"));
                return None;
            }
            Some(Kind::Str { max_length })
        }
    }
}

fn parse_number(int: bool, s: &Map<String, Json>, d: &Json, w: &str, errors: &mut Vec<String>) -> Option<Kind> {
    if !(d.is_number() && (!int || is_int(d))) {
        errors.push(format!("{w}: the default is not {}", if int { "an integer" } else { "a number" }));
        return None;
    }
    let dv = d.as_f64()?;
    let lo = opt_num(s, "min", int, w, errors)?;
    let hi = opt_num(s, "max", int, w, errors)?;
    if lo.is_some() && hi.is_some() && lo > hi {
        errors.push(format!("{w}: min is above max"));
    }
    if lo.is_some_and(|l| dv < l) || hi.is_some_and(|h| dv > h) {
        errors.push(format!("{w}: the default is out of range"));
    }
    if int {
        return Some(Kind::Int { min: lo.map(|v| v as i64), max: hi.map(|v| v as i64) });
    }
    let step = match s.get("step") {
        None => None,
        Some(v) => match v.as_f64().filter(|x| *x > 0.0) {
            Some(x) => Some(x),
            None => {
                errors.push(format!("{w}: step must be a positive number"));
                None
            }
        },
    };
    Some(Kind::Float { min: lo, max: hi, step })
}

fn parse_choice(s: &Map<String, Json>, d: &Json, w: &str, errors: &mut Vec<String>) -> Option<Kind> {
    let Some(list) = s.get("choices").and_then(Json::as_array).filter(|c| c.len() >= 2) else {
        errors.push(format!("{w}: a choice needs two choices or more"));
        return None;
    };
    let mut choices = Vec::new();
    for c in list {
        match (c.get("value").and_then(Json::as_str), c.get("name").and_then(Json::as_str)) {
            (Some(v), Some(n)) => choices.push(Choice { value: v.to_string(), name: n.to_string() }),
            _ => {
                errors.push(format!("{w}: each choice is {{value, name}}, both text"));
                return None;
            }
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    if !choices.iter().all(|c| seen.insert(c.value.clone())) {
        errors.push(format!("{w}: a value twice among the choices"));
    }
    if !d.as_str().is_some_and(|x| choices.iter().any(|c| c.value == x)) {
        errors.push(format!("{w}: the default is not one of the choices"));
        return None;
    }
    Some(Kind::Choice { choices })
}

/// Checks `settings` (a list; a key twice is a problem).
pub fn parse_settings(j: Option<&Json>, errors: &mut Vec<String>) -> Vec<SettingDef> {
    let Some(j) = j else { return Vec::new() };
    let Some(list) = j.as_array() else {
        errors.push("settings: not a list".to_string());
        return Vec::new();
    };
    let defs: Vec<SettingDef> = list.iter().enumerate().filter_map(|(i, s)| SettingDef::parse(i, s, errors)).collect();
    let mut seen = std::collections::BTreeSet::new();
    let dup: Vec<&str> = defs.iter().filter(|d| !seen.insert(d.key.as_str())).map(|d| d.key.as_str()).collect();
    if !dup.is_empty() {
        errors.push(format!("settings: a key twice ({})", dup.join(", ")));
    }
    defs
}

/// Checks `presets` against the settings.
pub fn parse_presets(j: Option<&Json>, defs: &[SettingDef], errors: &mut Vec<String>) -> Vec<Preset> {
    let Some(j) = j else { return Vec::new() };
    let Some(list) = j.as_array() else {
        errors.push("presets: not a list".to_string());
        return Vec::new();
    };
    let mut out: Vec<Preset> = Vec::new();
    for (i, p) in list.iter().enumerate() {
        let id = p.get("id").and_then(Json::as_str).filter(|s| is_key(s));
        let name = p.get("name").and_then(Json::as_str).filter(|s| !s.is_empty());
        let values = p.get("values").and_then(Json::as_object);
        let (Some(id), Some(name), Some(values)) = (id, name, values) else {
            errors.push(format!("presets[{i}]: {{id (a-z, 0-9, _), name, values}}"));
            continue;
        };
        for (key, value) in values {
            match defs.iter().find(|d| d.key == *key) {
                None => errors.push(format!("preset {id}: {key} is not a setting of the mod")),
                Some(d) => {
                    if let Some(why) = d.value_problem(value) {
                        errors.push(format!("preset {id}: {key} {why}"));
                    }
                }
            }
        }
        out.push(Preset { id: id.to_string(), name: name.to_string(), values: values.clone() });
    }
    let mut seen = std::collections::BTreeSet::new();
    if !out.iter().all(|p| seen.insert(p.id.clone())) {
        errors.push("presets: an id twice".to_string());
    }
    out
}

/// The values the mod sees: for each declared setting (same order), `given` when valid, else the default.
pub fn resolve(defs: &[SettingDef], given: &Map<String, Json>) -> Vec<Json> {
    defs.iter()
        .map(|d| match given.get(&d.key) {
            Some(j) if d.value_problem(j).is_none() => d.typed(j),
            _ => d.default.clone(),
        })
        .collect()
}

/// Two values of one setting are the same (floats within a hair).
pub fn same(a: &Json, b: &Json) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) if a.is_number() && b.is_number() => (x - y).abs() < 1e-9,
        _ => a == b,
    }
}

/// Rounds a slider value to the setting's step (and away the binary noise: 0.30000000000000004 -> 0.3).
pub fn snap(d: &SettingDef, v: f64) -> f64 {
    match &d.kind {
        Kind::Float { min, max, step } => {
            let mut v = v;
            if let Some(s) = step {
                let base = min.unwrap_or(0.0);
                v = base + ((v - base) / s).round() * s;
            }
            v = (v * 1e6).round() / 1e6;
            v.clamp(min.unwrap_or(f64::MIN), max.unwrap_or(f64::MAX))
        }
        _ => v,
    }
}

/// Which settings a preset button touches: every keybind, and every setting some preset names.
fn covered(defs: &[SettingDef], presets: &[Preset]) -> Vec<bool> {
    defs.iter().map(|d| d.is_keybind() || presets.iter().any(|p| p.values.contains_key(&d.key))).collect()
}

/// What choosing preset `p` writes: (key, value) for each covered setting - the preset's own value, else the default.
pub fn preset_effect(defs: &[SettingDef], presets: &[Preset], p: &Preset) -> Vec<(String, Json)> {
    let cov = covered(defs, presets);
    defs.iter()
        .zip(cov)
        .filter(|(_, c)| *c)
        .map(|(d, _)| (d.key.clone(), p.values.get(&d.key).map(|v| d.typed(v)).unwrap_or_else(|| d.default.clone())))
        .collect()
}

/// What "Default" writes: every setting back to its declared default.
pub fn default_effect(defs: &[SettingDef]) -> Vec<(String, Json)> {
    defs.iter().map(|d| (d.key.clone(), d.default.clone())).collect()
}

/// The button that is lit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lit {
    Preset(usize),
    Default,
}

/// The first preset whose values the settings have (a named preset wins over the plain Default when both fit); else Default when
/// every setting is at its default; else None ("custom").
pub fn lit(defs: &[SettingDef], presets: &[Preset], current: &[Json]) -> Option<Lit> {
    let fits = |effect: Vec<(String, Json)>| effect.iter().all(|(k, v)| defs.iter().position(|d| d.key == *k).is_some_and(|i| same(&current[i], v)));
    if let Some(i) = presets.iter().position(|p| fits(preset_effect(defs, presets, p))) {
        return Some(Lit::Preset(i));
    }
    fits(default_effect(defs)).then_some(Lit::Default)
}

/// True when every setting is at its default.
pub fn all_default(defs: &[SettingDef], current: &[Json]) -> bool {
    defs.iter().zip(current).all(|(d, c)| same(c, &d.default))
}

/// Keybinds of the same group that share an input with `key`: their names.
pub fn bind_conflicts(defs: &[SettingDef], current: &[Json], key: &str) -> Vec<String> {
    let Some(i) = defs.iter().position(|d| d.key == key) else { return Vec::new() };
    let Kind::Keybind { group: g, .. } = &defs[i].kind else { return Vec::new() };
    let mine = bind_names(current[i].as_str().unwrap_or(""));
    defs.iter()
        .enumerate()
        .filter(|(j, d)| *j != i && matches!(&d.kind, Kind::Keybind { group, .. } if group == g))
        .filter(|(j, _)| bind_names(current[*j].as_str().unwrap_or("")).iter().any(|n| mine.contains(n)))
        .map(|(_, d)| d.name.clone())
        .collect()
}

/// `<mods dir>\settings.json`. Read fresh before every write so what the game's page wrote in between is not lost; everything this code
/// does not know (other mods, other keys, other top-level keys) is kept.
#[derive(Clone, Debug, Default)]
pub struct SettingsFile {
    pub path: PathBuf,
    root: Map<String, Json>,
    /// The file says `format` newer than 1: values are read, but never written.
    pub newer: bool,
    /// The file could not be read as JSON (values are defaults; the first write moves it to settings.json.broken).
    pub broken: Option<String>,
}

pub const FORMAT: u64 = 1;

impl SettingsFile {
    pub fn load(path: &Path) -> SettingsFile {
        let mut f = SettingsFile { path: path.to_path_buf(), ..Default::default() };
        f.refresh();
        f
    }

    /// Re-reads the file.
    pub fn refresh(&mut self) {
        self.root = Map::new();
        self.newer = false;
        self.broken = None;
        let bytes = match std::fs::read(&self.path) {
            Ok(b) => b,
            Err(_) => return,
        };
        let text = String::from_utf8_lossy(&bytes);
        match serde_json::from_str::<Json>(text.trim_start_matches('\u{feff}')) {
            Ok(Json::Object(o)) => {
                self.newer = o.get("format").and_then(Json::as_u64).is_some_and(|n| n > FORMAT);
                self.root = o;
            }
            Ok(_) => self.broken = Some("it is not a JSON object".to_string()),
            Err(e) => self.broken = Some(e.to_string()),
        }
    }

    /// The stored values of mod `id` (empty when none).
    pub fn mod_values(&self, id: &str) -> Map<String, Json> {
        self.root.get("mods").and_then(|m| m.get(id)).and_then(Json::as_object).cloned().unwrap_or_default()
    }

    /// Sets (`Some`) or removes (`None`) keys of mod `id` and writes the file. Err = the reason nothing was written.
    pub fn apply(&mut self, id: &str, changes: &[(String, Option<Json>)]) -> Result<(), String> {
        self.refresh();
        if self.newer {
            return Err("settings.json was written by a newer version; it is left as it is".to_string());
        }
        if let Some(why) = self.broken.take() {
            let bad = self.path.with_extension("json.broken");
            let _ = std::fs::remove_file(&bad);
            std::fs::rename(&self.path, &bad).map_err(|e| format!("settings.json is damaged ({why}) and could not be moved aside: {e}"))?;
        }
        self.root.insert("format".to_string(), Json::from(FORMAT));
        let mods = self.root.entry("mods".to_string()).or_insert_with(|| Json::Object(Map::new()));
        if !mods.is_object() {
            *mods = Json::Object(Map::new());
        }
        let mods = mods.as_object_mut().ok_or("settings.json: mods is not an object")?;
        let entry = mods.entry(id.to_string()).or_insert_with(|| Json::Object(Map::new()));
        if !entry.is_object() {
            *entry = Json::Object(Map::new());
        }
        let values = entry.as_object_mut().ok_or("settings.json: the mod's values are not an object")?;
        for (k, v) in changes {
            match v {
                Some(v) => {
                    values.insert(k.clone(), v.clone());
                }
                None => {
                    values.remove(k);
                }
            }
        }
        if values.is_empty() {
            mods.remove(id);
        }
        self.write()
    }

    fn write(&self) -> Result<(), String> {
        let mut text = serde_json::to_string_pretty(&Json::Object(self.root.clone())).map_err(|e| e.to_string())?;
        text.push('\n');
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| format!("settings.json could not be written: {e}"))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| format!("settings.json could not be written: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn defs_of(j: Json) -> (Vec<SettingDef>, Vec<String>) {
        let mut errors = Vec::new();
        let d = parse_settings(Some(&j), &mut errors);
        (d, errors)
    }

    fn sample() -> Vec<SettingDef> {
        let (d, e) = defs_of(json!([
            { "key": "rate", "name": "Rate", "type": "float", "default": 1.0, "min": 0.5, "max": 3.0, "step": 0.1 },
            { "key": "n", "name": "Count", "type": "int", "default": 10, "min": 1, "max": 600 },
            { "key": "on", "name": "On", "type": "bool", "default": true },
            { "key": "mode", "name": "Mode", "type": "choice", "default": "a", "choices": [ {"value":"a","name":"A"}, {"value":"b","name":"B"} ] },
            { "key": "hi", "name": "Greeting", "type": "string", "default": "hi", "max_length": 5 },
            { "key": "jump", "name": "Jump", "type": "keybind", "default": "Space", "group": "play" },
            { "key": "fire", "name": "Fire", "type": "keybind", "default": "KeyF", "binds": 1, "group": "play" },
            { "key": "adv", "name": "Adv", "type": "bool", "default": false, "group": "advanced" }
        ]));
        assert!(e.is_empty(), "{e:?}");
        d
    }

    #[test]
    fn every_type_parses_with_its_limits() {
        let d = sample();
        assert_eq!(d.len(), 8);
        assert_eq!(d[0].kind, Kind::Float { min: Some(0.5), max: Some(3.0), step: Some(0.1) });
        assert_eq!(d[1].kind, Kind::Int { min: Some(1), max: Some(600) });
        assert!(matches!(&d[3].kind, Kind::Choice { choices } if choices.len() == 2));
        assert_eq!(d[5].kind, Kind::Keybind { binds: 2, group: "play".into() });
        assert_eq!(d[6].kind, Kind::Keybind { binds: 1, group: "play".into() });
        assert!(d[7].advanced && !d[5].advanced);
        assert_eq!(d[0].default, json!(1.0));
    }

    #[test]
    fn bad_declarations_are_reported_not_crashed_on() {
        for bad in [
            json!([{ "key": "A", "name": "x", "type": "bool", "default": true }]),
            json!([{ "key": "a", "name": "x", "type": "wibble", "default": true }]),
            json!([{ "key": "a", "name": "x", "type": "int", "default": 5.5 }]),
            json!([{ "key": "a", "name": "x", "type": "int", "default": 50, "max": 10 }]),
            json!([{ "key": "a", "name": "x", "type": "choice", "default": "z", "choices": [{"value":"a","name":"A"},{"value":"b","name":"B"}] }]),
            json!([{ "key": "a", "name": "x", "type": "choice", "default": "a", "choices": [{"value":"a","name":"A"}] }]),
            json!([{ "key": "a", "name": "x", "type": "keybind", "default": "Nope" }]),
            json!([{ "key": "a", "name": "x", "type": "string", "default": "toolong", "max_length": 3 }]),
            json!([{ "key": "a", "name": "x", "type": "bool", "default": true }, { "key": "a", "name": "y", "type": "bool", "default": true }]),
            json!({ "not": "a list" }),
            json!(["not an object"]),
        ] {
            let (_, e) = defs_of(bad.clone());
            assert!(!e.is_empty(), "{bad} should be a problem");
        }
    }

    #[test]
    fn values_that_do_not_fit_are_the_default() {
        let d = sample();
        let given = json!({ "rate": 9.0, "n": "x", "on": false, "mode": "zzz", "hi": "toolong", "jump": "Nope", "fire": "KeyG,KeyH", "extra": 1 });
        let r = resolve(&d, given.as_object().unwrap());
        assert_eq!(r[0], json!(1.0), "out of range");
        assert_eq!(r[1], json!(10), "wrong type");
        assert_eq!(r[2], json!(false), "valid is kept");
        assert_eq!(r[3], json!("a"));
        assert_eq!(r[4], json!("hi"));
        assert_eq!(r[5], json!("Space"));
        assert_eq!(r[6], json!("KeyF"), "two binds on a 1-bind setting");
        let r = resolve(&d, &Map::new());
        assert!(all_default(&d, &r));
        let int_as_float = resolve(&d, json!({ "rate": 2 }).as_object().unwrap());
        assert_eq!(int_as_float[0], json!(2.0), "a whole number for a float is a float");
    }

    #[test]
    fn snap_rounds_to_the_step() {
        let d = sample();
        assert_eq!(snap(&d[0], 0.30000000000000004 + 0.5), 0.8);
        assert_eq!(snap(&d[0], 1.04), 1.0);
        assert_eq!(snap(&d[0], 9.0), 3.0);
    }

    fn presets_of(d: &[SettingDef], j: Json) -> (Vec<Preset>, Vec<String>) {
        let mut e = Vec::new();
        let p = parse_presets(Some(&j), d, &mut e);
        (p, e)
    }

    #[test]
    fn presets_cover_keybinds_and_named_settings_and_leave_the_rest() {
        let d = sample();
        let (p, e) = presets_of(&d, json!([
            { "id": "easy", "name": "Easy", "values": { "rate": 2.0, "mode": "b", "jump": "KeyJ" } },
            { "id": "hard", "name": "Hard", "values": { "rate": 0.5 } }
        ]));
        assert!(e.is_empty(), "{e:?}");
        let eff = preset_effect(&d, &p, &p[1]);
        let keys: Vec<&str> = eff.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, vec!["rate", "mode", "jump", "fire"], "rate and mode are named by some preset, both keybinds always; n, on, hi, adv are not covered");
        assert_eq!(eff[0].1, json!(0.5));
        assert_eq!(eff[1].1, json!("a"), "mode is not in Hard: back to its default");
        assert_eq!(eff[2].1, json!("Space"), "keybinds not named go back to their defaults");
        let easy = preset_effect(&d, &p, &p[0]);
        assert_eq!(easy[2].1, json!("KeyJ"));
    }

    #[test]
    fn the_lit_button_follows_the_values_and_a_named_preset_beats_default() {
        let d = sample();
        let (p, _) = presets_of(&d, json!([
            { "id": "std", "name": "Standard", "values": { "rate": 1.0, "mode": "a" } },
            { "id": "easy", "name": "Easy", "values": { "rate": 2.0, "mode": "b" } }
        ]));
        let mut cur = resolve(&d, &Map::new());
        assert_eq!(lit(&d, &p, &cur), Some(Lit::Preset(0)), "all default and Standard says the same: Standard is lit");
        for (k, v) in preset_effect(&d, &p, &p[1]) {
            let i = d.iter().position(|x| x.key == k).unwrap();
            cur[i] = v;
        }
        assert_eq!(lit(&d, &p, &cur), Some(Lit::Preset(1)));
        cur[2] = json!(false); // "on" is not covered by any preset: the lit preset does not change
        assert_eq!(lit(&d, &p, &cur), Some(Lit::Preset(1)));
        cur[0] = json!(1.7);
        assert_eq!(lit(&d, &p, &cur), None, "custom");
        let (none, _) = presets_of(&d, json!([]));
        assert_eq!(lit(&d, &none, &resolve(&d, &Map::new())), Some(Lit::Default));
        assert_eq!(lit(&d, &none, &cur), None);
    }

    #[test]
    fn presets_with_a_bad_value_are_reported() {
        let d = sample();
        let (_, e) = presets_of(&d, json!([{ "id": "x", "name": "X", "values": { "rate": 99.0, "ghost": 1 } }]));
        assert_eq!(e.len(), 2, "{e:?}");
        let (_, e) = presets_of(&d, json!([{ "id": "x", "name": "X", "values": {} }, { "id": "x", "name": "Y", "values": {} }]));
        assert!(e.iter().any(|m| m.contains("twice")));
    }

    #[test]
    fn keybinds_of_a_group_warn_when_they_share_an_input() {
        let d = sample();
        let mut cur = resolve(&d, &Map::new());
        assert!(bind_conflicts(&d, &cur, "jump").is_empty());
        cur[5] = json!("KeyF,Space");
        assert_eq!(bind_conflicts(&d, &cur, "jump"), vec!["Fire".to_string()]);
        assert_eq!(bind_conflicts(&d, &cur, "fire"), vec!["Jump".to_string()]);
        assert!(bind_conflicts(&d, &cur, "rate").is_empty());
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("racl_settings_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_file_round_trips_and_keeps_what_it_does_not_know() {
        let d = scratch("rt");
        let p = d.join("settings.json");
        std::fs::write(&p, "\u{feff}{\"format\":1,\"future\":{\"a\":1},\"mods\":{\"other-mod\":{\"zz\":[1,2]},\"arsenal-mastery\":{\"xp_rate\":2.0,\"unknown_key\":\"keep\"}}}").unwrap();
        let mut f = SettingsFile::load(&p);
        assert_eq!(f.mod_values("arsenal-mastery")["xp_rate"], json!(2.0));
        f.apply("arsenal-mastery", &[("perks".into(), Some(json!(false))), ("xp_rate".into(), None)]).unwrap();
        let back: Json = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(back["format"], json!(1));
        assert_eq!(back["future"], json!({"a": 1}), "unknown top-level key kept");
        assert_eq!(back["mods"]["other-mod"], json!({"zz": [1, 2]}), "unknown mod kept");
        assert_eq!(back["mods"]["arsenal-mastery"], json!({"perks": false, "unknown_key": "keep"}));
        let bytes = std::fs::read(&p).unwrap();
        assert_ne!(&bytes[..3], &[0xEF, 0xBB, 0xBF], "no BOM written");
        assert!(!d.join("settings.json.tmp").exists(), "atomic write leaves no temp file");
        // reload gives the same
        assert_eq!(SettingsFile::load(&p).mod_values("arsenal-mastery").len(), 2);
        // removing the last known key of a mod with nothing else removes the empty entry only
        f.apply("lonely", &[("a".into(), Some(json!(1)))]).unwrap();
        f.apply("lonely", &[("a".into(), None)]).unwrap();
        assert!(f.mod_values("lonely").is_empty());
        let back: Json = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert!(back["mods"].get("lonely").is_none() && back["mods"].get("other-mod").is_some());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_write_does_not_lose_what_the_game_wrote_in_between() {
        let d = scratch("game");
        let p = d.join("settings.json");
        let mut f = SettingsFile::load(&p);
        f.apply("m", &[("a".into(), Some(json!(1)))]).unwrap();
        // the game's page changes another key behind our back
        std::fs::write(&p, "{\"format\":1,\"mods\":{\"m\":{\"a\":1,\"b\":2}}}").unwrap();
        f.apply("m", &[("a".into(), Some(json!(5)))]).unwrap();
        assert_eq!(SettingsFile::load(&p).mod_values("m"), json!({"a": 5, "b": 2}).as_object().unwrap().clone());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn missing_broken_and_newer_files() {
        let d = scratch("bad");
        let p = d.join("settings.json");
        let mut f = SettingsFile::load(&p);
        assert!(f.mod_values("m").is_empty() && f.broken.is_none(), "no file is fine");
        std::fs::write(&p, "{ nope").unwrap();
        let mut f2 = SettingsFile::load(&p);
        assert!(f2.broken.is_some() && f2.mod_values("m").is_empty());
        f2.apply("m", &[("a".into(), Some(json!(1)))]).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("settings.json.broken")).unwrap(), "{ nope", "the damaged file is kept aside");
        assert_eq!(SettingsFile::load(&p).mod_values("m")["a"], json!(1));
        std::fs::write(&p, "{\"format\":7,\"mods\":{\"m\":{\"a\":3}}}").unwrap();
        let mut f3 = SettingsFile::load(&p);
        assert!(f3.newer);
        assert_eq!(f3.mod_values("m")["a"], json!(3), "a newer file is read");
        assert!(f3.apply("m", &[("a".into(), Some(json!(4)))]).is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{\"format\":7,\"mods\":{\"m\":{\"a\":3}}}", "and never overwritten");
        std::fs::write(&p, "[1,2]").unwrap();
        assert!(SettingsFile::load(&p).broken.is_some());
        f.apply("n", &[("k".into(), Some(json!("v")))]).unwrap();
        let _ = std::fs::remove_dir_all(&d);
    }
}
