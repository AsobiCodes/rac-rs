//! The rows of the Settings page: which option key, how it reads for a player, its choices and the game's default.
//! Same options as the game's own Pause -> Options menu (they are the same file); the launcher adds the renderer.
use crate::pcopts::{self, Options};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Graphics,
    Enhanced,
    Gameplay,
    Language,
}

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    /// (file value, label) pairs.
    Choice(&'static [(&'static str, &'static str)]),
    /// The Camera row: Classic / Static / Smooth, stored as camera= and camfeel=.
    Camera,
    /// A percent slider, 10..200 in steps of 10: Camera speed (no key = the game's own Slow / Medium / Fast row) and Mouse speed
    /// (no key = the row's `default`, 50).
    Percent,
}

#[derive(Clone, Copy, Debug)]
pub struct Row {
    pub key: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub kind: Kind,
    /// The file value the game uses when the key is missing ("" = no default: the game decides at the title screen).
    pub default: &'static str,
    pub group: Group,
}

const ON_OFF: &[(&str, &str)] = &[("on", "On"), ("off", "Off")];
const LEVELS: &[(&str, &str)] = &[("off", "Off"), ("low", "Low"), ("medium", "Medium"), ("high", "High")];
const LEVELS_CLASSIC: &[(&str, &str)] = &[("classic", "Classic"), ("low", "Low"), ("medium", "Medium"), ("high", "High")];

