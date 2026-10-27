//! Version gating: which game version first knows an option or a feature. The launcher greys out what the install's version does not know.
//! One small table: add a line when a game version brings a new option.
use crate::update::compare_version;
use std::cmp::Ordering;

/// Feature name of the texture packs in `mods\` (the game side ships in a later game version).
pub const TEXTURE_PACKS: &str = "texture-packs";
/// Feature name of the script and native mods (OmniLoader) in `mods\`; they come in the same game version as the texture packs.
pub const SCRIPT_MODS: &str = "script-mods";

/// (option key or feature name, first game version that knows it).
pub const FIRST_VERSION: &[(&str, &str)] = &[
    // pc_options.txt: the graphics system (Vulkan / DirectX 12)
    ("renderer", "0.4.0"),
    // pc_options.txt: the mouse speed of the Modern camera (before 0.4.0 the mouse followed camspeed)
    ("mousespeed", "0.4.0"),
    // pc_options.txt keys that 0.2.1 / 0.2.2 do not read (they keep unknown lines and ignore them): all came with 0.3.0.
    // The 0.2.x file knows: lang camera strafe buyall lighting ao shadows display water particles sunrays (water / particles only classic|enhanced).
    ("camfeel", "0.3.0"),
    ("camspeed", "0.3.0"),
    ("culling", "0.3.0"),
    ("debug", "0.3.0"),
    ("draw", "0.3.0"),
    ("gi", "0.3.0"),
    ("hud", "0.3.0"),
    ("lockon", "0.3.0"),
    ("smoothing", "0.3.0"),
    ("vegetation", "0.3.0"),
    ("waves", "0.3.0"),
    // pc_options.txt: the volumetric fog (Graphics -> Effects -> Fog) came with 0.4.0
    ("fog", "0.4.0"),
    ("clouds", "0.4.0"),
    ("texfilter", "0.4.0"),
    // pc_options.txt: Skip intro (intro=play|skip)
    ("intro", "0.4.0"),
    // texture packs (and the Mods page master ticks, graphics presets) need this game version: the next public release is 0.4.0 (0.3.2 and 0.3.5 were never released)
    (TEXTURE_PACKS, "0.4.0"),
    (SCRIPT_MODS, "0.4.0"),
];

/// (option key, one value of it, first game version where that value does something). Per-value rows: the key itself is older.
/// 0.2.x reads water / particles as classic|enhanced only: low and medium are unknown values there (the game keeps its default look),
/// while high is the same picture as the old "enhanced", so it stays usable.
pub const FIRST_VERSION_VALUE: &[(&str, &str, &str)] = &[
    ("water", "low", "0.3.0"),
    ("water", "medium", "0.3.0"),
    ("particles", "low", "0.3.0"),
    ("particles", "medium", "0.3.0"),
];

/// The first game version that knows `key`; None = every version knows it.
pub fn first_version(key: &str) -> Option<&'static str> {
    FIRST_VERSION.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

/// True when the install at `version` knows `key` (keys that are not in the table are known everywhere).
pub fn available(version: &str, key: &str) -> bool {
    match first_version(key) {
        Some(first) => compare_version(version, first) != Ordering::Less,
        None => true,
    }
}

/// True when the install at `version` does something with `value` of `key` (the key must be known, too).
pub fn value_available(version: &str, key: &str, value: &str) -> bool {
    if !available(version, key) {
        return false;
    }
    match FIRST_VERSION_VALUE.iter().find(|(k, v, _)| *k == key && *v == value) {
        Some((_, _, first)) => compare_version(version, first) != Ordering::Less,
        None => true,
    }
}

/// Joins labels the way a person says them: "Low", "Low and Medium", "Low, Medium and High".
fn join_labels(labels: &[&str]) -> String {
    match labels {
        [] => String::new(),
        [one] => (*one).to_string(),
        [init @ .., last] => format!("{} and {}", init.join(", "), last),
    }
}

