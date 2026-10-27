//! Graphics presets of the Settings page: Low, Medium, High and Custom.
//!
//! A preset is a fixed set of values for the picture options (one table, `TABLE`). High is the game's own default (the picture the game
//! shows with no pc_options.txt); Low and Medium turn the costly options down. Only picture quality options are in the table: camera,
//! controls, language, window mode and the graphics system (Vulkan / DirectX 12) are never touched by a preset.
//!
//! The game version decides what a preset can set (`gating`): options the version does not know are skipped, and a level the version does
//! not know (Water / Particles Low and Medium before 0.3.0) becomes the next lower level it knows. Custom is not a set of values: it is what
//! the selector shows when the options match none of the presets.
use super::rows;
use crate::gating;
use crate::pcopts::Options;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Low,
    Medium,
    High,
    Custom,
}

impl Preset {
    /// Selector order.
    pub const ALL: [Preset; 4] = [Preset::Low, Preset::Medium, Preset::High, Preset::Custom];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Low => "Low",
            Preset::Medium => "Medium",
            Preset::High => "High",
            Preset::Custom => "Custom",
        }
    }

    /// Index into a `TABLE` row (Custom has no values).
    fn column(self) -> Option<usize> {
        match self {
            Preset::Low => Some(0),
            Preset::Medium => Some(1),
            Preset::High => Some(2),
            Preset::Custom => None,
        }
    }
}

/// THE PRESET TABLE: (pc_options.txt key, [Low, Medium, High]). High must stay equal to the game's defaults (a test checks it against the
/// rows of the Settings page). Lighting stays Classic everywhere; the Enhanced effects below it only show once Lighting is Enhanced, and a
/// preset sets them too, so "Low + Lighting Enhanced" is a light Enhanced picture.
pub const TABLE: &[(&str, [&str; 3])] = &[
    //  key            Low         Medium      High (= default)
    ("lighting", ["classic", "classic", "classic"]),
    ("draw", ["low", "low", "medium"]),
    ("smoothing", ["off", "on", "on"]),
    ("hud", ["original", "sharp", "sharp"]),
    ("ao", ["low", "medium", "high"]),
    ("shadows", ["low", "medium", "high"]),
    ("water", ["low", "medium", "high"]),
    ("particles", ["low", "medium", "high"]),
    ("sunrays", ["low", "medium", "high"]),
    ("vegetation", ["off", "moderate", "moderate"]),
    ("gi", ["off", "off", "off"]),
    ("waves", ["off", "on", "on"]),
    ("fog", ["classic", "low", "high"]),
    ("clouds", ["classic", "low", "high"]),
    ("texfilter", ["smooth", "smooth", "smooth"]),
];

fn default_of(key: &str) -> Option<&'static str> {
    rows::ROWS.iter().find(|r| r.key == key).map(|r| r.default)
}

/// The levels of Water and Particles, lowest first (the only values the gating table knows per version).
const SCALE: [&str; 4] = ["classic", "low", "medium", "high"];