pub const ROWS: &[Row] = &[
    Row {
        key: "renderer",
        label: "Graphics system",
        help: "What the game draws with. Vulkan is best for most PCs. If the game does not start or the picture is broken, try DirectX 12.",
        kind: Kind::Choice(&[("vulkan", "Vulkan"), ("dx12", "DirectX 12")]),
        default: "vulkan",
        group: Group::Graphics,
    },
    Row {
        key: "display",
        label: "Window",
        help: "Play in a window or fill the screen (without borders).",
        kind: Kind::Choice(&[("windowed", "Windowed"), ("fullscreen", "Fullscreen")]),
        default: "windowed",
        group: Group::Graphics,
    },
    Row {
        key: "lighting",
        label: "Lighting",
        help: "Classic is the picture of the PlayStation 2 game. Enhanced adds modern lighting; the effects below need it.",
        kind: Kind::Choice(&[("classic", "Classic"), ("enhanced", "Enhanced")]),
        default: "classic",
        group: Group::Graphics,
    },
    Row {
        key: "draw",
        label: "Draw distance",
        help: "How far away you can see. Low is the original distance.",
        kind: Kind::Choice(&[("low", "Low"), ("medium", "Medium"), ("high", "High")]),
        default: "medium",
        group: Group::Graphics,
    },
    Row {
        key: "smoothing",
        label: "Frame smoothing",
        help: "Adds in-between pictures so movement looks smooth on a fast screen.",
        kind: Kind::Choice(ON_OFF),
        default: "on",
        group: Group::Graphics,
    },
    Row {
        key: "hud",
        label: "Menus and HUD",
        help: "Sharp draws the menus and the on-screen display again at your screen's resolution. Original keeps the old low-resolution look.",
        kind: Kind::Choice(&[("sharp", "Sharp"), ("original", "Original")]),
        default: "sharp",
        group: Group::Graphics,
    },
    Row {
        key: "texfilter",
        label: "Texture filtering",
        help: "Smooth blends texture detail as you move. Classic keeps the original detail steps. Requires Lighting Enhanced.",
        kind: Kind::Choice(&[("classic", "Classic"), ("smooth", "Smooth")]),
        default: "smooth",
        group: Group::Enhanced,
    },
    Row {
        key: "ao",
        label: "Ambient occlusion",
        help: "Soft shading in corners and creases.",
        kind: Kind::Choice(LEVELS),
        default: "high",
        group: Group::Enhanced,
    },
    Row {
        key: "shadows",
        label: "Dynamic shadows",
        help: "Shadows that move with the characters and objects.",
        kind: Kind::Choice(LEVELS),
        default: "high",
        group: Group::Enhanced,
    },
    Row {
        key: "water",
        label: "Water",
        help: "Reflections and movement of water. Classic is the original look.",
        kind: Kind::Choice(LEVELS_CLASSIC),
        default: "high",
        group: Group::Enhanced,
    },
    Row {
        key: "waves",
        label: "Waves",
        help: "Real moving waves on the sea (with Water on Medium or High).",
        kind: Kind::Choice(ON_OFF),
        default: "on",
        group: Group::Enhanced,
    },
    Row {
        key: "fog",
        label: "Volumetric fog",
        help: "Volumetric fog that drifts and catches the sun, in each planet's own fog colour. Classic is the original look.",
        kind: Kind::Choice(LEVELS_CLASSIC),
        default: "high",
        group: Group::Enhanced,
    },
    Row {
        key: "clouds",
        label: "Volumetric clouds",
        help: "Real 3D clouds that drift and change shape, lit by the sun. Classic is the original sky.",
        kind: Kind::Choice(LEVELS_CLASSIC),
        default: "high",
        group: Group::Enhanced,
    },
    Row {
        key: "particles",
        label: "Particles",
        help: "Smoke, sparks, fire and other effects. Classic is the original look.",
        kind: Kind::Choice(LEVELS_CLASSIC),
        default: "high",
        group: Group::Enhanced,
    },
    Row {
        key: "sunrays",
        label: "Sun rays",
        help: "Beams of light through the air.",
        kind: Kind::Choice(LEVELS),
        default: "high",
        group: Group::Enhanced,
    },
    Row {
        key: "vegetation",
        label: "Vegetation",
        help: "How many plants and how lush the greenery is.",
        kind: Kind::Choice(&[("off", "Off"), ("moderate", "Moderate"), ("lush", "Lush")]),
        default: "moderate",
        group: Group::Enhanced,
    },
    Row {
        key: "gi",
        label: "Global illumination",
        help: "Light that bounces off surfaces. Looks great but needs a strong graphics card.",
        kind: Kind::Choice(LEVELS),
        default: "off",
        group: Group::Enhanced,
    },
    Row { key: "camera", label: "Camera", help: "Classic is the original camera. Static is a plain modern camera. Smooth is the modern camera that follows Ratchet smoothly.", kind: Kind::Camera, default: "smooth", group: Group::Gameplay },
    Row { key: "camspeed", label: "Camera speed", help: "How fast the right stick turns the camera, in percent. Default uses the game's own setting.", kind: Kind::Percent, default: "", group: Group::Gameplay },
    Row { key: "mousespeed", label: "Mouse speed", help: "How fast the mouse turns the camera (Static and Smooth camera), in percent.", kind: Kind::Percent, default: "50", group: Group::Gameplay },
    Row {
        key: "strafe",
        label: "Strafe aiming",
        help: "Hold the aim button to move sideways while aiming.",
        kind: Kind::Choice(ON_OFF),
        default: "on",
        group: Group::Gameplay,
    },
    Row {
        key: "lockon",
        label: "Lock-on circle",
        help: "The red circle that locks onto a target. Auto shows it with a controller and hides it with keyboard and mouse.",
        kind: Kind::Choice(&[("auto", "Auto"), ("on", "On"), ("off", "Off")]),
        default: "auto",
        group: Group::Gameplay,
    },
    Row {
        key: "buyall",
        label: "Buy all ammo",
        help: "A shortcut at the Gadgetron vendor that fills up your ammo in one press.",
        kind: Kind::Choice(ON_OFF),
        default: "on",
        group: Group::Gameplay,
    },
    Row {
        key: "intro",
        label: "Skip intro (go straight to the title screen)",
        help: "Starts the game without the opening logos and movie. Off plays them as on the console.",
        kind: Kind::Choice(&[("skip", "On"), ("play", "Off")]),
        default: "play",
        group: Group::Gameplay,
    },
    Row {
        key: "lang",
        label: "Language",
        help: "Leave it unmarked and the game asks at the title screen, as on the console.",
        kind: Kind::Choice(&[("en", "English"), ("fr", "Français"), ("de", "Deutsch"), ("es", "Español"), ("it", "Italiano")]),
        default: "",
        group: Group::Language,
    },
];

