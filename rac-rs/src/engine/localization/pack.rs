// SPDX-License-Identifier: GPL-3.0-only
//! Versioned, declarative localization metadata. Mod priority is the launcher's priority.
use super::{read_json, relative};
use crate::omni::json::Json;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Clone, Debug)]
pub struct Pack {
    pub mod_id: String,
    pub id: String,
    pub name: String,
    pub base: String,
    pub root: PathBuf,
    pub strings: PathBuf,
    pub manifest: Json,
}

impl Pack {
    pub fn file(&self, key: &str, default: &str) -> Option<PathBuf> {
        relative(
            &self.root,
            self.manifest
                .get(key)
                .and_then(Json::str)
                .unwrap_or(default),
        )
    }
    pub fn flag(&self, key: &str) -> bool {
        self.manifest.get(key).is_some_and(Json::bool)
    }
    pub fn text(&self, key: &str) -> Option<&str> {
        self.manifest.get(key).and_then(Json::str)
    }
}

pub fn parse(mod_id: &str, root: &Path, doc: Json) -> Option<Pack> {
    if doc.get("format").and_then(Json::as_num) != Some(1.0) {
        return None;
    }
    let id = doc.get("language")?.str()?;
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    {
        return None;
    }
    let base = doc.get("base").and_then(Json::str).unwrap_or("en");
    if !["en", "fr", "de", "es", "it"].contains(&base) {
        return None;
    }
    let name = doc.get("name").and_then(Json::str).unwrap_or(id);
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return None;
    }
    let strings = relative(
        root,
        doc.get("strings")
            .and_then(Json::str)
            .unwrap_or("strings.json"),
    )?;
    Some(Pack {
        mod_id: mod_id.into(),
        id: id.into(),
        name: name.into(),
        base: base.into(),
        root: root.into(),
        strings,
        manifest: doc,
    })
}

pub fn packs() -> &'static [Pack] {
    static PACKS: OnceLock<Vec<Pack>> = OnceLock::new();
    PACKS.get_or_init(|| {
        let Some(root) = crate::texpack::mods::mods_root() else {
            return Vec::new();
        };
        let store = crate::texpack::store::Store::load(&root);
        store
            .mods
            .iter()
            .filter_map(|m| {
                let dir = root.join(&m.folder);
                let doc = read_json(&dir.join("localization.json"))?;
                let pack = parse(&m.id, &dir, doc);
                if pack.is_none() {
                    eprintln!(
                        "localization: unsupported or invalid manifest in {}",
                        m.folder
                    );
                }
                pack
            })
            .take(32)
            .collect()
    })
}

pub fn active_pack() -> Option<&'static Pack> {
    static ACTIVE: OnceLock<Option<usize>> = OnceLock::new();
    ACTIVE
        .get_or_init(|| {
            let root = crate::texpack::mods::mods_root()?;
            let store = crate::texpack::store::Store::load(&root);
            store.ids.iter().find_map(|id| {
                if store.master_of(id).is_some_and(|m| !store.masters.is_on(m)) {
                    return None;
                }
                packs().iter().position(|p| {
                    if &p.mod_id != id {
                        return false;
                    }
                    if let Some(serials) = p.manifest.get("serials") {
                        let serial = std::env::var("ORACLE_ISO")
                            .ok()
                            .and_then(|iso| ee_oracle::ver::detect(&iso))
                            .map(|d| d.serial());
                        if !serial.is_some_and(|s| serials.arr().iter().any(|v| v.str() == Some(s)))
                        {
                            eprintln!("localization: {} does not support this disc", p.mod_id);
                            return false;
                        }
                    }
                    true
                })
            })
        })
        .map(|i| &packs()[i])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::omni::json;
    #[test]
    fn manifest_is_versioned_and_language_independent() {
        let make = |s| parse("example", Path::new("pack"), json::parse(s).unwrap());
        assert_eq!(
            make(r#"{"format":1,"language":"tr-TR","name":"Türkçe"}"#)
                .unwrap()
                .base,
            "en"
        );
        assert!(make(r#"{"format":2,"language":"cs"}"#).is_none());
        assert!(make(r#"{"format":1,"language":"../cs"}"#).is_none());
        assert!(make(r#"{"format":1,"language":"cs","strings":"../outside.json"}"#).is_none());
        assert!(make(r#"{"format":1,"language":"cs","base":"unknown"}"#).is_none());
    }
}