/// The plain-English line under a row whose key is known but some of its values are not. `values` = (file value, label) of every choice.
/// None when every value works on this version.
pub fn value_needs_text(version: &str, key: &str, values: &[(&str, &str)]) -> Option<String> {
    if !available(version, key) {
        return None;
    }
    let locked: Vec<&str> = values.iter().filter(|(v, _)| !value_available(version, key, v)).map(|(_, l)| *l).collect();
    if locked.is_empty() {
        return None;
    }
    let first = values
        .iter()
        .filter(|(v, _)| !value_available(version, key, v))
        .filter_map(|(v, _)| FIRST_VERSION_VALUE.iter().find(|(k, vv, _)| *k == key && vv == v).map(|(_, _, f)| *f))
        .max_by(|a, b| compare_version(a, b))?;
    let verb = if locked.len() == 1 { "needs" } else { "need" };
    Some(format!("{} {verb} game version {first} or newer (this install is {version}). Update the game to use {}.", join_labels(&locked), if locked.len() == 1 { "it" } else { "them" }))
}

/// The plain-English line for a greyed option.
pub fn needs_text(version: &str, key: &str) -> Option<String> {
    let first = first_version(key).filter(|_| !available(version, key))?;
    Some(format!("Needs game version {first} or newer (this install is {version}). Update the game to use it."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderer_needs_040() {
        assert!(!available("0.3.1", "renderer"));
        assert!(!available("0.2.9", "renderer"));
        assert!(available("0.4.0", "renderer"));
        assert!(available("0.10.0", "renderer"), "numeric compare: 10 > 4");
        assert!(!available("0.3.10", "renderer"), "0.3.x is older than 0.4.0");
        assert!(available("1.0.0", "renderer"));
    }

    #[test]
    fn unknown_keys_are_available_and_texts_read_well() {
        assert!(available("0.1.0", "lighting"));
        assert!(needs_text("0.1.0", "lighting").is_none());
        assert!(needs_text("0.4.0", "renderer").is_none());
        let t = needs_text("0.3.1", "renderer").unwrap();
        assert!(t.contains("0.4.0") && t.contains("0.3.1"));
        assert!(!available("0.3.1", TEXTURE_PACKS) && available("0.4.0", TEXTURE_PACKS));
    }

    #[test]
    fn single_values_are_gated_by_their_own_rows() {
        // 0.2.x: water / particles know classic and the old enhanced (= high) only
        for k in ["water", "particles"] {
            assert!(value_available("0.2.2", k, "classic"));
            assert!(value_available("0.2.2", k, "high"));
            assert!(!value_available("0.2.2", k, "low"));
            assert!(!value_available("0.2.2", k, "medium"));
            assert!(value_available("0.3.0", k, "low") && value_available("0.3.0", k, "medium"));
        }
        // a key that the table does not know, and a value that has no row, are available everywhere
        assert!(value_available("0.1.0", "ao", "low"));
        // a key that is itself too new makes every one of its values unavailable
        assert!(!value_available("0.3.1", "renderer", "vulkan"));
        assert!(!value_available("0.2.2", "waves", "on"));
    }

    #[test]
    fn value_lines_name_the_locked_values() {
        let levels = [("classic", "Classic"), ("low", "Low"), ("medium", "Medium"), ("high", "High")];
        let t = value_needs_text("0.2.2", "water", &levels).unwrap();
        assert!(t.starts_with("Low and Medium need game version 0.3.0 or newer"), "{t}");
        assert!(t.contains("0.2.2"));
        assert!(value_needs_text("0.3.0", "water", &levels).is_none());
        assert!(value_needs_text("0.2.2", "ao", &levels).is_none(), "no rows for ao");
        assert!(value_needs_text("0.2.2", "waves", &[("on", "On"), ("off", "Off")]).is_none(), "whole-key gating has its own line");
        assert_eq!(join_labels(&["A", "B", "C"]), "A, B and C");
        assert_eq!(join_labels(&["A"]), "A");
    }
}