pub fn rows_of(group: Group) -> impl Iterator<Item = &'static Row> {
    ROWS.iter().filter(move |r| r.group == group)
}

pub const CAMERA_LABELS: [&str; 3] = ["Classic", "Static", "Smooth"];

/// Index of the current choice (the value the player set, else the game's default); None = nothing to mark.
pub fn current_index(o: &Options, row: &Row) -> Option<usize> {
    match row.kind {
        Kind::Choice(list) => {
            let v = o.get(row.key).unwrap_or(row.default);
            list.iter().position(|(val, _)| *val == v)
        }
        Kind::Camera => Some(o.camera_byte() as usize),
        Kind::Percent => None,
    }
}

/// The player picked choice `idx` of a row: written into the options (the caller stores the file).
pub fn choose(o: &mut Options, row: &Row, idx: usize) {
    match row.kind {
        Kind::Choice(list) => {
            if let Some((v, _)) = list.get(idx) {
                o.set(row.key, Some(v));
            }
        }
        Kind::Camera => o.set_camera_byte(idx.min(2) as u8),
        Kind::Percent => {}
    }
}

/// The percent the player set in a `Kind::Percent` row (`camspeed`, `mousespeed`); None = the row's default.
pub fn percent(o: &Options, key: &str) -> Option<u32> {
    o.get(key).and_then(|v| v.parse().ok())
}

pub fn set_percent(o: &mut Options, key: &str, pct: Option<u32>) {
    match pct {
        Some(p) => o.set(key, Some(&pcopts::snap_camspeed(p).to_string())),
        None => o.set(key, None),
    }
}

/// True when the row is a graphics effect that only works with Lighting = Enhanced.
pub fn needs_enhanced(row: &Row) -> bool {
    row.group == Group::Enhanced
}

pub fn lighting_enhanced(o: &Options) -> bool {
    o.get("lighting").unwrap_or("classic") == "enhanced"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_choice_value_is_accepted_by_the_options_file() {
        for r in ROWS {
            if let Kind::Choice(list) = r.kind {
                for (v, _) in list {
                    assert!(pcopts::canon(r.key, v).as_deref() == Some(*v), "{} = {v}", r.key);
                }
                if !r.default.is_empty() {
                    assert!(list.iter().any(|(v, _)| *v == r.default), "default of {}", r.key);
                }
            }
        }
    }

    #[test]
    fn defaults_are_marked_and_choices_write_through() {
        let mut o = Options::new();
        let water = ROWS.iter().find(|r| r.key == "water").unwrap();
        assert_eq!(current_index(&o, water), Some(3));
        choose(&mut o, water, 1);
        assert_eq!(o.get("water"), Some("low"));
        let cam = ROWS.iter().find(|r| r.key == "camera").unwrap();
        assert_eq!(current_index(&o, cam), Some(2));
        choose(&mut o, cam, 1);
        assert_eq!((o.get("camera"), o.get("camfeel")), (Some("modern"), Some("static")));
        assert_eq!(o.camera_byte(), 1);
        choose(&mut o, cam, 0);
        assert_eq!(o.camera_byte(), 0);
        let lang = ROWS.iter().find(|r| r.key == "lang").unwrap();
        assert_eq!(current_index(&o, lang), None);
        choose(&mut o, lang, 2);
        assert_eq!(o.get("lang"), Some("de"));
        set_percent(&mut o, "camspeed", Some(73));
        assert_eq!(percent(&o, "camspeed"), Some(70));
        set_percent(&mut o, "mousespeed", Some(140));
        assert_eq!((percent(&o, "camspeed"), percent(&o, "mousespeed")), (Some(70), Some(140)), "two keys");
        set_percent(&mut o, "camspeed", None);
        assert_eq!(percent(&o, "camspeed"), None);
        assert_eq!(o.get("mousespeed"), Some("140"));
    }
}