/// What `value` of `key` becomes on this game version: None = the version does not know the option (a preset skips it); a level it does
/// not know is replaced by the next lower one it knows.
fn effective(version: &str, key: &str, value: &'static str) -> Option<&'static str> {
    if !gating::available(version, key) {
        return None;
    }
    let Some(mut i) = SCALE.iter().position(|s| *s == value) else { return Some(value) };
    while i > 0 && !gating::value_available(version, key, SCALE[i]) {
        i -= 1;
    }
    Some(SCALE[i])
}

/// Sets the options of `preset` (Custom changes nothing). A value equal to the game's default removes the key, so High leaves the file as
/// it would be with no options at all for these keys.
pub fn apply(o: &mut Options, version: &str, preset: Preset) {
    let Some(col) = preset.column() else { return };
    for (key, values) in TABLE {
        let Some(v) = effective(version, key, values[col]) else { continue };
        o.set(key, if default_of(key) == Some(v) { None } else { Some(v) });
    }
}

/// True when every option of the preset that this game version knows has the preset's value (a key never set counts as the default).
pub fn matches(o: &Options, version: &str, preset: Preset) -> bool {
    let Some(col) = preset.column() else { return false };
    TABLE.iter().all(|(key, values)| match effective(version, key, values[col]) {
        Some(v) => o.get(key).or_else(|| default_of(key)) == Some(v),
        None => true,
    })
}

/// The preset the options are: High first (the game's default), then Medium, then Low; Custom when none matches.
pub fn detect(o: &Options, version: &str) -> Preset {
    [Preset::High, Preset::Medium, Preset::Low].into_iter().find(|p| matches(o, version, *p)).unwrap_or(Preset::Custom)
}

/// The table as a plain data block, one `<preset>.<key>=<value>` line each (low, medium, high). The game can ship the same block as a file;
/// `TABLE` stays the one place the preset code reads its values from.
pub fn table_text() -> String {
    let mut out = String::new();
    for (col, name) in ["low", "medium", "high"].into_iter().enumerate() {
        for (key, values) in TABLE {
            out += &format!("{name}.{key}={}
", values[col]);
        }
    }
    out
}

/// The line under the selector when this game version knows only some of the options (None = it knows them all).
pub fn version_note(version: &str) -> Option<String> {
    let all_known = TABLE.iter().all(|(key, values)| values.iter().all(|v| effective(version, key, v) == Some(*v)));
    (!all_known).then(|| format!("Game version {version} knows only some of these options, so a preset sets just those."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::rows::{Group, Kind};

    fn keys() -> Vec<&'static str> {
        TABLE.iter().map(|(k, _)| *k).collect()
    }

    #[test]
    fn high_is_the_game_default_and_every_value_is_one_the_rows_offer() {
        for (key, values) in TABLE {
            let row = rows::ROWS.iter().find(|r| r.key == *key).unwrap_or_else(|| panic!("{key} has no row"));
            assert_eq!(values[2], row.default, "High of {key} must be the game's default");
            let Kind::Choice(list) = row.kind else { panic!("{key} is not a choice row") };
            for v in values {
                assert!(list.iter().any(|(lv, _)| lv == v), "{key}={v} is not a choice of its row");
                assert_eq!(crate::pcopts::canon(key, v).as_deref(), Some(*v), "{key}={v} is not written the way the game reads it");
            }
        }
    }

    #[test]
    fn only_picture_quality_options_are_in_the_table() {
        for k in keys() {
            let row = rows::ROWS.iter().find(|r| r.key == k).unwrap();
            assert!(matches!(row.group, Group::Graphics | Group::Enhanced), "{k}");
        }
        for never in ["camera", "camfeel", "camspeed", "mousespeed", "strafe", "lockon", "buyall", "lang", "renderer", "display", "debug", "culling", "keys", "pad"] {
            assert!(!keys().contains(&never), "{never} must not be part of a preset");
        }
        let mut seen = keys();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), TABLE.len(), "a key twice in the table");
    }

    #[test]
    fn no_options_file_is_high_and_high_writes_nothing() {
        let o = Options::new();
        assert_eq!(detect(&o, "0.4.0"), Preset::High);
        let mut p = Options::parse("version=1\nao=low\ndraw=low\nwater=classic\n");
        apply(&mut p, "0.4.0", Preset::High);
        assert_eq!(p.to_text(), "version=1\n", "High removes the keys: the game's own defaults");
        assert_eq!(detect(&p, "0.4.0"), Preset::High);
        // the same defaults written out are still High
        assert_eq!(detect(&Options::parse("ao=high\nshadows=high\nwater=high\nhud=sharp\nwaves=on\n"), "0.4.0"), Preset::High);
    }

    #[test]
    fn low_and_medium_lower_the_costly_options_and_are_found_again() {
        let mut o = Options::new();
        apply(&mut o, "0.4.0", Preset::Low);
        assert_eq!(o.get("draw"), Some("low"));
        assert_eq!((o.get("smoothing"), o.get("hud"), o.get("waves"), o.get("vegetation")), (Some("off"), Some("original"), Some("off"), Some("off")));
        assert_eq!((o.get("ao"), o.get("water"), o.get("particles")), (Some("low"), Some("low"), Some("low")));
        assert_eq!(detect(&o, "0.4.0"), Preset::Low);
        apply(&mut o, "0.4.0", Preset::Medium);
        assert_eq!((o.get("draw"), o.get("ao"), o.get("water")), (Some("low"), Some("medium"), Some("medium")));
        assert_eq!(o.get("smoothing"), None, "default On: no key");
        assert_eq!(detect(&o, "0.4.0"), Preset::Medium);
        apply(&mut o, "0.4.0", Preset::High);
        assert_eq!(detect(&o, "0.4.0"), Preset::High);
        // each step is a real change
        let (mut l, mut m) = (Options::new(), Options::new());
        apply(&mut l, "0.4.0", Preset::Low);
        apply(&mut m, "0.4.0", Preset::Medium);
        assert_ne!(l.to_text(), m.to_text());
        assert_ne!(m.to_text(), Options::new().to_text());
    }

    #[test]
    fn changing_one_value_afterwards_makes_it_custom_and_a_preset_never_touches_other_options() {
        let mut o = Options::parse("version=1\nlang=de\ncamera=classic\nstrafe=off\nlockon=on\ncamspeed=70\nrenderer=dx12\ndisplay=fullscreen\ndebug=on\nkeys=cross:KeyK\nfuture=1\n");
        let before = o.to_text();
        apply(&mut o, "0.4.0", Preset::High);
        assert_eq!(o.to_text(), before, "High on an options file with only other options changes nothing");
        apply(&mut o, "0.4.0", Preset::Low);
        for line in before.lines() {
            assert!(o.to_text().lines().any(|l| l == line), "{line} lost");
        }
        assert_eq!(detect(&o, "0.4.0"), Preset::Low);
        // other options do not make it Custom
        o.set("renderer", Some("vulkan"));
        o.set("display", Some("windowed"));
        assert_eq!(detect(&o, "0.4.0"), Preset::Low);
        // one picture option does
        let ao = rows::ROWS.iter().find(|r| r.key == "ao").unwrap();
        rows::choose(&mut o, ao, 3);
        assert_eq!(detect(&o, "0.4.0"), Preset::Custom);
        assert!(!matches(&o, "0.4.0", Preset::Custom), "Custom is not a set of values");
        let mut c = o.clone();
        apply(&mut c, "0.4.0", Preset::Custom);
        assert_eq!(c, o, "applying Custom does nothing");
        // Lighting is part of the presets
        let mut e = Options::new();
        e.set("lighting", Some("enhanced"));
        assert_eq!(detect(&e, "0.4.0"), Preset::Custom);
    }

    #[test]
    fn older_game_versions_get_only_what_they_know() {
        // 0.2.2 knows lighting ao shadows water particles sunrays, and Water / Particles only as classic or high
        let mut o = Options::new();
        apply(&mut o, "0.2.2", Preset::Low);
        let t = o.to_text();
        assert_eq!((o.get("water"), o.get("particles")), (Some("classic"), Some("classic")), "{t}");
        assert_eq!((o.get("ao"), o.get("shadows"), o.get("sunrays")), (Some("low"), Some("low"), Some("low")));
        for key in ["draw", "smoothing", "hud", "vegetation", "gi", "waves"] {
            assert_eq!(o.get(key), None, "{key} is unknown to 0.2.2: {t}");
        }
        assert_eq!(detect(&o, "0.2.2"), Preset::Low);
        apply(&mut o, "0.2.2", Preset::Medium);
        assert_eq!((o.get("water"), o.get("particles"), o.get("ao")), (Some("classic"), Some("classic"), Some("medium")));
        assert_eq!(detect(&o, "0.2.2"), Preset::Medium);
        // 0.3.0 knows every level and every key except fog (0.4.0)
        let mut p = Options::new();
        apply(&mut p, "0.3.0", Preset::Medium);
        assert_eq!((p.get("water"), p.get("draw")), (Some("medium"), Some("low")));
        // the note
        assert!(version_note("0.2.2").unwrap().contains("0.2.2"));
        assert_eq!(p.get("fog"), None, "fog is unknown to 0.3.0");
        assert!(version_note("0.4.0").is_none() && version_note("0.3.0").is_some());
    }

    #[test]
    fn the_table_as_a_data_block() {
        let t = table_text();
        assert_eq!(t.lines().count(), TABLE.len() * 3);
        assert!(t.lines().any(|l| l == "low.draw=low") && t.lines().any(|l| l == "medium.water=medium") && t.lines().any(|l| l == "high.draw=medium"));
    }

    #[test]
    fn the_level_walk_stops_at_what_the_version_knows() {
        assert_eq!(effective("0.2.2", "water", "medium"), Some("classic"));
        assert_eq!(effective("0.2.2", "water", "high"), Some("high"));
        assert_eq!(effective("0.3.0", "particles", "low"), Some("low"));
        assert_eq!(effective("0.2.2", "waves", "off"), None, "unknown key");
        assert_eq!(effective("0.2.2", "ao", "low"), Some("low"), "rows without per-value gating are untouched");
        assert_eq!(effective("0.4.0", "draw", "medium"), Some("medium"));
    }
}
